# panel-editor — egui レイアウト調整ツール

`ym38x6-ui/src/panel.rs`（VST/gesture-app 共有の音色エディタ・ノブパネル）の
レイアウトを視覚的に組み、`panel.rs` 風の Rust を生成する自己完結の HTML ツール。

依存・ビルド不要。ブラウザで `index.html` を開くだけで動く（外部リソース参照なし）。

```powershell
Start-Process tools\panel-editor\index.html
```

## できること

- 起動時に現行 `panel.rs` のレイアウトを読み込み済み（「実物に戻す」でいつでも復帰）。
- ウィジェット（knob / bool_checkbox / waveform_selector / enum_selector / checkstack）を
  ドラッグで並べ替え、クリックで種類・ラベル・ハンドル式・`add_enabled_ui` 条件を編集。
- `justified_row` の `natural_widths` 配列と slot 数をライブ表示（幅手調整の痛点直撃）。
- パネル / raw ブロックの追加・削除・上下移動、タイトル編集。
- `eg_preview` / patchbay ケーブル / CHANNEL+CHIP LFO の高さ合わせ等の bespoke な箇所は
  **raw ブロック（テキスト編集可）** として原文保持 → 往復で消えない。
- 右ペイン：`space-between` で `justified_row` の隙間計算を再現するプレビュー
  （ウィンドウ幅スライダーで `panel_width` を変えて幅超過を可視化）＋ Rust 出力タブ。
- 他の `panel.rs` を貼り付けて取り込み（`ui.group`＋`justified_row`・`for` ループを構造化解析、
  解釈できない箇所は raw 保持）。

## 割り切り

- モデルは「ブロック → 行 → ウィジェット」の木。ハンドル式（`op.tl` / `params.feedback` 等）は
  文字列として保持する。
- **バイト単位の完全一致は保証しない**。保証するのは構造・幅配列・並び順・ラベルの往復で、
  コメントや `let two_col_usable = …` 等の glue は raw ブロック側に入る。
  生成結果は `cargo check` の前に一度目視確認する前提で使う。
