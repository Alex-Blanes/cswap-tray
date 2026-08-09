<#
.SYNOPSIS
  Creates (or removes) a cswap-tray shortcut in the user's Startup folder.

.EXAMPLE
  .\install-autostart.ps1
  .\install-autostart.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [switch]$Uninstall,
    # Defaults to the repository's own release binary.
    [string]$ExePath
)

$ErrorActionPreference = 'Stop'

$startup = [Environment]::GetFolderPath('Startup')
$link = Join-Path $startup 'cswap-tray.lnk'

if ($Uninstall) {
    if (Test-Path $link) {
        Remove-Item $link
        Write-Host "Removed from startup: $link"
    } else {
        Write-Host 'It was not installed at startup.'
    }
    return
}

if (-not $ExePath) {
    $ExePath = Join-Path (Split-Path $PSScriptRoot -Parent) 'target\release\cswap-tray.exe'
}
if (-not (Test-Path $ExePath)) {
    throw "$ExePath does not exist. Build it first with 'cargo build --release'."
}
$ExePath = (Resolve-Path $ExePath).Path

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($link)
$shortcut.TargetPath = $ExePath
$shortcut.WorkingDirectory = Split-Path $ExePath -Parent
$shortcut.Description = 'Quick Claude Code account switching'
$shortcut.Save()

Write-Host "Installed at startup: $link"
Write-Host "  -> $ExePath"
