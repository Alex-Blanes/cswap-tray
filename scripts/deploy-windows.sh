#!/bin/sh
# Installs dist/cswap-tray.exe (from build-windows.sh) on a Windows machine over
# SSH and starts it on that user's desktop.
#
#   scripts/deploy-windows.sh alex-@192.168.1.132     # rbspc, at home
#
# Starting it straight from the SSH session would leave it in a session with no
# desktop, so start-tray.ps1 goes through a one-off scheduled task instead.
set -eu
host=${1:?usage: deploy-windows.sh user@host}
repo=$(cd "$(dirname "$0")/.." && pwd)
scp -q -o BatchMode=yes "$repo/dist/cswap-tray.exe" "$repo/scripts/install-autostart.ps1" \
    "$repo/scripts/start-tray.ps1" "$host:AppData/Local/Temp/"
for script in 'install-autostart.ps1 -ExePath %TEMP%\cswap-tray.exe' 'start-tray.ps1'; do
    ssh -o BatchMode=yes -o ConnectTimeout=8 "$host" \
        "powershell -NoProfile -ExecutionPolicy Bypass -File %TEMP%\\$script"
done
