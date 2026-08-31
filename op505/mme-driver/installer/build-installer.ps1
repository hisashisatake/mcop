# Builds the op505 Standalone + MME driver installer (op505-setup.exe) via NSIS.
#
# Prerequisites (build these first, this script only packages them):
#   cargo build --release -p op505-standalone
#   cd op505\mme-driver; cargo build --release; cargo build --release --target i686-pc-windows-msvc
#   (see CLAUDE.md "i686" section for the rustup-managed cargo path needed for the x86 build)
#   Copy-Item target\release\op505mme.dll dist\x64\op505mme.dll
#   Copy-Item target\i686-pc-windows-msvc\release\op505mme.dll dist\x86\op505mme.dll

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..\..\..")

$standaloneExe = Join-Path $repoRoot "target\release\op505-standalone.exe"
$dllX64 = Join-Path $scriptDir "..\dist\x64\op505mme.dll"
$dllX86 = Join-Path $scriptDir "..\dist\x86\op505mme.dll"

if (-not (Test-Path $standaloneExe)) { throw "Missing $standaloneExe. Run: cargo build --release -p op505-standalone" }
if (-not (Test-Path $dllX64)) { throw "Missing $dllX64. Build op505-mme-driver for x64 and copy it to dist\x64 first." }
if (-not (Test-Path $dllX86)) { throw "Missing $dllX86. Build op505-mme-driver for i686-pc-windows-msvc and copy it to dist\x86 first." }

# Resolve to absolute paths: the .nsi's System::Call CopyFileW calls resolve relative paths
# against the installer's runtime working directory (not the .nsi's own directory, unlike
# File statements), so a relative !define here would silently make the "try a direct
# overwrite first" fast path always fail at install time.
$standaloneExeAbs = (Resolve-Path $standaloneExe).Path
$dllX64Abs = (Resolve-Path $dllX64).Path
$dllX86Abs = (Resolve-Path $dllX86).Path

$makensis = "C:\Program Files (x86)\NSIS\makensis.exe"
if (-not (Test-Path $makensis)) {
    $cmd = Get-Command makensis -ErrorAction SilentlyContinue
    if (-not $cmd) { throw "makensis.exe not found. Install NSIS (winget install NSIS.NSIS)." }
    $makensis = $cmd.Source
}

New-Item -ItemType Directory -Force -Path (Join-Path $scriptDir "dist") | Out-Null

& $makensis "/DSTANDALONE_EXE=$standaloneExeAbs" "/DDLL_X64=$dllX64Abs" "/DDLL_X86=$dllX86Abs" (Join-Path $scriptDir "op505-installer.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE" }

Write-Output "Built: $(Join-Path $scriptDir 'dist\op505-setup.exe')"
