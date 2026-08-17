// ---------------------------------------------------------------------------
// TimeEg 1本ぶんの編集UI（OP505のOP1〜4/PITCH FG/CUTOFF FG/GAIN FG各パネル用、Step 8）。
//
// `time_eg_preview`（読み取り専用の折れ線プレビュー）の姉妹モジュールで、実際に値を編集できる
// ようにしたもの。Step 7の判断ゲートで確定した「折れ線ドラッグ編集と数値編集をタブで切り替える
// ハイブリッド方式」を実装する（数値編集側は当初KNOBS=ノブ主体だったが、STAGES=8だと横に
// 並びきらずスクロール必須になる問題が実機確認で判明し、VALUE=spin_control主体へ変更した）。
//
// 段×フィールドごとに`IntParamHandle`を196個(28値×7本)構築するのは高コストなので、
// `TimeEgHandle`（EG1本ぶんの値ハンドル、`param_handle.rs`）を起点に、このモジュール内だけで
// 使う使い捨てアダプター（`TimeEgFieldHandle`/`TimeEgBoolFieldHandle`）を都度導出して
// 既存の`spin_control`/`bool_checkbox`（`IntParamHandle`/`BoolParamHandle`前提）へ渡す。
// ---------------------------------------------------------------------------

use egui::{Pos2, Rect, Shape, Stroke, Ui, Vec2};
use sound_core::time_eg::{seconds_to_time, time_to_seconds};
use sound_core::{TimeEgParams, MAX_STAGES};

use crate::eg_preview::{tl_to_db, EgAmplitudeMapping, COLOR_BEZEL, COLOR_HELD, COLOR_PANEL, COLOR_RELEASE};
use crate::knob::{bool_checkbox, spin_control};
use crate::param_handle::{BoolParamHandle, IntParamHandle, TimeEgHandle};
use crate::time_eg_preview::{draw_geometry, time_eg_editor_layout, TimeEgGeometry, TIME_MAX_SECONDS, TIME_MIN_SECONDS};

/// ヘッダ行（EG名+GRAPH/VALUEタブ）の見込み高さ（px）。`time_eg_editor`の固定枠`size`から
/// 差し引いてコンテンツ領域（GRAPH/VALUE）の高さを導出するための概算値。実際の描画高と
/// px単位で厳密には一致しない（フォントメトリクス依存）が、GRAPH/VALUE切替・段数変更で
/// 枠の外形自体は変わらないため実用上問題ない（Step 2の固定枠化）。
const HEADER_HEIGHT: f32 = 20.0;
/// STAGES/LOOP/L.START/L.END/RELのspin行（`stage_spin_row`）の見込み高さ（px）。
/// `HEADER_HEIGHT`と同じ扱い。
const SPIN_ROW_HEIGHT: f32 = 35.0;
const GRAPH_PAD: f32 = 6.0;
/// VALUEモードのTIME欄の数値欄幅（px）。ミリ秒表示は最大`300000`(6桁)になるため、
/// 他の数値欄(24px、最大3桁の0〜255向け)より広く取る。見た目は実機確認で微調整する。
const TIME_MS_SPIN_WIDTH: f32 = 56.0;

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
/// ループ区間の背景へ敷く帯の色。保持色を薄く重ねて「どこが繰り返すか」を示す
/// （折れ線・頂点より背面）。淡すぎると見えず濃すぎると折れ線が読みにくいので、
/// 実機確認で詰める前提の暫定値。
const COLOR_LOOP_BAND: egui::Color32 = egui::Color32::from_rgba_premultiplied(20, 46, 22, 0);
/// ループ区間マーカー（三角形）を描く、グラフ下端からのオフセット（px）。
const LOOP_MARKER_OFFSET: f32 = 6.0;
/// ループ区間マーカー（三角形）の半径（px）。
const LOOP_MARKER_RADIUS: f32 = 4.0;

/// EG種別ごとの編集制約。`panel.xml`の`<time-eg-editor min-stages=".." terminal-level="..">`で宣言し、
/// `ui-codegen`が生成コードへ埋め込む。既定（`Default`）はOP1〜4 EG/Pitch FG/Cutoff FG用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeEgProfile {
    /// STAGESの下限。OP EG/Pitch FG/Cutoff FGは2で、リリース段を必ず1本持たせる
    /// （キーオフで必ずレベル0へ着地させるため）。Gain FGだけ1を許す。
    pub min_stages: u8,
    /// trueなら最終段のlevelを0に固定する（GRAPHの縦ドラッグ禁止・VALUEのLV欄をグレーアウト）。
    ///
    /// OP EGで必須なのは、ボイス解放条件が「全4オペレーターが`is_idle()`」（op505-coreのVoice）で
    /// あるため。必ず0へ到達してからIdleになれば、ボイスリークもキーオフ時のクリックも起きない。
    /// Pitch/Cutoff FGはlevel 0＝変調量ゼロ＝ニュートラルなので同じ扱いで自然な意味になる。
    /// Gain FGはボイス解放に関与せず、level 0が「無音」を意味してしまう（透過既定は255で
    /// ゲートを閉じない）ため`false`。
    pub terminal_level_zero: bool,
}

impl Default for TimeEgProfile {
    fn default() -> Self {
        Self { min_stages: 2, terminal_level_zero: true }
    }
}

impl TimeEgProfile {
    /// Gain FG用。STAGE=1（リリース区間が空＝ゲートを一切閉じない透過既定）を許し、
    /// 最終段のlevelも自由にする。
    pub const GAIN_FG: Self = Self { min_stages: 1, terminal_level_zero: false };
}

/// EG種別ごとの不変条件を満たすようパラメーターを整える。編集操作（段の挿入/削除・STAGES変更・
/// リリース点ドラッグ等）の直後に必ず通す。
///
/// ここで直すのは「今の操作の結果として範囲外になった値」だけに限る。ユーザーが触っていない値を
/// 裏で書き換えるとUIが信用できなくなる（過去にRELの自動補正で実際に問題になった）ため、
/// 例えば`loop_enabled`のフラグ自体は消さない——ループ区間が成立しないときは
/// エンジン側・プレビュー側とも「ループなし」として扱うので、リリース点を右へ戻せば設定も戻る。
fn normalize(mut params: TimeEgParams, profile: TimeEgProfile) -> TimeEgParams {
    let min_stages = profile.min_stages.clamp(1, MAX_STAGES as u8);
    params.stage_count = params.stage_count.clamp(min_stages, MAX_STAGES as u8);
    let n = params.stage_count as usize;

    // リリース点の上限。リリース段を必ず1本残すEG(min_stages>=2)はn-2まで、
    // 空リリース（＝リリース無し）を許すGain FGはn-1まで。
    let max_release_point = if profile.min_stages >= 2 { n.saturating_sub(2) } else { n - 1 };
    params.release_point = params.release_point.min(max_release_point as u8);

    // ループ区間は`loop_start..=release_point`。1段ループ（`loop_start == release_point`）も
    // 有効で、その段の入口レベルへ跳ね戻すのでノコギリ波になる（sound-core/time_eg.rsのadvance参照）。
    params.loop_start = params.loop_start.min(params.release_point);

    if profile.terminal_level_zero {
        params.stages[n - 1].level = 0;
    }
    params
}

/// GRAPH/VALUEタブの選択状態。パッチデータではないためegui memoryに保持する（`time_eg_editor`参照）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Graph,
    Value,
}

/// GRAPHモードでドラッグ中の対象。フレーム跨ぎでegui memoryに保持する
/// （`draw_graph_mode`のdrag_id参照）。`Vertex.point`は`TimeEgGeometry::points`のindex
/// （`stage_of_point[point]`で対応する段が分かる）。
///
/// ドラッグできるのは頂点だけ。ループ開始・リリース点の三角マーカーはグラフ下端に描くため、
/// level=0の頂点とヒットテスト範囲が重なって丸を掴めなくなる問題があり、表示専用にした
/// （実機確認で判明。編集はspin行のL.START/RELで行う）。
#[derive(Clone, Copy)]
enum DragTarget {
    Vertex { point: usize },
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
/// `points[0]`（note-on開始点、どの段にも対応しない）と`sustain_terminal_point`
/// （STAGE=1で保持側と同じ段を指す目印。ドラッグしても`points[1]`と同じ値しか動かせないため
/// 別に掴ませる意味がない）は対象外。
fn hit_test_vertex(geometry: &TimeEgGeometry, pointer: Pos2) -> Option<usize> {
    geometry
        .points
        .iter()
        .enumerate()
        .skip(1)
        .filter(|&(i, _)| Some(i) != geometry.sustain_terminal_point)
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

/// 区間マーカー（小さい三角形）を1つ描く。ループ開始は保持色、リリース点はリリース色で描き分ける。
/// 表示専用（ドラッグ不可、`draw_graph_mode`のdocコメント参照）。
fn draw_loop_marker(painter: &egui::Painter, center: Pos2, color: egui::Color32) {
    let r = LOOP_MARKER_RADIUS;
    let points = vec![Pos2::new(center.x, center.y - r), Pos2::new(center.x - r, center.y + r), Pos2::new(center.x + r, center.y + r)];
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// `TimeEgParams`内の1スカラーフィールドを指す種別（`TimeEgFieldHandle`が読み書きする対象）。
/// `stage_count`(1〜8)・`loop_start`/`release_point`は0〜255統一ルールの例外
/// （連続量ではなく段インデックス。MULの0〜15と同じ性質）。
#[derive(Clone, Copy)]
enum TimeEgField {
    StageCount,
    /// ループ開始段。0始まりの段indexをそのまま表示する（グラフ上のマーカー位置と対応）。
    LoopStart,
    /// 保持区間とリリース区間の境界。`TimeEgParams::release_point`は0始まりの段indexだが、
    /// **UIでは1始まりのクリック点番号として表示する**（グラフ上でユーザーが数える点の番号と
    /// 一致させるため。`stage{i+1}`表示と同じ既存慣習）。
    ReleasePoint,
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
    profile: TimeEgProfile,
}

impl<'a> TimeEgFieldHandle<'a> {
    fn new(handle: &'a dyn TimeEgHandle, field: TimeEgField, profile: TimeEgProfile) -> Self {
        Self { handle, field, profile }
    }

    /// 現在の段数（プロファイルの下限を効かせたもの）。
    fn stage_count(&self) -> usize {
        self.handle.params().stage_count.clamp(self.profile.min_stages.max(1), MAX_STAGES as u8) as usize
    }

    /// `release_point`(0始まり段index)の上限。リリース段を必ず1本残すEGはn-2、
    /// 空リリースを許すGain FGはn-1（`normalize`と同じ規則）。
    fn max_release_point(&self) -> usize {
        let n = self.stage_count();
        if self.profile.min_stages >= 2 {
            n.saturating_sub(2)
        } else {
            n - 1
        }
    }
}

impl IntParamHandle for TimeEgFieldHandle<'_> {
    fn value(&self) -> i32 {
        let p = self.handle.params();
        match self.field {
            // 生値0は「1として扱う」特殊値（`sound_core::time_eg::clamp_stage_count`と同じ床）。
            // ここでmax(1)しないとSTAGES表示が0のまま、実際には1段ぶんドラッグ可能な点が
            // 存在するという表示と実体の食い違いが起きる（実機確認で発覚）。
            TimeEgField::StageCount => p.stage_count.max(1) as i32,
            TimeEgField::LoopStart => p.loop_start as i32,
            // 0始まりの内部値を1始まりのクリック点番号へ変換して見せる。
            TimeEgField::ReleasePoint => p.release_point as i32 + 1,
            TimeEgField::StageTime(i) => p.stages[i].time as i32,
            TimeEgField::StageLevel(i) => p.stages[i].level as i32,
        }
    }

    fn min(&self) -> i32 {
        match self.field {
            TimeEgField::StageCount => self.profile.min_stages.max(1) as i32,
            // クリック点(1)より手前に境界は置けない。
            TimeEgField::ReleasePoint => 1,
            _ => 0,
        }
    }

    fn max(&self) -> i32 {
        match self.field {
            TimeEgField::StageCount => MAX_STAGES as i32,
            // ループ区間は`loop_start..=release_point`。リリース点と同じ段まで選べる
            // （1段ループ＝ノコギリ波）。
            TimeEgField::LoopStart => self.handle.params().release_point as i32,
            TimeEgField::ReleasePoint => self.max_release_point() as i32 + 1,
            TimeEgField::StageTime(_) | TimeEgField::StageLevel(_) => 255,
        }
    }

    fn default(&self) -> i32 {
        match self.field {
            TimeEgField::StageCount => self.profile.min_stages.max(1) as i32,
            TimeEgField::ReleasePoint => 1,
            _ => 0,
        }
    }

    fn name(&self) -> String {
        let base = self.handle.name();
        match self.field {
            TimeEgField::StageCount => format!("{base} STAGES"),
            TimeEgField::LoopStart => format!("{base} L.START"),
            TimeEgField::ReleasePoint => format!("{base} REL"),
            TimeEgField::StageTime(i) => format!("{base} stage{} TIME", i + 1),
            TimeEgField::StageLevel(i) => format!("{base} stage{} LEVEL", i + 1),
        }
    }

    fn display(&self) -> String {
        match self.field {
            // 生値(0〜255)ではなく実ミリ秒を表示する（スピン欄の直接入力は生値のみ受け付ける、
            // param_handle.rsのTimeEgHandle docコメント参照）。単位記号は付けない
            // （VALUEモードの列見出し「TIME(m)」側でまとめて示す、`format_time_ms_plain`参照）。
            TimeEgField::StageTime(_) => format_time_ms_plain(self.value().clamp(0, 255) as u8),
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
            TimeEgField::StageCount => {
                let old = p.stage_count.clamp(1, MAX_STAGES as u8);
                // 新しく有効になる段は前段(old-1)を複製する（`insert_stage_after`と同じ方針）。
                // 単純にTimeStage::defaultのまま(time=0)にすると、time=0は「瞬時」を表す特殊値
                // のためGRAPHビュー上で直前の頂点と同じx座標に重なって描画され、増やしたはずの
                // クリック点が見えなくなる（実機確認で発覚）。
                let source = (old - 1) as usize;
                for i in (old as usize)..(clamped as usize) {
                    p.stages[i] = p.stages[source];
                }
                p.stage_count = clamped;
            }
            TimeEgField::LoopStart => p.loop_start = clamped,
            // 1始まりのクリック点番号を0始まりの段indexへ戻す。
            TimeEgField::ReleasePoint => p.release_point = clamped.saturating_sub(1),
            TimeEgField::StageTime(i) => p.stages[i].time = clamped,
            TimeEgField::StageLevel(i) => p.stages[i].level = clamped,
        }
        self.handle.set_params(normalize(p, self.profile));
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
    // 単位は1文字("m"=ミリ秒/"s"=秒)に短縮する。VALUEモードのspin_control欄が24px幅しかなく
    // "56ms"のような2文字単位だと末尾が切れて読めなくなるため（実機確認で発覚）。
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

/// time値(0〜255)をミリ秒の素の数値（単位記号なし）へ変換する。VALUEモードのspin_control専用
/// （列見出し「TIME(m)」で単位をまとめて示すため、値欄ごとに"m"/"s"を付けない。GRAPHモードの
/// ドラッグ中フローティング表示は`format_time_seconds`のまま——見出しが無い文脈では単位が要る）。
fn format_time_ms_plain(time: u8) -> String {
    if time == 0 {
        return "0".to_string();
    }
    format!("{:.0}", time_to_seconds(time) * 1000.0)
}

/// GRAPHモードのY座標(px)をlevel(0〜255)へ逆写像する（Step Dのドラッグ編集が使う）。
/// `inner`は`time_eg_editor_layout`に渡したのと同じ描画可能領域。軸の床は`axis_floor_db`で
/// パネル種別ごとに決まる（`layout_impl`の描画側と必ず同じ床を使う。ずれると見た目の点位置と
/// ドラッグ後の実際の値が食い違う）。床付近では複数のlevelがほぼ同じdBに潰れて逆写像が縮退するため、
/// 床から`FLOOR_SNAP_PX`以内はlevel=0へスナップし、天井（`tl_db`）を超える位置は255へクランプする
/// （縮退帯の精密編集はVALUEモードが受け皿になる）。
fn y_to_level(mapping: EgAmplitudeMapping, tl: u8, inner: Rect, y: f32) -> u8 {
    if inner.bottom() - y <= FLOOR_SNAP_PX {
        return 0;
    }
    let floor = crate::time_eg_preview::axis_floor_db(mapping);
    let db = floor + ((inner.bottom() - y) / inner.height()) * (-floor);
    let tl_db = tl_to_db(tl);
    let contribution = db - tl_db;
    if contribution >= 0.0 {
        return 255;
    }
    let level_linear = match mapping {
        EgAmplitudeMapping::DbLinear | EgAmplitudeMapping::RawLinear => 1.0 - contribution / floor,
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
/// `loop_start`/`release_point`は追従して+1し、最後に`normalize`で不変条件を整える。
fn insert_stage_after(params: &TimeEgParams, index: usize, profile: TimeEgProfile) -> TimeEgParams {
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
    out.release_point = shift(params.release_point);
    normalize(out, profile)
}

/// `index`段を削除する（右クリックメニュー「この段を削除」、Step E）。`stage_count`がプロファイルの
/// 下限にある場合は何もしない。削除位置より後ろの`loop_start`/`release_point`は追従して-1し、
/// 削除された段を指していたものは同じ数値のまま新しい範囲へクランプする。
fn remove_stage(params: &TimeEgParams, index: usize, profile: TimeEgProfile) -> TimeEgParams {
    let mut out = *params;
    let n = (params.stage_count as usize).clamp(1, MAX_STAGES);
    if n <= profile.min_stages.max(1) as usize {
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
    out.release_point = shift(params.release_point);
    normalize(out, profile)
}

/// spin行の各欄を有効にするか（`spin_row_enabled`の戻り値）。
struct SpinRowEnabled {
    rel: bool,
    loop_toggle: bool,
    loop_start: bool,
}

/// spin行のグレーアウト判定。**「選択肢が2つ以上あるときだけ有効」で揃える**のが原則
/// （1つしか選べない欄を押せる見た目で残すと「数字は見えるのに動かせない死んだUI」になり、
/// 過去に却下された形になる）。UIから切り離してテストできるよう純粋関数にしてある。
fn spin_row_enabled(params: &TimeEgParams, profile: TimeEgProfile) -> SpinRowEnabled {
    let n = params.stage_count.clamp(profile.min_stages.max(1), MAX_STAGES as u8) as usize;
    let max_release_point = if profile.min_stages >= 2 { n.saturating_sub(2) } else { n - 1 };
    SpinRowEnabled {
        // リリース点はクリック点(1)〜(max_release_point+1)から選ぶ（STAGE=2は(1)固定）。
        rel: max_release_point > 0,
        // LOOPはリリース点が動かせる段数（＝STAGE>=3）で使えるようにする。1段ループも有効なので
        // release_point=0でも「段0を繰り返す」形が成立する（入口レベル0へ跳ね戻すノコギリ）。
        loop_toggle: max_release_point > 0,
        // L.STARTは0〜release_pointから選ぶ。release_point=0だと0しか選べないため無効に保つ
        // （選択肢が1つの欄を押せる見た目で残さない原則）。
        loop_start: params.release_point >= 1,
    }
}

/// STAGES/LOOP/L.START/RELのspin行（GRAPH/VALUE両モード共通、旧`time_eg_block`相当）。
/// 旧L.END欄は廃止した（ループ終端＝リリース点で同じ境界なので、REL1本で足りる）。
///
/// 段数によって固定になる欄は、消さずに**グレーアウトして残す**。欄の位置が段数で動くと
/// 目線が迷うため（段数を変えながら詰める操作が多い）。
fn stage_spin_row(ui: &mut Ui, handle: &dyn TimeEgHandle, profile: TimeEgProfile) {
    let params = handle.params();
    let SpinRowEnabled { rel: rel_enabled, loop_toggle: loop_enabled, loop_start: loop_start_enabled } =
        spin_row_enabled(&params, profile);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("STAGES").size(8.0));
            spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::StageCount, profile), egui::TextStyle::Small, 24.0);
        });
        ui.add_enabled_ui(loop_enabled, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("LOOP").size(8.0));
                bool_checkbox(ui, &TimeEgBoolFieldHandle::new(handle, TimeEgBoolField::LoopEnabled), "");
            });
        });
        ui.add_enabled_ui(loop_start_enabled, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("L.START").size(8.0));
                spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::LoopStart, profile), egui::TextStyle::Small, 24.0);
            });
        });
        ui.add_enabled_ui(rel_enabled, |ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("REL").size(8.0));
                spin_control(ui, &TimeEgFieldHandle::new(handle, TimeEgField::ReleasePoint, profile), egui::TextStyle::Small, 24.0);
            });
        });
    });
}

/// VALUEモード: 段ぶんのTIME/LEVEL/CURVE行を垂直ScrollAreaで収める（1段=1横並び行、
/// 段が増えると下に伸びる。GRAPHタブに切り替えれば同じ形をいつでも見られるため、
/// 小プレビューは重複表示として置かない——実機確認で「要らない」と判明）。
/// TIME/LEVELは`knob`(62×66のダイヤル込みセル)ではなく`spin_control`(数値欄のみ)を使う。
/// ダイヤルは場所を取りすぎてSTAGES=8だと並びきらずスクロール必須になっていたため、
/// 数値欄だけに絞って1段あたりの専有面積を削る（実機確認で「数値欄のみでいい」と判明）。
/// 段番号("S1"等)は付けない（縦に並ぶ行の並び順が段番号を兼ねる——実機確認で「要らない」と判明）。
/// 1段は1行構成: TIME(m)+LV+CVチェックを横一列に並べる（実機確認でこの割り付けを指定）。
/// 行ごとに独立した`ui.horizontal`だと、TIME欄の桁数（"0"〜"300000"）で実際の描画幅が
/// 行ごとに微妙に変わり、LV欄以降の開始位置が段によってずれて見えた（実機確認で発覚）ため、
/// `egui::Grid`で列位置を強制的に揃える。
/// `size`はコンテンツ領域全体（`time_eg_editor`がヘッダ・spin行を差し引いた残り）。
/// 段数1〜8でスクロール量が変わるだけで外形（`size`）自体は変わらない（Step 2の固定枠化）。
fn draw_value_mode(ui: &mut Ui, size: Vec2, handle: &dyn TimeEgHandle, profile: TimeEgProfile) {
    let params = handle.params();
    let n = (params.stage_count as usize).clamp(profile.min_stages.max(1) as usize, MAX_STAGES);
    egui::ScrollArea::vertical()
        .id_salt(("time_eg_editor", handle.name(), "value_scroll"))
        .max_height(size.y)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(size.x);
            egui::Grid::new(("time_eg_editor", handle.name(), "value_grid"))
                .num_columns(5)
                .spacing([4.0, 2.0])
                // 既定の最小列幅（ボタン程度）だと"LV"のような短いラベル列に余白が残るため0にする
                // （実機確認で「ラベルと数値欄の間が離れている」と判明）。
                .min_col_width(0.0)
                .show(ui, |ui| {
                    for i in 0..n {
                        ui.label(egui::RichText::new("TIME(m)").size(8.0));
                        spin_control(
                            ui,
                            &TimeEgFieldHandle::new(handle, TimeEgField::StageTime(i), profile),
                            egui::TextStyle::Small,
                            TIME_MS_SPIN_WIDTH,
                        );
                        ui.label(egui::RichText::new("LV").size(8.0));
                        // 最終段のlevelは0固定のEG（OP EG/Pitch FG/Cutoff FG）がある。
                        // 動かせないことが分かるようグレーアウトする（`TimeEgProfile`参照）。
                        let level_editable = !(profile.terminal_level_zero && i == n - 1);
                        ui.add_enabled_ui(level_editable, |ui| {
                            spin_control(
                                ui,
                                &TimeEgFieldHandle::new(handle, TimeEgField::StageLevel(i), profile),
                                egui::TextStyle::Small,
                                24.0,
                            );
                        });
                        bool_checkbox(ui, &TimeEgBoolFieldHandle::new(handle, TimeEgBoolField::StageCurve(i)), "CV");
                        ui.end_row();
                    }
                });
        });
}

/// GRAPHモード: 折れ線をコンテンツ領域いっぱいに描画し、頂点のドラッグ（time/level編集）と
/// 右クリックメニュー（段の挿入/削除・カーブ切替）で編集する。`size`はコンテンツ領域全体
/// （`time_eg_editor`がヘッダ・spin行を差し引いた残り）。面積が増えるぶん頂点のヒットテストが楽になる。
///
/// ループ開始・リリース点の三角マーカーは**表示専用**（ドラッグ不可）。グラフ下端に置くため、
/// level=0の頂点とヒットテスト範囲が重なって「丸を掴みたいのにマーカーが取られる」状態になり、
/// 操作性を損なっていた（実機確認で判明）。両者の編集はspin行のL.START/RELで行う。
fn draw_graph_mode(ui: &mut Ui, size: Vec2, handle: &dyn TimeEgHandle, mapping: EgAmplitudeMapping, tl: u8, profile: TimeEgProfile) {
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

    let n = (params.stage_count as usize).clamp(profile.min_stages.max(1) as usize, MAX_STAGES);
    let marker_y = inner.bottom() + LOOP_MARKER_OFFSET;
    // ループ区間の背景へ淡い帯を敷いて「どこが繰り返すか」を一目で分かるようにする。
    // 折れ線・頂点より前に描いて背面へ回す。`loop_span`はループが成立するときだけSome。
    let loop_start_pos = geometry.loop_span.map(|(lo, hi)| {
        let band = Rect::from_x_y_ranges(geometry.points[lo].x..=geometry.points[hi].x, inner.y_range());
        painter.rect_filled(band, 0.0, COLOR_LOOP_BAND);
        Pos2::new(geometry.points[lo].x, marker_y)
    });

    draw_geometry(painter, &params, &geometry, 1.0, EDITOR_DOT_RADIUS_PX);

    // 区間マーカーは表示専用（ヒットテストしない）。ループ終端はリリース点と同じ境界なので
    // マーカーはリリース点の1つで足りる。
    let release_pos = Pos2::new(geometry.points[geometry.release_point].x, marker_y);
    if let Some(start_pos) = loop_start_pos {
        painter.add(Shape::line(vec![start_pos, release_pos], Stroke::new(1.5, COLOR_HELD)));
        draw_loop_marker(painter, start_pos, COLOR_HELD);
    }
    draw_loop_marker(painter, release_pos, COLOR_RELEASE);

    if response.drag_started() {
        if let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) {
            if let Some(point) = hit_test_vertex(&geometry, pointer) {
                drag = Some(DragTarget::Vertex { point });
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
                    // 最終段のlevelを0に固定するEG（OP EG/Pitch FG/Cutoff FG）では縦方向のドラッグを
                    // 無視し、横（time）だけ効かせる（`TimeEgProfile::terminal_level_zero`参照）。
                    if !(profile.terminal_level_zero && stage == n - 1) {
                        p.stages[stage].level = new_level;
                    }
                    let shown_level = p.stages[stage].level;
                    handle.set_params(normalize(p, profile));
                    painter.text(
                        inner.right_top(),
                        egui::Align2::RIGHT_TOP,
                        format!("T {}  L {shown_level}", format_time_seconds(new_time)),
                        egui::FontId::monospace(9.0),
                        egui::Color32::from_gray(220),
                    );
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
        let min_stages = profile.min_stages.max(1) as usize;
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
                if n > min_stages && ui.button("この段を削除").clicked() {
                    handle.begin_edit();
                    handle.set_params(remove_stage(&params, stage, profile));
                    handle.end_edit();
                    ui.close();
                }
            }
            CtxTarget::Segment(stage) => {
                if n < MAX_STAGES && ui.button("この後ろに段を挿入").clicked() {
                    handle.begin_edit();
                    handle.set_params(insert_stage_after(&params, stage, profile));
                    handle.end_edit();
                    ui.close();
                }
            }
        }
    });
}

/// TimeEg 1本ぶんのハイブリッドエディタ（GRAPH/VALUEタブ＋STAGES等のspin行）。
/// `size`は外形の固定枠（Step 2）。GRAPH↔VALUE切替・段数(1〜8)変更で`size`自体は変わらず、
/// VALUEモードの段カラムはみ出し分は内部の水平ScrollAreaが吸収する。
/// `mapping`/`tl`は`time_eg_preview`と同じ意味（TLを持たないFGパネルはtl=255で呼ぶ）。
/// `handle.name()`はegui memoryのId salt兼見出しラベルに使うため、呼び出し側で
/// EGごとに一意な名前（"OP1 EG"/"PITCH FG"等）を渡すこと。
/// `profile`はEG種別ごとの編集制約（`TimeEgProfile`参照）。panel.xmlの
/// `min-stages`/`terminal-level`属性からui-codegenが埋め込む。
pub fn time_eg_editor(ui: &mut Ui, size: Vec2, handle: &dyn TimeEgHandle, mapping: EgAmplitudeMapping, tl: u8, profile: TimeEgProfile) {
    let mode_id = ui.id().with(("time_eg_editor", handle.name(), "mode"));
    let mut mode = ui.memory(|m| m.data.get_temp::<Mode>(mode_id)).unwrap_or(Mode::Graph);

    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(handle.name()).size(9.0));
            ui.selectable_value(&mut mode, Mode::Graph, "GRAPH");
            ui.selectable_value(&mut mode, Mode::Value, "VALUE");
        });

        let content_size = Vec2::new(size.x, (size.y - HEADER_HEIGHT - SPIN_ROW_HEIGHT).max(0.0));
        match mode {
            Mode::Graph => draw_graph_mode(ui, content_size, handle, mapping, tl, profile),
            Mode::Value => draw_value_mode(ui, content_size, handle, profile),
        }

        stage_spin_row(ui, handle, profile);
    });

    ui.memory_mut(|m| m.data.insert_temp(mode_id, mode));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// STAGES増加時の複製ロジック検証用モックハンドル（`interpret.rs`のMockTimeEgと同じ設計）。
    struct MockTimeEg {
        value: Cell<TimeEgParams>,
    }

    impl TimeEgHandle for MockTimeEg {
        fn params(&self) -> TimeEgParams {
            self.value.get()
        }
        fn set_params(&self, params: TimeEgParams) {
            self.value.set(params);
        }
        fn name(&self) -> String {
            "TEST EG".to_string()
        }
        fn begin_edit(&self) {}
        fn end_edit(&self) {}
    }

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
            release_point: 3,
         ..Default::default()}
    }

    #[test]
    fn format_time_seconds_covers_ms_and_seconds() {
        assert_eq!(format_time_seconds(0), "0m");
        assert!(format_time_seconds(1).ends_with('m'), "time=1(最速)はミリ秒表示のはず");
        assert!(format_time_seconds(255).ends_with('s'), "time=255(300秒)は秒表示のはず");
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
                    let floor = crate::time_eg_preview::axis_floor_db(mapping);
                    let tl_db = tl_to_db(tl);
                    // 実際に描画へ使うstage_target_dbを直接呼ぶ（floorでクランプ済みの値）。
                    let db = crate::time_eg_preview::stage_target_db(mapping, floor, tl_db, level);
                    if db <= floor + 1.0 {
                        continue; // 縮退帯（別テストでlevel=0に丸まることを確認済み）
                    }
                    let y = inner.bottom() - ((db - floor) / -floor) * inner.height();
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
        let floor = crate::eg_preview::DB_FLOOR;
        let db = (tl_db + contribution).max(floor);
        assert_eq!(db, floor, "このケースはDB_FLOORへクランプされる前提のテスト");
        let y = inner.bottom() - ((db - floor) / -floor) * inner.height();
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
        let out = insert_stage_after(&p, 1, TimeEgProfile::default());
        assert_eq!(out.stage_count, 6);
        assert_eq!(out.stages[2].time, out.stages[1].time, "index+1へindexの複製を挿入するはず");
        // 元のrelease_point=3はindex(1)より後ろなので+1される。loop_start=0は変わらない。
        assert_eq!(out.loop_start, 0);
        assert_eq!(out.release_point, 4);
    }

    #[test]
    fn insert_stage_after_is_noop_at_max_stages() {
        let mut p = gain_switch_params();
        p.stage_count = MAX_STAGES as u8;
        let out = insert_stage_after(&p, 0, TimeEgProfile::default());
        assert_eq!(out.stage_count, MAX_STAGES as u8, "8段のときは挿入しないはず");
    }

    #[test]
    fn remove_stage_shifts_loop_markers() {
        let p = gain_switch_params();
        let out = remove_stage(&p, 1, TimeEgProfile::default());
        assert_eq!(out.stage_count, 4);
        // 元のrelease_point=3はindex(1)より後ろなので-1される。loop_start=0は変わらない。
        assert_eq!(out.loop_start, 0);
        assert_eq!(out.release_point, 2);
    }

    #[test]
    fn remove_stage_is_noop_at_min_stages() {
        let mut p = gain_switch_params();
        p.stage_count = 2;
        let out = remove_stage(&p, 0, TimeEgProfile::default());
        assert_eq!(out.stage_count, 2, "既定プロファイルの下限(2段)では削除しないはず");
    }

    #[test]
    fn gain_fg_profile_can_remove_down_to_one_stage() {
        let mut p = gain_switch_params();
        p.stage_count = 2;
        let out = remove_stage(&p, 1, TimeEgProfile::GAIN_FG);
        assert_eq!(out.stage_count, 1, "Gain FGは1段まで減らせる（透過既定の形）");
    }

    /// リリース点は必ずリリース段を1本残す位置までしか置けない（既定プロファイル）。
    /// これで「保持区間が段リスト全体を占めてリリースが空」になることが起きなくなり、
    /// キーオフで必ずレベル0へ着地する。
    #[test]
    fn normalize_keeps_at_least_one_release_stage() {
        let mut p = gain_switch_params();
        p.release_point = 4; // = stage_count-1。リリース段が無くなる位置
        let out = normalize(p, TimeEgProfile::default());
        assert_eq!(out.release_point, 3, "n-2へクランプされるはず");
        assert_eq!(out.stages[4].level, 0, "最終段のlevelは0固定のはず");
    }

    /// Gain FGはリリース区間が空（＝ゲートを一切閉じない透過既定）になる形を許し、
    /// 最終段のlevelも書き換えない。
    #[test]
    fn normalize_allows_empty_release_for_gain_fg() {
        let mut p = gain_switch_params();
        p.stage_count = 1;
        p.stages[0].level = 255;
        p.release_point = 0;
        let out = normalize(p, TimeEgProfile::GAIN_FG);
        assert_eq!(out.stage_count, 1);
        assert_eq!(out.release_point, 0, "空リリースが許されるはず");
        assert_eq!(out.stages[0].level, 255, "Gain FGの最終段levelは0へ書き換えないはず");
    }

    /// ループ区間は`loop_start..=release_point`なので、loop_startはリリース点と同じ段まで許す
    /// （1段ループ＝入口レベルへ跳ね戻すノコギリ波）。それを超える値だけクランプする。
    #[test]
    fn normalize_clamps_loop_start_to_release_point() {
        let mut p = gain_switch_params();
        p.loop_start = 3;
        p.release_point = 1;
        let out = normalize(p, TimeEgProfile::default());
        assert_eq!(out.loop_start, 1, "loop_startはrelease_pointまで（同値＝1段ループ）");
        assert_eq!(out.loop_enabled, 1, "ユーザーが触っていないLOOPフラグは書き換えないはず");
    }

    /// spin行のグレーアウト判定は「選択肢が2つ以上あるときだけ有効」で揃っているか。
    /// 特にL.STARTは、LOOPが成立していても選択肢が0だけなら無効に保つ
    /// （REL=(2)＝release_point=1のとき。実機確認で発覚した不具合の回帰テスト）。
    #[test]
    fn spin_row_greys_out_fields_with_only_one_choice() {
        let profile = TimeEgProfile::default();
        let build = |stage_count: u8, release_point: u8| TimeEgParams { stage_count, release_point, ..TimeEgParams::default() };

        // STAGE=2: リリース点はクリック点(1)固定、ループ不可。
        let e = spin_row_enabled(&build(2, 0), profile);
        assert!(!e.rel && !e.loop_toggle && !e.loop_start, "STAGE=2は全て無効のはず");

        // STAGE=3 / REL=(1): 段0の1段ループは組めるので、LOOPは有効。
        // ただしL.STARTは0しか選べないため無効に保つ。
        let e = spin_row_enabled(&build(3, 0), profile);
        assert!(e.rel, "STAGE=3ならRELは(1)と(2)の2択");
        assert!(e.loop_toggle, "1段ループが組めるのでLOOPは有効");
        assert!(!e.loop_start, "選択肢が0だけならL.STARTは無効に保つ");

        // STAGE=3 / REL=(2): L.STARTが0（段0〜1の2段ループ）と1（段1の1段ループ＝ノコギリ）の2択。
        let e = spin_row_enabled(&build(3, 1), profile);
        assert!(e.loop_toggle && e.loop_start, "release_point=1でL.STARTが動かせるようになる");

        // STAGE=4 / REL=(3): L.STARTは0〜2の3択。
        let e = spin_row_enabled(&build(4, 2), profile);
        assert!(e.loop_toggle && e.loop_start);
    }

    /// STAGESを増やしてからRELを上げると、LOOPが有効になる条件(`release_point >= 1`)を満たすか。
    /// spin行のグレーアウト条件が実際に解除されるところまで、編集の連鎖を通しで確認する。
    #[test]
    fn raising_stages_then_release_point_unlocks_loop() {
        let profile = TimeEgProfile::default();
        let handle = MockTimeEg { value: Cell::new(TimeEgParams::default()) };
        // 既定は2段・release_point=0 → LOOPは無効（ループ区間が1段になり平坦で無意味なため）。
        assert_eq!(handle.params().stage_count, 2);
        assert_eq!(handle.params().release_point, 0);

        TimeEgFieldHandle::new(&handle, TimeEgField::StageCount, profile).set(4);
        assert_eq!(handle.params().stage_count, 4);
        // STAGESを増やしただけではRELは動かない（ユーザーが触っていない値は書き換えない）。
        assert_eq!(handle.params().release_point, 0, "RELは据え置きのはず");

        let rel = TimeEgFieldHandle::new(&handle, TimeEgField::ReleasePoint, profile);
        assert_eq!(rel.max(), 3, "STAGES=4ならRELはクリック点(3)まで");
        rel.set(3);
        assert_eq!(handle.params().release_point, 2, "クリック点(3)は内部値2");
        assert!(handle.params().release_point >= 1, "ここでLOOPのグレーアウトが解除される");
    }

    /// RELのspin欄は0始まりの内部値ではなく1始まりのクリック点番号を見せる
    /// （グラフ上でユーザーが数える点の番号と一致させるため）。
    #[test]
    fn release_point_field_is_displayed_one_based() {
        let handle = MockTimeEg { value: Cell::new(gain_switch_params()) };
        let field = TimeEgFieldHandle::new(&handle, TimeEgField::ReleasePoint, TimeEgProfile::default());
        assert_eq!(field.value(), 4, "release_point=3はクリック点(4)として表示するはず");
        assert_eq!(field.min(), 1);
        assert_eq!(field.max(), 4, "stage_count=5ならクリック点(4)まで（リリース段を1本残す）");

        field.set(2);
        assert_eq!(handle.params().release_point, 1, "クリック点(2)は内部値1のはず");
    }

    #[test]
    fn stage_count_increase_duplicates_last_stage_instead_of_zero() {
        // STAGESの＋ボタン（TimeEgFieldHandle::set経由）で段数を増やしたとき、新しい段が
        // TimeStage::default()(time=0)のままだと「瞬時」特殊値でGRAPHビュー上の頂点が
        // 直前の頂点と同じx座標に重なってしまう（実機確認で発覚したバグ）。
        // 直前の有効段(old_count-1)を複製すれば、time=0のまま据え置かれる場合でも
        // 直前段自体がtime!=0であればx座標が分離される。
        let handle = MockTimeEg { value: Cell::new(gain_switch_params()) };
        let field = TimeEgFieldHandle::new(&handle, TimeEgField::StageCount, TimeEgProfile::default());
        field.set(6); // 5 -> 6

        let out = handle.params();
        assert_eq!(out.stage_count, 6);
        let last_original = gain_switch_params().stages[4];
        assert_eq!(out.stages[5], last_original, "新しい段は直前の段(index4)を複製するはず");
    }

    #[test]
    fn stage_count_decrease_does_not_touch_stage_data() {
        let handle = MockTimeEg { value: Cell::new(gain_switch_params()) };
        let field = TimeEgFieldHandle::new(&handle, TimeEgField::StageCount, TimeEgProfile::default());
        field.set(3); // 5 -> 3 (減少側は複製ロジックを通らない)

        let out = handle.params();
        assert_eq!(out.stage_count, 3);
        assert_eq!(out.stages[0], gain_switch_params().stages[0]);
    }
}
