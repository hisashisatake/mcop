---
paths:
  - "ym38x6/core/**"
  - "ym38x6/vst/**"
  - "ym38x6/ui/**"
  - "gesture-app/src-tauri/**"
---

# 共有パラメーター構造体のフィールド追加ガード

このルールは `ym38x6-core`（ChannelParams/OperatorParams等）・`ym38x6-vst`・`ym38x6-ui`
（PanelParams）・`gesture-app/src-tauri`（DTO）を編集するときだけ読み込まれる。

- `ChannelParams`/`OperatorParams`/`PanelParams`/各DTO（`ChannelParamsDto`等）にフィールドを
  追加したら、その場でワークスペース全体を対象に構造体名をgrepし（例: `StructName {`）、
  構造体リテラルで初期化している全箇所（`ym38x6-vst/src/lib.rs`の`build_patch`、
  `gesture-app/src-tauri/src/ym38x6_dto.rs`の`From`実装等）に新フィールドを反映してから
  完了とみなす。`cargo check --workspace`で初めて漏れに気づく、という手戻りを避けるため。
- 追加フィールドには`#[serde(default)]`（または`#[serde(default = "...")]`）を付け、
  既存の`.38x6`プリセットJSON・DAWオートメーション状態との後方互換を保つ
  （このプロジェクトの既存フィールドすべてがこの慣習に従っている）。
