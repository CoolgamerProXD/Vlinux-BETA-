# Terminal evidence on the real Arch of the lab VM (disposable CI runner, never a user's PC):
#  1. how long plain wsl.exe takes to run a command (no ConPTY),
#  2. the app's diagnostics terminal checks with terminal queries UNANSWERED (the M6.1 behaviour),
#  3. the same checks with the fixed behaviour. Each result carries the evidence trace.
. "$PSScriptRoot/common.ps1"
$ErrorActionPreference = "Continue"
$t = @()
foreach ($i in 1..3) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $o = wsl.exe -d archlinux --exec /bin/echo hello
  $t += "plain wsl.exe --exec echo (no ConPTY) #$i : $([int]$sw.Elapsed.TotalMilliseconds) ms -> $o"
}
$sw = [Diagnostics.Stopwatch]::StartNew()
$o = wsl.exe -d archlinux --exec /bin/bash -lc 'echo login-shell-ok'
$t += "plain wsl.exe bash -lc echo : $([int]$sw.Elapsed.TotalMilliseconds) ms -> $o"
$t += "whoami: " + (Capture { wsl.exe -d archlinux --exec whoami })
$global:LASTEXITCODE = 0
Say "Terminal lab: plain wsl.exe timing" ($t -join "`n")

New-Item -ItemType Directory -Force "$PSScriptRoot/../../dist" | Out-Null
Set-Content "$PSScriptRoot/../../dist/index.html" "<html></html>"
Push-Location "$PSScriptRoot/../../src-tauri"
$env:VLINUX_LAB_DISTRO = "archlinux"
$out = cargo test --locked --lib real_terminal_experiments_in_the_wsl_lab -- --nocapture 2>&1 | Out-String
$global:LASTEXITCODE = 0
Pop-Location
$lines = (Clean $out) -split "`n" | Where-Object { $_ -match 'LAB-(RESULT|SUMMARY|BUILD)' -or $_ -match '^(error|thread|panicked)' }
$chunk = ""
$n = 1
foreach ($l in $lines) {
  if (($chunk.Length + $l.Length) -gt 5500) { Say "Terminal lab ($n)" $chunk; $n++; $chunk = "" }
  $chunk += $l + "`n"
}
if ($chunk) { Say "Terminal lab ($n)" $chunk }
if (-not $lines) { Say "Terminal lab" ("no LAB lines; tail of output:`n" + $out.Substring([Math]::Max(0, $out.Length - 3000))) }
