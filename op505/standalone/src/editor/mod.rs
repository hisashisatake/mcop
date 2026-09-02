//! トレイ起動のネイティブ音色エディタ（Step 1）。
//!
//! `tray::run`のGetMessageWループがメインスレッドを占有したまま、この機能はエディタ専用の
//! 常駐スレッド1本で提供する。winitの`EventLoop`はプロセスにつき1回しか作れない
//! （`EVENT_LOOP_CREATED`がプロセスグローバル、`winit-0.30.13/src/event_loop.rs`参照）ため、
//! 「トレイメニューから開くたびに新規スレッドを起動してrun_nativeを呼ぶ」素直な実装は
//! 2回目で必ず`RecreationAttempt`エラーになる。eframeはEventLoopをスレッドローカルに
//! 保持して再利用する設計（`eframe-0.34.3/src/native/run.rs`のコメント「We reuse the
//! event-loop so we can support closing and opening an eframe window multiple times」参照）
//! なので、**スレッド自体を常駐させ、`req_rx.recv()`でパークして使い回す**必要がある。
//!
//! DPI awareness（`SetProcessDpiAwarenessContext`相当）はプロセス全体設定で、winitは
//! `EventLoop`生成時に既定で`become_dpi_aware()`を呼ぶ（`PlatformSpecificEventLoopAttributes`
//! の`dpi_aware`既定値がtrue）。トレイ（tray-icon/muda）は既にメインスレッドでDPI-unawareの
//! まま起動しているため、エディタ側が実行中にこれを変更するとトレイメニューの表示倍率が
//! 変わり得る。`with_dpi_aware(false)`でこの呼び出し自体を抑止する（エディタ自身がHiDPIで
//! やや粗く表示される可能性はあるが、実行中に他の窓の見え方を変えてしまうより安全側に倒す）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use winit::platform::windows::EventLoopBuilderExtWindows;

use crate::log;
use crate::midi_source::MidiSink;
use crate::shared::SharedEditState;

mod app;
mod keyboard;
mod panel_params;
mod preset_host;

use app::EditorApp;

/// エディタが開いた状態から自然に閉じる/ホストが閉じるよう要求してから、
/// スレッドが後始末を終えるまで`shutdown()`が待つ上限。
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// トレイメニューから操作する、常駐エディタスレッドへのハンドル。内部は全てArc/Sender
/// （安価にClone可能）なので、パイプ経由のOpenEditorフレーム受信スレッド
/// （`sources::pipe_src`）へもクローンを渡して`show()`を呼べる。
#[derive(Clone)]
pub struct EditorHandle {
    req_tx: Sender<()>,
    open: Arc<AtomicBool>,
    ctx: Arc<Mutex<Option<egui::Context>>>,
}

impl EditorHandle {
    /// エディタ専用の常駐スレッドを1本起動する。スレッドは`req_rx.recv()`でパークし、
    /// [`show`](Self::show)が呼ばれるたびに[`eframe::run_native`]を1回実行して戻ってくる
    /// （ウィンドウが閉じられたら`run_native`が戻り、次の`recv()`へ戻る。プロセス終了まで
    /// スレッド自体は生き続ける——モジュールdoc参照）。
    ///
    /// `midi_sink`はエディタ下部の鍵盤（`keyboard.rs`）が試聴用のNote On/Offを積むための
    /// ハンドル。実際のMIDI入力（`sources::pipe_src`等）と同じ`MidiQueue`を共有する。
    pub fn spawn(shared: Arc<SharedEditState>, midi_sink: MidiSink) -> Self {
        let (req_tx, req_rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();
        let open = Arc::new(AtomicBool::new(false));
        let ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));

        let thread_open = Arc::clone(&open);
        let thread_ctx = Arc::clone(&ctx);
        std::thread::spawn(move || {
            while req_rx.recv().is_ok() {
                run_editor_once(&shared, &midi_sink, &thread_ctx);
                // ウィンドウが閉じた（またはrun_native自体が失敗した）。次回のshow()を
                // 受け付けられるよう、必ずこの順で後始末する（ctx→openの順。逆順だと
                // 「open=falseなのにctxがまだ残っている」窓ができ、その間のfocus()が
                // 既に無効なコンテキストへコマンドを送ってしまう）。
                *thread_ctx.lock().unwrap() = None;
                thread_open.store(false, Ordering::Release);
            }
        });

        Self { req_tx, open, ctx }
    }

    /// エディタを開く。既に開いていればウィンドウにフォーカスを当てるだけ（二重起動しない）。
    pub fn show(&self) {
        if self.open.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            if self.req_tx.send(()).is_err() {
                // エディタスレッドが既に終了している（プロセス終了処理中などで通常は
                // 起こらないが、念のため状態を戻しておく）。
                self.open.store(false, Ordering::Release);
            }
        } else {
            self.focus();
        }
    }

    fn focus(&self) {
        if let Some(ctx) = self.ctx.lock().unwrap().clone() {
            // ViewportCommand::Focusは「最小化中は効果なし」の仕様（egui-0.34.3のdoc参照）。
            // 最小化されている場合に備え、先にMinimized(false)で復元してからFocusする。
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
    }

    /// プロセス終了前に呼ぶ（`tray::run`のメッセージループを抜けた直後、`main`がcpalの
    /// `stream`をDropする前）。エディタが開いていれば閉じるよう要求し、閉じ終わるか
    /// タイムアウトまで待つ。エディタが開いていなければ即座に戻る。
    pub fn shutdown(&self) {
        if !self.open.load(Ordering::Acquire) {
            return;
        }
        if let Some(ctx) = self.ctx.lock().unwrap().clone() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
        } else {
            return;
        }
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while self.open.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.open.load(Ordering::Acquire) {
            log::log("音色エディタの終了待ちがタイムアウトしました。");
        }
    }
}

fn run_editor_once(shared: &Arc<SharedEditState>, midi_sink: &MidiSink, ctx_slot: &Arc<Mutex<Option<egui::Context>>>) {
    let native_options = eframe::NativeOptions {
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_any_thread(true).with_dpi_aware(false);
        })),
        // 幅はop505-uiパネルの最小幅（panel.xmlから算出、ノブ等が折り返さず収まるサイズ）+
        // PRESETSサイドバー(`app::PRESETS_SIDEBAR_WIDTH`と同値)+余白。
        // 高さは4オペレーター分のTimeEgエディタが縦に並ぶため大きめに確保し、収まらない分は
        // ScrollArea（`app.rs`）に任せる。
        viewport: egui::ViewportBuilder::default()
            .with_title("op505 Tone Editor")
            .with_inner_size([op505_ui::PANEL_MIN_WIDTH + 200.0 + 40.0, 720.0]),
        ..Default::default()
    };

    let initial_patch = shared.current_patch();
    let shared_for_app = Arc::clone(shared);
    let midi_sink_for_app = midi_sink.clone();
    let ctx_slot = Arc::clone(ctx_slot);
    let result = eframe::run_native(
        "op505_standalone_editor",
        native_options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            *ctx_slot.lock().unwrap() = Some(cc.egui_ctx.clone());
            // 新規ウィンドウ作成時、Windowsは背景プロセスからの前面化を既定で拒否する
            // （フルスクリーン/最大化以外はwinitが`force_window_active`を自動で呼ばないため、
            // `EditorHandle::show()`から起こされた場合は何もしないと背面に留まる）。
            // `ViewportCommand::Focus`は「可視・非最小化・非フォアグラウンド」の条件を満たすと
            // winit側でAltキー送出によるフォーカス強奪ハックを行う（`focus()`の既存経路と同じ）。
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            Ok(Box::new(EditorApp::new(shared_for_app, midi_sink_for_app, initial_patch)))
        }),
    );
    if let Err(err) = result {
        log::log(&format!("音色エディタの起動に失敗しました: {err}"));
    }
    // ウィンドウがどう閉じられたか（Xボタン/EditorHandle::shutdown/run_native自体の失敗）に
    // 関わらず、必ず編集対象chを「なし」へ戻す。EditorApp::ui()内の差分検知だけに頼ると、
    // Xボタンで閉じられた場合は次のui()呼び出しが来ないため反映されない。
    shared.set_edit_channel(None);
}

/// eguiの既定フォントにはCJKグリフが含まれず日本語ラベルが豆腐(□)化するため、
/// Windows標準の日本語フォント(游ゴシック)をフォールバックとして追加する
/// （`op505/tools/xml-panel-dsl/preview-native`と同じ方針。このマシン専用の常駐アプリの
/// ため、バイナリへのフォント埋め込みはせずシステムフォントを都度読む）。
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\YuGothR.ttc") {
        let mut font_data = egui::FontData::from_owned(bytes);
        font_data.index = 0; // .ttc内の最初のフェイス(Yu Gothic Regular)
        fonts.font_data.insert("jp".to_owned(), std::sync::Arc::new(font_data));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("jp".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}
