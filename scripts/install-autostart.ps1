<#
.SYNOPSIS
  Installs cswap-tray for the current user and starts it with Windows.

.DESCRIPTION
  Copies the built binary to %LOCALAPPDATA%\cswap-tray and points a Startup
  shortcut at that copy, rather than at the repository's own target\release.

  The copy is the point. target\release is cargo's output directory: a
  `cargo clean`, a failed build or a rebuild in progress leaves the machine
  with no tray at the next boot, or with a half-written binary. The installed
  copy only changes when this script is run.

  Re-run it after every `cargo build --release` to deploy the new build.

.EXAMPLE
  .\install-autostart.ps1
  .\install-autostart.ps1 -ExePath D:\somewhere\cswap-tray.exe
  .\install-autostart.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [switch]$Uninstall,
    # Source binary. Defaults to the repository's own release build.
    [string]$ExePath
)

$ErrorActionPreference = 'Stop'

$startup = [Environment]::GetFolderPath('Startup')
$link = Join-Path $startup 'cswap-tray.lnk'
$installDir = Join-Path $env:LOCALAPPDATA 'cswap-tray'
$installed = Join-Path $installDir 'cswap-tray.exe'

# A running tray holds its own binary open, so it has to go before either
# replacing or removing the copy.
function Stop-InstalledTray {
    Get-Process cswap-tray -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $installed } |
        ForEach-Object {
            Stop-Process -Id $_.Id -Force
            Write-Host "Stopped the running tray (PID $($_.Id))."
        }
}

if ($Uninstall) {
    if (Test-Path $link) {
        Remove-Item $link
        Write-Host "Removed from startup: $link"
    } else {
        Write-Host 'It was not installed at startup.'
    }
    Stop-InstalledTray
    if (Test-Path $installDir) {
        Remove-Item $installDir -Recurse -Force
        Write-Host "Removed the installed copy: $installDir"
    }
    Write-Host 'Your settings in %APPDATA%\cswap-tray were left alone.'
    return
}

if (-not $ExePath) {
    $ExePath = Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\cswap-tray.exe'
}
if (-not (Test-Path $ExePath)) {
    throw "$ExePath does not exist. Build it first with 'cargo build --release'."
}
$ExePath = (Resolve-Path $ExePath).Path

if ($ExePath -eq $installed) {
    throw 'The source is the installed copy itself. Point -ExePath at a build.'
}

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Stop-InstalledTray
Copy-Item $ExePath $installed -Force

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($link)
$shortcut.TargetPath = $installed
$shortcut.WorkingDirectory = $installDir
$shortcut.Description = 'Quick Claude Code account switching'
$shortcut.Save()

Write-Host "Installed: $installed"
Write-Host "At startup: $link"
Write-Host 'Start it now with:'
Write-Host "  Start-Process '$installed'"
