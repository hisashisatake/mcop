# xml-panel-dsl — パネルレイアウトXML DSL（op505-ui/ym38x6-ui共通、2026-08-14改訂）

`op505-ui`/`ym38x6-ui`の`panel.rs`（VST/gesture-app 共有の音色エディタ・ノブパネル）のレイアウトを
XMLで宣言的に記述し、実egui描画プレビューと `draw_op505_panel()`/`draw_param_panel()` 本体の
Rustコードを同じXMLパース結果から生成する、自己完結の HTML ツール。プレビュー描画の
インタープリタ（`interpret.rs`）はチップ非依存の`ui-core`へ移設済みのため、
`op505-ui/src/panel.xml`・`ym38x6-ui/src/panel.xml`のどちらも同じツールで開ける。

**2026-08-14: `ym38x6/tools/xml-panel-dsl`から`op505/tools`へ移動**（op505が主力、ym38x6は凍結の
ため）。`preview-native`の起動時自動読み込み先もこの移動に伴い`op505/ui/src/panel.xml`へ切り替わった
（相対パス`CARGO_MANIFEST_DIR/../../../ui/src/panel.xml`は変更していないが、ディレクトリの祖先が
`op505/tools/xml-panel-dsl`になったことで解決先が変わる）。同様に`preview-wasm`クレート本体も
`ym38x6/ui/preview-wasm`から`op505/ui/preview-wasm`へ移動した。

依存・ビルド不要。ブラウザで `index.html` を開くだけで動く（外部リソース参照なし）。

```powershell
Start-Process tools\xml-panel-dsl\index.html
```

旧panel-editor（ドラッグ編集式のJSモデル）の後継として設計。実運用に移行したため
panel-editor自体は削除済み。詳しい経緯は `docs/session_history.txt` の2026-07-22セッションを参照。

## アーキテクチャ（2026-07-22改訂: フェーズB・実egui描画プレビュー化）

XMLパース・Rustコード生成・`<row>`/`<stack>`のレイアウト計算（taffy）は、egui非依存の
純粋Rustクレート **`ui-codegen`**（`ui/codegen`、`ui-layout`に依存）に一本化されている。
プレビュー描画自体は`ui-core`の`preview`フィーチャ配下の**インタープリタ**
（`ui/core/src/interpret.rs`、旧`ym38x6-ui/src/interpret.rs`から移設・チップ非依存化済み）が担う。
IRの`Widget`記述子をmatchし、`ui-codegen`と同じ構造化記述子から実際の`knob`/`bool_checkbox`/
`eg_preview`/`algorithm_diagram`/`time_eg_editor`等の関数を直接呼び出す
（「コード生成→ブラウザでコンパイル」は不可能だが「IR→インタープリタ」は可能、という発想）。
これを3ターゲットで共有する：

- **ネイティブ（VST/gesture-app）**: `op505-ui`/`ym38x6-ui`それぞれの`build.rs`が`ui-codegen`を
  build-dependencyとして呼び、各`src/panel.xml`（正本）から`$OUT_DIR/panel_generated.rs`を生成する。
  `panel.rs`はこれを`include!`するだけで、手動でのコード貼り替えは不要
  （XMLを編集して`cargo build`し直すだけで反映される）。`preview`フィーチャは無効のままビルドされ、
  XMLパーサー（roxmltree等）やインタープリタは製品バイナリに含まれない。
- **ブラウザ（このツール）**: `op505-ui/preview-wasm`が`ui-core`（`features=["preview"]`）を
  wasm-bindgen＋eframeでラップし、`build-preview-wasm.ps1`が`wasm32-unknown-unknown`向けにビルド。
  生成された`--target no-modules`のJSグルーコードと、base64エンコードした
  wasmバイナリ本体を、`index.html`内の`<!-- BEGIN/END GENERATED -->`マーカー間に
  **直接埋め込む**（fetchはfile://上のCORSで使えないため、`wasm_bindgen.initSync()`に
  デコード済みbyte列を直接渡す）。これにより**wasmを使いながらも自己完結HTML1枚のまま**
  （サーバー不要・file://で開くだけで動く）を維持している。eframeのWebRunnerが
  `--target no-modules`＋file://で実際に動くかが最大の技術リスクだったが、実機（Edge）で
  ノブ・EGグラフ・アルゴ結線図・enumドロップダウン・パッチベイケーブルの実描画を確認済み。

```powershell
# panel.xmlやui-codegen/interpret.rsのロジックを変更したら再実行する
pwsh -File tools\xml-panel-dsl\build-preview-wasm.ps1
```

旧`codegen-wasm`（`generate_rust`/`parse_ir_preview`/`solve_tree_json`のみを公開しSVGで
近似描画していた版）は`preview-wasm`へ統合され削除済み。`index.html`側に残るJSは、
DOM操作・ファイル開く/保存・プレビュー分離・キャンバス起動のグルーのみ（SVG描画コードは全廃）。

**初期状態は空（2026-07-26）**。以前は`panel.xml`のコピーを初期サンプルとして`index.html`に
埋め込んでいたが、正本を編集するたびに黙って陳腐化するため撤去した（実際に2026-07-26時点で
「正本と同一内容」というコメントを付けたまま数世代分ずれていた）。起動後は
**「ファイルを開く」か、エディタへのドラッグ&ドロップ**で`op505/ui/src/panel.xml`・
`ym38x6/ui/src/panel.xml`のどちらかを読み込む。

`file://`で開いたページから`panel.xml`を**自動で読み込むことはできない**。
`fetch()`は`file:`スキームを一切サポートせず、`XMLHttpRequest`もブラウザ起動時に
`--allow-file-access-from-files`を付けない限りブロックされるためで、wasmバイナリをbase64で
埋め込んでいるのと同じ制約（このツールがサーバー不要である代償）。自動読み込みを実現するには
ローカルHTTPサーバー経由にする必要があり、自己完結という利点そのものを失うため採らない。
正本を常に自動で開きたい場合は`preview-native`（後述）を使う。

**キャンバス内に描く文字列はASCIIに限る**。wasm版のeguiは既定フォントしか持たずCJKグリフが
無いため、日本語を`ui.label`等へ渡すと豆腐(□)になる（`preview-native`はシステムの游ゴシックを
フォールバックできるがwasmで同じことをするとフォントをbase64で抱え込み自己完結HTMLが破綻する）。
日本語のエラー本文は`index.html`側のDOM（`#errBox`）が表示するので、キャンバス側は短い英語で足りる。

**Rust出力タブは廃止済み（2026-07-26）**。`preview-wasm`は代わりにXMLの妥当性検査のみを行う
`validate_xml`を公開し、上部OK/NGバッジの表示に使う。`ui_codegen::generate_rust`自体
（実際に`op505-ui/build.rs`が使う本番のコード生成関数）は削除しておらず、生成結果を
目視したい場合は下記`preview-native`の「Rust出力を表示」チェックボックスを使う
（プレビューと生成コード表示を1ウィンドウに同居させる意味が薄いため、HTML版はプレビュー専用に寄せた）。

## ネイティブプレビュー `preview-native`（2026-07-25追加: エディタ強化・プレビュー分離）

ブラウザ版（`preview-wasm`、WebGLキャンバス）は、**同一wasmインスタンス内で**プレビューを
別ウィンドウへ分離・ドッキングする機能が原理的に実現できない（キャンバスを別ウィンドウへ
移すとWebGLコンテキストが失われる。wasmの`web-sys::window()`もメインウィンドウ固定のため
ポップアップに2つ目のeframeを起こすのも不可）。
この機能を含むフル機能版として、`tools/xml-panel-dsl/preview-native`にeframe **ネイティブ**アプリを
新設した。プレビュー描画自体は`ui_core::interpret::draw_panel_from_ir`（`preview-wasm`と共有）を
そのまま呼ぶだけで、`panel.xml`の生成経路（build.rs/interpret.rs）には一切変更を加えていない。

```powershell
cd tools\xml-panel-dsl\preview-native
cargo run   # scoop版の既定cargoでビルド可（wasm32/rustup/wasm-bindgen不要）
```

**起動時に`op505/ui/src/panel.xml`正本を自動で読み込み、rfdの保存ダイアログで同じパスへ
上書きできる**（`CARGO_MANIFEST_DIR`基準の相対パスなのでCWDに依存しない。既定は
op505側だが、「ファイルを開く」で`ym38x6/ui/src/panel.xml`へ切り替えて編集・保存も可能）。
ブラウザ版は
サンドボックスの制約でこれができない（読み込みは手動、保存はダウンロードフォルダ行き）ため、
`build.rs`が`rerun-if-changed=src/panel.xml`で拾う「編集→`cargo build`→VST/gesture-appへ反映」
というループを1ステップで閉じられるのはこちらだけ。**正本を実際に編集する作業はpreview-native、
依存ゼロで気軽に開くのがブラウザ版**、という役割分担になっている。
正本が読めなかった場合はサンプルXMLで代用せず空で開始する（紛らわしいコピーを持たないため）。

ワークスペースの`members`には含めず`exclude`のみ（`gesture-app/editor-wasm`と同じ扱い。
`tools/xml-panel-dsl`自体がワークスペースメンバーではないため、`preview-wasm`のような
空`[workspace]`テーブルは不要）。`cargo check --workspace`には影響しない。

- **エディタのTabインデント**（4スペース、選択時は複数行ブロックインデント/デデント）:
  `egui::TextEditState::load/store`でカーソル位置を直接操作し、`Tab`キーイベントを
  ウィジェット描画前に`ctx.input_mut(|i| i.consume_key(...))`で横取りする
  （`Modifiers::matches_logically`は「余分なshift/altを無視」する向きに緩いため、
  `Shift+Tab`を`Tab`単体より先にconsumeしないと単独Tab判定に食われる点に注意）。
  ブラウザ版`index.html`にも同等のJS実装を追加済み（`document.execCommand('insertText', ...)`で
  行う。`.value`の直接代入だとブラウザのCtrl+Z履歴が壊れることを実機で確認したため）。
- **プレビューの分離・ドッキング**: egui 0.34のマルチビューポート（`show_viewport_immediate`、
  `&mut self`へアクセスできる方の版）。分離ウィンドウを閉じる（`close_requested()`検知）と
  自動的にドッキング状態へ戻る（ブラウザ版`index.html`にも別方式で同等機能を追加済み、後述）。
- **エディタ/プレビュー境界のリサイズ**: `egui::Panel::left(...).resizable(true)`で標準対応
  （ブラウザ版にも同等のドラッグ可能なスプリッタdivを追加済み）。
- **日本語ラベルの文字化け対策**: eframeの既定フォントにはCJKグリフが無く豆腐(□)化するため、
  起動時に`C:\Windows\Fonts\YuGothR.ttc`（Windows標準の游ゴシック、`.ttc`は`FontData.index`で
  面を指定）をフォールバックフォントとして追加している（このマシン専用ツールのためバイナリへの
  フォント埋め込みはせず、システムフォントを都度読む）。

## ブラウザ版のプレビュー分離（2026-07-26追加: window.open + postMessage）

`preview-native`が解決した「同一wasmインスタンス内ではWebGLキャンバスを別ウィンドウへ移せない」
という制約は、キャンバス自体を移動しようとする限り回避できない。`index.html`側はこれを、
**キャンバスを移動するのではなく、`index.html`自身を`?popout=1`付きでもう1枚開き、
そこで独立した2個目の`preview-wasm`インスタンスを起こす**という別方式で回避している
（新規ウィンドウは完全に別のブラウジングコンテキストなので、同一インスタンス内マルチビュー
ポートの制約はそもそも関係ない）。

- 分離ウィンドウ側（`?popout=1`）は`body.popout`クラスでエディタペイン・ヘッダー・
  パネルタブを`display:none`にし、`<canvas>`をウィンドウ全面に表示する
  （CSSのみで切替。JS側の分岐は`IS_POPOUT`定数1個）。
- 編集中のXMLはDOM共有ではなく`postMessage`で同期する（`file://`はブラウザによっては
  ウィンドウ間でオリジンが不透明扱いになることがあるため、`targetOrigin`は`"*"`固定。
  `window.open()`の戻り値・`window.opener`とも、`postMessage`/`closed`/`focus()`は
  クロスオリジンでも仕様上アクセス可能なため問題ない）。ハンドシェイクは
  分離側の`PreviewHandle.start()`完了後に`popout-ready`を送り返す方式（親側はこれを
  受け取るまで`xml`メッセージを送らないため、初期化順序のレースは起きない）。
- 分離中は親側のローカルプレビューを`canvasWrap`ごと非表示にするだけでなく、
  `PreviewHandle.destroy()`（+`free()`）で描画ループ自体を止める（`display:none`の
  キャンバスも`requestAnimationFrame`はタブが可視である限り止まらず、CPU/GPUを
  無駄に使い続けるため）。分離ウィンドウが閉じたことは`popoutWin.closed`のポーリング
  （500ms）で検知し、閉じたらローカル側の`PreviewHandle`を作り直して復帰する。
- `preview-native`のマルチビューポート方式（1プロセス内でOSウィンドウを追加）と比べ、
  実体は「もう1個ページを開いてXMLを流し込むだけ」なので実装・保守コストが低い。
  代わりに真のドッキング（1ウィンドウ内での分離）ではなく別ウィンドウ2枚constructionになる、
  波形メモリ等の状態を持つわけではないので初期化コスト（wasm再インスタンス化）が
  分離のたびに発生する、という違いがある。

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
    <panel repeat="operators" as="op" index="i" title="..." span="4" columns="2">
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

### `columns`（`repeat`パネルのN列グリッド折り返し、2026-08-14追加）

`<panel repeat="..." columns="N">`（`repeat`属性との併用が必須、`repeat`なしパネルに付けるとパースエラー）
を指定すると、繰り返し展開される各要素をN個ごとに`ui.horizontal`で折り返し、縦横のグリッド表示にする。
例（OP1〜4を2×2グリッドに）:

```xml
<panel repeat="operators" as="op" index="i" columns="2">...</panel>
```

- 1セルの幅は「そのパネルに割り当てられた幅（`span`から計算した幅）」を`columns`等分し、
  列間ギャップ（`<style><panels gap>`）とパネル自身の枠（inner/outer margin＋枠線）ぶんを差し引いて計算する
  （`gen_panels_group`のN個パネル幅計算をセル単位に適用したもの）。
- 要素数が`columns`で割り切れない場合は最終行が単純に埋まらないだけ（エラーにはならない）。
- `<panel repeat>`本文中の`index`変数（例: `{index+1}`）は行×列から復元したフラットな連番のまま使える
  （グリッド化してもテンプレート側の書き方は変わらない）。
- コード生成（`codegen.rs`の`gen_repeat_grid`）とプレビューインタープリタ（`interpret.rs`の`draw_panel`のグリッド分岐）
  の両方に同じ意味論を実装済み。

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

新しい`compute`/`enabled-if`述語を増やす場合は、`ui-codegen`の`ir::Compute` enumへ
variantを追加し、`codegen.rs`の`compute_expr`にマッチ節を足す（網羅性チェックで両者の同期が守られる）。

### レイアウト語彙（`<row>`/`<stack>`内、taffyの`Style`に1:1対応）

- `<row justify="...">` / `<stack>` — 入れ子可。`grow="true"`で親の余剰スペースを取りにいく
  （`layout.rs`の`row_grow`/`stack_grow`に対応、2026-08-14`stack_grow`追加）。`gap`は数値、または`"spacing"`で
  `ui.spacing().item_spacing.x`を参照する記号値（`outer_gap`変数として生成）。
  `<stack grow="true">`は、既定の`align-items: stretch`により中に置いた`<row>`群がこの幅いっぱいに
  引き伸ばされる。「グラフの右に横幅の要るノブ群を複数行へ折り返す」（`<panel repeat columns="N">`で
  セル幅が半分以下になったOPパネル等）用途に使う。例:
  ```xml
  <row justify="start" gap="spacing">
    <time-eg-editor .../>
    <stack grow="true" gap="spacing">
      <row justify="between"><knob .../><knob .../></row>
      <row justify="between"><knob .../><knob .../></row>
    </stack>
  </row>
  ```
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
| `<sync-rate>` | `label`, `handle`（TimeEgの`sync_rate_handle()`） | `salt`（省略時0）。連続ノブ＋音価ドロップダウンの複合ウィジェット（100×70、下記） |
| `<eg-preview>` | `mapping`(`DbLinear`/`AmplitudeLinear`) + 各EGParamsフィールド | 各フィールドは`{field}="handle"`か`{field}-value="リテラル"`のどちらか。フィールド: `tl`,`ar`,`d1r`,`d1l`,`d2r`,`rr`,`floor`,`loop`(→`loop_enabled`),`curve`,`delay` |
| `<algorithm-diagram>` | `handle` | `width`/`height`で個別サイズ上書き（`<style>`既定は150×100） |
| `<time-eg-editor>` | `handle`（`TimeEgHandle`実装） | `mapping`（省略時`DbLinear`）、`tl`/`tl-value`、`width`/`height`（`<style>`既定は260×245）、`min-stages`・`terminal-level`（下記） |
| `<raw width height>` | テキスト内容 = Rustコード | **最終手段**（要サイズ指定）。文法上は温存しているが`panel.xml`本体では未使用 |

全leafウィジェット共通で`margin="..."`属性が使える（`<style>`の既定値・タグ別上書きをさらに上書きする、前節参照）。
`<eg-preview>`は`width`/`height`で個別サイズ上書きも可能（`<style>`既定は84×66）。

#### `<sync-rate>`（TimeEgのテンポ同期レート）

`TimeEgParams::sync_rate`(0〜255)を、**連続ノブと音価ドロップダウンの2つの見え方**で1セルに
まとめたウィジェット。値が1つしかないため、ノブを回すとドロップダウンの表示が最寄り音価へ
追従し（アンカーから外れていれば`~1/8`のようにチルダ付き）、ドロップダウンで音価を選ぶと
ノブがその音価のアンカー値へ飛ぶ。両者がずれることは構造的に起こらない。

候補名は`SYNC_NOTE_NAMES`固定なので`<enum>`と違い`names`属性は取らない。
宣言サイズは100×70で、これは`<enum>`(100×66)を置き換えても`<time-eg-editor>`の固定高さ245pxに
収まる（FGパネルの`<stack>`は`5+66(DEP) + 5+20(SYNC) + 5+70(RATE) + 5+66(RETRIG) = 242px`）。

20音価が`sound_core::sync_note_anchor()`のアンカー値へ厳密に乗る仕組み（理論値アンカー＋
指数補間）は`sound/core/src/time_eg.rs`のコメント参照。

#### `<time-eg-editor>`のEG種別制約（`min-stages` / `terminal-level`）

TimeEgは保持区間`0..=release_point`とリリース区間`release_point+1..stage_count`に段リストを
分割する。この2属性はEG種別ごとに「どこまで縮められるか」「最終段のlevelを固定するか」を宣言する
（`ui_core::TimeEgProfile`へそのまま渡る）。

| 属性 | 既定 | 意味 |
|---|---|---|
| `min-stages` | `2` | STAGESの下限。2ならリリース段が必ず1本残る |
| `terminal-level` | `zero` | `zero`＝最終段のlevelを0に固定（縦ドラッグ禁止・VALUEのLV欄をグレーアウト）／`free`＝自由 |

- **OP1〜4 EG / Pitch FG / Cutoff FG は既定のまま**。OP EGはボイス解放条件が「全4オペレーターが
  `is_idle()`」なので、必ずレベル0へ着地させないとボイスリークとキーオフ時のクリックが起きる。
  Pitch/Cutoff FGはlevel 0＝変調量ゼロ＝ニュートラルなので同じ扱いで意味も自然。
- **Gain FGだけ`min-stages="1" terminal-level="free"`**。出力への乗算でボイス解放に関与せず、
  level 0が「無音」を意味してしまうため。1段＝リリース区間が空＝note-offで何も起きない
  （`op505-core`の`default_gain_fg`＝ゲートを一切閉じない透過既定）。

### handle解決ルール

- `<panel repeat="operators" as="op">`配下では、`.`を含まない`handle`値は`op.`を前置
  （例: `handle="tl"` → `op.tl`）。
- `.`を含む値は**フルパスとしてそのまま**使う（自分で`params.`から書く。
  例: `handle="params.pitch_fg.depth"`）。
- `<panel>`直下（`repeat`なし）では暗黙のベースは`params`。`<readout>`の`args`・
  `enabled-if`の述語（`index_var`のみ）も同じ解決規則に従う。

## 既知の割り切り・未実装

- インタープリタの`<raw>`はプレースホルダ（グレーの箱＋"raw"ラベル）描画のみ。生Rustは
  解釈できないため（`panel.xml`本体では`<raw>`は未使用、文法としてのみ温存）。
- `repeat="operators"`の要素数（4）は`interpret.rs`の`repeat_count()`に小さな対応表として
  ハードコードしてある。新しい`repeat`名を増やす場合はそこへ追記する。
- `enabled-if`/`<readout compute=...>`が参照する派生計算（`is_carrier`/`mul-fine-ratio`）は
  `ui-codegen::Compute` enumの閉じた語彙。新しい計算を追加する場合は`ir.rs`にvariant追加、
  `codegen.rs`（Rust文字列生成）と`interpret.rs`（実関数呼び出し）の両方にmatch節を足す
  （網羅性チェックが両者の同期を守る）。
- 生成（各クレートの`build.rs`、`ym38x6-ui/build.rs`/`op505-ui/build.rs`）は`cargo check`時に
  自動反映される。`panel.rs`の構造体定義・ヘルパー関数はこのツールの生成対象外なので、
  既存の`panel.rs`上部はそのまま残し`draw_param_panel`/`draw_op505_panel`本体だけが
  `include!`で差し替わる。
