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
現在は **フェーズ5：実機音色資産の取り込みと音作り基盤**（進行中）。

---

## 技術スタック

### クレート構成

```
ym38x6/                  ← ワークスペースルート
  Cargo.toml
  spec.md
  CLAUDE.md

  sound-core/            ← 基盤ライブラリ（WaveTable・AdsrParams・SoundEngineトレイト）
    Cargo.toml
    src/lib.rs             ← nice-plug・Tauri・cpal に無依存な純粋Rustロジック
                             波形変換パイプライン（32サンプルi8 → 1024サンプル対数フォーマット）

  ym38x6-core/           ← 38x6 FMエンジン実装（sound-coreに依存）
    Cargo.toml
    src/lib.rs             ← Ym38x6Engine（4opFM合成 + フィルター + 音色LFO + チャンネル管理）
    src/operator.rs        ← Operator（オシレーター + EG + パラメーター）
    src/algorithm.rs       ← アルゴリズム結線テーブル（ymfm由来）
    src/waveform.rs        ← ビルトイン32波形生成（OPZ由来サイン8 + ノコギリ/矩形/三角の独自拡張）
    src/mapping.rs         ← パラメーターマッピング関数群
    src/tone_lfo.rs        ← 音色LFO
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
  モジュレーション層      ← LFO・EG（Pitch/Filter/TVA）・VCF・VCA・表情コントローラー・ルーティング
  MasterEffects          ← Reverb/Chorus（出力後段）
        ▲ implements VCO
        │
ym38x6-core（VCO実装の一つ＝FM発振源）
  Ym38x6Engine           ← 4opFM合成（差し替え対象。将来はPCM/減算/物理モデル等に置換可能）
```

- **現状（未実現・要注意）**: VCO抽象はまだ"目標"であって実装されていない。`SoundEngine` トレイトは
  存在するが形骸化しており、実質的に機能している契約は `render()`（音声プル）のみ。
  `note_on(wave_slot, AdsrParams)` は旧WMS-1由来の語彙で、ym38x6では未使用
  （`Ym38x6Engine::note_on` 内コメント参照）。消費側（ym38x6-vst・gesture-app）は具象 `Ym38x6Engine` と
  `Ym38x6Patch` に直接結合しており、トレイト越しのポリモーフィズムは使っていない。
  sound-core に `PerformanceLfo`/`MasterEffects` がある点だけは土台として有効。
- **WMS-1同居時代の実態（参考）**: かつて gesture-app は `enum EngineHandle { Wms1, Ym38x6 }` で
  2エンジンを切り替えたが、共有できたのは `render()` のみ。発音はエンジン別の並行コマンド群
  （`note_on系` と `ym38x6_note_on系`）で、フロントが `engine_type` で分岐していた。
  ＝当時もVCOポリモーフィズムは無く、WMS-1廃止で具象1本へ収束した。
- **フェーズ7（MC-505風モジュレーション拡張）の方針**: 新しいモジュレーション層（LFO拡張・EG・VCF/VCA・
  表情ルーティング）を **sound-core 側に実装**し、発振源（VCO）を差し替え可能な抽象として
  **このフェーズで初めて確立する**。ym38x6-core のFMエンジンは「VCO実装の一つ」として扱う。
- **将来**: VCOを別の発振源（PCM・減算合成・物理モデル等）に置換しても、同じモジュレーション層・
  UI・MIDI実装を再利用できる状態を目指す。
- **留意（実装時に確定）**: 現状フィルター（VCF）相当は ym38x6-core 側（`filter.rs`）にある。
  フェーズ7でVCFをモジュレーション層へ移す/共有する設計は、VCO抽象の切り出しと併せて実装時に決める。

---

## このセッションで参照した主要資料

- ymfm（Aaron Giles）: https://github.com/aaronsgiles/ymfm
- PSR70-reverse（Jari Kangas）: https://github.com/JKN0/PSR70-reverse
- OPQプログラマーズガイド: https://www.dtech.lv/files_ym/OPQ_ProgGuide_Jari20210423.pdf
- Retro&Reverseブログ: https://retroandreverse.blogspot.com/search/label/PSR-70%20reverse%20engineering
- Hackaday.io PSR-70プロジェクト: https://hackaday.io/project/177168
- あちゃぴー氏CLP-100解析: https://achapi.cloudfree.jp/sound/yamaha_clp100/index.html
