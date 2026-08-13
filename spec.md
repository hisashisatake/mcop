# 架空FM音源「38x6」設計仕様書

## 概要

- YAMAHAのYM3806(OPQ)をベースに、OPZ系の波形拡張を加えた架空のFM音源として出発（チップ名ym38x6/38x6）
- 梅本竜氏がSynthEdit+VOPMで構築したYM-2609（2008年）と同じ発想：
  「PCM音源へ移行する前に、FM音源があと一歩進化していたとしたら」
- ソフトウェア実装（Rust）なので制約なし
- 作曲支援アプリのエンジンとしての役割も持つ
- 作曲支援アプリはTauriで実装。まずWindowsデスクトップ版から開始
- **⚠️ 方針転換（2026-08-12）**: 後継チップ**OP505**（EG方式をレート方式からN点Time/Level方式へ
  全面移行した派生チップ、`op505/`）への一本化を決定し、**ym38x6（38x6チップ本体）は開発中止・凍結**した。
  以後の新規開発は全てop505が対象。ym38x6とop505はFM合成の基本部分（アルゴリズム・波形・チップ内LFO等）を
  `sound-fm`経由で共有しており、この部分の投資は無駄にならずop505に引き継がれている
  （詳細はクレート構成、進捗はspec-roadmap.md冒頭「方針転換」節参照）

---

## 構成

本ドキュメントは設計仕様の全体像（実装ロードマップ・技術スタック・参照資料）を扱う。
詳細仕様は以下の文書に分割されている。

- [spec-roadmap.md](spec-roadmap.md)：実装フェーズ一覧と現在地
- [spec-sound.md](spec-sound.md)：38x6音源エンジンの仕様（パラメーター・MIDI実装・OPQコンバーター・波形メモリ専用音色バンク等）。
  **ym38x6凍結（2026-08-12）時点の仕様として保持**。ただしVCO抽象・FG（Pitch/Cutoff/Gain）・質感LFO・
  三層モデル等の`sound-core`/`sound-fm`共有層の記述はop505にもそのまま適用される。`.38x6`ファイル形式・
  ym38x6-vstのCC/NRPN・OPQコンバーター設計等のym38x6固有部分は凍結対象
- [spec-app.md](spec-app.md)：作曲支援アプリのUI設計仕様
- [spec-fm.md](spec-fm.md)：FM音源変換ツール群（`ym38x6/tools/`）の横断知見。ym38x6凍結に伴い、
  op505向け変換ツール（`op505/tools/`）移植時に参照する過去知見という位置づけ

OP505（主力チップ）は専用のspec文書をまだ持たない。進捗はspec-roadmap.mdの該当フェーズに記録し、
パラメーター仕様は`op505/core`のソースコード直下のドキュメントコメント（`lib.rs`/`demo.rs`等）を正本とする。
将来的に`spec-sound.md`からop505固有のspec文書を分離する可能性がある（未着手）。

---

## 実装ロードマップ

フェーズ一覧と現在地は [spec-roadmap.md](spec-roadmap.md) に分離した。
2026-08-12の方針転換によりym38x6は凍結、op505へ一本化。op505ツール群のデフォーク・gesture-app既定
エンジンのOP505化・共有クレートのsound/uiグループ再編が完了しており、残タスクは
op505用VST3/CLAPプラグイン（op505-vst）の新設（フェーズ8）と、smf2wav/wavetest/opzref4x6/patchlab等
検証・音作りツール群のop505移行（フェーズ5.5）に整理されている（詳細はspec-roadmap.md参照）。

---

## 技術スタック

### クレート構成

ディレクトリ＝グループ、パッケージ名は`<group>-<part>`という規約で構成する。各グループの主クレートは
`core`という名前に統一する。詳細（各クレートの役割・依存関係の制約）はCLAUDE.md「クレート構成」を参照。

```
（リポジトリルート）
  Cargo.toml             ← ワークスペース
  spec.md
  CLAUDE.md

  sound/                 ← 音源レイヤーの共有基盤（製品非依存）
    core/                ← クレート名sound-core。WaveTable・AdsrParams・Eg（FG=Pitch/Cutoff/Gain共通部品）・
                            TimeEg（N点Time/Level方式EG、OP505用）・PerformanceLfo・MasterEffects・Vcoトレイト
    fm/                  ← クレート名sound-fm。FM合成チップ間で共有するEG非依存の汎用部品
                            （アルゴリズム結線・パラメーターマッピング・チップ内LFO・波形生成）

  ui/                    ← UIレイヤーの共有基盤（製品非依存、egui依存）
    core/                ← クレート名ui-core。ノブ・EGプレビュー・TimeEgエディタ・アルゴリズム結線図等
    layout/              ← クレート名ui-layout。taffyベースのパネルレイアウト計算（egui非依存）
    codegen/             ← クレート名ui-codegen。パネルXML DSL（panel.xml）のパーサー・IR・Rustコード生成器

  ym38x6/                ← 38x6製品一式（レート方式5段EG）。**2026-08-12より開発中止・凍結**
                            （新機能追加なし、op505側の参考資産として保持）
    core/                ← 38x6 FMエンジン実装（クレート名ym38x6-core。sound-core/sound-fmに依存、
                            Vco実装の一つ。波形メモリ専用音色バンクも生成）
    ui/                  ← エディタパネル定義（クレート名ym38x6-ui。src/panel.xmlが正本）
    vst/                 ← 38x6 VST3/CLAPプラグイン（クレート名ym38x6-vst、nice-plug）
    tools/               ← レガシーFM音源コンバーター群・音色設計/性能検証ツール（patchlab等）

  op505/                 ← OP505製品一式（N点Time/Level方式EG）。**唯一の主力チップ（2026-08-12〜）**
    core/                ← OP505 FMエンジン実装（クレート名op505-core。sound-core/sound-fmに依存、
                            Vco実装の一つ。デフォーク完了により`ym38x6-core`への依存は
                            `[dev-dependencies]`のみ、製品コードは非依存）
    ui/                  ← エディタパネル定義（クレート名op505-ui）
    vst/                 ← OP505 VST3/CLAPプラグイン（クレート名op505-vst、nice-plug。フェーズ1完了、
                            2026-08-12）。DAWパラメーター75個+TimeEg 7本（`#[persist]`状態、203値）の
                            ハイブリッド構成。詳細はCLAUDE.md「クレート構成」・spec-roadmap.mdフェーズ8参照
    tools/               ← レガシーFM音源→OP505直接変換ツール群（ym38x6依存ゼロ、opz2op505等）

  gesture-app/           ← 作曲支援デスクトップアプリ（メイン開発対象）
    package.json
    src/                   ← フロントエンド（HTML/JS）
      index.html
      main.js              ← キャリブレーション・ジェスチャーUI・音源エンジン切替（38x6/OP505）
    src-tauri/             ← Rustバックエンド
      Cargo.toml
      build.rs
      tauri.conf.json
      src/main.rs          ← cpalで音声出力、Tauriコマンド（note_on/note_off等）
      icons/               ← アプリアイコン
      capabilities/        ← Tauri v2 パーミッション設定

```

### 各層の技術

```
言語:           Rust
アプリ:         Tauri（VST3/CLAP両対応）
音声出力:       cpal（デスクトップ）/ Core Audio（iOS、将来）
参照実装:       ymfm（C++、BSD 3-Clause）
VSTプラグイン:  nice-plug（ym38x6-vstは凍結。op505-vstはフェーズ1完了・フェーズ2未着手）
ターゲット:     Windowsデスクトップ → タブレット（iOS/Android）→ VST
```

### 設計方針：VCO抽象とモジュレーション層

層の役割を「発振源（VCO）」と「モジュレーション/処理層」に分離し、発振源を差し替え可能にする。

```
sound-core（モジュレーション/処理層 + VCO抽象）
  VCO抽象トレイト        ← 「ピッチ付き発振源」のインターフェース
  モジュレーション層      ← FG（Pitch/Cutoff/Gain、一発にもループにもなるEG）・質感LFO（5波形専用）・VCF・VCA・表情コントローラー・ルーティング
  MasterEffects          ← Reverb/Chorus（出力後段）
        ▲ implements VCO
        │
op505-core（VCO実装の主力＝FM発振源、N点Time/Level方式EG）
  Op505Engine            ← 4opFM合成（2026-08-12以降の唯一の主力実装）
        │
ym38x6-core（VCO実装の一つ、レート方式5段EG、凍結）
  Ym38x6Engine           ← 4opFM合成（新機能追加なし、資産として保持）
```

**モジュレーションの三層モデル：** モジュレーション層の値は、帰属を①音色（パッチ.38x6）／②パート状態
（MIDIチャンネル単位のCC）／③ジェスチャー（揮発）の三層に分けて管理する（決め台詞「パッチが定義し、
CCが補正し、ジェスチャーが今を動かす」）。FG（Pitch/Cutoff/Gain）・質感LFOをはじめ各モジュレーション量は、
①基準値に②③を加算した実効値で作用するため、ホイールを触らなくてもパッチ定義の揺れは鳴る（GM2互換）。
詳細は[spec-sound.md「モジュレーションの三層モデル」](spec-sound.md#モジュレーションの三層モデル音色パート状態ジェスチャー)を参照。

- **現状（フェーズ7ステップ1で実現済み）**: VCO抽象境界を`sound-core`の`Vco`トレイト（発振原理に依存しない
  演奏ライフサイクル: note_on/note_off/render/pitch_bend系/channel_volume系の7メソッド）として確立した。
  `Ym38x6Engine`（レート方式EG）と`Op505Engine`（N点Time/Level方式EG、`TimeEg`使用）が現在この2実装
  （`impl Vco for Ym38x6Engine` / `impl Vco for Op505Engine`）。あわせて後段DSPの共通境界`AudioProcessor`
  （`process(&mut [f32], num_channels)`）も定義し、`MasterEffects`がこれを実装する。
  音色パッチ（`Ym38x6Patch`）はトレイトに含めず、38x6固有の具象API（`set_patch`/`set_channel_params`/
  `set_operator_params`等）のまま残した。`note_on`はパッチ引数を廃止し、事前に`set_patch`で設定した
  カレントパッチを使う形に統一した（呼び出し側は`set_patch`→`note_on`の2段呼び出しになる）。
  この「エンジン全体で単一カレントパッチ」前提は、三層モデルの②パート状態をMIDIチャンネル単位で独立させることで
  マルチパート（マルチティンバー）化できる前提条件でもある（マルチパート実装自体は将来のスコープ）。
  旧`SoundEngine`トレイトは単一実装でポリモーフィズムが一度も使われていなかったため撤去済みだったが、
  今回は「発振エンジンを差し替え可能にしたい」という明示的な意図のもとで再導入した点が異なる。
- **WMS-1同居時代の実態（参考）**: かつて gesture-app は `enum EngineHandle { Wms1, Ym38x6 }` で
  2エンジンを切り替えたが、共有できたのは `render()` のみ。発音はエンジン別の並行コマンド群
  （`note_on系` と `ym38x6_note_on系`）で、フロントが `engine_type` で分岐していた。
  ＝当時もVCOポリモーフィズムは無く、WMS-1廃止で具象1本へ収束した。
- **現状（フェーズ7ステップ2で実現済み）**: EG形式は5段OPM形式（AR/D1R/D1L/D2R/RR、38x6本体のFM EGと
  同じ形式）で確定した。状態機械そのものは`sound-core`の`Eg`（`sound/core/src/eg.rs`）に実装し、
  ボイス単位のモジュレーション用トレイト`Vcf`/`Vca`（`sound/core/src/vcf.rs`/`sound/core/src/vca.rs`）
  として`sound-core`に実装した。`Vcf`はSVF本体 + Cutoffを変調するキーオン連動EGを一体で保持する
  （具象実装`VoiceFilter`）。`Vca`はボイス単位の振幅EGオーバーレイで、既定パラメーター
  （ar=255,d1r=0,d1l=255,d2r=0,rr=255）ではアタック・リリースとも数サンプルで完了しほぼ常時ゲイン1.0となり、
  FM本来のキャリアEG（`operator.rs`）を変えない透過的レイヤーとして働く（具象実装`VoiceAmp`）。
  `Vcf`/`Vca`は`Ym38x6Engine`のChannel内部に留め、`Vco`トレイト自体
  （note_on/note_off/render/pitch_bend系/channel_volume系の7メソッド）は変更していない。
  `Vcf`/`Vca`は`AudioProcessor`（全ボイス合算後のマスター段バッファ一括加工）とは別の粒度
  （ボイス単位・キーオン連動EG・サンプル単位）のトレイトである点に注意。
  旧フィルターEGの4段ADSR（`ym38x6/core/src/filter.rs`の`FilterEnvelope`）は撤去し、`Vcf`の
  cutoff EGへ統合済み（`ym38x6/core/src/filter.rs`自体も削除し、SVF本体も`sound/core/src/vcf.rs`へ移設した）。
- **フェーズ7の残り（モジュレーション層本体）**: チャンネルLFO三層再編（ステップ5）とVCF/VCAファンクション
  ジェネレーター統合（ステップ5.5、FG=Pitch/Cutoff/Gain＋質感LFOへの再編）の設計・spec改訂が完了、
  velocity→音量「量」は完了（`OperatorParams.velocity_gain`、詳細はspec-roadmap.mdフェーズ7参照）。
  これらも**ボイス内・キーオン連動EG/持続する揺れ・サンプル単位**の処理であり、`AudioProcessor`とは別レイヤー。
- **将来**: VCOを別の発振源（PCM・減算合成・物理モデル等）に置換しても、同じモジュレーション層・
  UI・MIDI実装を再利用できる状態を目指す。

---

## このセッションで参照した主要資料

- ymfm（Aaron Giles）: https://github.com/aaronsgiles/ymfm
- PSR70-reverse（Jari Kangas）: https://github.com/JKN0/PSR70-reverse
- OPQプログラマーズガイド: https://www.dtech.lv/files_ym/OPQ_ProgGuide_Jari20210423.pdf
- Retro&Reverseブログ: https://retroandreverse.blogspot.com/search/label/PSR-70%20reverse%20engineering
- Hackaday.io PSR-70プロジェクト: https://hackaday.io/project/177168
- あちゃぴー氏CLP-100解析: https://achapi.cloudfree.jp/sound/yamaha_clp100/index.html
