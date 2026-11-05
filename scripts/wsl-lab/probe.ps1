. "$PSScriptRoot/common.ps1"
$ErrorActionPreference = "Continue"
$r = @()
$r += "wsl --version: " + (Capture { wsl.exe --version })
$r += "wsl --status: " + (Capture { wsl.exe --status })
$r += "wsl -l -v: " + (Capture { wsl.exe -l -v })
foreach ($f in "Microsoft-Windows-Subsystem-Linux", "VirtualMachinePlatform") {
  try { $r += "$f : " + (Get-WindowsOptionalFeature -Online -FeatureName $f).State } catch { $r += "$f : error $_" }
}
$cs = Get-CimInstance Win32_ComputerSystem
$r += "HypervisorPresent=$($cs.HypervisorPresent) Model=$($cs.Model) Cores=$($cs.NumberOfLogicalProcessors)"
$r += "OS: " + (Get-CimInstance Win32_OperatingSystem).Caption
Say "WSL lab: host" ($r -join "`n")
