# Runs the app's REAL diagnostics (same code path as the app) against the real Arch of the lab VM.
. "$PSScriptRoot/common.ps1"
$ErrorActionPreference = "Continue"
New-Item -ItemType Directory -Force "$PSScriptRoot/../../dist" | Out-Null
Set-Content "$PSScriptRoot/../../dist/index.html" "<html></html>"
Push-Location "$PSScriptRoot/../../src-tauri"
$env:VLINUX_LAB_DISTRO = "archlinux"
$out = cargo test --locked --lib real_diagnostics_in_the_wsl_lab -- --nocapture 2>&1 | Out-String
$global:LASTEXITCODE = 0
Pop-Location
$lines = (Clean $out) -split "`n" | Where-Object { $_ -match '^(LAB-|test result|error|thread|panicked)' -or $_ -match 'LAB-' }
$chunk = ""
$n = 1
foreach ($l in $lines) {
  if ($l -match '^LAB-CHECK') { continue }
  if (($chunk.Length + $l.Length) -gt 5000) { Say "Real diag in lab ($n)" $chunk; $n++; $chunk = "" }
  $chunk += $l + "`n"
}
if ($chunk) { Say "Real diag in lab ($n)" $chunk }
if (-not $lines) { Say "Real diag in lab" ("no LAB lines; tail of output:`n" + $out.Substring([Math]::Max(0, $out.Length - 3000))) }
