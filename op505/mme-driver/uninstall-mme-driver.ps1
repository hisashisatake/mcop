# Removes the op505 MME driver registrations and DLLs installed by install-mme-driver.ps1.
# Only ever removes Drivers32 slots whose value is exactly "op505mme.dll" - never touches
# "midi" / "midi1" (Windows MIDI Services) or any other entry.

$ErrorActionPreference = "Stop"

$principal = New-Object System.Security.Principal.WindowsPrincipal([System.Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "This script must run elevated (Administrator)."
}
if (-not [Environment]::Is64BitProcess) {
    throw "Run this from a 64-bit PowerShell."
}

$key64 = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Drivers32"
$key32 = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows NT\CurrentVersion\Drivers32"

function Remove-Op505Entries([string]$key) {
    $existing = Get-ItemProperty -Path $key -ErrorAction Stop
    for ($i = 2; $i -le 9; $i++) {
        $name = "midi$i"
        if ($existing.$name -eq "op505mme.dll") {
            Remove-ItemProperty -Path $key -Name $name
            Write-Output "$key : removed $name"
        }
    }
}

Remove-Op505Entries $key64
Remove-Op505Entries $key32

foreach ($path in @((Join-Path $env:WINDIR "System32\op505mme.dll"), (Join-Path $env:WINDIR "SysWOW64\op505mme.dll"))) {
    if (Test-Path $path) {
        try {
            Remove-Item -Path $path -Force
            Write-Output "Removed $path"
        } catch {
            Write-Warning "Could not delete $path (likely still loaded by a running app). It is unregistered and harmless, but the file itself is orphaned until the holder process exits."
        }
    }
}

Write-Output "Uninstall complete. Restart any WinMM client apps to drop the device."
