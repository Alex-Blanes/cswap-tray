<#
.SYNOPSIS
  Walks you through re-logging an account whose refresh token died.

.DESCRIPTION
  cswap-tray opens this window when you click the "session expired"
  notification, or the account itself in the tray menu. It runs the commands
  around the login so the only thing left to do by hand is the /login itself.

  It starts by asking cswap who Claude Code is logged in as right now: if you
  already logged in and simply never ran `cswap add`, saving the token is all
  that is left and the whole login step is skipped.

  Otherwise the order matters, and it is why this exists as a script: the
  account is made active FIRST, so Claude Code writes the new credential over
  the dead one instead of over a live account's. The switch is skipped when
  the live session already belongs to the target, which would clobber a fresh
  login with the stored dead copy.

.EXAMPLE
  .\relogin.ps1 -Account 2 -Email me@example.com -Lang es
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][int]$Account,
    [string]$Email = '',
    [ValidateSet('en', 'es')][string]$Lang = 'en'
)

$ErrorActionPreference = 'Continue'
$who = if ($Email) { "$Account ($Email)" } else { "$Account" }

$t = if ($Lang -eq 'es') {
    @{
        title    = "Cuenta $who - token caducado"
        intro    = 'Los comandos los lanzo yo; tu solo haces el /login, y ni eso si ya lo hiciste.'
        nocswap  = "No encuentro 'cswap' en el PATH. Instalalo con:  uv tool install claude-swap"
        noclaude = "No encuentro 'claude' en el PATH. Abre Claude Code a mano, haz /login y vuelve aqui."
        look     = 'Miro con que cuenta esta Claude Code ahora mismo...'
        found    = 'Ya habias iniciado sesion con esta cuenta: me salto el /login y solo guardo el token.'
        retry    = 'Con eso no ha bastado, asi que vamos por el camino largo.'
        s1       = "[1/3] Activo la cuenta $Account para que el login caiga en su sitio"
        s1skip   = '[1/3] Claude Code ya esta en esta cuenta, no toco nada (cambiar ahora pisaria el login)'
        s2       = '[2/3] Abro Claude Code. Dentro, escribe:'
        s2a      = '/login'
        s2b      = if ($Email) { "      ...e inicia sesion como $Email" } else { '      ...e inicia sesion con la cuenta correcta' }
        s2c      = '      Cuando termine, sal con /exit para volver aqui.'
        s3       = '[3/3] Guardo el token nuevo (cswap add)'
        save     = 'Guardo el token (cswap add)'
        verify   = 'Comprobacion:'
        ok       = "Listo: la cuenta $Account vuelve a estar operativa."
        bad      = "La cuenta $Account sigue marcada como caducada. Repite el /login asegurandote de entrar con la cuenta correcta."
        dunno    = "No he podido leer el estado de la cuenta $Account. Miralo con:  cswap list"
        cont     = 'Pulsa Enter para continuar (Ctrl+C para dejarlo)...'
        close    = 'Pulsa Enter para cerrar.'
    }
} else {
    @{
        title    = "Account $who - dead token"
        intro    = 'I run the commands; you only do the /login, and not even that if you already did.'
        nocswap  = "'cswap' is not on PATH. Install it with:  uv tool install claude-swap"
        noclaude = "'claude' is not on PATH. Open Claude Code by hand, run /login, then come back here."
        look     = 'Asking which account Claude Code is on right now...'
        found    = 'You had already logged in with this account: skipping the /login, just saving the token.'
        retry    = 'That was not enough, so we take the long way round.'
        s1       = "[1/3] Making account $Account active, so the login lands in the right slot"
        s1skip   = '[1/3] Claude Code is already on this account, leaving it alone (switching now would clobber the login)'
        s2       = '[2/3] Opening Claude Code. Inside it, type:'
        s2a      = '/login'
        s2b      = if ($Email) { "      ...and sign in as $Email" } else { '      ...and sign in with the right account' }
        s2c      = '      When you are done, leave with /exit to come back here.'
        s3       = '[3/3] Saving the new token (cswap add)'
        save     = 'Saving the token (cswap add)'
        verify   = 'Check:'
        ok       = "Done: account $Account is usable again."
        bad      = "Account $Account is still flagged as expired. Try the /login again, making sure you sign in with the right account."
        dunno    = "Could not read the state of account $Account. Check it with:  cswap list"
        cont     = 'Press Enter to continue (Ctrl+C to quit)...'
        close    = 'Press Enter to close.'
    }
}

function Step($text) {
    Write-Host ''
    Write-Host $text -ForegroundColor Cyan
}

# Same rule the tray uses: anything that is not empty and not "ok" is a problem.
function Test-StatusOk($status) {
    return (-not $status -or $status -eq 'ok')
}

# The live Claude Code identity, which is not the same question as "which slot
# does cswap point at": that is exactly the case this shortcut is here for.
function Get-LiveAccount {
    try {
        return (cswap status --json 2>$null | ConvertFrom-Json).active
    } catch {
        return $null
    }
}

# $true / $false / $null when the snapshot could not be read — an unreadable
# answer must never pass for a good one.
function Test-AccountAlive($number) {
    try {
        $snap = cswap list --json 2>$null | ConvertFrom-Json
        $acc = $snap.accounts | Where-Object { $_.number -eq $number }
        if (-not $acc) { return $null }
        return (Test-StatusOk $acc.usageStatus)
    } catch {
        return $null
    }
}

# `Out-Host` on purpose: what a native command prints inside a function is part
# of that function's return value, and a `cswap` banner mixed into the verdict
# would read as "not alive" and send you down the long path for nothing.
function Save-Token {
    Write-Host '> cswap add' -ForegroundColor DarkGray
    cswap add | Out-Host
    Step $t.verify
    Write-Host '> cswap list' -ForegroundColor DarkGray
    cswap list | Out-Host
    return (Test-AccountAlive $Account)
}

$host.UI.RawUI.WindowTitle = $t.title
Write-Host ''
Write-Host "  $($t.title)  " -ForegroundColor Black -BackgroundColor Yellow
Write-Host ''
Write-Host $t.intro

if (-not (Get-Command cswap -ErrorAction SilentlyContinue)) {
    Write-Host ''
    Write-Host $t.nocswap -ForegroundColor Red
    Read-Host $t.close
    return
}

Step $t.look
$live = Get-LiveAccount
$liveIsTarget = $false
if ($live) {
    if ($Email) { $liveIsTarget = ($live.email -eq $Email) } else { $liveIsTarget = ($live.number -eq $Account) }
}

$alive = $null

# Already logged in, just never saved: the token in front of us is the good one.
if ($liveIsTarget -and (Test-StatusOk $live.usageStatus)) {
    Write-Host $t.found -ForegroundColor Green
    Step $t.save
    $alive = Save-Token
    if ($alive -ne $true) {
        Write-Host ''
        Write-Host $t.retry -ForegroundColor Yellow
    }
}

if ($alive -ne $true) {
    if ($liveIsTarget) {
        Step $t.s1skip
    } else {
        Step $t.s1
        Write-Host "> cswap switch $Account" -ForegroundColor DarkGray
        cswap switch $Account
    }

    Step $t.s2
    Write-Host "  $($t.s2a)" -ForegroundColor Yellow
    Write-Host $t.s2b
    Write-Host $t.s2c
    Read-Host $t.cont
    if (Get-Command claude -ErrorAction SilentlyContinue) {
        Write-Host '> claude' -ForegroundColor DarkGray
        claude
    } else {
        Write-Host $t.noclaude -ForegroundColor Red
        Read-Host $t.cont
    }

    Step $t.s3
    $alive = Save-Token
}

Write-Host ''
if ($alive -eq $true) {
    Write-Host $t.ok -ForegroundColor Green
} elseif ($alive -eq $false) {
    Write-Host $t.bad -ForegroundColor Red
} else {
    Write-Host $t.dunno -ForegroundColor Yellow
}
Write-Host ''
Read-Host $t.close
