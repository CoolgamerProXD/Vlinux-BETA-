# Real Windows input and window control for the CI desktop smoke test (scripts/desktop-smoke.mjs).
# Sends REAL keystrokes to the foreground window and moves/restores the REAL window; it touches
# nothing but the process named by -Proc and never needs administrator rights.
param(
  [Parameter(Mandatory = $true)][ValidateSet('focus', 'keys', 'move', 'iconic', 'restore', 'info')][string]$Action,
  [string]$Arg = '',
  [string]$Proc = 'vlinux-cdp'
)
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class VlxWin {
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
}
"@
$p = Get-Process -Name $Proc -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { throw "no window for process $Proc" }
$h = $p.MainWindowHandle

function Set-Focus {
  $fg = [VlxWin]::GetForegroundWindow()
  [uint32]$ignored = 0
  $t1 = [VlxWin]::GetWindowThreadProcessId($fg, [ref]$ignored)
  $t2 = [VlxWin]::GetCurrentThreadId()
  [void][VlxWin]::AttachThreadInput($t2, $t1, $true)
  [void][VlxWin]::SetForegroundWindow($h)
  [void][VlxWin]::AttachThreadInput($t2, $t1, $false)
  Start-Sleep -Milliseconds 250
}

$VK = @{ 'ctrl' = 0x11; 'alt' = 0x12; 'shift' = 0x10; 'esc' = 0x1B; 'enter' = 0x0D; 'space' = 0x20 }
function Get-Vk([string]$k) {
  if ($VK.ContainsKey($k)) { return [int]$VK[$k] }
  if ($k -match '^f(\d{1,2})$') { return 0x6F + [int]$Matches[1] }
  if ($k.Length -eq 1) { return [int][char]$k.ToUpper() }
  throw "unknown key '$k'"
}
function Send-Key([int]$vk, [bool]$down) {
  $scan = [byte][VlxWin]::MapVirtualKey([uint32]$vk, 0)
  [VlxWin]::keybd_event([byte]$vk, $scan, $(if ($down) { 0 } else { 2 }), [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 25
}
# "Ctrl+Alt+Q" -> hold Ctrl, hold Alt, tap Q, release in reverse order. Several chords: space-separated.
function Send-Chord([string]$chord) {
  $parts = $chord.ToLower().Split('+')
  $codes = @($parts | ForEach-Object { Get-Vk $_ })
  foreach ($c in $codes) { Send-Key $c $true }
  [array]::Reverse($codes)
  foreach ($c in $codes) { Send-Key $c $false }
}

switch ($Action) {
  'focus'   { Set-Focus; Write-Output 'ok' }
  'keys'    { Set-Focus; foreach ($ch in ($Arg -split ' ')) { if ($ch) { Send-Chord $ch } }; Start-Sleep -Milliseconds 200; Write-Output 'ok' }
  'move'    { $v = $Arg.Split(',') | ForEach-Object { [int]$_ }; [void][VlxWin]::MoveWindow($h, $v[0], $v[1], $v[2], $v[3], $true); Write-Output 'ok' }
  'info'    { $fg = [VlxWin]::GetForegroundWindow(); Write-Output ("session=$($p.SessionId) interactive=$([Environment]::UserInteractive) foregroundIsVlinux=$($fg -eq $h) visible=$([VlxWin]::IsWindowVisible($h)) iconic=$([VlxWin]::IsIconic($h))") }
  'iconic'  { Write-Output ([VlxWin]::IsIconic($h)) }
  'restore' { [void][VlxWin]::ShowWindow($h, 9); Set-Focus; Write-Output 'ok' }
}
