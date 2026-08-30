# Installs the op505 MME driver (Drivers32 user-mode DLL, phase-0 spike build) for both
# 64-bit and 32-bit WinMM clients. Must run elevated, from a 64-bit PowerShell.
#
# Safety rules (do not weaken without re-reading the design notes):
#   - Never touches the "midi" / "midi1" entries (wdmaud.drv / wdmaud2.drv), which belong to
#     Windows MIDI Services. Aborts if they are missing or hold an unexpected value.
#   - Only writes to a free "midi2".."midi9" slot, or reuses a slot that already points at
#     op505mme.dll (idempotent re-run).
#   - Backs up both Drivers32 keys via `reg export` before any write.

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$dllX64 = Join-Path $scriptDir "dist\x64\op505mme.dll"
$dllX86 = Join-Path $scriptDir "dist\x86\op505mme.dll"

if (-not (Test-Path $dllX64)) { throw "Missing $dllX64. Build with build-mme-driver.ps1 first." }
if (-not (Test-Path $dllX86)) { throw "Missing $dllX86. Build with build-mme-driver.ps1 first." }

$principal = New-Object System.Security.Principal.WindowsPrincipal([System.Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "This script must run elevated (Administrator)."
}

if (-not [Environment]::Is64BitProcess) {
    throw "Run this from a 64-bit PowerShell (not a WOW64 32-bit host); System32 would resolve to SysWOW64 otherwise."
}

$key64 = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32"
$key32 = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows NT\CurrentVersion\Drivers32"

# --- Safety check: never touch midi / midi1 (Windows MIDI Services in-box drivers) ---
foreach ($key in @($key64, $key32)) {
    $props = Get-ItemProperty -Path $key -ErrorAction Stop
    if ($props.midi -and $props.midi -ne "wdmaud.drv") {
        throw "$key : 'midi' is '$($props.midi)', expected 'wdmaud.drv'. Aborting to avoid touching Windows MIDI Services entries."
    }
    if ($props.midi1 -and $props.midi1 -ne "wdmaud2.drv") {
        throw "$key : 'midi1' is '$($props.midi1)', expected 'wdmaud2.drv'. Aborting to avoid touching Windows MIDI Services entries."
    }
}

# --- Backup ---
$backupDir = Join-Path $env:LOCALAPPDATA "op505"
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backup64 = Join-Path $backupDir "drivers32-64bit-$timestamp.reg"
$backup32 = Join-Path $backupDir "drivers32-32bit-$timestamp.reg"
reg export "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32" $backup64 /y | Out-Null
reg export "HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows NT\CurrentVersion\Drivers32" $backup32 /y | Out-Null
Write-Output "Backed up Drivers32 to:"
Write-Output "  $backup64"
Write-Output "  $backup32"

# --- Find a free slot (midi2..midi9), or reuse an existing op505mme.dll entry (idempotent) ---
function Find-Slot([string]$key) {
    $existing = Get-ItemProperty -Path $key -ErrorAction Stop
    for ($i = 2; $i -le 9; $i++) {
        $name = "midi$i"
        $value = $existing.$name
        if (-not $value) { return [pscustomobject]@{ Name = $name; Reused = $false } }
        if ($value -eq "op505mme.dll") { return [pscustomobject]@{ Name = $name; Reused = $true } }
    }
    return $null
}

$slot64 = Find-Slot $key64
$slot32 = Find-Slot $key32
if (-not $slot64) { throw "$key64 : no free midi2..midi9 slot." }
if (-not $slot32) { throw "$key32 : no free midi2..midi9 slot." }

# --- Copy DLLs ---
$sys32 = Join-Path $env:WINDIR "System32\op505mme.dll"
$syswow64 = Join-Path $env:WINDIR "SysWOW64\op505mme.dll"
Copy-Item -Path $dllX64 -Destination $sys32 -Force
Copy-Item -Path $dllX86 -Destination $syswow64 -Force
Write-Output "Copied:"
Write-Output "  $dllX64 -> $sys32"
Write-Output "  $dllX86 -> $syswow64"

# --- Register ---
Set-ItemProperty -Path $key64 -Name $slot64.Name -Value "op505mme.dll" -Type String
Set-ItemProperty -Path $key32 -Name $slot32.Name -Value "op505mme.dll" -Type String
Write-Output ("Registered as {0} (64-bit) / {1} (32-bit). Reused={2}/{3}" -f $slot64.Name, $slot32.Name, $slot64.Reused, $slot32.Reused)

Write-Output ""
Write-Output "Done. Restart any WinMM client apps (e.g. Domino) so they re-enumerate MIDI OUT devices."
