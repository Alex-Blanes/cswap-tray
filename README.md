# cswap-tray

*[Léeme en español](README.es.md)*

A Windows tray icon that shows which Claude Code account is active and how much
quota it has left, and switches accounts in one click.

It is a thin layer over [claude-swap](https://github.com/realiti4/claude-swap):
`cswap` stays the owner of the credentials, the quota readings and the switch.
This app only reads `cswap list --json` and runs `cswap switch`. claude-swap
ships a `menubar` for macOS; this fills the gap on Windows.

## The icon

```
┌──────────────┐
│ ██████    ▓ ░│   letter    = the account in use, in its own colour
│ ██  ██    ▓ ░│   left bar  = last 5 hours
│ ██████    ▓ ▓│   right bar = last 7 days
│ ██        ▓ ▓│
│ ██        ▓ ▓│   filled from the bottom up
└──────────────┘   green <70%, amber 70-90%, red ≥90%
```

- **Left click**: rotate to the next account. With two accounts, it toggles.
- **Right click**: menu with every account and its usage (including when each
  window resets), auto-switch, prewarming, the icon legend and the actions.
- **Hover**: a summary of all accounts.

Everything is drawn at runtime: no image assets, no external font.

## Requirements

- Windows 10/11
- [claude-swap](https://github.com/realiti4/claude-swap) installed with at least
  one account added (`uv tool install claude-swap`, then `cswap add`)

## Build

```powershell
cargo build --release
# target\release\cswap-tray.exe  (~0.5 MB, no runtime)
```

## Run at startup

```powershell
.\scripts\install-autostart.ps1            # install
.\scripts\install-autostart.ps1 -Uninstall # remove
```

Windows 11 hides new icons in the overflow (the `^` arrow in the tray). To pin
it: *Settings → Personalization → Taskbar → Other system tray icons* and turn
`cswap-tray` on.

## Auto-switch

You live on your preferred account; when it runs out the app moves on its own
to another one with room; and it returns as soon as the preferred one recovers.

```
       preferred at 90 %  ──────────────►  another account with room
       preferred < 80 %   ◄──────────────
```

Turn it on from the *Auto-switch* submenu, which also spells the rule out in
place. The values live in `config.json`:

| Setting | Default | What it does |
|---|---|---|
| `enabled` | `false` | Master switch |
| `preferred` | first personal account detected | Where you want to be by default |
| `switch_at_pct` | `90` | The active account is left at this point |
| `return_below_pct` | `80` | Return to the preferred one below this |
| `cooldown_seconds` | `300` | Minimum wait between switches |

Whichever window binds **first** is the one that counts, be it the 5h or the 7d
one. If every account is spent it does nothing, and an account whose usage
cannot be read is never picked as a target.

The gap between `switch_at_pct` and `return_below_pct` is the hysteresis:
without it, an account hovering around the threshold would flap constantly.

> **Do not combine it with `cswap auto`.** That engine has no preferred
> account: its `best` strategy jumps to whichever has the most quota and stays
> there. With both running, each would undo the other's switches.

> **A switch can catch you mid-conversation.** Claude Code picks up the new
> credentials on the next message, so that session will continue against the
> other account without warning. That is the point of turning it on, but it is
> worth knowing: it mixes personal and work spend.

## Prewarming the reserve

An account's 5h window **is opened by its first message** — not by the login or
the account switch. If you always work on the same account, the other one has
its clock stopped, and the day you need it its window starts right then.

Prewarming does not reserve quota: it **moves the clock forward**.

```
without prewarming   09:00 ─────────── 13:00 first use ──────────── 18:00 renews
with prewarming      09:00 message ──────────────────── 14:00 renews
```

When the app notices you are working while the reserve sits cold, it warns once
with a Windows notification. The *Prewarm* submenu offers two ways to start it:

- **Send my next message through X** — switches to that account and brings you
  back on its own as soon as it detects the message landed. It piggybacks on a
  message you were going to send anyway, so it adds no extra usage. If nothing
  arrives within `arm_timeout_minutes`, the switch is undone and you are told.
- **Start X's clock now** — sends a minimal message via
  `cswap run <n> -- claude -p "ok"`, which applies the credential **to that
  process only**: the account you are using is untouched.

| Setting | Default | What it does |
|---|---|---|
| `notify` | `true` | Warn when the reserve is cold |
| `arm_timeout_minutes` | `15` | Wait before undoing an armed prewarm |

Detection is indirect: a message is recognised because that account gains a 5h
window or its usage goes up. Since polling runs every 30 s, the switch back
takes up to a minute — and in that gap any other message also goes through the
reserve. Worth keeping in mind with several Claude sessions open at once.

To check Windows notifications get through:

```powershell
.\target\release\cswap-tray.exe --test-toast
```

## Appearance and language

Generated in `%APPDATA%\cswap-tray\config.json` on first run, guessing which
account is personal from the mail domain:

```json
{
  "language": "auto",
  "refresh_seconds": 30,
  "accounts": {
    "you@hotmail.com":  { "name": "Personal", "letter": "P", "color": "#3B82F6" },
    "you@company.com":  { "name": "Company",  "letter": "W", "color": "#F59E0B" }
  }
}
```

`letter` accepts A-Z and 0-9. `language` is `"auto"` (follows the Windows UI
language), `"en"` or `"es"` — the whole interface is translated. Changes apply
on restart. Accounts are still added and removed with `cswap`, not here; new
ones show up in the file on their own.

`refresh_seconds` is only the read cadence: cswap caches usage on disk, so
polling does not trigger API calls.

## How it is put together

| File | Responsibility |
|---|---|
| `src/cswap.rs` | Invoke the CLI and deserialize `list --json` (schemaVersion 1) |
| `src/auto.rs` | Auto-switch policy: a pure function over the snapshot |
| `src/prewarm.rs` | Cold-reserve detection and message-landed evidence |
| `src/icon.rs` | Draw the RGBA icon: own 5×7 font plus bars |
| `src/i18n.rs` | UI strings in English and Spanish |
| `src/config.rs` | Per-account appearance and the personal/work heuristic |
| `src/main.rs` | Tray icon, menu, Win32 message loop and worker |

Polling and switching run on a separate thread so the CLI — which is Python and
takes about a second to start — never blocks the interface.

```powershell
cargo test                                        # 28 tests
cargo test preview -- --ignored --nocapture       # preview the icon as ASCII
```

## Known limitations

- Windows only: it drives the Win32 message loop directly.
- The icon size is decided at startup (`SM_CXSMICON`); restart the app if you
  change the display scaling.
- Windows truncates tooltips at 127 characters, so with many accounts the hover
  summary is cut. The menu does show them all.

## License

MIT — see [LICENSE](LICENSE).
