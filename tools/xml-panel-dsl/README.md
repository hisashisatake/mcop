# xml-panel-dsl — ym38x6-ui パネルレイアウトXML DSL

`ym38x6-ui/src/panel.rs`（VST/gesture-app 共有の音色エディタ・ノブパネル）のレイアウトを
XMLで宣言的に記述し、プレビュー（SVG）と `draw_param_panel()` 本体のRustコードを
同じXMLパース結果から生成する、自己完結の HTML ツール。

依存・ビルド不要。ブラウザで `index.html` を開くだけで動く（外部リソース参照なし）。

```powershell
Start-Process tools\xml-panel-dsl\index.html
```

[panel-editor](../panel-editor/)（ドラッグ編集式のJSモデル）の後継として設計。
詳しい経緯は `docs/session_history.txt` の2026-07-22セッションを参照。

## 位置づけ・割り切り（2026-07-22の方針決定）

- **ドラッグ&ドロップ編集UIは持たない**。XML手書き＋プレビュー専用ツール。
- **`panel.rs`の`draw_param_panel()`本体は生成専用**（手編集禁止）。逆にRust→XMLの
  取り込み機能も持たない（XML→Rust/HTMLの一方向）。ただし`PanelParams`等の構造体定義や
  `mul_fine_ratio`等のヘルパー関数は引き続き手書きのまま（XMLの`handle`属性が参照する
  「データ契約」なので、生成対象は描画コードのみ）。
- 自己完結HTML1枚（`panel-editor`と同形式。ただし将来変わる可能性はある）。
- レイアウト計算は`ym38x6-ui/src/layout.rs`（bare taffy採用）のJS移植。
  `<row>`/`<stack>`の中でのみtaffy的な配置計算を行い、パネル同士の縦積み・`<columns>`の
  高さ合わせは`panel.rs`の現行実装と同じ命令的コード（`ui.group`の逐次呼び出し・
  `response.rect.height()`計測）をそのまま生成する。

## ウィジェット自然サイズは実測不要と判明

`knob`/`waveform_selector`/`eg_preview`/`algorithm_diagram`/`enum_selector`は全て
`ui.allocate_exact_size`/`ui.allocate_ui_with_layout`で内部的に固定サイズを強制しているため、
`panel.rs`の`KNOB_W`等の定数は近似値ではなくウィジェット自身の宣言値と完全一致する
（ソースを読めば自明）。**実測が必要だったのは`bool_checkbox`（内容依存の可変幅）だけ**で、
その実測値（70px、"CURVE"ラベル基準）はこのツールの`WIDGET_SIZE`テーブルと
`panel.rs`の`CHECKBOX_W`の両方に反映済み。

## XML語彙

```
<layout>                          ルート。<panel>/<columns>を任意個
  <panel repeat="operators" as="op" index="i" title="...">
    <let name="is_carrier" expr="...(Rust式)"/>   任意個、本体の前に`let`文として出力
    <header>                       ui.horizontal{}として出力（<raw>/<jack>のみ）
      <raw>...Rustコード...</raw>
      <jack kind="dest" dest-index="N" label="..." handle="..."/>
    </header>
    <row justify="start|between|around|evenly|center|end" grow="true" gap="spacing|<数値>">
      ...レイアウト語彙・内容語彙（下記）...
    </row>
    <space size="N"/>
    <jack kind="source"/>          または kind="dest" ...
    <raw>...</raw>                 パネル本体直下のraw文（headerの外でも可）
  </panel>

  <columns match-height="true">
    <column width="1" title="...">...<panel>と同じ本体...</column>
    <column width="2" title="...">...</column>
  </columns>
</layout>
```

### レイアウト語彙（`<row>`/`<stack>`内、taffyの`Style`に1:1対応）

- `<row justify="...">` / `<stack>` — 入れ子可。`grow="true"`で親の余剰スペースを取りにいく
  （`layout.rs`の`row_grow`に対応）。`gap`は数値、または`"spacing"`で
  `ui.spacing().item_spacing.x`を参照する記号値（`outer_gap`変数として生成）。

### 内容語彙（leafウィジェット、`<row>`/`<stack>`直下にのみ置ける）

| 要素 | 必須属性 | 備考 |
|---|---|---|
| `<knob>` | `label`, `handle` | `enabled-if="<Rust式>"`で`ui.add_enabled_ui`ラップ |
| `<checkbox>` | `label`, `handle` | 単体（稀）|
| `<checkbox-stack>` | 子に`<checkbox>`複数 | `ui.vertical`で縦積み。幅は常に70(実測値)固定 |
| `<waveform>` | `handle` | `index`（省略時0） |
| `<enum>` | `label`, `handle`, `names`（Rust定数名） | `salt`（省略時0） |
| `<eg-preview>` | `mapping`(`DbLinear`/`AmplitudeLinear`) + 各EGParamsフィールド | 各フィールドは`{field}="handle"`か`{field}-value="リテラル"`のどちらか。フィールド: `tl`,`ar`,`d1r`,`d1l`,`d2r`,`rr`,`floor`,`loop`(→`loop_enabled`),`curve`,`delay` |
| `<algorithm-diagram>` | `handle` | |
| `<raw width height>` | テキスト内容 = Rustコード | `<row>`/`<stack>`内の最終手段（要サイズ指定） |

### handle解決ルール

- `<panel repeat="operators" as="op">`配下では、`.`を含まない`handle`値は`op.`を前置
  （例: `handle="tl"` → `op.tl`）。
- `.`を含む値は**フルパスとしてそのまま**使う（自分で`params.`から書く。
  例: `handle="params.pitch_fg.depth"`）。
- `<panel>`直下（`repeat`なし）では暗黙のベースは`params`。

## 既知の割り切り・未実装

- `<stack grow="true">`は`layout.rs`に`stack_grow`が無いため未対応（パースエラーにする）。
- プレビュー（SVG）はegui実描画の忠実再現ではなく、構造・サイズ・位置関係の確認用途。
  幅が自然サイズより狭い場合はCSS flexbox既定の`flex-shrink`相当で全要素を比例縮小する
  近似計算をしている（実際のegui描画はウィジェットごとに固定サイズを`allocate_exact_size`
  するため、狭すぎる場合の実際の見た目とは一致しない）。
- 生成結果は`cargo check`の前に一度目視確認する前提で使う（`panel.rs`の構造体定義・
  ヘルパー関数はこのツールの生成対象外なので、既存の`panel.rs`上部はそのまま残し
  `draw_param_panel`本体だけを置き換える）。
