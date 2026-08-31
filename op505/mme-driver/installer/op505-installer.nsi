; op505 Standalone + MMEドライバ 統合インストーラ。
;
; install-mme-driver.ps1 / uninstall-mme-driver.ps1 の安全ロジックをNSISへ移植したもの
; （両スクリプトの安全ルールを弱めずに踏襲する。変更時は3ファイルを揃えて見直すこと）:
;   - Drivers32の"midi"/"midi1"（Windows MIDI Servicesの標準ドライバ）には絶対に触れない。
;     想定外の値なら中断する。
;   - 書き込み前に両方のDrivers32キーをバックアップする（reg export）。
;   - "midi2".."midi9"の空きスロット、またはop505mme.dllを指す既存スロット（再実行の冪等性）
;     にのみ書き込む。
;   - アンインストール時は値が厳密に"op505mme.dll"のスロットのみ削除する。
;
; NSISは既定で32bitプロセスとして動くため、64bit側のSystem32・レジストリを操作するには
; x64.nshの${DisableX64FSRedirection}とSetRegView 64を使う（PowerShell版で
; [Environment]::Is64BitProcessを要求していたのと同じ理由）。DLLの実際の上書きは
; System::Call経由でCopyFileW（Win32 API）を直接呼び、戻り値で成否を判定する
; （NSIS標準のFile命令はビルド時に埋め込み先が固定されるため、
; 「まず直接上書きを試み、ロード中なら失敗を検知してフォールバックする」という
; PowerShell版と同じ分岐を再現するのに向かない）。
; ロード中で上書きできない場合はDelete /REBOOTOK + Rename /REBOOTOKで次回OS再起動時の
; 差し替えを予約する（MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)のNSISネイティブ相当）。
;
; NSIS関数は呼び出し元と$0-$9レジスタを共有するため、全関数でPush/Popにより
; 使用するレジスタを退避・復元する（呼び出し元の値を破壊しない。CLAUDE.mdに倣い
; 「馴染みのない仕組みは説明を添える」の精神でここに明記する）。

Unicode true

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!insertmacro GetTime

Name "op505 Standalone"
OutFile "dist\op505-setup.exe"
InstallDir "$PROGRAMFILES64\op505"
InstallDirRegKey HKLM "Software\op505" "InstallDir"
RequestExecutionLevel admin
ShowInstDetails show
ShowUninstDetails show

!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ビルド元（build-installer.ps1が絶対パスを/Dで渡す。System::Call経由のCopyFileWは
; File命令と違い実行時のカレントディレクトリ基準で相対パスを解決してしまうため、
; 必ず絶対パスでなければならない。/D未指定時のフォールバック値はローカル動作確認用）。
!ifndef STANDALONE_EXE
  !define STANDALONE_EXE "..\..\..\target\release\op505-standalone.exe"
!endif
!ifndef DLL_X64
  !define DLL_X64 "..\dist\x64\op505mme.dll"
!endif
!ifndef DLL_X86
  !define DLL_X86 "..\dist\x86\op505mme.dll"
!endif

!define DRIVERS32_KEY "SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32"
!define WOW6432_DRIVERS32_KEY "SOFTWARE\WOW6432Node\Microsoft\Windows NT\CurrentVersion\Drivers32"
!define DLL_NAME "op505mme.dll"

Var FoundSlot
Var Slot64Name
Var Slot32Name
Var TimeStamp

; ============================================================================
; インストール
; ============================================================================

Section "Install"
    SetOutPath "$INSTDIR"
    SetOverwrite on
    File "${STANDALONE_EXE}"

    ; --- 安全チェック: midi/midi1（Windows MIDI Servicesの標準ドライバ）には絶対に触れない ---
    SetRegView 64
    Call CheckMidiSlotsUntouched
    SetRegView 32
    Call CheckMidiSlotsUntouched

    ; --- バックアップ（reg export、%LOCALAPPDATA%\op505\backup、アンインストール後も残る場所） ---
    CreateDirectory "$LOCALAPPDATA\op505\backup"
    Call MakeTimeStamp
    ExecWait 'reg.exe export "HKLM\${DRIVERS32_KEY}" "$LOCALAPPDATA\op505\backup\drivers32-64bit-$TimeStamp.reg" /y'
    ExecWait 'reg.exe export "HKLM\${WOW6432_DRIVERS32_KEY}" "$LOCALAPPDATA\op505\backup\drivers32-32bit-$TimeStamp.reg" /y'
    DetailPrint "Backed up Drivers32 to: $LOCALAPPDATA\op505\backup\"

    ; --- 空きスロット（midi2..midi9）走査、またはop505mme.dllを指す既存スロットを再利用 ---
    SetRegView 64
    Call FindSlot
    StrCpy $Slot64Name "$FoundSlot"
    ${If} $Slot64Name == ""
        MessageBox MB_OK|MB_ICONSTOP "Setup aborted: no free midi2..midi9 slot in HKLM (64-bit) Drivers32."
        Abort
    ${EndIf}

    SetRegView 32
    Call FindSlot
    StrCpy $Slot32Name "$FoundSlot"
    ${If} $Slot32Name == ""
        MessageBox MB_OK|MB_ICONSTOP "Setup aborted: no free midi2..midi9 slot in HKLM (32-bit/WOW6432Node) Drivers32."
        Abort
    ${EndIf}

    ; --- DLL配置（ロード中なら次回再起動後へフォールバック） ---
    ${DisableX64FSRedirection}
    Call InstallDllX64
    ${EnableX64FSRedirection}
    ; 32bitプロセスなので$WINDIR\System32は既定でSysWOW64へリダイレクトされる。
    Call InstallDllX86

    ; --- レジストリ登録 ---
    SetRegView 64
    WriteRegStr HKLM "${DRIVERS32_KEY}" "$Slot64Name" "${DLL_NAME}"
    SetRegView 32
    WriteRegStr HKLM "${DRIVERS32_KEY}" "$Slot32Name" "${DLL_NAME}"
    DetailPrint "Registered in Drivers32: $Slot64Name (64-bit) / $Slot32Name (32-bit)"

    ; --- インストール先の記録・ショートカット・アンインストーラ ---
    WriteRegStr HKLM "Software\op505" "InstallDir" "$INSTDIR"
    CreateDirectory "$SMPROGRAMS\op505"
    CreateShortcut "$SMPROGRAMS\op505\op505 Standalone.lnk" "$INSTDIR\op505-standalone.exe"
    CreateShortcut "$SMPROGRAMS\op505\Uninstall.lnk" "$INSTDIR\uninstall.exe"
    WriteUninstaller "$INSTDIR\uninstall.exe"

    MessageBox MB_OK|MB_ICONINFORMATION "Setup complete. Restart any existing MIDI client to see 'op505' in its MIDI OUT device list."
SectionEnd

; 現在のSetRegViewの向きで、midi/midi1が想定外の値なら中断する。
Function CheckMidiSlotsUntouched
    Push $0
    ReadRegStr $0 HKLM "${DRIVERS32_KEY}" "midi"
    ${If} $0 != ""
    ${AndIf} $0 != "wdmaud.drv"
        MessageBox MB_OK|MB_ICONSTOP "Setup aborted: Drivers32 'midi' entry has an unexpected value ($0). Refusing to touch the Windows MIDI Services configuration."
        Abort
    ${EndIf}
    ReadRegStr $0 HKLM "${DRIVERS32_KEY}" "midi1"
    ${If} $0 != ""
    ${AndIf} $0 != "wdmaud2.drv"
        MessageBox MB_OK|MB_ICONSTOP "Setup aborted: Drivers32 'midi1' entry has an unexpected value ($0). Refusing to touch the Windows MIDI Services configuration."
        Abort
    ${EndIf}
    Pop $0
FunctionEnd

; GetTime（FileFunc.nsh）はDay/Month/Year/DayOfWeek/Hour/Min/Secの順で7個返す。
; $TimeStampへ"YYYYMMDD-HHMMSS"形式の文字列を組み立てる。
Function MakeTimeStamp
    Push $0
    Push $1
    Push $2
    Push $3
    Push $4
    Push $5
    Push $6
    ${GetTime} "" "L" $0 $1 $2 $3 $4 $5 $6
    ; $0=Day $1=Month $2=Year $3=DayOfWeek $4=Hour $5=Min $6=Sec
    StrCpy $TimeStamp "$2$1$0-$4$5$6"
    Pop $6
    Pop $5
    Pop $4
    Pop $3
    Pop $2
    Pop $1
    Pop $0
FunctionEnd

; 現在のSetRegViewの向きでmidi2..midi9を走査し、空きスロットまたはop505mme.dllを指す
; 既存スロット（冪等な再実行）の名前を$FoundSlotへ返す。無ければ空文字列。
Function FindSlot
    Push $0
    Push $1
    Push $2
    StrCpy $FoundSlot ""
    StrCpy $0 2
    ${DoWhile} $0 <= 9
        StrCpy $1 "midi$0"
        ReadRegStr $2 HKLM "${DRIVERS32_KEY}" "$1"
        ${If} $2 == ""
            StrCpy $FoundSlot "$1"
            ${Break}
        ${ElseIf} $2 == "${DLL_NAME}"
            StrCpy $FoundSlot "$1"
            ${Break}
        ${EndIf}
        IntOp $0 $0 + 1
    ${Loop}
    Pop $2
    Pop $1
    Pop $0
FunctionEnd

; $WINDIR\System32\op505mme.dll（呼び出し時点のリダイレクト設定に従う）へDLL_X64を配置する。
; CopyFileWで直接上書きを試み、ロード中で失敗した場合のみ次回再起動後フォールバックへ回る。
Function InstallDllX64
    Push $0
    System::Call 'kernel32::CopyFileW(w "${DLL_X64}", w "$WINDIR\System32\${DLL_NAME}", i 0) i .r0'
    ${If} $0 == 0
        DetailPrint "$WINDIR\System32\${DLL_NAME} is in use; it will be replaced after the next PC restart."
        SetOutPath "$WINDIR\System32"
        SetOverwrite try
        File "/oname=${DLL_NAME}.op505-pending" "${DLL_X64}"
        Delete /REBOOTOK "$WINDIR\System32\${DLL_NAME}"
        Rename /REBOOTOK "$WINDIR\System32\${DLL_NAME}.op505-pending" "$WINDIR\System32\${DLL_NAME}"
    ${Else}
        DetailPrint "Installed: $WINDIR\System32\${DLL_NAME}"
    ${EndIf}
    Pop $0
FunctionEnd

; 同上、32bit版（呼び出し時点でX64FSRedirectionが有効=既定のままなのでSysWOW64へ届く）。
Function InstallDllX86
    Push $0
    System::Call 'kernel32::CopyFileW(w "${DLL_X86}", w "$WINDIR\System32\${DLL_NAME}", i 0) i .r0'
    ${If} $0 == 0
        DetailPrint "$WINDIR\SysWOW64\${DLL_NAME} is in use; it will be replaced after the next PC restart."
        SetOutPath "$WINDIR\System32"
        SetOverwrite try
        File "/oname=${DLL_NAME}.op505-pending" "${DLL_X86}"
        Delete /REBOOTOK "$WINDIR\System32\${DLL_NAME}"
        Rename /REBOOTOK "$WINDIR\System32\${DLL_NAME}.op505-pending" "$WINDIR\System32\${DLL_NAME}"
    ${Else}
        DetailPrint "Installed: $WINDIR\SysWOW64\${DLL_NAME}"
    ${EndIf}
    Pop $0
FunctionEnd

; ============================================================================
; アンインストール
; ============================================================================

Section "Uninstall"
    SetRegView 64
    Call un.RemoveOp505Slots
    SetRegView 32
    Call un.RemoveOp505Slots

    ${DisableX64FSRedirection}
    Delete /REBOOTOK "$WINDIR\System32\${DLL_NAME}"
    ${EnableX64FSRedirection}
    ; 32bitプロセスの既定リダイレクトでSysWOW64へ届く。
    Delete /REBOOTOK "$WINDIR\System32\${DLL_NAME}"

    Delete "$SMPROGRAMS\op505\op505 Standalone.lnk"
    Delete "$SMPROGRAMS\op505\Uninstall.lnk"
    RMDir "$SMPROGRAMS\op505"

    Delete "$INSTDIR\op505-standalone.exe"
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"

    DeleteRegKey HKLM "Software\op505"

    MessageBox MB_OK|MB_ICONINFORMATION "Uninstall complete. Restart any existing MIDI client to remove 'op505' from its device list."
SectionEnd

; 現在のSetRegViewの向きでmidi2..midi9を走査し、値が厳密に"op505mme.dll"のスロットのみ削除する。
Function un.RemoveOp505Slots
    Push $0
    Push $1
    Push $2
    StrCpy $0 2
    ${DoWhile} $0 <= 9
        StrCpy $1 "midi$0"
        ReadRegStr $2 HKLM "${DRIVERS32_KEY}" "$1"
        ${If} $2 == "${DLL_NAME}"
            DeleteRegValue HKLM "${DRIVERS32_KEY}" "$1"
        ${EndIf}
        IntOp $0 $0 + 1
    ${Loop}
    Pop $2
    Pop $1
    Pop $0
FunctionEnd
