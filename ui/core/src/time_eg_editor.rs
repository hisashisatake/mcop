// ---------------------------------------------------------------------------
// TimeEg 1本ぶんの編集UI（OP505のOP1〜4/PITCH FG/CUTOFF FG/GAIN FG各パネル用、Step 8）。
//
// `time_eg_preview`（読み取り専用の折れ線プレビュー）の姉妹モジュールで、実際に値を編集できる
// ようにしたもの。Step 7の判断ゲートで確定した「折れ線ドラッグ編集とノブ編集をタブで切り替える
// ハイブリッド方式」を実装する。
//
// 段×フィールドごとに`IntParamHandle`を196個(28値×7本)構築するのは高コストなので、
// `TimeEgHandle`（EG1本ぶんの値ハンドル、`param_handle.rs`）を起点に、このモジュール内だけで
// 使う使い捨てアダプター（`TimeEgFieldHandle`/`TimeEgBoolFieldHandle`）を都度導出して
// 既存の`knob`/`spin_control`/`bool_checkbox`（`IntParamHandle`/`BoolParamHandle`前提）へ渡す。
// ---------------------------------------------------------------------------

use egui::{Pos2, Rect, Shape, Stroke, Ui, Vec2};
use sound_core::time_eg::{seconds_to_time, time_to_seconds};
use sound_core::{TimeEgParams, MAX_STAGES};

use crate::eg_preview::{tl_to_db, EgAmplitudeMapping, COLOR_BEZEL, COLOR_HELD, COLOR_PANEL, DB_FLOOR};
use crate::knob::{bool_checkbox, knob, spin_control};
use crate::param_handle::{BoolParamHandle, IntParamHandle, TimeEgHandle};
use crate::time_eg_preview::{draw_geometry, time_eg_editor_layout, time_eg_preview, TimeEgGeometry, TIME_MAX_SECONDS, TIME_MIN_SECONDS};

/// ヘッダ行（EG名+GRAPH/KNOBSタブ）の見込み高さ（px）。`time_eg_editor`の固定枠`size`から
/// 差し引いてコンテンツ領域（GRAPH/KNOBS）の高さを導出するための概算値。実際の描画高と
/// px単位で厳密には一致しない（フォントメトリクス依存）が、GRAPH/KNOBS切替・段数変更で
/// 枠の外形自体は変わらないため実用上問題ない（Step 2の固定枠化）。
const HEADER_HEIGHT: f32 = 20.0;
/// STAGES/LOOP/L.START/L.END/RELのspin行（`stage_spin_row`）の見込み高さ（px）。
/// `HEADER_HEIGHT`と同じ扱い。
const SPIN_ROW_HEIGHT: f32 = 35.0;
/// KNOBSモードの小プレビューの幅（px）。高さはコンテンツ領域いっぱいに伸ばす（`draw_knobs_mode`）。
const KNOBS_PREVIEW_WIDTH: f32 = 130.0;
const GRAPH_PAD: f32 = 6.0;

/// ドラッグでlevelを0へスナップする、グラフ下端からの距離（px）。TL<255時、`DB_FLOOR`付近の
/// 逆写像が縮退する（複数のlevelがほぼ同じdBに潰れる）ことの吸収策（Step D）。
const FLOOR_SNAP_PX: f32 = 3.0;
/// ドラッグでtimeを0（瞬時）へスナップする、開始点からの距離（px）。重みの割合ではなく絶対px
/// なのは、`time=1`(0.001秒)〜`time=15`程度の小さいが有効な値は重みが小さく（(t-1)/254、
/// 例:t=15で約0.055）、割合しきい値だと本物の短い時間まで巻き込んで0へ潰してしまうため
/// （実機テストで発覚。「グラフ左端の数pxだけ」の掴みやすさを保証する目的に絞る）。
const ZERO_SNAP_PX: f32 = 2.0;
/// 頂点・ループマーカーのドラッグ／右クリックのヒットテスト半径（px）。`EDITOR_DOT_RADIUS_PX`と
/// 同じ倍率で拡大してあり、見た目の点の大きさとクリック可能範囲がずれないようにしている
/// （実機確認で「点が小さすぎて掴みにくい」と判明し、旧来値(8.0/2.5px)から2.5倍に拡大した。
/// 当初5倍で試したが実機確認で大きすぎると判明し2.5倍へ調整した）。
const HIT_RADIUS_PX: f32 = 20.0;
/// GRAPHモードで描く頂点ドットの半径（px）。読み取り専用プレビューの`DOT_RADIUS`（2.5px、
/// パネルサイズに応じて`ui_scale`倍される）とは独立の固定値。編集操作の対象になるドットは
/// パネルサイズに関わらず十分な大きさで掴めるようにするため、`ui_scale`をかけない。
const EDITOR_DOT_RADIUS_PX: f32 = crate::time_eg_preview::DOT_RADIUS * 2.5;
/// ループ区間マーカー（三角形）を描く、グラフ下端からのオフセット（px）。
const LOOP_MARKER_OFFSET: f32 = 6.0;
/// ループ区間マーカー（三角形）の半径（px）。
const LOOP_MARKER_RADIUS: f32 = 4.0;

/// GRAPH/KNOBSタブの選択状態。パッチデータではないためegui memoryに保持する（`time_eg_editor`参照）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Graph,
    Knobs,
}

/// GRAPHモードでドラッグ中の対象。フレーム跨ぎでegui memoryに保持する
/// （`draw_graph_mode`のdrag_id参照）。`Vertex.point`は`TimeEgGeometry::points`のindex
/// （`stage_of_point[point]`で対応する段が分かる）。`LoopStart`/`LoopEnd`はループ区間マーカー
/// （現在保持区間として描画されている頂点、すなわち`0..=loop_end`の範囲にのみスナップできる。
/// `loop_end`を現在より後ろへ伸ばしたい場合はまだ描画されていない段にスナップ先が無いため、
/// L.ENDのspin_control（`stage_spin_row`）を使う——GRAPH/KNOBS両モードに共通のこの行を
/// 残している理由の一つ）。
#[derive(Clone, Copy)]
enum DragTarget {
    Vertex { point: usize },
    LoopStart,
    LoopEnd,
}

/// 右クリックメニューの対象。`secondary_clicked()`の瞬間に段indexへ解決して保持する
/// （メニュー表示中にポインタが動いてもターゲットがずれないように、`geometry`ではなく
/// 解決済みの段indexそのものを保存する）。
#[derive(Clone, Copy)]
enum CtxTarget {
    /// 頂点上の右クリック: 対象段自体（削除・カーブ切替の対象）。
    Vertex(usize),
    /// セグメント上の右クリック: この段の直後に新しい段を挿入する（挿入位置の基準段）。
    Segment(usize),
}

/// `geometry.points[1..]`から`pointer`に最も近い頂点を探す（`HIT_RADIUS_PX`以内のみ）。
/// `points[0]`（note-on開始点、どの段にも対応しない）は対象外。
fn hit_test_vertex(geometry: &TimeEgGeometry, pointer: Pos2) -> Option<usize> {
    geometry
        .points
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, p)| (i, p.distance(pointer)))
        .filter(|&(_, d)| d <= HIT_RADIUS_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// `pointer.x`を含む線分(`points[i-1]`〜`points[i]`)を探し、その始点の段indexを返す
/// （右クリックメニューの「この後ろに段を挿入」基準段。`points[0]`は便宜上`points[1]`と
/// 同じ段を指すため、最初の線分でも自然に段0を返す）。
fn hit_test_segment(geometry: &TimeEgGeometry, x: f32) -> Option<usize> {
    for i in 1..geometry.points.len() {
        let (a, b) = (geometry.points[i - 1].x, geometry.points[i].x);
        let (lo, hi) = (a.min(b), a.max(b));
        if x >= lo - HIT_RADIUS_PX && x <= hi + HIT_RADIUS_PX {
            return Some(geometry.stage_of_point[i - 1]);
        }
    }
    None
}

/// 保持区間（`points[1..=release_point]`、現在描画されている`0..=loop_end`の段）のうち、
/// `x`に最も近い頂点が指す段indexを返す（ループマーカーのドラッグ先スナップ用）。
fn nearest_held_stage(geometry: &TimeEgGeometry, x: f32) -> usize {
    (1..=geometry.release_point)
        .min_by(|&a, &b| (geometry.points[a].x - x).abs().total_cmp(&(geometry.points[b].x - x).abs()))
        .map(|i| geometry.stage_of_point[i])
        .unwrap_or(0)
}

/// ループ区間マーカー（小さい三角形）を1つ描く。
fn draw_loop_marker(painter: &egui::Painter, center: Pos2) {
    let r = LOOP_MARKER_RADIUS;
    let points = vec![Pos2::new(center.x, center.y - r), Pos2::new(center.x - r, center.y + r), Pos2::new(center.x + r, center.y + r)];
    painter.add(Shape::convex_polygon(points, COLOR_HELD, Stroke::NONE));
}

/// `TimeEgParams`内の1スカラーフィールドを指す種別（`TimeEgFieldHandle`が読み書きする対象）。
/// `stage_count`(1〜8)・`loop_start`/`loop_end`/`release_start`(0〜7)は0〜255統一ルールの例外
/// （連続量ではなく段インデックス。MULの0〜15と同じ性質）。
#[derive(Clone, Copy)]
enum TimeEgField {
    StageCount,
    LoopStart,
    LoopEnd,
    ReleaseStart,
    /// 段i(0-indexed)のtime。`display()`を秒数表示へオーバーライドする。
    StageTime(usize),
    /// 段i(0-indexed)のlevel。
    StageLevel(usize),
}

/// `TimeEgHandle`から`TimeEgField`1つぶんの`IntParamHandle`を導出する使い捨てアダプター
/// （フレーム内でのみ構築し、`knob`/`spin_control`へ`&dyn IntParamHandle`として渡す）。
struct TimeEgFieldHandle<'a> {
    handle: &'a dyn TimeEgHandle,
    field: TimeEgField,
}

impl<'a> TimeEgFieldHandle<'a> {
    fn new(handle: &'a dyn TimeEgHandle, field: TimeEgField) -> Self {
        Self { handle, field }
    }
}

impl IntParamHandle for TimeEgFieldHandle<'_> {
    fn value(&self) -> i32 {
        let p = self.handle.params();
        match self.field {
            TimeEgField::StageCount => p.stage_count as i32,
            TimeEgField::LoopStart => p.loop_start as i32,
            TimeEgField::LoopEnd => p.loop_end as i32,
            TimeEgField::ReleaseStart => p.release_start as i32,
            TimeEgField::StageTime(i) => p.stages[i].time as i32,
            TimeEgField::StageLevel(i) => p.stages[i].level as i32,
        }
    }

    fn min(&self) -> i32 {
        match self.field {
            TimeEgField::StageCount => 1,
            _ => 0,
        }
    }

    fn max(&self) -> i32 {
        match self.field {
            TimeEgField::StageCount => MAX_STAGES as i32,
            TimeEgField::LoopStart | TimeEgField::LoopEnd | TimeEgField::ReleaseStart => (MAX_STAGES - 1) as i32,
            TimeEgField::StageTime(_) | TimeEgField::StageLevel(_) => 255,
        }
    }

    fn default(&self) -> i32 {
        match self.field {
            TimeEgField::StageCount => 1,
            _ => 0,
        }
    }

    fn name(&self) -> String {
        let base = self.handle.name();
        match self.field {
            TimeEgField::StageCount => format!("{base} STAGES"),
            TimeEgField::LoopStart => format!("{base} L.START"),
            TimeEgField::LoopEnd => format!("{base} L.END"),
            TimeEgField::ReleaseStart => format!("{base} REL"),
            TimeEgField::StageTime(i) => format!("{base} stage{} TIME", i + 1),
            TimeEgField::StageLevel(i) => format!("{base} stage{} LEVEL", i + 1),
        }
    }

    fn display(&self) -> String {
        match self.field {
            // 生値(0〜255)ではなく実秒数を表示する（スピン欄の直接入力は生値のみ受け付ける、
            // param_handle.rsのTimeEgHandle docコメント参照）。
            TimeEgField::StageTime(_) => format_time_seconds(self.value().clamp(0, 255) as u8),
            _ => self.value().to_string(),
        }
    }

    fn begin_edit(&self) {
        self.handle.begin_edit();
    }

    fn set(&self, value: i32) {
        let mut p = self.handle.params();
        let clamped = value.clamp(self.min(), self.max()) as u8;
        match self.field {
            TimeEgField::StageCount => p.stage_count = clamped,
            TimeEgField::LoopStart => p.loop_start = clamped,
            TimeEgField::LoopEnd => p.loop_end = clamped,
            TimeEgField::ReleaseStart => p.release_start = clamped,
            TimeEgField::StageTime(i) => p.stages[i].time = clamped,
            TimeEgField::StageLevel(i) => p.stages[i].level = clamped,
        }
        self.handle.set_params(p);
    }

    fn end_edit(&self) {
        self.handle.end_edit();
    }
}

/// `TimeEgParams`内の1真偽フィールドを指す種別。`curve`はu8だが現状0/非0の2値として扱う
/// （将来curveの種類が増えた場合はこのアダプターを廃し`TimeEgField`側へ移す。局所化のためここに明記）。
#[derive(Clone, Copy)]
enum TimeEgBoolField {
    LoopEnabled,
    /// 段i(0-indexed)のcurve。
    StageCurve(usize),
}

struct TimeEgBoolFieldHandle<'a> {
    handle: &'a dyn TimeEgHandle,
    field: TimeEgBoolField,
}

impl<'a> TimeEgBoolFieldHandle<'a> {
    fn new(handle: &'a dyn TimeEgHandle, field: TimeEgBoolField) -> Self {
        Self { handle, field }
    }
}

impl BoolParamHandle for TimeEgBoolFieldHandle<'_> {
    fn value(&self) -> bool {
        let p = self.handle.params();
        match self.field {
            TimeEgBoolField::LoopEnabled => p.loop_enabled != 0,
            TimeEgBoolField::StageCurve(i) => p.stages[i].curve != 0,
        }
    }

    fn begin_edit(&self) {
        self.handle.begin_edit();
    }

    fn set(&self, value: bool) {
        let mut p = self.handle.params();
        match self.field {
            TimeEgBoolField::LoopEnabled => p.loop_enabled = value as u8,
            TimeEgBoolField::StageCurve(i) => p.stages[i].curve = value as u8,
        }
        self.handle.set_params(p);
    }

    fn end_edit(&self) {
        self.handle.end_edit();
    }
}

/// time値(0〜255)を人が読める秒数表示へ変換する（"0ms"/"1.8ms"/"412ms"/"2.41s"）。
/// `time=0`は瞬時を表す特殊値なので秒数計算を経由せず"0ms"を返す。
fn format_time_seconds(time: u8) -> String {
    // 単位は1文字("m"=ミリ秒/"s"=秒)に短縮する。KNOBSモードのspin_control欄が24px幅しかなく
    // "56ms"のような2文字単位だと末尾が切れて読めなくなるため（実機確認で発覚）。
    // 正確な値はknob()のホバーツールチップ(name+display)で確認できる。
    if time == 0 {
        return "0m".to_string();
    }
    let seconds = time_to_seconds(time);
    if seconds < 1.0 {
        format!("{:.0}m", seconds * 1000.0)
    } else {
        format!("{seconds:.1}s")
    }
}

/// GRAPHモードのY座標(px)をlevel(0〜255)へ逆写像する（Step Dのドラッグ編集が使う）。
/// `inner`は`time_eg_editor_layout`に渡したのと同じ描画可能領域。
/// TL<255時、`DB_FLOOR`付近では複数のlevelがほぼ同じdBに潰れて逆写像が縮退するため、
/// 床から`FLOOR_SNAP_PX`以内はlevel=0へスナップし、天井（`tl_db`）を超える位置は255へクランプする
/// （縮退帯の精密編集はKNOBSモードが受け皿になる）。
fn y_to_level(mapping: EgAmplitudeMapping, tl: u8, inner: Rect, y: f32) -> u8 {
    if inner.bottom() - y <= FLOOR_SNAP_PX {
        return 0;
    }
    let db = DB_FLOOR + ((inner.bottom() - y) / inner.height()) * (-DB_FLOOR);
    let tl_db = tl_to_db(tl);
    let contribution = db - tl_db;
    if contribution >= 0.0 {
        return 255;
    }
    let level_linear = match mapping {
        EgAmplitudeMapping::DbLinear => 1.0 - contribution / DB_FLOOR,
        EgAmplitudeMapping::AmplitudeLinear => 10f32.powf(contribution / 20.0),
    };
    (level_linear.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// GRAPHモードの横幅(px)をtime値(0〜255)へ逆写像する（Step Dのドラッグ編集が使う）。
/// `scale`は`time_eg_editor_layout`計算時の`width/drawn_count`（1段あたりのピクセル数）。
/// `time_eg_preview.rs`の`log_width`と同じレンジ（`TIME_MIN_SECONDS`〜`TIME_MAX_SECONDS`）を
/// 使って往復させる（`time_to_seconds`の内部テーブルと式レベルで一致するため誤差1step以内で
/// 往復する）。`ZERO_SNAP_PX`未満の微小幅は`time=0`（瞬時）へスナップする。
fn width_to_time(width_px: f32, scale: f32) -> u8 {
    if width_px <= ZERO_SNAP_PX {
        return 0;
    }
    if scale <= 0.0 {
        return 0;
    }
    let weight = (width_px / scale).max(0.0);
    let lo = TIME_MIN_SECONDS.log10();
    let hi = TIME_MAX_SECONDS.log10();
    let seconds = 10f32.powf(lo + weight.min(1.0) * (hi - lo));
    seconds_to_time(seconds)
}

/// `index`段の直後に複製段を1つ挿入する（右クリックメニュー「この後ろに段を挿入」、Step E）。
/// `stage_count`が`MAX_STAGES`に達している場合は何もしない（呼び出し側がメニュー項目を
/// 無効化する想定だが、純粋関数としても安全側に倒す）。挿入位置より後ろの
/// `loop_start`/`loop_end`/`release_start`は追従して+1する。
fn insert_stage_after(params: &TimeEgParams, index: usize) -> TimeEgParams {
    let mut out = *params;
    let n = (params.stage_count as usize).clamp(1, MAX_STAGES);
    if n >= MAX_STAGES {
        return out;
    }
    let index = index.min(n - 1);
    for i in (index + 1..n).rev() {
        out.stages[i + 1] = out.stages[i];
    }
    out.stages[index + 1] = out.stages[index];
    out.stage_count = (n + 1) as u8;
    let shift = |p: u8| if (p as usize) > index { p + 1 } else { p };
    out.loop_start = shift(params.loop_start);
    out.loop_end = shift(params.loop_end);
    out.release_start = shift(params.release_start);
    out
}

/// `index`段を削除する（右クリックメニュー「この段を削除」、Step E）。`stage_count`が1の場合は
/// 何もしない（最後の1段は消せない）。削除位置より後ろの`loop_start`/`loop_end`/`release_start`は
/// 追従して-1し、削除された段を指していたものは同じ数値のまま新しい範囲へクランプする
/// （エンジンは不整合値でも到達段で静止するだけで安全、sound-core/time_eg.rsの不変条件参照）。
fn remove_stage(params: &TimeEgParams, index: usize) -> TimeEgParams {
    let mut out = *params;
    let n = (params.stage_count as usize).clamp(1, MAX_STAGES);
    if n <= 1 {
        return out;
    }
    let index = index.min(n - 1);
    for i in index..n - 1 {
        out.stages[i] = out.stages[i + 1];
    }
    let new_n = n - 1;
    out.stage_count = new_n as u8;
    let shift = |p: u8| -> u8 {
        let p = p as usize;
        let shifted = if p > index { p - 1 } else { p };
        shifted.min(new_n - 1) as u8
    };
    out.loop_start = shift(params.loop_start);
    out.loop_end = shift(params.loop_end);
    out.release_start = shift(params.release_start);
    out
}

/// STAGES/LOOP/L.START/L.END/RELのspin行（GRAPH/KNOBS両モード共通、旧`time_eg_block`相当）。
fn stage_spin_row(ui: &mut Ui, handle: &dyn TimeEgHandle) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("STAGES").size(8.0));
            spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::StageCount), egui::TextStyle::Small);
        });
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("LOOP").size(8.0));
            bool_checkbox(ui, &TimeEgBoolFieldHandle::new(handle, TimeEgBoolField::LoopEnabled), "");
        });
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("L.START").size(8.0));
            spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::LoopStart), egui::TextStyle::Small);
        });
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("L.END").size(8.0));
            spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::LoopEnd), egui::TextStyle::Small);
        });
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("REL").size(8.0));
            spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::ReleaseStart), egui::TextStyle::Small);
        });
    });
}

/// KNOBSモード: 左に小プレビュー、右に段ぶんのTIME/LEVEL/CURVE列を水平ScrollAreaで収める。
/// `size`はコンテンツ領域全体（`time_eg_editor`がヘッダ・spin行を差し引いた残り）。
/// 段数1〜8でスクロール量が変わるだけで外形（`size`）自体は変わらない（Step 2の固定枠化）。
fn draw_knobs_mode(ui: &mut Ui, size: Vec2, handle: &dyn TimeEgHandle, mapping: EgAmplitudeMapping, tl: u8) {
    let params = handle.params();
    let preview_size = Vec2::new(KNOBS_PREVIEW_WIDTH, size.y);
    let n = (params.stage_count as usize).clamp(1, MAX_STAGES);
    ui.horizontal(|ui| {
        time_eg_preview(ui, preview_size, mapping, tl, params);
        egui::ScrollArea::horizontal()
            .id_salt(("time_eg_editor", handle.name(), "knobs_scroll"))
            .max_height(size.y)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_height(size.y);
                ui.horizontal(|ui| {
                    for i in 0..n {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(format!("S{}", i + 1)).size(8.0));
                            knob(ui, &TimeEgFieldHandle::new(handle, TimeEgField::StageTime(i)), "TIME");
                            knob(ui, &TimeEgFieldHandle::new(handle, TimeEgField::StageLevel(i)), "LEVEL");
                            bool_checkbox(ui, &TimeEgBoolFieldHandle::new(handle, TimeEgBoolField::StageCurve(i)), "CURVE");
                        });
                    }
                });
            });
    });
}

/// GRAPHモード: 折れ線をコンテンツ領域いっぱいに描画し、頂点のドラッグ（time/level編集）・
/// ループ区間マーカーのドラッグ（loop_start/loop_end編集）・右クリックメニュー（段の挿入/削除・
/// カーブ切替）で編集する。`size`はコンテンツ領域全体（`time_eg_editor`がヘッダ・spin行を
/// 差し引いた残り）。面積が増えるぶん頂点・ループマーカーのヒットテストが楽になる。
fn draw_graph_mode(ui: &mut Ui, size: Vec2, handle: &dyn TimeEgHandle, mapping: EgAmplitudeMapping, tl: u8) {
    let base_id = ui.id().with(("time_eg_editor", handle.name()));
    let drag_id = base_id.with("drag");
    let ctx_id = base_id.with("ctx");
    let mut drag: Option<DragTarget> = ui.memory(|m| m.data.get_temp::<Option<DragTarget>>(drag_id)).flatten();

    let params = handle.params();
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, COLOR_BEZEL);
    let inner = rect.shrink(GRAPH_PAD);
    painter.rect_filled(inner, 2.0, COLOR_PANEL);
    let geometry = time_eg_editor_layout(&params, inner, mapping, tl);
    draw_geometry(painter, &params, &geometry, 1.0, EDITOR_DOT_RADIUS_PX);

    let marker_y = inner.bottom() + LOOP_MARKER_OFFSET;
    let loop_markers = (params.loop_enabled != 0)
        .then_some(geometry.loop_span)
        .flatten()
        .map(|(lo, hi)| (Pos2::new(geometry.points[lo].x, marker_y), Pos2::new(geometry.points[hi].x, marker_y)));
    if let Some((start_pos, end_pos)) = loop_markers {
        painter.add(Shape::line(vec![start_pos, end_pos], Stroke::new(1.5, COLOR_HELD)));
        draw_loop_marker(painter, start_pos);
        draw_loop_marker(painter, end_pos);
    }

    if response.drag_started() {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            if let Some((start_pos, end_pos)) = loop_markers {
                if pointer.distance(start_pos) <= HIT_RADIUS_PX {
                    drag = Some(DragTarget::LoopStart);
                } else if pointer.distance(end_pos) <= HIT_RADIUS_PX {
                    drag = Some(DragTarget::LoopEnd);
                }
            }
            if drag.is_none() {
                if let Some(point) = hit_test_vertex(&geometry, pointer) {
                    drag = Some(DragTarget::Vertex { point });
                }
            }
            if drag.is_some() {
                handle.begin_edit();
            }
        }
    }

    match drag {
        Some(DragTarget::Vertex { point }) => {
            if response.dragged() {
                if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
                    let stage = geometry.stage_of_point[point];
                    let prev_x = geometry.points[point - 1].x;
                    let new_time = width_to_time(pointer.x - prev_x, geometry.scale);
                    let new_level = y_to_level(mapping, tl, inner, pointer.y);
                    let mut p = params;
                    p.stages[stage].time = new_time;
                    p.stages[stage].level = new_level;
                    handle.set_params(p);
                    painter.text(
                        inner.right_top(),
                        egui::Align2::RIGHT_TOP,
                        format!("T {}  L {new_level}", format_time_seconds(new_time)),
                        egui::FontId::monospace(9.0),
                        egui::Color32::from_gray(220),
                    );
                }
            }
        }
        Some(DragTarget::LoopStart) => {
            if response.dragged() {
                if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
                    let stage = nearest_held_stage(&geometry, pointer.x);
                    let mut p = params;
                    p.loop_start = (stage as u8).min(p.loop_end);
                    handle.set_params(p);
                }
            }
        }
        Some(DragTarget::LoopEnd) => {
            if response.dragged() {
                if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
                    let stage = nearest_held_stage(&geometry, pointer.x);
                    let mut p = params;
                    p.loop_end = (stage as u8).max(p.loop_start);
                    handle.set_params(p);
                }
            }
        }
        None => {}
    }
    if drag.is_some() && response.drag_stopped() {
        handle.end_edit();
        drag = None;
    }
    ui.memory_mut(|m| m.data.insert_temp(drag_id, drag));

    if response.secondary_clicked() {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            let target = hit_test_vertex(&geometry, pointer)
                .map(|point| CtxTarget::Vertex(geometry.stage_of_point[point]))
                .or_else(|| hit_test_segment(&geometry, pointer.x).map(CtxTarget::Segment));
            if let Some(target) = target {
                ui.memory_mut(|m| m.data.insert_temp(ctx_id, target));
            }
        }
    }
    response.context_menu(|ui| {
        let Some(target) = ui.memory(|m| m.data.get_temp::<CtxTarget>(ctx_id)) else {
            ui.label("(対象なし)");
            return;
        };
        let n = (params.stage_count as usize).clamp(1, MAX_STAGES);
        let curve_stage = match target {
            CtxTarget::Vertex(stage) => Some(stage),
            CtxTarget::Segment(stage) => Some(stage),
        };
        if let Some(stage) = curve_stage {
            let curve_handle = TimeEgBoolFieldHandle::new(handle, TimeEgBoolField::StageCurve(stage));
            let mut curve_on = curve_handle.value();
            if ui.checkbox(&mut curve_on, "カーブ: レイズドコサイン").changed() {
                curve_handle.begin_edit();
                curve_handle.set(curve_on);
                curve_handle.end_edit();
                ui.close();
            }
        }
        match target {
            CtxTarget::Vertex(stage) => {
                if n > 1 && ui.button("この段を削除").clicked() {
                    handle.begin_edit();
                    handle.set_params(remove_stage(&params, stage));
                    handle.end_edit();
                    ui.close();
                }
            }
            CtxTarget::Segment(stage) => {
                if n < MAX_STAGES && ui.button("この後ろに段を挿入").clicked() {
                    handle.begin_edit();
                    handle.set_params(insert_stage_after(&params, stage));
                    handle.end_edit();
                    ui.close();
                }
            }
        }
    });
}

/// TimeEg 1本ぶんのハイブリッドエディタ（GRAPH/KNOBSタブ＋STAGES等のspin行）。
/// `size`は外形の固定枠（Step 2）。GRAPH↔KNOBS切替・段数(1〜8)変更で`size`自体は変わらず、
/// KNOBSモードの段カラムはみ出し分は内部の水平ScrollAreaが吸収する。
/// `mapping`/`tl`は`time_eg_preview`と同じ意味（TLを持たないFGパネルはtl=255で呼ぶ）。
/// `handle.name()`はegui memoryのId salt兼見出しラベルに使うため、呼び出し側で
/// EGごとに一意な名前（"OP1 EG"/"PITCH FG"等）を渡すこと。
pub fn time_eg_editor(ui: &mut Ui, size: Vec2, handle: &dyn TimeEgHandle, mapping: EgAmplitudeMapping, tl: u8) {
    let mode_id = ui.id().with(("time_eg_editor", handle.name(), "mode"));
    let mut mode = ui.memory(|m| m.data.get_temp::<Mode>(mode_id)).unwrap_or(Mode::Graph);

    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(handle.name()).size(9.0));
            ui.selectable_value(&mut mode, Mode::Graph, "GRAPH");
            ui.selectable_value(&mut mode, Mode::Knobs, "KNOBS");
        });

        let content_size = Vec2::new(size.x, (size.y - HEADER_HEIGHT - SPIN_ROW_HEIGHT).max(0.0));
        match mode {
            Mode::Graph => draw_graph_mode(ui, content_size, handle, mapping, tl),
            Mode::Knobs => draw_knobs_mode(ui, content_size, handle, mapping, tl),
        }

        stage_spin_row(ui, handle);
    });

    ui.memory_mut(|m| m.data.insert_temp(mode_id, mode));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stages(entries: &[(u8, u8, u8)]) -> [sound_core::TimeStage; MAX_STAGES] {
        let mut stages = [sound_core::TimeStage::default(); MAX_STAGES];
        for (i, &(time, level, curve)) in entries.iter().enumerate() {
            stages[i] = sound_core::TimeStage { time, level, curve };
        }
        stages
    }

    fn gain_switch_params() -> TimeEgParams {
        TimeEgParams {
            stages: stages(&[(15, 230, 0), (40, 230, 0), (15, 40, 0), (40, 40, 0), (100, 0, 0)]),
            stage_count: 5,
            loop_enabled: 1,
            loop_start: 0,
            loop_end: 3,
            release_start: 4,
        }
    }

    #[test]
    fn format_time_seconds_covers_ms_and_seconds() {
        assert_eq!(format_time_seconds(0), "0m");
        assert!(format_time_seconds(1).ends_with('m'), "time=1(最速)はミリ秒表示のはず");
        assert!(format_time_seconds(255).ends_with('s'), "time=255(30秒)は秒表示のはず");
    }

    #[test]
    fn width_to_time_round_trips_within_one_step() {
        // time_eg_editor_layoutと同じscale概念で往復させる。境界(1,255)は誤差が出やすいので
        // 中間的な値で確認する。
        for t in [15u8, 50, 100, 200] {
            let seconds = time_to_seconds(t);
            let lo = TIME_MIN_SECONDS.log10();
            let hi = TIME_MAX_SECONDS.log10();
            let weight = ((seconds.max(TIME_MIN_SECONDS).log10() - lo) / (hi - lo)).clamp(0.0, 1.0);
            let scale = 100.0;
            let round_tripped = width_to_time(weight * scale, scale);
            assert!(
                (round_tripped as i32 - t as i32).abs() <= 1,
                "time={t}の往復がずれすぎ: round_tripped={round_tripped}"
            );
        }
    }

    #[test]
    fn width_to_time_snaps_small_width_to_zero() {
        assert_eq!(width_to_time(0.0, 100.0), 0);
        assert_eq!(width_to_time(1.0, 100.0), 0, "ZERO_SNAP_WEIGHT未満は瞬時(0)へスナップするはず");
    }

    #[test]
    fn y_to_level_round_trips_and_snaps() {
        // TL<255かつlevelが低いと、db = tl_db + level_contribution_dbがDB_FLOORへクランプされ
        // 複数のlevelが同じdBへ縮退する（設計上の既知の割り切り、y_to_levelのdocコメント参照）。
        // 縮退したケースはround-trip対象から除外する（別テストで0へ丸まることを確認する）。
        let inner = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(200.0, 100.0));
        for level in [40u8, 128, 200] {
            for tl in [255u8, 200] {
                for mapping in [EgAmplitudeMapping::DbLinear, EgAmplitudeMapping::AmplitudeLinear] {
                    let tl_db = tl_to_db(tl);
                    let contribution = crate::eg_preview::level_contribution_db(mapping, level as f32 / 255.0);
                    let db_unclamped = tl_db + contribution;
                    if db_unclamped <= DB_FLOOR + 1.0 {
                        continue; // 縮退帯（別テストでlevel=0に丸まることを確認済み）
                    }
                    let y = inner.bottom() - ((db_unclamped - DB_FLOOR) / -DB_FLOOR) * inner.height();
                    let round_tripped = y_to_level(mapping, tl, inner, y);
                    assert!(
                        (round_tripped as i32 - level as i32).abs() <= 2,
                        "level={level} tl={tl} mapping={mapping:?}の往復がずれすぎ: round_tripped={round_tripped}"
                    );
                }
            }
        }
    }

    #[test]
    fn y_to_level_degenerate_low_tl_low_level_settles_at_zero() {
        // TL=200・level=40(DbLinear)は上のround-tripテストで除外した縮退ケース。
        // dbがDB_FLOORへクランプされグラフ下端に張り付くため、0へ丸まるのが正しい挙動。
        let inner = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(200.0, 100.0));
        let tl_db = tl_to_db(200);
        let contribution = crate::eg_preview::level_contribution_db(EgAmplitudeMapping::DbLinear, 40.0 / 255.0);
        let db = (tl_db + contribution).max(DB_FLOOR);
        assert_eq!(db, DB_FLOOR, "このケースはDB_FLOORへクランプされる前提のテスト");
        let y = inner.bottom() - ((db - DB_FLOOR) / -DB_FLOOR) * inner.height();
        assert_eq!(y_to_level(EgAmplitudeMapping::DbLinear, 200, inner, y), 0);
    }

    #[test]
    fn y_to_level_floor_snap_and_ceiling_clamp() {
        let inner = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(200.0, 100.0));
        assert_eq!(y_to_level(EgAmplitudeMapping::DbLinear, 255, inner, inner.bottom()), 0, "床は0へスナップ");
        assert_eq!(
            y_to_level(EgAmplitudeMapping::DbLinear, 128, inner, inner.top() - 50.0),
            255,
            "TL天井を超える位置は255へクランプ"
        );
    }

    #[test]
    fn insert_stage_after_shifts_loop_markers() {
        let p = gain_switch_params();
        let out = insert_stage_after(&p, 1);
        assert_eq!(out.stage_count, 6);
        assert_eq!(out.stages[2].time, out.stages[1].time, "index+1へindexの複製を挿入するはず");
        // 元のloop_end=3, release_start=4はindex(1)より後ろなので+1される。loop_start=0は変わらない。
        assert_eq!(out.loop_start, 0);
        assert_eq!(out.loop_end, 4);
        assert_eq!(out.release_start, 5);
    }

    #[test]
    fn insert_stage_after_is_noop_at_max_stages() {
        let mut p = gain_switch_params();
        p.stage_count = MAX_STAGES as u8;
        let out = insert_stage_after(&p, 0);
        assert_eq!(out.stage_count, MAX_STAGES as u8, "8段のときは挿入しないはず");
    }

    #[test]
    fn remove_stage_shifts_loop_markers() {
        let p = gain_switch_params();
        let out = remove_stage(&p, 1);
        assert_eq!(out.stage_count, 4);
        // 元のloop_end=3, release_start=4はindex(1)より後ろなので-1される。loop_start=0は変わらない。
        assert_eq!(out.loop_start, 0);
        assert_eq!(out.loop_end, 2);
        assert_eq!(out.release_start, 3);
    }

    #[test]
    fn remove_stage_is_noop_at_single_stage() {
        let mut p = gain_switch_params();
        p.stage_count = 1;
        let out = remove_stage(&p, 0);
        assert_eq!(out.stage_count, 1, "1段のときは削除しないはず");
    }
}
