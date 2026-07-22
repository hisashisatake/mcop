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

## アーキテクチャ（2026-07-22改訂: ビルド時トランスパイル化）

XMLパース・Rustコード生成・`<row>`/`<stack>`のレイアウト計算（taffy）は、egui非依存の
純粋Rustクレート **`ym38x6-ui/panel-codegen`**（`ym38x6-ui/panel-layout`に依存）に一本化されている。
両クレートは`ym38x6-ui`配下に置く（ビルド時にしか使われないym38x6-ui自身のためのツールという
位置づけで、tools側がそれを参照する方向にするため）。これを2ターゲットで共有する：

- **ネイティブ**: `ym38x6-ui/build.rs`が`panel-codegen`をbuild-dependencyとして呼び、
  `ym38x6-ui/src/panel.xml`（正本）から`$OUT_DIR/panel_generated.rs`を生成する。
  `panel.rs`はこれを`include!`するだけで、手動でのコード貼り替えは不要
  （XMLを編集して`cargo build`し直すだけで反映される）。
- **ブラウザ（このツール）**: `panel-codegen`を`tools/xml-panel-dsl/codegen-wasm`で
  wasm-bindgenラップし、`build-wasm.ps1`が`wasm32-unknown-unknown`向けにビルド。
  生成された`--target no-modules`のJSグルーコードと、base64エンコードした
  wasmバイナリ本体を、`index.html`内の`<!-- BEGIN/END GENERATED -->`マーカー間に
  **直接埋め込む**（fetchはfile://上のCORSで使えないため、`wasm_bindgen.initSync()`に
  デコード済みbyte列を直接渡す）。これにより**wasmを使いながらも自己完結HTML1枚のまま**
  （サーバー不要・file://で開くだけで動く）を維持している。

```powershell
# panel.xmlやpanel-codegenのロジックを変更したら再実行する
powershell -File tools\xml-panel-dsl\build-wasm.ps1
```

これにより、以前の「JS版レイアウトエンジンがRust版(`layout.rs`)を非公式に移植したもの」という
二重実装（移植ズレでバグが3件見つかった経緯がある）を解消した。ブラウザのプレビューも
本物のtaffy（`panel_layout::solve`をwasm経由で呼ぶ）で矩形計算するため、近似計算は無くなった。
`index.html`側に残るJSは、DOM操作・SVG描画・ファイル開く/保存・タブ切替のグルーのみ。

## 位置づけ・割り切り（2026-07-22の方針決定）

- **ドラッグ&ドロップ編集UIは持たない**。XML手書き＋プレビュー専用ツール。
- **`panel.rs`の`draw_param_panel()`本体は生成専用**（手編集禁止）。逆にRust→XMLの
  取り込み機能も持たない（XML→Rust/HTMLの一方向）。ただし`PanelParams`等の構造体定義や
  `mul_fine_ratio`等のヘルパー関数は引き続き手書きのまま（XMLの`handle`属性が参照する
  「データ契約」なので、生成対象は描画コードのみ）。
- 自己完結HTML1枚（wasmバイナリをbase64埋め込みすることで維持。詳細は上記アーキテクチャ節）。

## ウィジェット自然サイズは実測不要と判明

`knob`/`waveform_selector`/`eg_preview`/`algorithm_diagram`/`enum_selector`は全て
`ui.allocate_exact_size`/`ui.allocate_ui_with_layout`で内部的に固定サイズを強制しているため、
`panel.rs`の`KNOB_W`等の定数は近似値ではなくウィジェット自身の宣言値と完全一致する
（ソースを読めば自明）。**実測が必要だったのは`bool_checkbox`（内容依存の可変幅）だけ**で、
その実測値（70px、"CURVE"ラベル基準）は`panel-codegen`（`build_leaf_info`）と
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

- `<stack grow="true">`は`panel-layout`に`stack_grow`が無いため未対応（パースエラーにする）。
- プレビュー（SVG）の`<row>`/`<stack>`矩形計算は本物のtaffy（wasm経由）だが、パネル同士の
  縦積み・`<columns>`の高さ合わせはSVG側の簡易な逐次配置計算（JS）のまま
  （`panel.rs`実行時の`ui.group`逐次呼び出し・`response.rect.height()`計測を模した近似）。
- 生成（`ym38x6-ui/build.rs`）は`cargo check`時に自動反映される。`panel.rs`の構造体定義・
  ヘルパー関数はこのツールの生成対象外なので、既存の`panel.rs`上部はそのまま残し
  `draw_param_panel`本体だけが`include!`で差し替わる。
