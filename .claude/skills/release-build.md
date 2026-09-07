# release-build

op505-vstのVST3/CLAPバンドル生成、op505-standalone常駐MIDIアプリとMMEドライバのインストール、
NSIS統合インストーラ（op505-setup.exe）のビルド手順をまとめたスキル。
いずれもリリースビルド・配布物作成のときだけ必要な手順で、日常の開発では使わない。

`.claude/skills/`にスキル定義を収録している。スラッシュコマンドとして使うには
`~/.claude/skills/`にコピーが必要（プロジェクト内の定義はドキュメント兼Claude参照用）。

## VST3/CLAPバンドル（op505-vst）

```powershell
# cargo-nice-plugが未インストールの場合（初回のみ）
cargo install cargo-nice-plug

# バンドル生成（target\bundled\<crate>.vst3 / .clap が生成される）
cargo nice-plug bundle op505-vst --release
```

REAPER等のDAWで動作確認する場合は `target\bundled` をVST plug-in pathsに追加してRe-scanする。

**REAPERでGUIを目視確認する際の罠（nice-plug VST3のリサイズ非対称性）**: nice-plug 0.1.9の
VST3ラッパーは「プラグイン発リサイズ（プラグイン自身の右下角ドラッグ）」のみに対応し、
「ホスト発リサイズ（REAPER本体ウィンドウやFXウィンドウの最大化・枠ドラッグ）」には追随しない
（`onSize()`が未実装のスタブ、詳細はmemory`project_niceplug_vst3_clap_resize_unimplemented`）。
REAPER本体を最大化しただけでは埋め込みGUIは初期サイズのまま変わらず、その状態でスクリーン
ショットを撮ると、本来は正常なパネルが表示領域の都合で意図せず一部だけ潰れて見えたり、
無地に見えたりすることがある（実際に2026-09-02、この状態を「OP1のTimeEgエディタがGRAPHタブで
描画されない」という**実在しないバグ**と誤認した事例あり、詳細はmemory
`feedback_niceplug_vst3_unresized_gui_false_positive`）。GUI確認・スクリーンショット取得の
前には**必ずプラグイン自身の右下リサイズハンドルを手動（またはgui-probe経由で）ドラッグして
広げてから**判定すること。何かが「描画されていない」ように見えたら、まずリサイズ不足を疑う。

## op505-standalone（常駐MIDIアプリ）とMMEドライバのインストール

```powershell
# standaloneのビルド・起動（タスクトレイに常駐。終了はトレイメニューから）
cargo build --release -p op505-standalone
Start-Process target\release\op505-standalone.exe
```

Dominoからop505をMIDI OUTデバイスとして選べるようにするには、`op505/mme-driver`をx64/x86両方
ビルドし（CLAUDE.md「i686」節）、`dist\x64`/`dist\x86`へ配置してから管理者権限の**64bit** PowerShellで
`install-mme-driver.ps1`を実行する（Drivers32の空きスロットへ登録、`midi`/`midi1`＝Windows
MIDI Servicesの標準ドライバは不可侵）。解除は`uninstall-mme-driver.ps1`。

**DLL更新時の注意**: `op505-standalone`自身もWinMM MIDI入力を扱うため、起動しているとWinMMの
一般挙動でDrivers32登録済みの`op505mme.dll`を自分自身にもロードしてしまう。DLLを再ビルドして
再インストールする前に、必ず`op505-standalone`を一旦終了させること（ロード中のままだと
`install-mme-driver.ps1`のコピーが失敗するか、`MoveFileEx`フォールバックで次回OS再起動まで
反映が遅延する）。DLL更新後にDominoなどの既存クライアントを再起動すればMIDI OUTデバイス
一覧が再列挙される（OS再起動は通常不要）。

## NSIS統合インストーラ（op505-setup.exe）

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
