# 架空FM音源「38x6」設計仕様書

## 概要

- YAMAHAのYM3806(OPQ)をベースに、OPZ系の波形拡張を加えた架空のFM音源
- 梅本竜氏がSynthEdit+VOPMで構築したYM-2609（2008年）と同じ発想：
  「PCM音源へ移行する前に、FM音源があと一歩進化していたとしたら」
- ソフトウェア実装（Rust）なので制約なし
- 作曲支援アプリのエンジンとしての役割も持つ
- 作曲支援アプリはTauriで実装。まずWindowsデスクトップ版から開始

---

## 構成

本ドキュメントは設計仕様の全体像（実装ロードマップ・技術スタック・参照資料）を扱う。
詳細仕様は以下の文書に分割されている。

- [spec-roadmap.md](spec-roadmap.md)：実装フェーズ一覧と現在地
- [spec-sound.md](spec-sound.md)：38x6音源エンジンの仕様（パラメーター・MIDI実装・OPQコンバーター・波形メモリ専用音色バンク等）
- [spec-app.md](spec-app.md)：作曲支援アプリのUI設計仕様

---

## 実装ロードマップ

フェーズ一覧と現在地は [spec-roadmap.md](spec-roadmap.md) に分離した。
現在は **フェーズ5：実機音色資産の取り込みと音作り基盤** と **フェーズ7：MC-505風モジュレーション拡張** を並行進行中
（フェーズ7はチャンネルLFO三層再編＝ステップ5とVCF/VCAファンクションジェネレーター統合＝ステップ5.5の設計・spec改訂が完了し、次はコア実装＝ステップ6）。

---

## 技術スタック

### クレート構成

```
ym38x6/                  ← ワークスペースルート
  Cargo.toml
  spec.md
  CLAUDE.md

  sound-core/            ← 基盤ライブラリ（WaveTable・AdsrParams・PerformanceLfo〈チャンネルLFO、改称予定〉・MasterEffects）
    Cargo.toml
    src/lib.rs             ← nice-plug・Tauri・cpal に無依存な純粋Rustロジック
                             波形変換パイプライン（32サンプルi8 → 1024サンプル対数フォーマット）

  ym38x6-core/           ← 38x6 FMエンジン実装（sound-coreに依存）
    Cargo.toml
    src/lib.rs             ← Ym38x6Engine（4opFM合成 + フィルター + チップ内LFO + チャンネル管理）
    src/operator.rs        ← Operator（オシレーター + EG + パラメーター）
    src/algorithm.rs       ← アルゴリズム結線テーブル（ymfm由来）
    src/waveform.rs        ← ビルトイン32波形生成（OPZ由来サイン8 + ノコギリ/矩形/三角の独自拡張）
    src/mapping.rs         ← パラメーターマッピング関数群
    src/tone_lfo.rs        ← チップ内LFO（旧称「音色LFO」。ファイル名はステップ6で改称予定）
    src/filter.rs          ← SVF + Filter EG

  ym38x6-vst/            ← 38x6 VST3/CLAPプラグイン（nice-plug）

  gesture-app/           ← 作曲支援デスクトップアプリ（メイン開発対象）
    package.json
    src/                   ← フロントエンド（HTML/JS）
      index.html
      main.js              ← キャリブレーション・ジェスチャーUI
    src-tauri/             ← Rustバックエンド
      Cargo.toml
      build.rs
      tauri.conf.json
      src/main.rs          ← cpalで音声出力、Tauriコマンド（note_on/note_off）
      icons/               ← アプリアイコン
      capabilities/        ← Tauri v2 パーミッション設定

```

### 各層の技術

```
言語:           Rust
アプリ:         Tauri（VST3/CLAP両対応）
音声出力:       cpal（デスクトップ）/ Core Audio（iOS、将来）
参照実装:       ymfm（C++、BSD 3-Clause）
VSTプラグイン:  nice-plug（ym38x6-vstに実装済み）
ターゲット:     Windowsデスクトップ → タブレット（iOS/Android）→ VST
```

### 設計方針：VCO抽象とモジュレーション層

層の役割を「発振源（VCO）」と「モジュレーション/処理層」に分離し、発振源を差し替え可能にする。

```
sound-core（モジュレーション/処理層 + VCO抽象）
  VCO抽象トレイト        ← 「ピッチ付き発振源」のインターフェース
  モジュレーション層      ← チャンネルLFO(LFO1/2)・EG（Pitch/Filter/TVA）・VCF・VCA・表情コントローラー・ルーティング
  MasterEffects          ← Reverb/Chorus（出力後段）
        ▲ implements VCO
        │
ym38x6-core（VCO実装の一つ＝FM発振源）
  Ym38x6Engine           ← 4opFM合成（差し替え対象。将来はPCM/減算/物理モデル等に置換可能）
```

**モジュレーションの三層モデル：** モジュレーション層の値は、帰属を①音色（パッチ.38x6）／②パート状態
（MIDIチャンネル単位のCC）／③ジェスチャー（揮発）の三層に分けて管理する（決め台詞「パッチが定義し、
CCが補正し、ジェスチャーが今を動かす」）。チャンネルLFO（LFO1/LFO2）をはじめ各モジュレーション量は、
①基準値に②③を加算した実効値で作用するため、ホイールを触らなくてもパッチ定義の揺れは鳴る（GM2互換）。
詳細は[spec-sound.md「モジュレーションの三層モデル」](spec-sound.md#モジュレーションの三層モデル音色パート状態ジェスチャー)を参照。

- **現状（フェーズ7ステップ1で実現済み）**: VCO抽象境界を`sound-core`の`Vco`トレイト（発振原理に依存しない
  演奏ライフサイクル: note_on/note_off/render/pitch_bend系/channel_volume系の7メソッド）として確立した。
  `Ym38x6Engine`はこの1実装（`impl Vco for Ym38x6Engine`）。あわせて後段DSPの共通境界`AudioProcessor`
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
  同じ形式）で確定した。状態機械そのものは`sound-core`の`Eg`（`sound-core/src/eg.rs`）に実装し、
  ボイス単位のモジュレーション用トレイト`Vcf`/`Vca`（`sound-core/src/vcf.rs`/`sound-core/src/vca.rs`）
  として`sound-core`に実装した。`Vcf`はSVF本体 + Cutoffを変調するキーオン連動EGを一体で保持する
  （具象実装`VoiceFilter`）。`Vca`はボイス単位の振幅EGオーバーレイで、既定パラメーター
  （ar=255,d1r=0,d1l=255,d2r=0,rr=255）ではアタック・リリースとも数サンプルで完了しほぼ常時ゲイン1.0となり、
  FM本来のキャリアEG（`operator.rs`）を変えない透過的レイヤーとして働く（具象実装`VoiceAmp`）。
  `Vcf`/`Vca`は`Ym38x6Engine`のChannel内部に留め、`Vco`トレイト自体
  （note_on/note_off/render/pitch_bend系/channel_volume系の7メソッド）は変更していない。
  `Vcf`/`Vca`は`AudioProcessor`（全ボイス合算後のマスター段バッファ一括加工）とは別の粒度
  （ボイス単位・キーオン連動EG・サンプル単位）のトレイトである点に注意。
  旧フィルターEGの4段ADSR（`ym38x6-core/src/filter.rs`の`FilterEnvelope`）は撤去し、`Vcf`の
  cutoff EGへ統合済み（`ym38x6-core/src/filter.rs`自体も削除し、SVF本体も`sound-core/src/vcf.rs`へ移設した）。
- **フェーズ7の残り（モジュレーション層本体）**: チャンネルLFO三層再編（ステップ5）とVCF/VCAファンクション
  ジェネレーター統合（ステップ5.5）の設計・spec改訂が完了、コア実装ステップ6〜smf2wavステップ9が未実装、
  手動ワウ・表情ルーティング・velocity→音量「量」・
  Pitch EGが未実装。これらも**ボイス内・キーオン連動EG/持続する揺れ・サンプル単位**の処理であり、
  `AudioProcessor`とは別レイヤー。
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
