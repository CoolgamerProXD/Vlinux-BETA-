# Real Windows input and window control for the CI desktop smoke test (scripts/desktop-smoke.mjs).
# Sends REAL keystrokes to the foreground window and moves/restores the REAL window; it touches
# nothing but the process named by -Proc and never needs administrator rights.
param(
  [Parameter(Mandatory = $true)][ValidateSet('focus', 'keys', 'move', 'iconic', 'restore')][string]$Action,
  [string]$Arg = '',
  [string]$Proc = 'vlinux-cdp'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
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

switch ($Action) {
  'focus'   { Set-Focus; Write-Output 'ok' }
  'keys'    { Set-Focus; [System.Windows.Forms.SendKeys]::SendWait($Arg); Start-Sleep -Milliseconds 200; Write-Output 'ok' }
  'move'    { $v = $Arg.Split(',') | ForEach-Object { [int]$_ }; [void][VlxWin]::MoveWindow($h, $v[0], $v[1], $v[2], $v[3], $true); Write-Output 'ok' }
  'iconic'  { Write-Output ([VlxWin]::IsIconic($h)) }
  'restore' { [void][VlxWin]::ShowWindow($h, 9); Set-Focus; Write-Output 'ok' }
}
