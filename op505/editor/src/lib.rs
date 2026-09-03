//! op505-vst / op505-standalone のエディタ重複（PRESETSパネル・パネル組み立て・
//! min/max/default定義）を吸収する共有クレート。
//!
//! ## 共有する理由（fork-on-write原則の例外条件、spec-fm.md 8章⑧）
//! 1. 複製すると正誤の基準が消える：「Saveで何がファイルに書かれるか」「Delete後どれが
//!    選ばれるか」がVSTとstandaloneで食い違ったとき、どちらが正しいかを決める外部基準が無い
//! 2. 両消費者が現役のop505製品（op505/ym38x6のような非対称性が無い）
//! 3. 依存6本（egui/op505-core/op505-ui/ui-core/sound-core/rfd）はすべて両ホストが
//!    既に持っている辺で、新しい依存辺は増えない
//!
//! ## 入れないもの
//! nice-plug / eframe / winit / cpal / midir / serde / Tauri。特に`serde`を入れないことで、
//! VSTの`#[persist = "op505_egs"]`対象である`Op505EgBank`をうっかり移設できない構造にする
//! （プロジェクトファイルにJSONとして焼かれているため移設厳禁）。
//!
//! 詳細設計・全11コミットの移行手順は`.claude/plans/fancy-wishing-toast.md`参照。

pub mod layout;
pub mod panel_source;
pub mod param_spec;
pub mod patch_source;
pub mod preset_panel;
pub mod undo;
