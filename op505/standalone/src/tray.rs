//! タスクトレイ常駐（アイコン・メニュー・Win32メッセージループ）。
//!
//! `tray-icon`クレート（内部でmuda+windows-sysを使う）を使うが、winitは引き込まない。
//! tray-iconは「同スレッドでWin32メッセージループが回っていること」だけを要求するので、
//! 純粋な`GetMessageW`ループをこちらで手書きする（`op505-mme-driver`・`sources::pipe_src`と
//! 同じく、必要な関数だけをkernel32/user32から直接FFI宣言する方針を踏襲）。
//!
//! MIDI入力ポート（midir経由）はここでメニューから動的に切り替えられるよう、
//! [`crate::midi_source::SourceRegistry`]には入れずこのモジュールが所有・管理する
//! （PipeSource等の「常時有効で切り替え不要な供給元」だけがSourceRegistry行き）。

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, TrayIconBuilder, TrayIconEvent};

use std::sync::Arc;

use crate::config;
use crate::editor::EditorHandle;
use crate::log;
use crate::midi_source::MidiSink;
use crate::sources::midir_src;
use crate::tempo_clock::TempoClock;

#[allow(non_snake_case, non_camel_case_types, dead_code)]
mod winapi {
    use std::ffi::c_void;

    pub type HWND = *mut c_void;
    pub type HANDLE = *mut c_void;
    pub type UINT = u32;
    pub type WPARAM = usize;
    pub type LPARAM = isize;
    pub type LRESULT = isize;
    pub type BOOL = i32;
    pub type DWORD = u32;

    #[repr(C)]
    pub struct POINT {
        pub x: i32,
        pub y: i32,
    }

    #[repr(C)]
    pub struct MSG {
        pub hwnd: HWND,
        pub message: UINT,
        pub w_param: WPARAM,
        pub l_param: LPARAM,
        pub time: DWORD,
        pub pt: POINT,
    }

    pub const WM_QUIT: UINT = 0x0012;
    pub const ERROR_ALREADY_EXISTS: i32 = 183;

    extern "system" {
        pub fn GetMessageW(lpMsg: *mut MSG, hWnd: HWND, wMsgFilterMin: UINT, wMsgFilterMax: UINT) -> BOOL;
        pub fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
        pub fn DispatchMessageW(lpMsg: *const MSG) -> LRESULT;
        pub fn PostQuitMessage(nExitCode: i32);
        pub fn CreateMutexW(lpMutexAttributes: *mut c_void, bInitialOwner: BOOL, lpName: *const u16) -> HANDLE;
    }
}

fn wide_null(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// 名前付きMutexで多重起動を検出する。`true`ならこのプロセスが最初の1個
/// （Mutexハンドルは意図的にリークする——プロセス終了時にOSが回収するため、
/// 明示的なCloseHandleは省略している）。
fn acquire_single_instance_lock() -> bool {
    let name = wide_null(r"Local\op505-standalone-single-instance");
    let handle = unsafe { winapi::CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    if handle.is_null() {
        // Mutex取得自体に失敗した場合は多重起動チェックを諦めて起動を続行する
        // （チェック機能が壊れているだけで、本来の機能は動かせる方が実害が少ないため）。
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(winapi::ERROR_ALREADY_EXISTS)
}

/// プレースホルダーの単色アイコン（32x32、不透明）を生成する。
/// 将来、専用デザインのアイコンに差し替える余地を残すため、生成ロジックをここへ孤立させている。
fn build_icon() -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for _ in 0..(SIZE * SIZE) {
        rgba.extend_from_slice(&[0x4a, 0x6c, 0xd6, 0xff]);
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("invalid placeholder icon data")
}

/// 現在のMIDI入力ポート一覧から、ポート選択サブメニューを構築する。
/// 各項目のMenuIdと対応するポート名（「自動」はNone）のペアを返す。
fn build_port_items(current: Option<&str>) -> (Submenu, Vec<(CheckMenuItem, Option<String>)>) {
    let submenu = Submenu::new("MIDI Input Port", true);
    let mut items = Vec::new();

    let auto_item = CheckMenuItem::new("(Auto)", true, current.is_none(), None);
    let _ = submenu.append(&auto_item);
    items.push((auto_item, None));

    for name in midir_src::list_input_ports() {
        let checked = current == Some(name.as_str());
        let item = CheckMenuItem::new(&name, true, checked, None);
        let _ = submenu.append(&item);
        items.push((item, Some(name)));
    }

    (submenu, items)
}

/// 内部レンダリングレート設定（Stage 3）のサブメニューを構築する。`High (48kHz)`＝
/// `internal_rate_div=1`（アップサンプラーを通さない既定）、`Low (24kHz)`＝`2`。
/// 各項目のMenuIdと対応する`internal_rate_div`値のペアを返す。
fn build_performance_items(current_div: u8) -> (Submenu, Vec<(CheckMenuItem, u8)>) {
    let submenu = Submenu::new("Performance (restart required)", true);
    let mut items = Vec::new();

    let high_item = CheckMenuItem::new("High (48kHz)", true, current_div != 2, None);
    let _ = submenu.append(&high_item);
    items.push((high_item, 1u8));

    let low_item = CheckMenuItem::new("Low (24kHz)", true, current_div == 2, None);
    let _ = submenu.append(&low_item);
    items.push((low_item, 2u8));

    (submenu, items)
}

fn open_path_in_explorer(path: &std::path::Path) {
    // ファイルならエクスプローラーで選択表示、ディレクトリならそのまま開く。
    let arg = if path.is_file() { format!("/select,{}", path.display()) } else { path.display().to_string() };
    if let Err(err) = std::process::Command::new("explorer.exe").arg(arg).spawn() {
        log::log(&format!("explorer.exeの起動に失敗しました: {err}"));
    }
}

/// トレイアイコン・メニューを構築し、Exitが選ばれるまでWin32メッセージループを
/// ブロッキングで回す。呼び出し元（`main`）はこの関数から戻った後、`stream`等の
/// ローカル変数のDropに任せて後片付けする。
///
/// `editor`（トレイ起動音色エディタ、Step 1）は「音色エディタ」メニュー項目、または
/// タスクトレイアイコン自体のダブルクリックから[`EditorHandle::show`]される
/// （`TrayIconEvent::DoubleClick`はWindows専用イベントだが、本アプリはWindows専用のため
/// 問題ない）。ループを抜けた直後（＝終了処理に入った後）に[`EditorHandle::shutdown`]を呼び、
/// エディタが開いていれば閉じ終わるまで待ってから戻る
/// （`main`側の`stream`Dropより先にウィンドウを畳んでおくため）。
pub fn run(sink: MidiSink, editor: EditorHandle, tempo: Arc<TempoClock>) {
    if !acquire_single_instance_lock() {
        log::log("既にop505-standaloneが起動中のため終了します。");
        return;
    }

    let cfg = config::load();
    let mut current_port_name = cfg.midi_in_port.clone();
    let mut current_midir = midir_src::connect(sink.clone(), current_port_name.as_deref(), tempo.clone());
    // 自動選択が実際に決まった場合、メニューのチェック初期状態をそれに合わせる
    // （設定未指定＋ポート1個のみで自動接続できたケース）。
    if current_port_name.is_none() {
        if let Some(source) = current_midir.as_ref() {
            current_port_name = Some(source.connected_port_name().to_string());
        }
    }

    let menu = Menu::new();
    let editor_item = MenuItem::new("Tone Editor", true, None);
    let _ = menu.append(&editor_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let (port_submenu, mut port_items) = build_port_items(current_port_name.as_deref());
    let _ = menu.append(&port_submenu);
    let _ = menu.append(&PredefinedMenuItem::separator());
    // 内部レンダリングレートを24kHzへ落としCPU負荷を下げる設定（Stage 3、詳細はplan/
    // spec-sound.md参照）。エンジン/エフェクトの作り直しが必要なため、選択は設定ファイルへの
    // 保存のみ行い、実際の適用は次回起動から。
    let (performance_submenu, mut performance_items) =
        build_performance_items(cfg.internal_rate_div.unwrap_or(1));
    let _ = menu.append(&performance_submenu);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let open_log_item = MenuItem::new("Open Log", true, None);
    let open_config_item = MenuItem::new("Open Config Folder", true, None);
    let _ = menu.append(&open_log_item);
    let _ = menu.append(&open_config_item);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let exit_item = MenuItem::new("Exit", true, None);
    let _ = menu.append(&exit_item);

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("op505-standalone")
        .with_icon(build_icon())
        // 既定(true)だと左クリック単発でもメニューが開き、TrackPopupMenuのモーダルループが
        // ダブルクリック2回目の入力を飲み込んでしまう（WM_LBUTTONDBLCLKが発生しなくなる）。
        // メニューは右クリック（既定通りmenu_on_right_click=true）のみで開く一般的なWindows
        // トレイアイコンの慣習に合わせ、左ボタンをダブルクリック用に空ける。
        .with_menu_on_left_click(false)
        .build()
        .expect("failed to build tray icon");

    let menu_channel = MenuEvent::receiver();
    let tray_channel = TrayIconEvent::receiver();

    loop {
        let mut msg: winapi::MSG = unsafe { std::mem::zeroed() };
        let ret = unsafe { winapi::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if ret == 0 {
            break; // WM_QUIT
        }
        if ret == -1 {
            log::log("GetMessageWがエラーを返したためメッセージループを終了します。");
            break;
        }
        unsafe {
            winapi::TranslateMessage(&msg);
            winapi::DispatchMessageW(&msg);
        }

        while let Ok(event) = menu_channel.try_recv() {
            handle_menu_event(
                event.id,
                &exit_item.id(),
                &editor_item.id(),
                &open_log_item.id(),
                &open_config_item.id(),
                &mut performance_items,
                &mut port_items,
                &sink,
                &mut current_midir,
                &editor,
                &tempo,
            );
        }

        while let Ok(event) = tray_channel.try_recv() {
            // 左ボタンのダブルクリックのみ拾う（右ダブルクリックはコンテキストメニューの
            // 誤操作の延長になりやすいため対象外）。単発クリックでは何もしない
            // （メニューを開くOS標準の挙動のみで足りる）。
            if matches!(event, TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }) {
                editor.show();
            }
        }
    }

    editor.shutdown();
}

#[allow(clippy::too_many_arguments)]
fn handle_menu_event(
    id: MenuId,
    exit_id: &MenuId,
    editor_id: &MenuId,
    open_log_id: &MenuId,
    open_config_id: &MenuId,
    performance_items: &mut [(CheckMenuItem, u8)],
    port_items: &mut [(CheckMenuItem, Option<String>)],
    sink: &MidiSink,
    current_midir: &mut Option<midir_src::MidirSource>,
    editor: &EditorHandle,
    tempo: &Arc<TempoClock>,
) {
    if &id == exit_id {
        unsafe { winapi::PostQuitMessage(0) };
        return;
    }
    if &id == editor_id {
        editor.show();
        return;
    }
    if &id == open_log_id {
        open_path_in_explorer(&log::path());
        return;
    }
    if &id == open_config_id {
        open_path_in_explorer(&config::file_path());
        return;
    }

    if let Some(pos) = performance_items.iter().position(|(item, _)| item.id() == &id) {
        for (item, _) in performance_items.iter() {
            item.set_checked(false);
        }
        performance_items[pos].0.set_checked(true);
        let div = performance_items[pos].1;

        let mut cfg = config::load();
        cfg.internal_rate_div = Some(div);
        config::save(&cfg);
        log::log(&format!(
            "Performance set to {}. Restart op505-standalone to apply.",
            if div == 2 { "Low (24kHz)" } else { "High (48kHz)" }
        ));
        return;
    }

    let Some(pos) = port_items.iter().position(|(item, _)| item.id() == &id) else {
        return;
    };
    for (item, _) in port_items.iter() {
        item.set_checked(false);
    }
    port_items[pos].0.set_checked(true);
    let chosen_name = port_items[pos].1.clone();

    // 現在のmidir接続を切ってから新しい設定で繋ぎ直す（Dropで既存ポートを解放）。
    *current_midir = None;
    *current_midir = midir_src::connect(sink.clone(), chosen_name.as_deref(), tempo.clone());

    // 他フィールド（env_amp_epsilon等）を消さないよう、既存設定を読み込んでから
    // midi_in_portだけ差し替える（フィールド追加のたびに全部埋め直す必要が無いように）。
    let mut cfg = config::load();
    cfg.midi_in_port = chosen_name;
    config::save(&cfg);
}
