# What does Windows really see and do with WSL links on \\wsl.localhost? Disposable runner only.
. "$PSScriptRoot/common.ps1"
$ErrorActionPreference = "Continue"
$root = "\\wsl.localhost\archlinux\tmp\labx"
function InDistro([string]$script) { Capture { wsl.exe -d archlinux --exec /bin/bash -c $script } }

$r = @()
$r += InDistro 'rm -rf /tmp/labx; mkdir -p /tmp/labx/target && echo hi > /tmp/labx/target/inside.txt && ln -s /tmp/labx/target /tmp/labx/dlink && ln -s /tmp/labx/target/inside.txt /tmp/labx/flink && ln -s /nonexistent /tmp/labx/dangling && ls -la /tmp/labx'
Say "Symlink lab: created inside the distro" ($r -join "`n")

$r = @()
foreach ($n in "target", "dlink", "flink", "dangling") {
  $p = "$root\$n"
  try {
    $i = Get-Item -LiteralPath $p -Force -ErrorAction Stop
    $r += "$n : Attributes=$($i.Attributes) LinkType=$($i.LinkType) Target=$($i.Target) Mode=$($i.Mode)"
  } catch { $r += "$n : Get-Item failed: $($_.Exception.Message)" }
  $r += "  fsutil: " + ((Capture { fsutil.exe reparsepoint query $p }) -replace "\n", " | ")
}
Say "Symlink lab: what Windows sees" ($r -join "`n")

function Fresh {
  InDistro 'rm -rf /tmp/labx/dlink /tmp/labx/flink /tmp/labx/dangling; ln -s /tmp/labx/target /tmp/labx/dlink; ln -s /tmp/labx/target/inside.txt /tmp/labx/flink; ln -s /nonexistent /tmp/labx/dangling' | Out-Null
}
function State { InDistro 'for n in dlink flink dangling; do if [ -L /tmp/labx/$n ]; then echo "$n:link"; elif [ -e /tmp/labx/$n ]; then echo "$n:OTHER"; else echo "$n:gone"; fi; done; ls /tmp/labx/target' }

$methods = [ordered]@{
  "[IO.File]::Delete"       = { param($p) [IO.File]::Delete($p) }
  "[IO.Directory]::Delete"  = { param($p) [IO.Directory]::Delete($p, $false) }
  "cmd del"                 = { param($p) $o = cmd.exe /c "del `"$p`" 2>&1"; if ($LASTEXITCODE -ne 0) { throw ($o -join " ") } }
  "cmd rmdir"               = { param($p) $o = cmd.exe /c "rmdir `"$p`" 2>&1"; if ($LASTEXITCODE -ne 0) { throw ($o -join " ") } }
  "Remove-Item"             = { param($p) Remove-Item -LiteralPath $p -Force -ErrorAction Stop }
}
foreach ($m in $methods.Keys) {
  $r = @()
  foreach ($n in "dlink", "flink", "dangling") {
    Fresh
    $err = "ok"
    try { & $methods[$m] "$root\$n" } catch { $err = $_.Exception.Message }
    $r += "$m on $n -> $err"
    $r += "   state after: " + ((State) -replace "\n", " ")
  }
  Say "Symlink lab: $m" ($r -join "`n")
}
$r = @()
Fresh
$r += "unlink in distro: " + (InDistro 'unlink /tmp/labx/dlink; echo rc=$?')
$r += "state: " + ((State) -replace "\n", " ")
$r += "host still lists dlink? " + ((Test-Path -LiteralPath "$root\dlink") )
$r += "host dir listing: " + ((Get-ChildItem -LiteralPath $root -Force | % Name) -join ",")
Say "Symlink lab: unlink inside the distro" ($r -join "`n")
InDistro 'rm -rf /tmp/labx' | Out-Null
