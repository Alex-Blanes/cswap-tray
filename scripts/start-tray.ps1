$exe = Join-Path $env:LOCALAPPDATA 'cswap-tray\cswap-tray.exe'
Unregister-ScheduledTask -TaskName cswap-tray-start -Confirm:$false -ErrorAction SilentlyContinue
$a = New-ScheduledTaskAction -Execute $exe -WorkingDirectory (Split-Path $exe)
$p = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
# A laptop on battery would leave the task queued forever otherwise.
$s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0
Register-ScheduledTask -TaskName cswap-tray-start -Action $a -Principal $p -Settings $s | Out-Null
Start-ScheduledTask -TaskName cswap-tray-start
Start-Sleep 5
Get-Process cswap-tray -ErrorAction SilentlyContinue | Select-Object Id, SessionId, Path | Format-List
Unregister-ScheduledTask -TaskName cswap-tray-start -Confirm:$false
