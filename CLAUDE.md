# CLAUDE.md

このファイルはClaude Codeがym38x6リポジトリで作業する際のガイドです。
設計の詳細は [spec.md](spec.md)（全体像）/ [spec-roadmap.md](spec-roadmap.md)（実装フェーズ）/ [spec-sound.md](spec-sound.md)（音源エンジン）/ [spec-app.md](spec-app.md)（作曲支援アプリUI）/ [spec-fm.md](spec-fm.md)（FM音源変換ツール群の横断知見）を参照。

## 設計経緯
詳細な議論の経緯は `docs/session_history.txt` を参照。
なぜOPQベースなのか、各パラメーターの判断理由などが含まれる。

---

## 環境

### シェル

作業はPowerShellを基本とする。bash（Git Bash）はForkに同梱されたものに依存しており、PCによって有無が変わるため使用しない。

```powershell
Set-ExecutionPolicy RemoteSigned -Scope Process
```

スクリプト実行がブロックされる環境向けの実行ポリシー緩和。`.claude/settings.json`のSessionStart Hookで自動実行される。

### git

```powershell
$git = "C:\Users\satake\AppData\Local\Fork\gitInstance\2.50.1\cmd\git.exe"
```

PowerShellのPATHにgitが含まれていないため、上記フルパスで呼び出す。

### Rust

```powershell
cargo --version  # rustupでインストール済み前提
```

### wasm32（gesture-app音色エディタ用、editor-wasm）

このマシンのデフォルトcargo/rustcはscoop版（rustup未管理、`x86_64-pc-windows-msvc`のみ）。
`gesture-app/editor-wasm`（egui-wasm音色エディタ）のビルドにはwasm32-unknown-unknownターゲットが必要なため、
**rustupを追加導入**し、PATHは変更せずrustup配下のcargoをフルパスで使う運用にしている。

```powershell
# 初回セットアップ（導入済みなら不要）
# rustup-init.exeはrust-lang公式(https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe)から取得
# （scoopのrustupマニフェストはハッシュが古く検証に失敗することがある）
.\rustup-init.exe -y --no-modify-path --default-toolchain stable
$rustup = "$env:USERPROFILE\.cargo\bin\rustup.exe"
& $rustup target add wasm32-unknown-unknown

$rustupCargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$env:RUSTFLAGS = "-C debuginfo=0"  # PDBサイズ制限(LNK1140)回避
& $rustupCargo install wasm-bindgen-cli --version 0.2.126 --force  # editor-wasm/Cargo.tomlのwasm-bindgen固定バージョンと一致させる
Remove-Item Env:\RUSTFLAGS
```

`cargo`コマンドは引き続きscoop版が解決される（`--no-modify-path`でPATH変更を避けたため）。
wasm32ビルドは`gesture-app/scripts/build-editor-wasm.ps1`が`%USERPROFILE%\.cargo\bin\cargo.exe`/`wasm-bindgen.exe`をフルパスで呼ぶ。

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
  sound-core/          ← WaveTable・AdsrParams・PerformanceLfo・MasterEffects・VCO抽象境界（基盤ライブラリ）
    Vcoトレイト        ← 発振エンジンの演奏ライフサイクル（note_on/note_off/render/pitch_bend系/channel_volume系）。Ym38x6Engineが実装
    AudioProcessorトレイト ← 後段DSP共通境界（process(&mut [f32], num_channels)）。MasterEffectsが実装
  ym38x6-core/         ← 38x6 FMエンジン実装（sound-coreに依存、Vco実装の一つ。波形メモリ音色も生成）
  ym38x6-ui/            ← エディタ共有描画ロジック（egui+sound-coreに依存。VST/gesture-app両対応）
  ym38x6-vst/          ← 38x6 VST3/CLAPプラグイン（nice-plug）
  gesture-app/         ← 作曲支援Tauriアプリ
    src-tauri/         ← Rustバックエンド（cpalで音声出力）
    src/               ← フロントエンド（ジェスチャーUI、editor-wasmの生成物はsrc/editor-wasm/）
    editor-wasm/       ← gesture-app埋め込み音色エディタ（egui-wasm。ワークスペース除外、wasm32専用）
    scripts/           ← build-editor-wasm.ps1（tauri dev/build時に自動実行）
```

`sound-core` と `ym38x6-core` はnice-plugにもTauriにも依存しない純粋なRustライブラリ。
音源エンジンの変更はこの2クレートに閉じる。
`ym38x6-ui` はegui+sound-coreに依存し、nice-plug/Tauri/cpalに依存しない（VSTとgesture-app双方の音色エディタが共有する描画ロジック。sound-coreはEG形状プレビュー計算用）。

---

## 音色試聴スキル（tools/patchlab）

`.claude/skills/` にスキル定義を収録している。スラッシュコマンドとして使うには
`~/.claude/skills/` にコピーが必要（プロジェクト内の定義はドキュメント兼 Claude 参照用）。

```powershell
# 初回セットアップ（スラッシュコマンド化）
Copy-Item .claude/skills/*.md "$env:USERPROFILE\.claude\skills\"
```

| スキル | 使い方 | 概要 |
|---|---|---|
| `/audition` | `/audition 4` または `/audition private/foo.38x6` | 単音 C2-C5 + ストラムアルペジオを生成 |
| `/fm-compare` | `/fm-compare 4 brightness 138,155,168` | 1パラメーターを多段比較WAVで一括生成 |
| `/phrase` | `/phrase 7 funk` | ストラム/ファンク/バロックの定型フレーズで試聴 |

## エンジン性能検証スキル（tools/smf2wav）

`.claude/skills/` にスキル定義を収録している。スラッシュコマンドとして使うには
`~/.claude/skills/` にコピーが必要（プロジェクト内の定義はドキュメント兼 Claude 参照用）。

| スキル | 使い方 | 概要 |
|---|---|---|
| `/perf-bench` | `/perf-bench bank.38x6 song.mid` | tools/smf2wavで実曲を交互A/B計測し、出力WAVのビット一致も検証しながらレンダリング性能を最適化する |

### グローバル専用スキル（プロジェクトコード非依存）

以下はこのプロジェクトでよく使うが、プロジェクトのソースコード（tools/等）を呼び出さない汎用Claude技法のため、
`~/.claude/skills/<name>/SKILL.md` にのみ定義を置き、リポジトリにはコミットしない
（コミット要否の判断基準は`.claude/skills/`のスキル作成時に参照）。

| スキル | 使い方 | 概要 |
|---|---|---|
| `/scope` | `/scope lfo.rsのS&H波形を見せて` | sound-core/ym38x6-coreの信号アルゴリズム（LFO波形・EG/ループEGカーブ・VCF応答等）を実ソースから忠実にJS移植し、Artifactでオシロスコープ風に図示する。設計判断のための波形比較に使う |
| `/handoff` | `/handoff` | 別PCで作業を継続するための引き継ぎZIPを作成する（CLAUDE.md・settings.json・当該プロジェクトのmemory一式・docs/を定型構成で梱包） |
| `/handoff-restore` | `/handoff-restore` | 別PCで作られたハンドオフZIPを展開し、memory・docs・skillsを各ディレクトリへ復元する（`/handoff`の逆操作） |
| `/gui-probe` | `/gui-probe` | ブラウザ（file://で開くHTMLツール）やネイティブexe（Tauri/eframe等）をPowerShellから自動操作しスクリーンショットで見た目・挙動を確認する（クリック・ドラッグ・キー入力・別ウィンドウ検証まで対応、Windows専用） |

---

## コマンド

### 音色設計ツール（tools/patchlab）

```powershell
# .venv 内の Python を呼び出す共通コマンド（uv で管理）
cd tools/patchlab
uv run python python/<script>.py

# 例: オルガン族テンプレートを生成
uv run python python/organ_template.py
uv run python python/organ_template.py --only 16,18
uv run python python/organ_template.py --bank private/hand_designed/organ_family.38x6
```

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

`tauri dev`/`tauri build`は`beforeDevCommand`/`beforeBuildCommand`で`scripts/build-editor-wasm.ps1`を自動実行し、
`editor-wasm`（音色エディタ）をwasm32向けにビルドして`src/editor-wasm/`へ出力する（生成物はgitignore対象）。
手動で単体ビルドしたい場合：

```powershell
cd gesture-app
powershell -File scripts/build-editor-wasm.ps1
```

**注意（editor-wasmのIPC引数/Tauriコマンドを変更した場合）**: `tauri dev`は`src-tauri`/`ym38x6-core`/`sound-core`の変更をファイル監視して自動リビルドするが、`editor-wasm`はwasm32専用でこの監視対象に**含まれない**（`beforeDevCommand`で起動時に一度ビルドされるだけ）。
さらに、`beforeDevCommand`（rustup版cargoでwasm32ビルド）と`devCommand`（scoop版cargoで通常ビルド）は`%USERPROFILE%\.cargo`のパッケージキャッシュを共有しており、同時に走るとロック待ちでレースが起き、`beforeDevCommand`の完了（`wasm-bindgen`まで）を待たずに`devCommand`側が`gesture-app.exe`を起動してしまうことがある（起動ログで両方が`Blocking waiting for file lock on package cache`を出すのがその兆候）。
`ipc.rs`のNoteOnArgs等、editor-wasm↔Tauriコマンド間のIPC引数を変更したら、`npm run tauri dev`の自動`beforeDevCommand`任せにせず、**先に単体で明示ビルドして成功ログを確認してから**`tauri dev`を起動する：

```powershell
cd gesture-app
powershell -File scripts/build-editor-wasm.ps1   # "editor-wasm built -> ..." が出るまで確認
npm run tauri dev
```

これを怠ると、起動中のgesture-appが古いwasmのまま残り、Rust側とIPCの形が食い違って**エラーも出さずに発音しなくなる**。

アプリ内ではEキーで音色エディタのオーバーレイ表示をトグルできる（VSTと同じ`ym38x6-ui`のノブパネル）。

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
  PerformanceLfo / PerformanceLfoTarget（共通Destination: 0=Pitch, 1=Volume）
  MasterEffects（Reverb/Chorus、各エンジンのrender()出力に後段適用）
  波形変換：32サンプルi8入力 → 1024サンプル対数フォーマット
  Vcoトレイト（発振エンジンの演奏ライフサイクル境界。Ym38x6Engineが実装）
  AudioProcessorトレイト（後段DSP共通境界。MasterEffectsが実装）

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
- タッチ版（フェーズ10・タブレット対応）: 指の間隔=インターバル、指の移動=ルート音シフト
- ∞ジェスチャー: 軌跡がそのままF-Numberに追従（ビブラート・装飾音）

### 音色エディタ（gesture-app/editor-wasm）

- `ym38x6-ui`をeframe(WebRunner)でwasm32コンパイルし、`index.html`の`#editor-canvas`に重ねて描画
- Eキーでオーバーレイ表示をトグル（`main.js`。keydownリスナーはキャプチャフェーズ登録— エディタにフォーカスがある間はeguiがbubbleフェーズまでイベントを伝播させないため）
- パラメーター変更は`editor-wasm`内部でローカル状態を更新しつつdirtyフラグを立て、1フレームに1回`ym38x6_set_patch`/`set_master_effects`のTauri IPCへ送る（src-tauriの`Ym38x6Engine`/`MasterEffects`を更新）
- パフォーマンスLFO（PERF LFO群）はローカル編集のみで、main.js側のホイール/Vキー制御とは未連動（二重書き込みでトレモロ/ビブラートが化けるのを避けるため意図的に未配線）

---

## パラメーターリファレンス（ソース再読不要・ここを参照）

### TL（Total Level）: 0〜255
- `tl=0` → **消音**（−95.25 dB）/ `tl=255` → **最大出力**（0 dB）
- キャリア: TL 大 = 出力大
- モジュレーター: TL 大 = 変調指数大（倍音豊か・明るい）

### EG レート: AR / D1R / D2R（0〜255）
- `rate=0` → **フリーズ**（OPM/OPN の rate=0 に準拠。AR=0 は発音しない）
- `rate=1` 最遅 / `rate=255` 最速（AR: 0.68 ms〜20.2 秒 / D1R・D2R: 8.71 ms〜284.9 秒）

### RR（0〜255）
- D1R と異なり **0 でもフリーズしない**（rate=0 で約 284.9 秒減衰）
- `rr=255` → 最速（8.71 ms）

### D1L（サステインレベル）: 0〜255
- `d1l=0` → ほぼ無音（−93 dB）/ `d1l=255` → **完全サステイン**（0 dB）
- オルガン: d1l=255 / ピアノ: d1l=180〜210 程度

### DT1（0〜255、中心=128）: ±50 セント
- `dt1=128` → デチューンなし / 両端 ≈ ±50 セント
- 計算式: `(dt1 - 128) / 128 * 50` セント

### MUL（0〜15）
- `mul=0` → 0.5 倍（サブ） / `mul=1` → 基音 / `mul=2〜15` → 等倍

### 音色 LFO（tone_lfo_freq / tone_lfo_pmd / pms）
- `tone_lfo_freq`: 0≈3 Hz〜255≈80 Hz（指数）。Leslie 風なら ≈50（約5 Hz）/ ≈60（約6.5 Hz）
- `pms=0` → **ビブラート完全オフ**（特殊値）。1=±5 セント、255=±700 セント（指数）
- 実効ビブラート幅 ≈ `pms_to_cents_range(pms) × (vib_depth / 255)`

### Algorithm 7
- 全 4 OP が**独立キャリア**（加算合成・FM 変調なし）
- OP ごとの TL で倍音バランス制御（ドローバー方式）
- `tl=0` で該当 OP を消音（波形メモリ音色もこれを利用）

---

## 開発方針

- `sound-core` と `ym38x6-core` は常にnice-plug・Tauri・cpalに無依存を保つ
- 波形フォーマットは1024×uint16_t対数で統一。変換パイプラインはsound-coreに実装
- パラメーターは全て0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、OPM/OPN/OPQ/OPZ共通のMultiple 4bitに準拠）
- `sound-core`/`ym38x6-core`に新機能を実装したら、同じタイミングで`ym38x6-vst`に配線し、VST単体でも機能が使える状態を保つ。MIDI CC/RPN/NRPNの受信処理やパラメーター追加など、VST側対応が必要な場合は実装範囲に含める
- VST3/CLAPプラグインフレームワークはnice-plug（nih-plugのフォーク、https://codeberg.org/RustAudio/nice-plug ）を使用する
- **nice-plug制限: `ProcessContext::set_parameter()`未実装（nice-plug-core 0.1.4時点）**。`process()`内からDAWパラメーターを書き戻せないため、NRPNとDAWオートメーションの共存には「シャドウフィールド＋差分検知方式」で迂回している（`last_algorithm`・`last_operator_waveforms`等）。nice-plugがこれを実装したら差分検知ロジックを削除しNRPN受信時に`context.set_parameter()`を呼ぶ方式へ移行できる
- Co-Authored-By:～はコミットメッセージに追加しない
