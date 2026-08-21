# 架空FM音源「op505」設計仕様書

## 概要

- YAMAHAのYM3806(OPQ)をベースに、OPZ系の波形拡張を加えた架空のFM音源として出発（初期チップ名ym38x6/38x6）
- 梅本竜氏がSynthEdit+VOPMで構築したYM-2609（2008年）と同じ発想：
  「PCM音源へ移行する前に、FM音源があと一歩進化していたとしたら」
- ソフトウェア実装（Rust）なので制約なし
- 作曲支援アプリのエンジンとしての役割も持つ
- 作曲支援アプリはTauriで実装。まずWindowsデスクトップ版から開始
- **⚠️ 方針転換（2026-08-12）**: 後継チップ**OP505**（EG方式をレート方式からN点Time/Level方式へ
  全面移行した派生チップ、`op505/`）への一本化を決定し、**ym38x6（38x6チップ本体）は開発中止・凍結**した。
  以後の新規開発は全てop505が対象。ym38x6とop505はFM合成の基本部分（アルゴリズム・波形等）を
  `sound-fm`経由で共有しており、この部分の投資は無駄にならずop505に引き継がれている
  （進捗はspec-roadmap.md冒頭「方針転換」節参照）。`sound-fm::chip_lfo`（チップ内LFO由来の
  数式ライブラリ）はop505エンジンからは2026-08-20に完全退役し、opz2op505等の変換ツールが
  実機レジスタ値を写像する数式ライブラリとしてのみ現役（詳細はspec-sound.md参照）。
  **2026-08-20、凍結中だったym38x6関連のコード・データ一式（`ym38x6/`ディレクトリ、gesture-appの
  デュアルエンジン構成等）を削除した**。op505-coreは元々ym38x6-coreに一切依存しない設計だった
  ため、この削除でop505側の挙動に変更はない。

---

## 構成

本ドキュメントは設計仕様の全体像（実装ロードマップ・技術スタック・参照資料）を扱う。
詳細仕様は以下の文書に分割されている。

- [spec-roadmap.md](spec-roadmap.md)：実装フェーズ一覧と現在地
- [spec-sound.md](spec-sound.md)：OP505音源エンジンの仕様（パラメーター・MIDI実装・波形メモリ専用音色バンク等）。
  VCO抽象・FG（Pitch/Cutoff/Gain、質感texture含む）・三層モデル等の`sound-core`/`sound-fm`共有層の記述はop505に
  適用される。ym38x6固有だった`.38x6`ファイル形式・OPQコンバーター設計等の記述は、ym38x6削除
  （2026-08-20）に伴い過去の設計判断の記録として残っている箇所がある
- [spec-app.md](spec-app.md)：作曲支援アプリのUI設計仕様
- [spec-fm.md](spec-fm.md)：FM音源変換ツール群（`op505/tools/`）の横断知見。元はym38x6向けの
  旧コンバーター（`ym38x6/tools/`、削除済み）で確立された知見だが、op505向け変換ツールへ
  そのまま引き継がれている

OP505（主力チップ）は専用のspec文書をまだ持たない。進捗はspec-roadmap.mdの該当フェーズに記録し、
パラメーター仕様は`op505/core`のソースコード直下のドキュメントコメント（`lib.rs`/`demo.rs`等）を正本とする。
将来的に`spec-sound.md`からop505固有のspec文書を分離する可能性がある（未着手）。

---

## 実装ロードマップ

フェーズ一覧と現在地は [spec-roadmap.md](spec-roadmap.md) に分離した。
2026-08-12の方針転換によりym38x6は凍結、op505へ一本化。op505ツール群のデフォーク・gesture-app既定
エンジンのOP505化・共有クレートのsound/uiグループ再編・op505用VST3/CLAPプラグイン（op505-vst、
フェーズ1・2）・検証/音作りツール群のop505移行（フェーズ5.5）が完了しており、
2026-08-20にym38x6凍結資産一式を削除した（詳細はspec-roadmap.md参照）。

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
                            （アルゴリズム結線・パラメーターマッピング・波形生成。chip_lfoは
                            エンジン退役済みで変換ツール向け数式ライブラリとしてのみ現役）

  ui/                    ← UIレイヤーの共有基盤（製品非依存、egui依存）
    core/                ← クレート名ui-core。ノブ・EGプレビュー・TimeEgエディタ・アルゴリズム結線図等
    layout/              ← クレート名ui-layout。taffyベースのパネルレイアウト計算（egui非依存）
    codegen/             ← クレート名ui-codegen。パネルXML DSL（panel.xml）のパーサー・IR・Rustコード生成器

  op505/                 ← OP505製品一式（N点Time/Level方式EG）
    core/                ← OP505 FMエンジン実装（クレート名op505-core。sound-core/sound-fmに依存、
                            Vco実装。他クレートへ一切依存しない）
    ui/                  ← エディタパネル定義（クレート名op505-ui）
    vst/                 ← OP505 VST3/CLAPプラグイン（クレート名op505-vst、nice-plug。フェーズ1・2完了、
                            2026-08-12）。DAWパラメーター75個+TimeEg 7本（`#[persist]`状態、203値）の
                            ハイブリッド構成。詳細はCLAUDE.md「クレート構成」・spec-roadmap.mdフェーズ8参照
    midi/                ← CC/NRPN解釈の共有クレート（クレート名op505-midi。op505-vstとsmf2op505が参照）
    tools/               ← レガシーFM音源→OP505直接変換ツール群・音色設計/性能検証ツール（opz2op505等）

  gesture-app/           ← 作曲支援デスクトップアプリ（メイン開発対象）
    package.json
    src/                   ← フロントエンド（HTML/JS）
      index.html
      main.js              ← キャリブレーション・ジェスチャーUI（音源エンジンはOP505単体、2026-08-20〜）
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
VSTプラグイン:  nice-plug（op505-vstはフェーズ1・2完了）
ターゲット:     Windowsデスクトップ → タブレット（iOS/Android）→ VST
```

### 設計方針：VCO抽象とモジュレーション層

層の役割を「発振源（VCO）」と「モジュレーション/処理層」に分離し、発振源を差し替え可能にする。

```
sound-core（モジュレーション/処理層 + VCO抽象）
  VCO抽象トレイト        ← 「ピッチ付き発振源」のインターフェース
  モジュレーション層      ← FG（Pitch/Cutoff/Gain、一発にもループにもなるEG。ループ区間はtextureでS&H/Random/Chaosの3種を追加可能）・VCF・VCA・表情コントローラー・ルーティング
  MasterEffects          ← Reverb/Chorus（出力後段）
        ▲ implements VCO
        │
op505-core（VCO実装＝FM発振源、N点Time/Level方式EG）
  Op505Engine            ← 4opFM合成
```

**モジュレーションの三層モデル：** モジュレーション層の値は、帰属を①音色（パッチ）／②パート状態
（MIDIチャンネル単位のCC）／③ジェスチャー（揮発）の三層に分けて管理する（決め台詞「パッチが定義し、
CCが補正し、ジェスチャーが今を動かす」）。FG（Pitch/Cutoff/Gain）をはじめ各モジュレーション量は、
①基準値に②③を加算した実効値で作用するため、ホイールを触らなくてもパッチ定義の揺れは鳴る（GM2互換）。
詳細は[spec-sound.md「モジュレーションの三層モデル」](spec-sound.md#モジュレーションの三層モデル音色パート状態ジェスチャー)を参照。

- **VCO抽象境界**: `sound-core`の`Vco`トレイト（発振原理に依存しない演奏ライフサイクル:
  note_on/note_off/render/pitch_bend系/channel_volume系の7メソッド）として確立した。当初は
  `Ym38x6Engine`（レート方式EG）と`Op505Engine`（N点Time/Level方式EG、`TimeEg`使用）の2実装が
  存在したが、ym38x6削除（2026-08-20）に伴い現在は`Op505Engine`が唯一の実装。あわせて後段DSPの
  共通境界`AudioProcessor`（`process(&mut [f32], num_channels)`）も定義し、`MasterEffects`が
  これを実装する。音色パッチはトレイトに含めず、op505固有の具象API（`set_patch`/`set_patch_live`等）
  のまま残している。`note_on`はパッチ引数を持たず、事前に`set_patch`で設定したカレントパッチを
  使う形で統一されている（呼び出し側は`set_patch`→`note_on`の2段呼び出しになる）。
  この「エンジン全体で単一カレントパッチ」前提は、三層モデルの②パート状態をMIDIチャンネル単位で独立させることで
  マルチパート（マルチティンバー）化できる前提条件でもある（マルチパート実装自体は将来のスコープ）。
- **EG形式**: op505は5段OPM形式のレート方式EGではなく、N点Time/Level方式の`TimeEg`
  （`sound/core/src/time_eg.rs`、最大8段・ループ可）を全EG系統（OP1〜4・Pitch/Cutoff/Gain FG）で使う。
  ボイス単位のモジュレーション用トレイト`Vcf`/`Vca`（`sound/core/src/vcf.rs`/`sound/core/src/vca.rs`）
  も`sound-core`に実装されているが、op505-coreは分解結線（SVF本体とCutoff/Gain FGを個別に持ち都度合成）
  を採用しており、これらのトレイト自体は使用していない（`op505/core/src/lib.rs`のコメント参照）。
  `Vcf`/`Vca`は`AudioProcessor`（全ボイス合算後のマスター段バッファ一括加工）とは別の粒度
  （ボイス単位・キーオン連動EG・サンプル単位）のトレイトである点に注意。
- **モジュレーション層本体**: チャンネルLFO三層再編・VCF/VCAファンクションジェネレーター統合
  （FG=Pitch/Cutoff/Gainへの再編。旧質感LFOは2026-08-20にFGの`texture`フィールドへ統合され退役済み）・
  velocity→音量「量」（`OperatorParams.velocity_gain`）は
  いずれも完了済み（詳細はspec-roadmap.md参照）。これらは**ボイス内・キーオン連動EG/持続する揺れ・
  サンプル単位**の処理であり、`AudioProcessor`とは別レイヤー。
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
