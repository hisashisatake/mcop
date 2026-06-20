# CLAUDE.md

このファイルはClaude Codeがym38x6リポジトリで作業する際のガイドです。
設計の詳細は [spec.md](spec.md)（全体像）/ [spec-sound.md](spec-sound.md)（音源エンジン）/ [spec-app.md](spec-app.md)（作曲支援アプリUI）を参照。

## 設計経緯
詳細な議論の経緯は `docs/session_history.txt` を参照。
なぜOPQベースなのか、各パラメーターの判断理由などが含まれる。

---

## 環境

### シェル

作業はPowerShellを基本とする。bash（Git Bash）はForkに同梱されたものに依存しており、PCによって有無が変わるため使用しない。

### git

```powershell
$git = "C:\Users\satake\AppData\Local\Fork\gitInstance\2.50.1\cmd\git.exe"
```

PowerShellのPATHにgitが含まれていないため、上記フルパスで呼び出す。

### Rust

```powershell
cargo --version  # rustupでインストール済み前提
```

---

## プロジェクト概要

架空FM音源「38x6」と、それを使った作曲支援アプリ（Tauri）のワークスペース。

- **38x6**: YM3806(OPQ)ベース + OPZ系波形拡張の架空FM音源
- **波形メモリ専用音色バンク**: 38x6のOP1のみを鳴らす1オペレーター音色を予約バンクに用意したもの（エンジンのモードではなく通常パッチのバンク。フェーズ1のプロトタイプ「WMS-1」＝波形オシレーター＋ADSRの独立クレート（wms1-core/wms1-vst、廃止済み）の後継。`ym38x6-core`の`waveform_memory_patch`が生成）
- **作曲支援アプリ**: グリッドなし・キャリブレーションベースのジェスチャーUIで、知識がなくても良い感じのコードが弾けることを目指す

---

## クレート構成

```
ym38x6/
  Cargo.toml           ← ワークスペース
  spec.md              ← 設計仕様書
  sound-core/          ← WaveTable・AdsrParams・SoundEngineトレイト（基盤ライブラリ）
  ym38x6-core/         ← 38x6 FMエンジン実装（sound-coreに依存。波形メモリ音色も生成）
  ym38x6-vst/          ← 38x6 VST3/CLAPプラグイン（nice-plug）
  gesture-app/         ← 作曲支援Tauriアプリ
    src-tauri/         ← Rustバックエンド（cpalで音声出力）
    src/               ← フロントエンド
```

`sound-core` と `ym38x6-core` はnice-plugにもTauriにも依存しない純粋なRustライブラリ。
音源エンジンの変更はこの2クレートに閉じる。

---

## コマンド

### ビルド・チェック

```powershell
# ワークスペース全体のコンパイルチェック
cargo check --workspace --message-format=short

# コアライブラリのみ
cargo check -p sound-core -p ym38x6-core --message-format=short

# テスト
cargo test -p sound-core
cargo test -p ym38x6-core
```

### アプリ起動（フェーズ1以降、Tauri設定後）

```powershell
cd gesture-app
npm run tauri dev
```

### ビルド

```powershell
cd gesture-app
npm run tauri build
```

### VST3/CLAPバンドル（ym38x6-vst）

```powershell
# cargo-nice-plugが未インストールの場合（初回のみ）
cargo install cargo-nice-plug

# バンドル生成（target\bundled\<crate>.vst3 / .clap が生成される）
cargo nice-plug bundle ym38x6-vst --release
```

REAPER等のDAWで動作確認する場合は `target\bundled` をVST plug-in pathsに追加してRe-scanする。

---

## アーキテクチャ

### 音源レイヤー（sound-core / ym38x6-core）

```
sound-core（基盤）
  WaveTable（1024×u16 log符号化）
  AdsrParams
  SoundEngineトレイト
  PerformanceLfo / PerformanceLfoTarget（共通Destination: 0=Pitch, 1=Volume）
  MasterEffects（Reverb/Chorus、SoundEngine::render()出力に後段適用）
  波形変換：32サンプルi8入力 → 1024サンプル対数フォーマット

ym38x6-core（38x6実装）
  Ym38x6Engine：4opFM合成 + フィルター + 音色LFO + チャンネル管理（無制限）
  PerformanceLfoTarget実装（共通Destination + 拡張Destination=2: TLキャリア一括）
  波形メモリ専用音色バンク：waveform_memory_patch（Algorithm 7・OP1のみ可聴・OP2〜4はTL=0の通常パッチ）
    予約バンクWAVEFORM_MEMORY_BANKにBank/Program経由で用意（エンジンのモードではない。旧WMS-1の後継）
```

コアは「この周波数でキーオン」「このパラメーターで発音」のAPIのみを提供する。
MIDI・ジェスチャー解釈・UIはコアの外側で行う。

### 音声出力（gesture-app/src-tauri）

cpalでWASAPIに直接出力。オーディオスレッドのコールバックでym38x6-coreを呼ぶ。

```rust
// コールバックイメージ
stream = device.build_output_stream(&config, move |output: &mut [f32], _| {
    engine.render(output);
}, ...);
```

### ジェスチャーUI

- キャリブレーションベース（C-F-Gの3点で座標系を定義）
- グリッドなし
- マウス版: 縦軸=ルート音、横軸=コード種類
- タッチ版（フェーズ8）: 指の間隔=インターバル、指の移動=ルート音シフト
- ∞ジェスチャー: 軌跡がそのままF-Numberに追従（ビブラート・装飾音）

---

## 開発方針

- `sound-core` と `ym38x6-core` は常にnice-plug・Tauri・cpalに無依存を保つ
- 波形フォーマットは1024×uint16_t対数で統一。変換パイプラインはsound-coreに実装
- パラメーターは全て0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、OPM/OPN/OPQ/OPZ共通のMultiple 4bitに準拠）
- `sound-core`/`ym38x6-core`に新機能を実装したら、同じタイミングで`ym38x6-vst`に配線し、VST単体でも機能が使える状態を保つ。MIDI CC/RPN/NRPNの受信処理やパラメーター追加など、VST側対応が必要な場合は実装範囲に含める
- VST3/CLAPプラグインフレームワークはnice-plug（nih-plugのフォーク、https://codeberg.org/RustAudio/nice-plug ）を使用する
- **nice-plug制限: `ProcessContext::set_parameter()`未実装（nice-plug-core 0.1.4時点）**。`process()`内からDAWパラメーターを書き戻せないため、NRPNとDAWオートメーションの共存には「シャドウフィールド＋差分検知方式」で迂回している（`last_algorithm`・`last_operator_waveforms`等）。nice-plugがこれを実装したら差分検知ロジックを削除しNRPN受信時に`context.set_parameter()`を呼ぶ方式へ移行できる
- Co-Authored-By:～はコミットメッセージに追加しない
