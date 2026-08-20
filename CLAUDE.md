# CLAUDE.md

このファイルはClaude Codeがop505リポジトリで作業する際のガイドです。
設計の詳細は [spec.md](spec.md)（全体像）/ [spec-roadmap.md](spec-roadmap.md)（実装フェーズ）/ [spec-sound.md](spec-sound.md)（音源エンジン）/ [spec-app.md](spec-app.md)（作曲支援アプリUI）/ [spec-fm.md](spec-fm.md)（FM音源変換ツール群の横断知見）を参照。

## 設計経緯
詳細な議論の経緯は `docs/session_history.txt` を参照。
なぜOPQベースなのか、各パラメーターの判断理由などが含まれる（初期チップ案ym38x6の設計判断。ym38x6自体は開発中止・凍結後、2026-08-20に完全削除済み）。

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

架空FM音源チップ**op505**と、それを使った作曲支援アプリ（Tauri）のワークスペース。

**⚠️ 方針転換（2026-08-12）**: 初期チップ案**ym38x6**（38x6、YM3806(OPQ)ベース）の開発を中止・凍結し、
後継チップ**op505**（N点Time/Level方式EG）へ一本化した。以後の新規開発は全てop505が対象。
詳細はspec-roadmap.md冒頭「方針転換」節を参照。
**2026-08-20、凍結中だったym38x6関連のコード・データを一式削除**（`ym38x6/`ディレクトリ、
gesture-appのデュアルエンジン構成、Cargo.tomlのワークスペースメンバー等）。op505-coreは
元々ym38x6-coreに一切依存しない設計だったため、この削除でop505側の挙動に変更はない。
過去の`.38x6`資産（ピアノ/オルガン族テンプレート等）は削除前にアーカイブへ退避済み。

- **op505（主力）**: N点Time/Level方式EGを持つFM音源チップ
- **波形メモリ専用音色バンク**: OP1のみを鳴らす1オペレーター音色を予約バンクに用意する仕組み
  （エンジンのモードではなく通常パッチのバンク）。ym38x6-coreが`waveform_memory_patch`として
  実装していたが、ym38x6削除に伴いop505には未移植（将来移植の可能性を残し、gesture-appのUIは
  Bank=16383指定のまま残してある）
- **作曲支援アプリ**: グリッドなし・キャリブレーションベースのジェスチャーUIで、知識がなくても良い感じのコードが弾けることを目指す

---

## クレート構成

リポジトリは**ディレクトリ＝グループ、パッケージ名は`<group>-<part>`**という規約で構成する
（2026-08-02にA案＝製品グルーピングで整理。2026-08-03に`mcop/`→`op505/`へ改名。
リポジトリ名/GitHubリモートは引き続き`mcop`のまま。2026-08-09に共有クレートも同じ規約へ揃え、
`sound-core`/`fm-common`→`sound/{core,fm}`・`ui-common`/`panel-layout`/`panel-codegen`→`ui/{core,layout,codegen}`へ移動）。

グループは3つ。`sound/`（音源レイヤーの共有基盤）・`ui/`（UIレイヤーの共有基盤）は製品非依存、
`op505/`が製品一式。各グループの主クレートは`core`という名前に統一する。

```
（リポジトリルート）
  Cargo.toml           ← ワークスペース
  spec.md              ← 設計仕様書
  sound/               ← 音源レイヤーの共有基盤（製品非依存）
    core/              ← クレート名sound-core。WaveTable・AdsrParams・PerformanceLfo・MasterEffects・VCO抽象境界
      Vcoトレイト      ← 発振エンジンの演奏ライフサイクル（note_on/note_off/render/pitch_bend系/channel_volume系）。Op505Engineが実装
      AudioProcessorトレイト ← 後段DSP共通境界（process(&mut [f32], num_channels)）。MasterEffectsが実装
      texture_lfo      ← TextureLfo構造体・texture_lfo_to_shape変換（VCO実装に依存しないモジュレーション層の部品）
    fm/                ← クレート名sound-fm。FM合成チップ間で共有する、EG非依存の汎用部品（op505-coreが依存）
      algorithm/mapping/chip_lfo/waveform ← アルゴリズム結線表・TL/KSR等のパラメーターマッピング・チップ内LFO・波形生成
      texture_lfo      ← FmLfoDestination（FM合成チップ固有のLFO適用先解釈のみ。TextureLfo本体はsound-coreを再エクスポート）
  ui/                  ← UIレイヤーの共有基盤（製品非依存、egui依存）
    core/              ← クレート名ui-core。ノブ・EGプレビュー・TimeEgエディタ・アルゴリズム結線図・
                          波形/enumセレクタ・パラメーターハンドルトレイト（op505-uiが依存）
    layout/            ← クレート名ui-layout。taffyベースのパネルレイアウト計算（egui非依存の純粋クレート）
    codegen/           ← クレート名ui-codegen。パネルXML DSL（panel.xml）のパーサー・IR・Rustコード生成器
                          （egui非依存。矩形計算はui-layoutへ委譲。op505-uiのbuild.rsが呼ぶ。
                           詳細はop505/tools/xml-panel-dsl/README.md）
  op505/               ← OP505製品一式（N点Time/Level方式EG）
    core/              ← OP505 FMエンジン実装（クレート名op505-core。sound-core/sound-fmに依存、Vco実装の一つ。
                          .op505バンク形式はpreset.rsのOp505PresetFile/Entry）
    ui/                ← エディタパネル定義（クレート名op505-ui。egui+sound-core+ui-coreに依存）。
                          `src/panel.xml`が正本、build.rsがui-codegen経由で`panel.rs`へinclude!するRustを生成。
                          `preview-wasm/`（xml-panel-dsl用のwasm-bindgenラッパー）が同居。ワークスペース非メンバー
    vst/               ← OP505 VST3/CLAPプラグイン（クレート名op505-vst、nice-plug。フェーズ1完了
                          2026-08-12、フェーズ2完了2026-08-12）。
                          パラメーターDAWパラメーター75個（TL/ALG/LFO/FG Depth等）+ TimeEg 7本
                          （OP1〜4 EG・Pitch/Cutoff/Gain FG、計203値）はnice-plugの`#[persist]`で
                          プロジェクト保存（DAWパラメーター化するとEGグラフの点操作1回で29個の
                          オートメーションイベントが走り記録単位が壊れるため）。オーディオスレッドは
                          `try_read()`でpersist状態を取り込み、ロック待ちで詰まらない設計。
                          NRPN（30エントリ）・表情CC（CC1/2/4/76/77/78、全16 MIDIチャンネル
                          独立）・CC66/67/120/121/123・OP単位キーオンCC103〜106・OP単位F-Number・
                          RPN(0,0)/(0,5)・MASTER EFFECTSの共有パネル統合まで実装済み（フェーズ2）。
                          CC/NRPN解釈は`op505-midi`クレートへ切り出し済み（フェーズ5.5、旧`src/midi.rs`は削除）
    midi/              ← CC/NRPN解釈の共有クレート（クレート名op505-midi。フェーズ5.5新設）。
                          `op505-vst`と`op505/tools/smf2op505`の両方が参照する、fork-on-write方針の
                          限定的な例外（VSTと参照実装の解釈が食い違うと正誤の基準が消えるため。
                          詳細はspec-fm.md 8章⑤）。`ControlTarget` enumがNRPNアドレス表を一元化し、
                          `_ =>`を使わせない全列挙で解釈のずれを構造的に防ぐ。依存は`op505-core`と
                          `sound-fm`の2本のみ
    tools/             ← レガシーFM音源→OP505直接変換ツール群 + 検証・音作りツール群
                          （実機レート→TimeEg直接変換、.38x6を経由しない）
      common/          ← クレート名op505-tools。op505ツール群専用の共有ユーティリティ
                          （WAV書き出し・ファイル名サニタイズ・音名変換・ゴールデンテストヘルパー・
                          マスターリバーブ後段適用）。詳細はCargo.tomlのコメント
      opz2op505/       ← TX81Z(OPZ/YM2414) syx → .op505直接変換（クレート名opz2op505。実機パーサ・
                          レート写像・EG変換ロジックをop505-core::eg_convert経由で自己完結。--attack bias/none/curveで
                          アタック立ち上がり表現をA/B可能、既定は`none`）
      psr2op505/ opm2op505/ mucom2op505/ ← PSR-70/OPM/MUCOM88 → .op505直接変換（opz2op505と同型のCLI構成）
      vgm2op505/       ← VGM/VGZ → .op505+SMF/WAV直接変換（クレート名vgm2op505。演奏ロジック
                          （VGM逐次デコード・OPM/OPN二系統・SSGコアレス処理等）を持つ。音色変換は
                          opm2op505/mucom2op505のvoice_to_op505_patchを再利用。SSG合成パッチも
                          Op505Patchをネイティブ構築）
      opzref/          ← ymfm(YM2414)を参照レンダラとするOPZ実機比較検証ツール（opz2op505ベース）
      wavetest/        ← 非sine波形試聴ツール（TimeEgネイティブデモを収録）
      patchlab/        ← 音色設計ツール群（PyO3バインディング。詳細は「音色設計ツール」節）
      smf2op505/       ← `.op505`音色バンクでSMFを再生しWAVへ書き出す（クレート名smf2op505。
                          エンジン性能検証（/perf-bench）とCC/NRPN解釈の参照実装を兼ねる。
                          CC/NRPN解釈は`op505-midi`を参照するため`op505-vst`と常に同じ解釈になる）
      xml-panel-dsl/   ← panel.xmlのXML DSL編集・プレビューツール（自己完結HTML + preview-native、
                          op505-ui向け。詳細はREADME.md）
  gesture-app/         ← 作曲支援Tauriアプリ
    src-tauri/         ← Rustバックエンド（cpalで音声出力）
    src/               ← フロントエンド（ジェスチャーUI、editor-wasmの生成物はsrc/editor-wasm/）
    editor-wasm/       ← gesture-app埋め込み音色エディタ（egui-wasm。ワークスペース除外、wasm32専用）
    scripts/           ← build-editor-wasm.ps1（tauri dev/build時に自動実行）
```

Cargoのワークスペースメンバーパスとパッケージ名は独立しているため、`cargo check -p sound-core`等は
ディレクトリ位置に関わらずそのまま使える。

`sound-core`・`sound-fm`・`ui-layout`・`ui-codegen`・`op505-core`・`op505-midi`はnice-plugにも
Tauriにも依存しない純粋なRustライブラリ。音源エンジンの変更は`sound/`と`op505/core/`に閉じる
（`op505-midi`はMIDI解釈層のため厳密には「音源エンジン」ではないが、同じく純粋ライブラリの原則を守る）。
`ui-core`・`op505-ui`はegui+sound-coreに依存し、nice-plug/Tauri/cpalに依存しない
（VSTとgesture-app双方の音色エディタが共有する描画ロジック。sound-coreはEG形状プレビュー計算用）。

---

## 音色試聴スキル（op505/tools/patchlab）

`.claude/skills/` にスキル定義を収録している。スラッシュコマンドとして使うには
`~/.claude/skills/` にコピーが必要（プロジェクト内の定義はドキュメント兼 Claude 参照用）。

```powershell
# 初回セットアップ（スラッシュコマンド化）
Copy-Item .claude/skills/*.md "$env:USERPROFILE\.claude\skills\"
```

| スキル | 使い方 | 概要 |
|---|---|---|
| `/audition` | `/audition 4` または `/audition private/foo.op505` | 単音 C2-C5 + ストラムアルペジオを生成 |
| `/fm-compare` | `/fm-compare 4 brightness 138,155,168` | 1パラメーターを多段比較WAVで一括生成 |
| `/phrase` | `/phrase 7 funk` | ストラム/ファンク/バロックの定型フレーズで試聴 |

## エンジン性能検証スキル（op505/tools/smf2op505）

`.claude/skills/` にスキル定義を収録している。スラッシュコマンドとして使うには
`~/.claude/skills/` にコピーが必要（プロジェクト内の定義はドキュメント兼 Claude 参照用）。

| スキル | 使い方 | 概要 |
|---|---|---|
| `/perf-bench` | `/perf-bench bank.op505 song.mid` | op505/tools/smf2op505で実曲を交互A/B計測し、出力WAVのビット一致も検証しながらレンダリング性能を最適化する |

### グローバル専用スキル（プロジェクトコード非依存）

以下はこのプロジェクトでよく使うが、プロジェクトのソースコード（op505/tools/等）を呼び出さない汎用Claude技法のため、
`~/.claude/skills/<name>/SKILL.md` にのみ定義を置き、リポジトリにはコミットしない
（コミット要否の判断基準は`.claude/skills/`のスキル作成時に参照）。

| スキル | 使い方 | 概要 |
|---|---|---|
| `/scope` | `/scope lfo.rsのS&H波形を見せて` | sound-coreの信号アルゴリズム（LFO波形・EG/ループEGカーブ・VCF応答等）を実ソースから忠実にJS移植し、Artifactでオシロスコープ風に図示する。設計判断のための波形比較に使う |
| `/handoff` | `/handoff` | 別PCで作業を継続するための引き継ぎZIPを作成する（CLAUDE.md・settings.json・当該プロジェクトのmemory一式・docs/を定型構成で梱包） |
| `/handoff-restore` | `/handoff-restore` | 別PCで作られたハンドオフZIPを展開し、memory・docs・skillsを各ディレクトリへ復元する（`/handoff`の逆操作） |
| `/gui-probe` | `/gui-probe` | ブラウザ（file://で開くHTMLツール）やネイティブexe（Tauri/eframe等）をPowerShellから自動操作しスクリーンショットで見た目・挙動を確認する（クリック・ドラッグ・キー入力・別ウィンドウ検証まで対応、Windows専用） |

---

## コマンド

### 音色設計ツール（op505/tools/patchlab）

```powershell
# .venv 内の Python を呼び出す共通コマンド（uv で管理）
cd op505/tools/patchlab
uv run python python/<script>.py

# 例: オルガン族テンプレートを生成
uv run python python/organ_template.py
uv run python python/organ_template.py --only 16,18
uv run python python/organ_template.py --bank private/hand_designed/organ_family.op505
```

### ビルド・チェック

```powershell
# ワークスペース全体のコンパイルチェック
cargo check --workspace --message-format=short

# コアライブラリのみ
cargo check -p sound-core -p op505-core --message-format=short

# テスト
cargo test -p sound-core
cargo test -p op505-core
```

### ゴールデンテストの更新（op505/tools/*）

opz2op505/psr2op505/mucom2op505/opm2op505/vgm2op505は`op505-tools::golden`（フィンガープリント+可読JSONの二層構成）で回帰を検出する。実装を意図的に変えてゴールデンを更新する場合：

```powershell
$env:UPDATE_GOLDEN=1; cargo test -p opz2op505; Remove-Item Env:\UPDATE_GOLDEN
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

**注意（editor-wasmのIPC引数/Tauriコマンドを変更した場合）**: `tauri dev`は`src-tauri`/`op505-core`/`sound-core`の変更をファイル監視して自動リビルドするが、`editor-wasm`はwasm32専用でこの監視対象に**含まれない**（`beforeDevCommand`で起動時に一度ビルドされるだけ）。
さらに、`beforeDevCommand`（rustup版cargoでwasm32ビルド）と`devCommand`（scoop版cargoで通常ビルド）は`%USERPROFILE%\.cargo`のパッケージキャッシュを共有しており、同時に走るとロック待ちでレースが起き、`beforeDevCommand`の完了（`wasm-bindgen`まで）を待たずに`devCommand`側が`gesture-app.exe`を起動してしまうことがある（起動ログで両方が`Blocking waiting for file lock on package cache`を出すのがその兆候）。
`ipc.rs`のNoteOnArgs等、editor-wasm↔Tauriコマンド間のIPC引数を変更したら、`npm run tauri dev`の自動`beforeDevCommand`任せにせず、**先に単体で明示ビルドして成功ログを確認してから**`tauri dev`を起動する：

```powershell
cd gesture-app
powershell -File scripts/build-editor-wasm.ps1   # "editor-wasm built -> ..." が出るまで確認
npm run tauri dev
```

これを怠ると、起動中のgesture-appが古いwasmのまま残り、Rust側とIPCの形が食い違って**エラーも出さずに発音しなくなる**。

アプリ内ではEキーで音色エディタのオーバーレイ表示をトグルできる（VSTと同じ`op505-ui`のノブパネル）。

### ビルド

```powershell
cd gesture-app
npm run tauri build
```

### VST3/CLAPバンドル（op505-vst）

```powershell
# cargo-nice-plugが未インストールの場合（初回のみ）
cargo install cargo-nice-plug

# バンドル生成（target\bundled\<crate>.vst3 / .clap が生成される）
cargo nice-plug bundle op505-vst --release
```

REAPER等のDAWで動作確認する場合は `target\bundled` をVST plug-in pathsに追加してRe-scanする。

---

## アーキテクチャ

### 音源レイヤー（sound-core / op505-core）

```
sound-core（基盤）
  WaveTable（1024×u16 log符号化）
  AdsrParams
  PerformanceLfo / PerformanceLfoTarget（共通Destination: 0=Pitch, 1=Volume）
  MasterEffects（Reverb/Chorus、各エンジンのrender()出力に後段適用）
  波形変換：32サンプルi8入力 → 1024サンプル対数フォーマット
  Vcoトレイト（発振エンジンの演奏ライフサイクル境界。Op505Engineが実装）
  AudioProcessorトレイト（後段DSP共通境界。MasterEffectsが実装）
  TimeEg（N点Time/Level方式EG。op505-coreの全EG系統（OP1〜4・Pitch/Cutoff/Gain FG）が使う）

op505-core（OP505実装）
  Op505Engine：4opFM合成 + フィルター + チャンネル管理
  PerformanceLfoTarget実装（共通Destination + 拡張Destination=2: TLキャリア一括、3: Cutoff）
```

コアは「この周波数でキーオン」「このパラメーターで発音」のAPIのみを提供する。
MIDI・ジェスチャー解釈・UIはコアの外側で行う。

### 音声出力（gesture-app/src-tauri）

cpalでWASAPIに直接出力。オーディオスレッドのコールバックでop505-coreを呼ぶ。

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

- `op505-ui`をeframe(WebRunner)でwasm32コンパイルし、`index.html`の`#editor-canvas`に重ねて描画
- Eキーでオーバーレイ表示をトグル（`main.js`。keydownリスナーはキャプチャフェーズ登録— エディタにフォーカスがある間はeguiがbubbleフェーズまでイベントを伝播させないため）
- パラメーター変更は`editor-wasm`内部でローカル状態を更新しつつdirtyフラグを立て、1フレームに1回`op505_set_patch`/`set_master_effects`のTauri IPCへ送る（src-tauriの`Op505Engine`/`MasterEffects`を更新）
- パフォーマンスLFO（Vキーのビブラート⇔トレモロ切替）はmain.js側のホイール/C・Bキー制御と連動し、`op505_set_performance_lfo`経由でPitch FG/質感LFOへ配線される（エディタ側のノブ操作とは独立した演奏系入力）

---

## 開発方針

- `sound-core`/`sound-fm`と`op505-core`は常にnice-plug・Tauri・cpalに無依存を保つ
- 波形フォーマットは1024×uint16_t対数で統一。変換パイプラインはsound-coreに実装
- パラメーターは全て0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、OPM/OPN/OPQ/OPZ共通のMultiple 4bitに準拠）
- `op505-vst`はフェーズ1（DAWパラメーター75個+persist EG7本、鳴らす・編集する・プリセット選択まで）・フェーズ2（NRPN・表情CC・ペダル・OP単位キーオン等のMIDI表現系）とも完了済み（2026-08-12）。`sound-core`/`op505-core`に新機能を実装したら、同じタイミングで`op505-vst`に配線しVST単体でも機能が使える状態を保つ（詳細はspec-roadmap.mdフェーズ8）
- VST3/CLAPプラグインフレームワークはnice-plug（nih-plugのフォーク、https://codeberg.org/RustAudio/nice-plug ）を使用する
- **nice-plug制限: `ProcessContext::set_parameter()`未実装（nice-plug-core 0.1.4時点）**。`process()`内からDAWパラメーターを書き戻せないため、NRPNとDAWオートメーションの共存には「シャドウフィールド＋差分検知方式」で迂回している（`last_algorithm`・`last_operator_waveforms`等）。nice-plugがこれを実装したら差分検知ロジックを削除しNRPN受信時に`context.set_parameter()`を呼ぶ方式へ移行できる
- Co-Authored-By:～はコミットメッセージに追加しない
