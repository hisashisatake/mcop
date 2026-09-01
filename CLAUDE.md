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

### wasm32（op505/ui/preview-wasm用）

このマシンのデフォルトcargo/rustcはscoop版（rustup未管理、`x86_64-pc-windows-msvc`のみ）。
`op505/ui/preview-wasm`（panel.xml DSLプレビューツールのegui-wasmラッパー）のビルドには
wasm32-unknown-unknownターゲットが必要なため、**rustupを追加導入**し、PATHは変更せずrustup配下の
cargoをフルパスで使う運用にしている（旧`gesture-app/editor-wasm`もこの構成を使っていたが、
gesture-appのMIDI送信化に伴い2026-09-01に削除済み。詳細はgesture-app節参照）。

```powershell
# 初回セットアップ（導入済みなら不要）
# rustup-init.exeはrust-lang公式(https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe)から取得
# （scoopのrustupマニフェストはハッシュが古く検証に失敗することがある）
.\rustup-init.exe -y --no-modify-path --default-toolchain stable
$rustup = "$env:USERPROFILE\.cargo\bin\rustup.exe"
& $rustup target add wasm32-unknown-unknown

$rustupCargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$env:RUSTFLAGS = "-C debuginfo=0"  # PDBサイズ制限(LNK1140)回避
& $rustupCargo install wasm-bindgen-cli --version 0.2.126 --force  # preview-wasm/Cargo.tomlのwasm-bindgen固定バージョンと一致させる
Remove-Item Env:\RUSTFLAGS
```

`cargo`コマンドは引き続きscoop版が解決される（`--no-modify-path`でPATH変更を避けたため）。
wasm32ビルドは`op505/tools/xml-panel-dsl/build-preview-wasm.ps1`が`%USERPROFILE%\.cargo\bin\cargo.exe`/`wasm-bindgen.exe`をフルパスで呼ぶ。

### i686（op505-mme-driver用、32bit WinMMホスト対応）

`op505/mme-driver`（Domino等がロードするMMEドライバDLL）はx64/x86両方のビルドが必要
（64bitプロセスはSystem32、32bitプロセスはWOW64経由でSysWOW64のDLLを読むため）。
x86ビルドには`i686-pc-windows-msvc`ターゲットが要るが、**scoop版cargo/rustcにはこの
ターゲットのstdライブラリが無い**（`rustup target list --installed`が使えるのは
上記wasm32セクションで導入したrustup管理版のみ）。i686も同じrustup管理版へ
`target add`し、ビルドはrustup版cargoをフルパスで呼ぶ。

```powershell
$rustup = "$env:USERPROFILE\.cargo\bin\rustup.exe"
& $rustup target add i686-pc-windows-msvc   # 初回のみ

cd op505/mme-driver
$rustupCargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
& $rustupCargo build --release                                        # x64 -> target\release\op505mme.dll
& $rustupCargo build --release --target i686-pc-windows-msvc            # x86 -> target\i686-pc-windows-msvc\release\op505mme.dll
```

`op505/mme-driver`はワークスペースのexcludeメンバーのためscoop版cargoでも一見ビルドが通ってしまうが、
x64ビルドもrustup版cargoで揃えること（rustcのバージョン差でx64/x86 DLLの生成コンパイラが
食い違うのを避けるため）。生成物は`dist\x64\op505mme.dll`/`dist\x86\op505mme.dll`へ手動コピーし、
`install-mme-driver.ps1`（要管理者権限・64bit PowerShell）がSystem32/SysWOW64へ配置しDrivers32へ登録する。

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
- **波形メモリ専用音色バンク**: OP1のみを鳴らす1オペレーター音色を予約バンク（Bank=16383）に
  用意する仕組み（エンジンのモードではなく通常パッチのバンク）。ym38x6-coreの
  `waveform_memory_patch`はym38x6削除に伴い現存しないが、2026-08-25にop505へ移植済み
  （`op505/tools/patchlab/python/waveform_memory_bank.py`が実体の`.op505`ファイルを生成する方式。
  op505は「特殊バンクを実行時コードでフォールバック生成する」パターン自体を廃止しているため、
  ym38x6と違いop505-core/vst/gesture-appのコード変更は不要）
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
      time_eg          ← TimeEg（N点Time/Level方式EG）。`texture`フィールド（0=OFF/1=S&H/2=Random/3=Chaos）が
                            旧質感LFOのS&H/Random/Chaos波形の後継（2026-08-20退役、詳細はspec-sound.md参照）。
                            `auto_release`フィールド（0=OFF/N≥1=保持区間通過でnote-off非依存の自動
                            リリース）はGM2リズムチャンネル向けのワンショット化機構（2026-08-24新設、
                            詳細はspec-sound.md「TimeEgのワンショット化」節）
    fm/                ← クレート名sound-fm。FM合成チップ間で共有する、EG非依存の汎用部品（op505-coreが依存）
      algorithm/mapping/waveform ← アルゴリズム結線表・TL/KSR等のパラメーターマッピング・波形生成。
                            旧chip_lfoモジュール（op505エンジンから退役済み・2026-08-20の実機LFO
                            レジスタ値変換テーブル）は、実際の直接利用者がop505グループに閉じたため
                            2026-09-01にop505-core::modulation_curvesへ移設済み（詳細はspec-sound.md参照）
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
                          CC/NRPN解釈は`op505-midi`クレートへ切り出し済み（フェーズ5.5、旧`src/midi.rs`は削除）。
                          2026-08-26、NRPN/RPN選択状態を含むMIDI受信状態を`op505_midi::ChannelState`
                          （smf2op505/standaloneと共有）へ全面移行し、全16 MIDIチャンネル完全独立化
                          （エフェクト系NRPN(0,2)〜(0,8)とCC91/93はMasterEffectsが1個のためグローバル
                          のまま。詳細はspec-fm.md 8章⑤）。
                          2026-08-27、PRESETSパネルをgesture-app同仕様のバンクレジストリ方式へ刷新
                          （Open/Save/Save As/+ New Voice/Deleteの5操作、`rfd`クレートのネイティブ
                          ダイアログ）。保存内容はMIDI Program Change解決側（オーディオスレッド）へも
                          `Arc<RwLock<Op505PresetBank>>`+dirtyフラグ経由で即時反映される（`cached_egs`/
                          `try_read()`と同じ非ブロッキングパターン）。レジストリ本体は`op505-core`の
                          `preset_registry`モジュールへ昇格し、gesture-appと共有（詳細はspec-fm.md 8章⑥）
    midi/              ← CC/NRPN解釈の共有クレート（クレート名op505-midi。フェーズ5.5新設）。
                          `op505-vst`と`op505/tools/smf2op505`の両方が参照する、fork-on-write方針の
                          限定的な例外（VSTと参照実装の解釈が食い違うと正誤の基準が消えるため。
                          詳細はspec-fm.md 8章⑤）。`ControlTarget` enumがNRPNアドレス表を一元化し、
                          `_ =>`を使わせない全列挙で解釈のずれを構造的に防ぐ。依存は`op505-core`と
                          `sound-fm`の2本のみ。GM2リズムチャンネル用のBank Select(CC0/32)+
                          Program Change判定（`rhythm`モジュール、`ChannelProgramState`状態機械）も
                          同じ理由でここに実装する（詳細はspec-sound.md「リズム（ドラム）
                          チャンネル」節）
    standalone/        ← op505をDominoから直接鳴らす常駐MIDIアプリ（クレート名op505-standalone。
                          フェーズ13新設）。MIDI入力元は`midi_source.rs`の`MidiSource`トレイトで
                          抽象化し、既存のmidir経由入力（実機MIDIキーボード/loopMIDI、
                          `sources/midir_src.rs`）とmme-driver経由の名前付きパイプ入力
                          （`sources/pipe_src.rs`）を同じ`MidiQueue`へ合流させる。
                          `tray-icon`をwinit抜き（生のWin32メッセージループ）で使い
                          タスクトレイ常駐化（`#![windows_subsystem = "windows"]`でコンソール
                          非表示）。設定は`%APPDATA%\op505\standalone.toml`、ログは
                          `%LOCALAPPDATA%\op505\standalone.log`（コンソールが無いため）。
                          CC/NRPN解釈はop505-vst/smf2op505と同じ`op505-midi::ChannelState`を使用
    mme-driver/        ← op505をWindowsのMIDI OUTデバイス一覧へ「op505」として登録する
                          ユーザーモードMMEドライバ（クレート名op505-mme-driver、
                          `[lib] name="op505mme"`。Drivers32方式、VirtualMIDISynthと同じ
                          カーネルドライバではない普通のDLL。フェーズ13新設）。相手アプリ
                          （Domino等）のプロセス空間にロードされる薄いシムで、MIDIバイト列を
                          名前付きパイプ`\\.\pipe\op505.mme.v1`経由でstandaloneへ転送するだけ。
                          エンジン・音声出力はstandalone側が単独所有する。ワークスペースの
                          `panic="abort"`は他プロセスに住むDLLには致命的なためルートCargo.toml
                          のworkspace excludeへ登録し、自クレートのみ`panic="unwind"`＋全
                          エクスポート関数を`catch_unwind`で保護。x64/x86両方のビルドが必要
                          （`i686-pc-windows-msvc`ターゲット、後述「i686」節）。
                          `install-mme-driver.ps1`/`uninstall-mme-driver.ps1`がDrivers32の
                          `midi2`〜`midi9`空きスロットへ登録/解除する（`midi`/`midi1`＝Windows
                          MIDI Servicesの標準ドライバは絶対に触らない）。DLLがロード中で
                          置換できない場合は`MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)`で
                          次回再起動時の差し替えへフォールバックする。standalone自身もWinMM
                          MIDI入力を扱うため起動しているとop505mme.dll自身をロードしてしまい、
                          DLL更新時はstandaloneを一旦終了させる必要がある
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
                          CC/NRPN解釈は`op505-midi`を参照するため`op505-vst`と常に同じ解釈になる）。
                          `--drum-bank <kit.op505>`（複数回指定可）でGM2リズムキットバンクを読み込み、
                          ch10のBank Select+Program ChangeでGM2リズムチャンネルとして再生できる
                          （未指定時はリズム機能を完全に無効化、既存出力はビット不変）
      xml-panel-dsl/   ← panel.xmlのXML DSL編集・プレビューツール（自己完結HTML + preview-native、
                          op505-ui向け。詳細はREADME.md）
  gesture-app/         ← 作曲支援Tauriアプリ（ジェスチャーをMIDIへ変換してop505-standaloneへ送る
                          だけのコントローラー。エンジン・音声出力は持たない、詳細はgesture-app節）
    src-tauri/         ← Rustバックエンド（`midi_out.rs`が名前付きパイプ`\\.\pipe\op505.mme.v1`
                          経由でstandaloneへ標準MIDIバイト列を送信する）
    src/               ← フロントエンド（ジェスチャーUI）
```

Cargoのワークスペースメンバーパスとパッケージ名は独立しているため、`cargo check -p sound-core`等は
ディレクトリ位置に関わらずそのまま使える。

`sound-core`・`sound-fm`・`ui-layout`・`ui-codegen`・`op505-core`・`op505-midi`はnice-plugにも
Tauriにも依存しない純粋なRustライブラリ。音源エンジンの変更は`sound/`と`op505/core/`に閉じる
（`op505-midi`はMIDI解釈層のため厳密には「音源エンジン」ではないが、同じく純粋ライブラリの原則を守る）。
`ui-core`・`op505-ui`はegui+sound-coreに依存し、nice-plug/Tauri/cpalに依存しない
（VSTとstandaloneの音色エディタが共有する描画ロジック。sound-coreはEG形状プレビュー計算用。
gesture-appは2026-09-01のMIDI送信化でエディタごとエンジンを手放したため、もうこの依存に含まれない）。

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

### アプリ起動

gesture-appはエンジンを持たないMIDIコントローラーのため、先に`op505-standalone`を起動しておく
（未起動でも`gesture-app`はパイプ接続失敗時に自動起動を試みる、`midi_out.rs`の`ensure_started`参照。
ただし確実なのは手動で先に立ち上げておくこと）。

```powershell
cargo build --release -p op505-standalone
Start-Process target\release\op505-standalone.exe

cd gesture-app
npm run tauri dev
```

音色エディタ（旧Eキーのオーバーレイ）はgesture-app側には無い。standaloneのタスクトレイメニューから開く
（`op505/standalone/src/editor/`、詳細はop505/standalone節）。

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

### op505-standalone（常駐MIDIアプリ）とMMEドライバのインストール

```powershell
# standaloneのビルド・起動（タスクトレイに常駐。終了はトレイメニューから）
cargo build --release -p op505-standalone
Start-Process target\release\op505-standalone.exe
```

Dominoからop505をMIDI OUTデバイスとして選べるようにするには、`op505/mme-driver`をx64/x86両方
ビルドし（前述「i686」節）、`dist\x64`/`dist\x86`へ配置してから管理者権限の**64bit** PowerShellで
`install-mme-driver.ps1`を実行する（Drivers32の空きスロットへ登録、`midi`/`midi1`＝Windows
MIDI Servicesの標準ドライバは不可侵）。解除は`uninstall-mme-driver.ps1`。

**DLL更新時の注意**: `op505-standalone`自身もWinMM MIDI入力を扱うため、起動しているとWinMMの
一般挙動でDrivers32登録済みの`op505mme.dll`を自分自身にもロードしてしまう。DLLを再ビルドして
再インストールする前に、必ず`op505-standalone`を一旦終了させること（ロード中のままだと
`install-mme-driver.ps1`のコピーが失敗するか、`MoveFileEx`フォールバックで次回OS再起動まで
反映が遅延する）。DLL更新後にDominoなどの既存クライアントを再起動すればMIDI OUTデバイス
一覧が再列挙される（OS再起動は通常不要）。

### NSIS統合インストーラ（op505-setup.exe）

`op505/mme-driver/installer/`に、standalone.exe配置＋MMEドライバのDrivers32登録を1本の
exeへ統合したNSISインストーラがある。上記2つの手順（PowerShellスクリプト2本）を
エンドユーザー向けに1本化したもので、中身のロジック（安全チェック・バックアップ・
空きスロット走査等）は`install-mme-driver.ps1`/`uninstall-mme-driver.ps1`と同一。

```powershell
# 事前ビルド（standalone.exeとmme-driver x64/x86 DLLをdistへ配置しておく）
cargo build --release -p op505-standalone
cd op505\mme-driver
cargo build --release
$rustupCargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
& $rustupCargo build --release --target i686-pc-windows-msvc
Copy-Item target\release\op505mme.dll dist\x64\op505mme.dll
Copy-Item target\i686-pc-windows-msvc\release\op505mme.dll dist\x86\op505mme.dll
cd ..\..\..

# インストーラのビルド（NSIS 3.11、winget install NSIS.NSISで導入可能）
pwsh -File op505\mme-driver\installer\build-installer.ps1
# -> op505\mme-driver\installer\dist\op505-setup.exe（管理者権限で実行、/Sでサイレントインストール）
```

**`.nsi`ファイルを編集したらUTF-8 BOMを再付与すること**: NSISの`Unicode true`はスクリプト
ファイル自体がBOM付きエンコードであることを要求する。WriteツールはBOM無しUTF-8で保存するため、
日本語コメントを含む本ファイルを編集した直後にBOM無しのままビルドすると
`Bad text encoding`エラーになる（PowerShellスクリプトのBOM問題と同種の罠、CLAUDE.md冒頭
「PowerShellスクリプト実行」参照）。

```powershell
$path = "op505\mme-driver\installer\op505-installer.nsi"
$content = [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8)
[System.IO.File]::WriteAllText($path, $content, (New-Object System.Text.UTF8Encoding($true)))
```

**罠（発見済み、修正済みだが再発に注意）**: `.nsi`内でロード中DLLの直接上書き成否を判定する
`System::Call`経由の`CopyFileW`は、`File`命令と違って**実行時のカレントディレクトリを基準に
相対パスを解決する**。`!define DLL_X64 "..\dist\x64\op505mme.dll"`のような相対パスのままだと
直接上書きが常に失敗し、常に`Delete`/`Rename`の`/REBOOTOK`フォールバック経路だけが動く
（最終的に正しく配置はされるため気づきにくいが、「ロードされていなければ直接上書き」という
設計意図が失われる）。`build-installer.ps1`が絶対パスを計算し`makensis /DDLL_X64=...`で
渡す方式で解消済み。`.nsi`側で新たに`System::Call`にファイルパスを渡す処理を追加する場合は
必ず絶対パスであることを確認する。

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
  固定音階（fixed_note_enable/fixed_note/fixed_note_fine）：note_onの周波数を無視し固定ピッチで
    鳴らす。GM2リズムチャンネル用（2026-08-24新設、詳細はspec-sound.md「固定音階」節）
```

コアは「この周波数でキーオン」「このパラメーターで発音」のAPIのみを提供する。
MIDI・ジェスチャー解釈・UIはコアの外側で行う。

### gesture-app（ジェスチャー→MIDIコントローラー）

2026-09-01のMIDI送信化（Step 3）で、gesture-appは`op505-standalone`（op505/standalone節参照）を
唯一のエンジン・音声出力の所有者とするコントローラーへ再設計された。cpal/`Op505Engine`の直接所有・
`editor-wasm`（音色エディタ）は撤去済み。詳細な設計判断はmemory
`project_gesture_app_controller_roadmap.md`参照。

- `src-tauri/src/midi_out.rs`が名前付きパイプ`\\.\pipe\op505.mme.v1`（`op505-mme-driver`と同じ
  フレーム形式）経由で標準MIDIバイト列をstandaloneへ送信する（`op505/mme-driver/src/client.rs`と
  同型のライタースレッド＋200ms再接続リトライ）。パイプ未接続なら同ディレクトリの
  `op505-standalone.exe`を自動起動する（`ensure_started`、単一インスタンスMutexがあるため
  二重起動しても安全）。
- Tauriコマンド（`note_on`/`note_off`/`op505_set_performance_lfo`/`set_master_effects`/
  `op505_set_program`）はいずれも生MIDIメッセージへ変換して送るだけで、エンジンには触れない。
  演奏系LFO（Vキーのビブラート⇔トレモロ切替）のEG形状自体はもう組み立てない——standalone側の
  `op505-midi`が「プリセットが形を持たずCC由来のdepthが正のときだけ標準形状を自動生成する」
  演奏用FGフォールバック（Step 2で実装済み）に委ねている。
- `set_tempo`（タップテンポ）はMIDI経由の対応先が無いため撤去済み（TimeEgのテンポ同期は
  MIDI Clock等を別途実装しない限り効かない、既知の制約）。
- `.op505`プリセットの一覧・名前解決（`op505_presets.rs`）は読み取り専用で残す。実際の音色選択は
  Bank Select(CC0/32) + Program Changeとして送るだけで、ファイル編集（Open/Save/Save As/
  +New Voice/Delete）はstandaloneのトレイ起動音色エディタが担う。

### ジェスチャーUI

- キャリブレーションベース（C-F-Gの3点で座標系を定義）
- グリッドなし
- マウス版: 縦軸=ルート音、横軸=コード種類
- タッチ版（フェーズ10・タブレット対応）: 指の間隔=インターバル、指の移動=ルート音シフト
- ∞ジェスチャー: 軌跡がそのままF-Numberに追従（ビブラート・装飾音）。2026-09-01時点で未実装
  （`midiFreq(root.midi+interval)`相当のMIDIノート番号ベースの12平均律のみ）

---

## 開発方針

- `sound-core`/`sound-fm`と`op505-core`は常にnice-plug・Tauri・cpalに無依存を保つ
- 波形フォーマットは1024×uint16_t対数で統一。変換パイプラインはsound-coreに実装
- パラメーターは全て0〜255（8bit）統一。例外は周波数（オクターブ3bit + F-Number 13bit = 16bit、常にOP単位×4）とMUL（0〜15、OPM/OPN/OPQ/OPZ共通のMultiple 4bitに準拠）
- `op505-vst`はフェーズ1（DAWパラメーター75個+persist EG7本、鳴らす・編集する・プリセット選択まで）・フェーズ2（NRPN・表情CC・ペダル・OP単位キーオン等のMIDI表現系）とも完了済み（2026-08-12）。`sound-core`/`op505-core`に新機能を実装したら、同じタイミングで`op505-vst`に配線しVST単体でも機能が使える状態を保つ（詳細はspec-roadmap.mdフェーズ8）
- VST3/CLAPプラグインフレームワークはnice-plug（nih-plugのフォーク、https://codeberg.org/RustAudio/nice-plug ）を使用する
- **nice-plug制限: `ProcessContext::set_parameter()`未実装（nice-plug-core 0.1.4時点）**。`process()`内からDAWパラメーターを書き戻せないため、NRPNとDAWオートメーションの共存には「シャドウフィールド＋差分検知方式」で迂回している（`last_algorithm`・`last_operator_waveforms`等）。nice-plugがこれを実装したら差分検知ロジックを削除しNRPN受信時に`context.set_parameter()`を呼ぶ方式へ移行できる
- Co-Authored-By:～はコミットメッセージに追加しない
