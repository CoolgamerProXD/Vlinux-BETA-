# Helpers for the WSL lab: a DISPOSABLE GitHub Actions runner VM, never a user's PC.
# CI logs cannot be downloaded from here, so evidence is published as annotations.
function Clean([string]$s) {
  if ($null -eq $s) { return "" }
  $s = $s -replace "`0", "" -replace "`r", ""
  return $s
}
function Say([string]$title, [string]$text, [string]$level = "warning") {
  $t = Clean $text
  if ($t.Length -gt 6000) { $t = $t.Substring(0, 6000) + "...(cut)" }
  $t = $t -replace "%", "%25" -replace "`n", "%0A"
  Write-Host "::$level title=$title::$t"
}
# Runs a native command, returns combined output as text (never throws on a non-zero exit).
function Capture([scriptblock]$cmd) {
  $out = & $cmd 2>&1 | Out-String
  $code = $LASTEXITCODE
  $global:LASTEXITCODE = 0
  return (Clean $out) + "[exit $code]"
}
