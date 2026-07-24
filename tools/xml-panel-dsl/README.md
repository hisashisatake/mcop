# xml-panel-dsl — ym38x6-ui パネルレイアウトXML DSL

`ym38x6-ui/src/panel.rs`（VST/gesture-app 共有の音色エディタ・ノブパネル）のレイアウトを
XMLで宣言的に記述し、実egui描画プレビューと `draw_param_panel()` 本体のRustコードを
同じXMLパース結果から生成する、自己完結の HTML ツール。

依存・ビルド不要。ブラウザで `index.html` を開くだけで動く（外部リソース参照なし）。

```powershell
Start-Process tools\xml-panel-dsl\index.html
```

旧panel-editor（ドラッグ編集式のJSモデル）の後継として設計。実運用に移行したため
panel-editor自体は削除済み。詳しい経緯は `docs/session_history.txt` の2026-07-22セッションを参照。

## アーキテクチャ（2026-07-22改訂: フェーズB・実egui描画プレビュー化）

XMLパース・Rustコード生成・`<row>`/`<stack>`のレイアウト計算（taffy）は、egui非依存の
純粋Rustクレート **`ym38x6-ui/panel-codegen`**（`ym38x6-ui/panel-layout`に依存）に一本化されている。
プレビュー描画自体は`ym38x6-ui`の`preview`フィーチャ配下の**インタープリタ**
（`ym38x6-ui/src/interpret.rs`）が担う。IRの`Widget`記述子をmatchし、`panel-codegen`と同じ
構造化記述子から実際の`knob`/`bool_checkbox`/`eg_preview`/`algorithm_diagram`等の関数を
直接呼び出す（「コード生成→ブラウザでコンパイル」は不可能だが「IR→インタープリタ」は可能、
という発想）。これを3ターゲットで共有する：

- **ネイティブ（VST/gesture-app）**: `ym38x6-ui/build.rs`が`panel-codegen`をbuild-dependencyとして呼び、
  `ym38x6-ui/src/panel.xml`（正本）から`$OUT_DIR/panel_generated.rs`を生成する。
  `panel.rs`はこれを`include!`するだけで、手動でのコード貼り替えは不要
  （XMLを編集して`cargo build`し直すだけで反映される）。`preview`フィーチャは無効のままビルドされ、
  XMLパーサー（roxmltree等）やインタープリタは製品バイナリに含まれない。
- **ブラウザ（このツール）**: `ym38x6-ui/preview-wasm`が`ym38x6-ui`（`features=["preview"]`）を
  wasm-bindgen＋eframeでラップし、`build-preview-wasm.ps1`が`wasm32-unknown-unknown`向けにビルド。
  生成された`--target no-modules`のJSグルーコードと、base64エンコードした
  wasmバイナリ本体を、`index.html`内の`<!-- BEGIN/END GENERATED -->`マーカー間に
  **直接埋め込む**（fetchはfile://上のCORSで使えないため、`wasm_bindgen.initSync()`に
  デコード済みbyte列を直接渡す）。これにより**wasmを使いながらも自己完結HTML1枚のまま**
  （サーバー不要・file://で開くだけで動く）を維持している。eframeのWebRunnerが
  `--target no-modules`＋file://で実際に動くかが最大の技術リスクだったが、実機（Edge）で
  ノブ・EGグラフ・アルゴ結線図・enumドロップダウン・パッチベイケーブルの実描画を確認済み。

```powershell
# panel.xmlやpanel-codegen/interpret.rsのロジックを変更したら再実行する
pwsh -File tools\xml-panel-dsl\build-preview-wasm.ps1
```

旧`codegen-wasm`（`generate_rust`/`parse_ir_preview`/`solve_tree_json`のみを公開しSVGで
近似描画していた版）は`preview-wasm`へ統合され削除済み。`index.html`側に残るJSは、
DOM操作・ファイル開く/保存・タブ切替・キャンバス起動のグルーのみ（SVG描画コードは全廃）。

## 位置づけ・割り切り（2026-07-22の方針決定）

- **ドラッグ&ドロップ編集UIは持たない**。XML手書き＋プレビュー専用ツール。
- **`panel.rs`の`draw_param_panel()`本体は生成専用**（手編集禁止）。逆にRust→XMLの
  取り込み機能も持たない（XML→Rust/HTMLの一方向）。ただし`PanelParams`等の構造体定義や
  `mul_fine_ratio`等のヘルパー関数は引き続き手書きのまま（XMLの`handle`属性が参照する
  「データ契約」なので、生成対象は描画コードのみ）。
- 自己完結HTML1枚（wasmバイナリをbase64埋め込みすることで維持。詳細は上記アーキテクチャ節）。
- **プレビューはパラメーター意味論を保証しない**。インタープリタのモックハンドルは
  ハンドルパス文字列（例:`"op.tl"`）をキーにした汎用レンジ(0〜255・既定128)で、実際の
  `PanelParams`のmin/max/デフォルトとは無関係（レイアウト・見た目の検証が目的）。

## ウィジェット自然サイズは実測不要と判明

`knob`/`waveform_selector`/`eg_preview`/`algorithm_diagram`/`enum_selector`は全て
`ui.allocate_exact_size`/`ui.allocate_ui_with_layout`で内部的に固定サイズを強制しているため、
`panel.rs`の`KNOB_W`等の定数は近似値ではなくウィジェット自身の宣言値と完全一致する
（ソースを読めば自明）。**実測が必要だったのは`bool_checkbox`（内容依存の可変サイズ）だけ**で、
`build_leaf_info`の宣言サイズは70×20（幅は"CURVE"ラベル基準の実測値、高さはegui既定
`interact_size.y`(18.0)に対する実測ベースの近似値）。他ウィジェットと違い66px（行の高さ規約）
より小さいのは意図的で、`<stack>`（後述）でN個縦積みしても行の高さが膨張しないようにするため。

## XML語彙（2026-07-23改訂: `<columns>`/`<column>`廃止・`<panels>`/`<panel>`へ統一）

`panel.xml`本体から`<let>`/`<raw>`を全廃し、見出し・派生値表示・条件グレーアウトを
すべて閉じた語彙で表現する（詳細は下記の設計ノート参照）。

**`<layout>`直下は常に`<panels>`のみ**（`<panel>`を単独で置くことはできない）。
1枚だけのパネルも`<panels><panel>...</panel></panels>`と書く（旧`<columns>`/`<column>`は廃止、
「単独パネルか複数カラムか」でタグを使い分ける必要をなくした）。

```
<layout>                                  ルート。<style>(省略可・1個まで)+<panels>を1個以上
  <style>                                マージン・eg-preview/algorithm-diagramの既定サイズ（後述）
  </style>
  <panels match-height="true">            <panel>を1個以上（1個なら実質フル幅、2個以上でNカラム）
    <panel repeat="operators" as="op" index="i" title="..." span="4">
      <header>                            ui.horizontal{}として出力
        <title>...</title>                見出し（下記参照）
        <readout compute="..." args="a,b" format="...{value...}..." tooltip="..."/>
        <jack kind="dest" dest-index="N" label="..." handle="..."/>
      </header>
      <row justify="start|between|around|evenly|center|end" grow="true" gap="spacing|<数値>">
        ...レイアウト語彙・内容語彙（下記）...
      </row>
      <space size="N"/>
      <jack kind="source"/>              または kind="dest" ...
    </panel>
    <panel span="8" title="...">...</panel>
  </panels>
</layout>
```

### `<style>`（マージン・eg-preview/algorithm-diagramの既定サイズ、`<layout>`直下に0〜1個）

```xml
<style>
  <panels gap="8"/>                                 <!-- 横並びパネル間の隙間 -->
  <panel inner-margin="6" outer-margin="0"/>        <!-- パネル枠の内側/外側余白（全パネル共通） -->
  <widget margin="0"/>                              <!-- 全ウィジェット共通の既定マージン -->
  <knob margin="0 2"/>                              <!-- タグ別の上書き（任意、knob/checkbox/waveform/enum/raw対応） -->
  <eg-preview width="84" height="66"/>
  <algorithm-diagram width="150" height="100"/>
</style>
```

マージン値はCSSショートハンド記法（`"4"`=4辺/`"4 8"`=上下・左右/`"4 8 6 2"`=上・右・下・左）。
`<panel>`のマージンは`egui::Margin`がi8ベースのため-128〜127の整数のみ。ウィジェットのマージンは小数可。

ウィジェットのマージンは3段カスケード（後勝ち）で解決する: `<style><widget margin>`（既定）→
`<style>`内のタグ別上書き（`<knob margin="...">`等）→インスタンス属性（`<knob margin="..." .../>`）。
`<eg-preview>`/`<algorithm-diagram>`のサイズも同様に、`<style>`の既定値をインスタンス側の
`width`/`height`属性で個別上書きできる（サイズを変えると中身の装飾も等方スケールする）。
`<style>`省略時はここに書いた値が既定値として使われる。

### `span`（12カラムグリッド、Bootstrap等のcol-span相当）

同じ`<panels>`内の`<panel>`は、CSSでおなじみの12分割グリッドの考え方で幅を指定する。

- `span="N"`（1〜12の整数）… 12分の`N`の幅を占める。**同じ`<panels>`内の全`<panel>`の
  span合計はちょうど12でなければならない**（書き漏れ等のミスをパースエラーで検出するため。
  例: CHANNEL(4)+CHIP LFO(8)=12）。
- `span`を**全`<panel>`で省略**すると12を要素数で均等割りする
  （1個なら12、2個なら6+6）。**一部の`<panel>`だけ省略するのは不可**（曖昧さを避けるため、
  明示するなら全部・省略するなら全部、のどちらか）。
- 均等割りが整数にならない場合（例: 5分割で12/5）はパースエラーになるので、その場合は
  各`<panel>`に明示的な`span=`を書く。

### 見出し（`<title>`・`title=`属性）

- `<panel>`に`title=`属性があり、かつ`<header>`が無ければ、見出しが`ui.horizontal`ラップで
  **自動挿入**される（`<panels>`内の`<panel>`が1個でも複数でも同じ扱い）。
- `<header>`内に明示的に`<title>`を書く場合:
  - `<title/>`（空）… 親の`title=`属性の値を使う（PITCH FG等、見出しとジャックを同じ行に並べたい場合）。
  - `<title>OP {index+1}</title>` … `{index+N}`を1箇所だけ含む動的テンプレート
    （`N`は整数リテラル。リピートパネルのループ変数+Nに展開される。それ以外はプレーンテキスト）。

### 派生値表示（`<readout>`、`<header>`内専用）

`compute`は閉じた語彙（既存の実装済み計算のみを参照でき、任意のRust式は書けない）:

| compute | args | 用途 |
|---|---|---|
| `mul-fine-ratio` | `mul,fine`（2個、ハンドル名） | MUL×FINEの実効周波数比 |

`format`はRustの`format!`テンプレートで、`{value...}`（`{value}`または`{value:.2}`等）が計算結果の
埋め込み位置。`tooltip`は任意（`.on_hover_text(...)`として付与）。

### 条件グレーアウト（`enabled-if`）

`enabled-if="[!]<述語名>"`。述語名も閉じた語彙（既存実装済みの判定のみ参照可）:

| 述語名 | 意味 |
|---|---|
| `is_carrier` | そのOPがキャリアかどうか（`carriers(algorithm).contains(&<loop変数>)`） |

新しい`compute`/`enabled-if`述語を増やす場合は、`panel-codegen`の`ir::Compute` enumへ
variantを追加し、`codegen.rs`の`compute_expr`にマッチ節を足す（網羅性チェックで両者の同期が守られる）。

### レイアウト語彙（`<row>`/`<stack>`内、taffyの`Style`に1:1対応）

- `<row justify="...">` / `<stack>` — 入れ子可。`grow="true"`で親の余剰スペースを取りにいく
  （`layout.rs`の`row_grow`に対応。`<stack grow="true">`は未対応、後述）。`gap`は数値、または`"spacing"`で
  `ui.spacing().item_spacing.x`を参照する記号値（`outer_gap`変数として生成）。
- `<stack>`は任意のleafウィジェット（`<knob>`/`<checkbox>`/`<waveform>`/`<enum>`等）を縦積みできる
  汎用コンテナ（旧`<checkbox-stack>`はこれに統合され廃止）。予約サイズは各子要素の宣言サイズの
  縦積み合計（幅は最大幅）になるため、`<row>`直下で他ウィジェットと並べる場合は行の高さへの
  影響を意識する（例: knob(66px)を2個stackすると132px相当になり行全体が伸びる。checkboxは
  宣言高さが20pxと小さいため、2〜3個stackしても66px行の中に収まる）。

### 内容語彙（leafウィジェット、`<row>`/`<stack>`直下にのみ置ける）

| 要素 | 必須属性 | 備考 |
|---|---|---|
| `<knob>` | `label`, `handle` | `enabled-if="[!]<述語名>"`で`ui.add_enabled_ui`ラップ |
| `<checkbox>` | `label`, `handle` | 単体、または`<stack>`直下 |
| `<waveform>` | `handle` | `index`（省略時0） |
| `<enum>` | `label`, `handle`, `names`（Rust定数名） | `salt`（省略時0） |
| `<eg-preview>` | `mapping`(`DbLinear`/`AmplitudeLinear`) + 各EGParamsフィールド | 各フィールドは`{field}="handle"`か`{field}-value="リテラル"`のどちらか。フィールド: `tl`,`ar`,`d1r`,`d1l`,`d2r`,`rr`,`floor`,`loop`(→`loop_enabled`),`curve`,`delay` |
| `<algorithm-diagram>` | `handle` | `width`/`height`で個別サイズ上書き（`<style>`既定は150×100） |
| `<raw width height>` | テキスト内容 = Rustコード | **最終手段**（要サイズ指定）。文法上は温存しているが`panel.xml`本体では未使用 |

全leafウィジェット共通で`margin="..."`属性が使える（`<style>`の既定値・タグ別上書きをさらに上書きする、前節参照）。
`<eg-preview>`は`width`/`height`で個別サイズ上書きも可能（`<style>`既定は84×66）。

### handle解決ルール

- `<panel repeat="operators" as="op">`配下では、`.`を含まない`handle`値は`op.`を前置
  （例: `handle="tl"` → `op.tl`）。
- `.`を含む値は**フルパスとしてそのまま**使う（自分で`params.`から書く。
  例: `handle="params.pitch_fg.depth"`）。
- `<panel>`直下（`repeat`なし）では暗黙のベースは`params`。`<readout>`の`args`・
  `enabled-if`の述語（`index_var`のみ）も同じ解決規則に従う。

## 既知の割り切り・未実装

- `<stack grow="true">`は`panel-layout`に`stack_grow`が無いため未対応（パースエラーにする）。
- インタープリタの`<raw>`はプレースホルダ（グレーの箱＋"raw"ラベル）描画のみ。生Rustは
  解釈できないため（`panel.xml`本体では`<raw>`は未使用、文法としてのみ温存）。
- `repeat="operators"`の要素数（4）は`interpret.rs`の`repeat_count()`に小さな対応表として
  ハードコードしてある。新しい`repeat`名を増やす場合はそこへ追記する。
- `enabled-if`/`<readout compute=...>`が参照する派生計算（`is_carrier`/`mul-fine-ratio`）は
  `panel-codegen::Compute` enumの閉じた語彙。新しい計算を追加する場合は`ir.rs`にvariant追加、
  `codegen.rs`（Rust文字列生成）と`interpret.rs`（実関数呼び出し）の両方にmatch節を足す
  （網羅性チェックが両者の同期を守る）。
- 生成（`ym38x6-ui/build.rs`）は`cargo check`時に自動反映される。`panel.rs`の構造体定義・
  ヘルパー関数はこのツールの生成対象外なので、既存の`panel.rs`上部はそのまま残し
  `draw_param_panel`本体だけが`include!`で差し替わる。
