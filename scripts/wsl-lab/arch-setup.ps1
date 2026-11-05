# Installs the official Arch WSL image on the DISPOSABLE runner (never on a user's PC) and
# configures it like the user's: systemd on, a normal default user.
. "$PSScriptRoot/common.ps1"
$ErrorActionPreference = "Continue"
$r = @()
$r += "list online: " + (Capture { wsl.exe --list --online })
$sw = [Diagnostics.Stopwatch]::StartNew()
$r += "install: " + (Capture { wsl.exe --install archlinux --no-launch })
$r += "install took $([int]$sw.Elapsed.TotalSeconds) s"
$r += "list: " + (Capture { wsl.exe -l -v })
Say "WSL lab: install archlinux" ($r -join "`n")

$setup = @'
set -e
id -u lab >/dev/null 2>&1 || useradd -m -s /bin/bash lab
printf '[boot]\nsystemd=true\n\n[user]\ndefault=lab\n' > /etc/wsl.conf
cat /etc/wsl.conf
'@
$r = @()
$r += "configure: " + (Capture { wsl.exe -d archlinux -u root --exec /bin/bash -c $setup })
$r += "terminate: " + (Capture { wsl.exe --terminate archlinux })
Start-Sleep -Seconds 3
$r += "whoami: " + (Capture { wsl.exe -d archlinux --exec whoami })
$r += "uname: " + (Capture { wsl.exe -d archlinux --exec uname -a })
$r += "shell/home: " + (Capture { wsl.exe -d archlinux --exec /bin/bash -c 'echo $SHELL $HOME; locale 2>&1 | head -3; id' })
$r += "systemd: " + (Capture { wsl.exe -d archlinux --exec /bin/bash -c 'systemctl is-system-running 2>&1; ls -ld /tmp' })
$r += "list: " + (Capture { wsl.exe -l -v })
$r += "share: " + (Capture { cmd.exe /c "dir \\wsl.localhost\archlinux\tmp 2>&1" })
Say "WSL lab: configured distro" ($r -join "`n")
