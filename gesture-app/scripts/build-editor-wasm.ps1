# Builds editor-wasm (egui patch editor, wasm32-unknown-unknown only crate) and emits
# JS bindings into gesture-app/src/editor-wasm/. Invoked from tauri.conf.json's
# beforeDevCommand/beforeBuildCommand.
#
# Uses cargo/wasm-bindgen under rustup's install (scoop's cargo has no wasm32 target).
# PATH is intentionally left unmodified, so full paths are required here.
# Note: this script is kept ASCII-only because the Tauri-spawned powershell.exe
# (Windows PowerShell 5.1, not pwsh) misreads BOM-less UTF-8 files using the system
# codepage, garbling non-ASCII characters and breaking the script.

$ErrorActionPreference = "Stop"

$rustupCargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$wasmBindgen = "$env:USERPROFILE\.cargo\bin\wasm-bindgen.exe"
$crateDir    = Join-Path $PSScriptRoot "..\editor-wasm"
$outDir      = Join-Path $PSScriptRoot "..\src\editor-wasm"
$wasmFile    = Join-Path $crateDir "target\wasm32-unknown-unknown\release\editor_wasm.wasm"

if (-not (Test-Path $rustupCargo)) {
    throw "rustup cargo not found: $rustupCargo`n`nSetup required:`n  scoop install rustup  (or get rustup-init.exe from https://rustup.rs)`n  rustup-init.exe -y --no-modify-path --default-toolchain stable`n  rustup target add wasm32-unknown-unknown`n  cargo install wasm-bindgen-cli --version 0.2.126 --force  (must match editor-wasm/Cargo.toml's wasm-bindgen version)"
}
if (-not (Test-Path $wasmBindgen)) {
    throw "wasm-bindgen-cli not found: $wasmBindgen`nRun: cargo install wasm-bindgen-cli --version 0.2.126 --force"
}

& $rustupCargo build --release --target wasm32-unknown-unknown --manifest-path (Join-Path $crateDir "Cargo.toml")
if ($LASTEXITCODE -ne 0) { throw "editor-wasm cargo build failed (exit $LASTEXITCODE)" }

& $wasmBindgen --target web --out-dir $outDir --out-name editor_wasm $wasmFile
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed (exit $LASTEXITCODE)" }

Write-Host "editor-wasm built -> $outDir"
