//! cswap-tray — switch Claude Code accounts from the Windows tray.
//!
//! A thin layer over the claude-swap CLI: `cswap` stays the owner of the
//! credentials, the quota readings and the switch itself. This only paints the
//! state and forwards orders.
//!
//! - Left click: rotate to the next account (`cswap switch`).
//! - Right click: menu with the accounts, their usage and the actions.

#![windows_subsystem = "windows"]

mod auto;
mod config;
mod cswap;
mod i18n;
mod icon;
mod prewarm;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use i18n::Lang;
use tauri_winrt_notification::{Duration as ToastDuration, Toast};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetSystemMetrics, MSG, SM_CXSMICON, SetProcessDPIAware, SetTimer,
    TranslateMessage,
};

/// How often the message loop wakes up to service menu and worker events.
const TICK_MS: u32 = 200;
/// Real limit of a tray tooltip on Windows.
const TOOLTIP_MAX: usize = 127;

enum Cmd {
    Refresh,
    Switch(Option<u32>),
}

type SnapResult = Result<cswap::Snapshot, String>;

fn main() {
    let cfg_lang = Lang::resolve(&config::load().language);
    // Standalone check that Windows notifications get through, handy when the
    // action center is muted or focus assist is on.
    if std::env::args().any(|a| a == "--test-toast") {
        notify("cswap-tray", cfg_lang.toast_test());
        return;
    }
    // Rehearses the whole re-login path — the toast, its button, and the guided
    // console the click opens — without waiting for a token to actually die.
    // It targets the *active* account on purpose: every command the guide runs
    // is then a no-op refresh of the account you are already on.
    if std::env::args().any(|a| a == "--test-relogin") {
        let acc = cswap::list().ok().and_then(|s| {
            s.accounts
                .iter()
                .find(|a| a.active)
                .or_else(|| s.accounts.first())
                .map(|a| (a.number, a.email.clone()))
        });
        let (number, email) = acc.unwrap_or((1, String::new()));
        notify_relogin(cfg_lang, number, "test", &email);
        // The click is served in-process, so the rehearsal has to outlive the
        // toast. The real tray is always up and needs none of this.
        std::thread::sleep(Duration::from_secs(90));
        return;
    }
    unsafe { SetProcessDPIAware() };

    let mut cfg = config::load();
    // Rewrite the file in its full shape, so settings added by a new version
    // are visible and editable.
    config::save(&cfg);
    let lang = Lang::resolve(&cfg.language);
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (snap_tx, snap_rx) = channel::<SnapResult>();
    spawn_worker(cfg.refresh_seconds, cmd_rx, snap_tx);

    let icon_size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let mut state = State {
        snapshot: None,
        error: None,
        last_auto_switch: None,
        armed: None,
        cold_notified: false,
        relogin_notified: std::collections::BTreeSet::new(),
    };

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(Menu::new()))
        .with_menu_on_left_click(false)
        .with_tooltip(lang.loading())
        .with_icon(make_icon(&cfg, &state, icon_size))
        .build()
        .expect("could not create the tray icon");

    let _ = cmd_tx.send(Cmd::Refresh);
    run_message_loop(&tray, &mut cfg, &mut state, lang, icon_size, &cmd_tx, &snap_rx);
}

struct State {
    snapshot: Option<cswap::Snapshot>,
    error: Option<String>,
    /// When the last automatic switch happened, to honour the cooldown.
    last_auto_switch: Option<Instant>,
    /// Prewarm in flight: we sit on the reserve waiting for the user's message
    /// so we can switch back afterwards.
    armed: Option<Armed>,
    /// Keeps the cold-reserve notice from repeating on every poll.
    cold_notified: bool,
    /// Accounts already warned about a dead token, so the toast fires on the
    /// transition and not on every poll.
    relogin_notified: std::collections::BTreeSet<u32>,
}

/// State of "send my next message through the reserve".
struct Armed {
    /// Account being prewarmed.
    target: u32,
    /// Account to return to once the message lands.
    back_to: u32,
    /// Reading of the reserve when armed, to recognise the new message.
    had_window: bool,
    base_pct: f32,
    since: Instant,
}

impl State {
    fn accounts(&self) -> &[cswap::Account] {
        self.snapshot.as_ref().map(|s| s.accounts.as_slice()).unwrap_or(&[])
    }

    fn active(&self) -> Option<&cswap::Account> {
        self.accounts().iter().find(|a| a.active)
    }

    /// Display name of an account number, falling back to a generic label.
    fn name_of(&self, cfg: &config::Config, lang: Lang, number: u32) -> String {
        self.accounts()
            .iter()
            .find(|a| a.number == number)
            .map(|a| config::presentation(cfg, &a.email, a.number).0)
            .unwrap_or_else(|| lang.account_fallback(number))
    }
}

fn spawn_worker(refresh_seconds: u64, cmd_rx: Receiver<Cmd>, snap_tx: Sender<SnapResult>) {
    let period = Duration::from_secs(refresh_seconds.clamp(5, 3600));
    std::thread::spawn(move || {
        loop {
            let cmd = match cmd_rx.recv_timeout(period) {
                Ok(c) => Some(c),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Some(Cmd::Refresh),
                // The main thread is gone: follow it out.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if let Some(Cmd::Switch(target)) = cmd {
                if let Err(e) = cswap::switch(target) {
                    if snap_tx.send(Err(e)).is_err() {
                        break;
                    }
                    continue;
                }
            }
            if snap_tx.send(cswap::list()).is_err() {
                break;
            }
        }
    });
}

fn run_message_loop(
    tray: &TrayIcon,
    cfg: &mut config::Config,
    state: &mut State,
    lang: Lang,
    icon_size: u32,
    cmd_tx: &Sender<Cmd>,
    snap_rx: &Receiver<SnapResult>,
) {
    unsafe { SetTimer(std::ptr::null_mut(), 1, TICK_MS, None) };
    let menu_events = MenuEvent::receiver();
    let tray_events = TrayIconEvent::receiver();
    let mut msg: MSG = unsafe { std::mem::zeroed() };

    loop {
        let got = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if got <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let mut dirty = false;
        while let Ok(result) = snap_rx.try_recv() {
            match result {
                Ok(snap) => {
                    let emails: Vec<String> = snap.accounts.iter().map(|a| a.email.clone()).collect();
                    let mut touched = config::ensure_accounts(cfg, &emails);
                    if cfg.auto.preferred.is_none() {
                        cfg.auto.preferred = config::default_preferred(&emails);
                        touched |= cfg.auto.preferred.is_some();
                    }
                    if touched {
                        config::save(cfg);
                    }
                    state.snapshot = Some(snap);
                    state.error = None;
                    // Prewarm wins: while it is armed the automatic policy must
                    // not move the account under our feet.
                    check_relogin(cfg, state, lang);
                    if state.armed.is_some() {
                        follow_up_prewarm(cfg, state, lang, cmd_tx);
                    } else {
                        apply_auto_policy(cfg, state, cmd_tx);
                        check_cold_reserve(cfg, state, lang);
                    }
                }
                Err(e) => state.error = Some(e),
            }
            dirty = true;
        }

        while let Ok(event) = menu_events.try_recv() {
            let id = event.id.0.as_str();
            match id {
                "quit" => return,
                "refresh" => {
                    let _ = cmd_tx.send(Cmd::Refresh);
                }
                "rotate" => {
                    let _ = cmd_tx.send(Cmd::Switch(None));
                }
                "tui" => cswap::open_tui(),
                "config" => open_config(),
                "auto:toggle" => {
                    cfg.auto.enabled = !cfg.auto.enabled;
                    config::save(cfg);
                    // Once switched on, let it act right away if it applies.
                    state.last_auto_switch = None;
                    apply_auto_policy(cfg, state, cmd_tx);
                    dirty = true;
                }
                "prewarm:output" => open_path(&config::prewarm_output_path()),
                "prewarm:cancel" => {
                    if let Some(armed) = state.armed.take() {
                        let _ = cmd_tx.send(Cmd::Switch(Some(armed.back_to)));
                    }
                    dirty = true;
                }
                _ => {
                    if let Some(n) = id.strip_prefix("prewarm:arm:").and_then(|n| n.parse().ok()) {
                        arm_prewarm(cfg, state, lang, cmd_tx, n);
                        dirty = true;
                    } else if let Some(n) = id.strip_prefix("prewarm:task:").and_then(|n| n.parse().ok())
                    {
                        run_prewarm_task(cfg, state, lang, n);
                    } else if let Some(n) = id.strip_prefix("switch:").and_then(|n| n.parse().ok()) {
                        let _ = cmd_tx.send(Cmd::Switch(Some(n)));
                    } else if let Some(n) = id.strip_prefix("relogin:").and_then(|n| n.parse::<u32>().ok())
                    {
                        if let Some(acc) = state.accounts().iter().find(|a| a.number == n) {
                            cswap::open_relogin_guide(&config::dir(), n, &acc.email, lang.code());
                        }
                    } else if let Some(n) = id.strip_prefix("prefer:").and_then(|n| n.parse::<u32>().ok())
                    {
                        if let Some(acc) = state.accounts().iter().find(|a| a.number == n) {
                            cfg.auto.preferred = Some(acc.email.clone());
                            config::save(cfg);
                            dirty = true;
                        }
                    }
                }
            }
        }

        while let Ok(event) = tray_events.try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event
            {
                // With two accounts this toggles; with more, it advances in order.
                if state.accounts().len() > 1 {
                    let _ = cmd_tx.send(Cmd::Switch(None));
                }
            }
        }

        if dirty {
            let _ = tray.set_icon(Some(make_icon(cfg, state, icon_size)));
            let _ = tray.set_tooltip(Some(tooltip(cfg, state, lang)));
            tray.set_menu(Some(Box::new(build_menu(cfg, state, lang))));
        }
    }
}

/// Evaluates the auto-switch policy against the latest snapshot and orders the
/// switch when it applies. The cooldown lives here so the decision itself
/// (`auto::decide`) stays a pure, testable function.
fn apply_auto_policy(cfg: &config::Config, state: &mut State, cmd_tx: &Sender<Cmd>) {
    if !cfg.auto.enabled {
        return;
    }
    let cooldown = Duration::from_secs(cfg.auto.cooldown_seconds);
    if state.last_auto_switch.is_some_and(|t| t.elapsed() < cooldown) {
        return;
    }
    if let Some(target) = auto::decide(&cfg.auto, state.accounts()) {
        state.last_auto_switch = Some(Instant::now());
        let _ = cmd_tx.send(Cmd::Switch(Some(target)));
    }
}

/// Fires a Windows notification. Failures are ignored: it is informational.
fn notify(title: &str, body: &str) {
    let _ = Toast::new(Toast::POWERSHELL_APP_ID)
        .title(title)
        .text1(body)
        .duration(ToastDuration::Short)
        .show();
}

/// Warns once per account whose refresh token died. Without this the only
/// symptom is an account silently dropping out of the rotation: auto-switch
/// refuses it as a target and nothing on screen says why.
fn check_relogin(cfg: &config::Config, state: &mut State, lang: Lang) {
    let dead: Vec<(u32, String, String)> = state
        .accounts()
        .iter()
        .filter(|a| !a.is_ok())
        .map(|a| {
            (a.number, config::presentation(cfg, &a.email, a.number).0, a.email.clone())
        })
        .collect();
    for (number, name, email) in &dead {
        if state.relogin_notified.insert(*number) {
            notify_relogin(lang, *number, name, email);
        }
    }
    // Recovered accounts re-arm the warning for next time.
    state.relogin_notified.retain(|n| dead.iter().any(|(d, _, _)| d == n));
}

/// The "session expired" toast. Clicking it — body or button — opens the
/// guided re-login, which is the whole point: the toast names a problem you
/// cannot fix from the toast, and the commands are the part people forget.
///
/// No COM activator is registered for this: Windows raises `Activated`
/// in-process, which is enough because the tray is always running.
fn notify_relogin(lang: Lang, number: u32, name: &str, email: &str) {
    let (mail, code) = (email.to_string(), lang.code());
    let _ = Toast::new(Toast::POWERSHELL_APP_ID)
        .title(&lang.relogin_title(name))
        .text1(lang.relogin_body())
        .add_button(lang.relogin_action(), "relogin")
        // Long, not Short: five seconds is not enough to react to it.
        .duration(ToastDuration::Long)
        .on_activated(move |_| {
            cswap::open_relogin_guide(&config::dir(), number, &mail, code);
            Ok(())
        })
        .show();
}

/// Warns once that you are working while the reserve sits cold.
fn check_cold_reserve(cfg: &config::Config, state: &mut State, lang: Lang) {
    if !cfg.prewarm.notify {
        return;
    }
    match prewarm::cold_reserve(state.accounts()) {
        Some(number) => {
            if !state.cold_notified {
                state.cold_notified = true;
                let name = state.name_of(cfg, lang, number);
                notify(&lang.cold_title(&name), lang.cold_body());
            }
        }
        // No longer cold: re-arm the notice for next time.
        None => state.cold_notified = false,
    }
}

/// Arms "send my next message through the reserve": switches to that account
/// and records how it looked, so the message can be recognised later.
fn arm_prewarm(cfg: &config::Config, state: &mut State, lang: Lang, cmd_tx: &Sender<Cmd>, target: u32) {
    let Some(back_to) = state.active().map(|a| a.number) else {
        return;
    };
    if back_to == target {
        return;
    }
    let (had_window, base_pct) = state
        .accounts()
        .iter()
        .find(|a| a.number == target)
        .and_then(|a| a.window(cswap::Win::FiveHour).map(|w| (true, w.pct)))
        .unwrap_or((false, 0.0));
    let name = state.name_of(cfg, lang, target);

    state.armed = Some(Armed { target, back_to, had_window, base_pct, since: Instant::now() });
    let _ = cmd_tx.send(Cmd::Switch(Some(target)));
    notify(&lang.prewarming_title(&name), lang.prewarming_body());
}

/// Runs the user's own prompt on another account. Opening that account's 5h
/// window is a side effect of work that was going to be done anyway — which is
/// why there is no built-in prompt: an answer nobody reads would make this a
/// hollow request wearing the costume of work.
fn run_prewarm_task(cfg: &config::Config, state: &State, lang: Lang, target: u32) {
    let out = config::prewarm_output_path();
    let name = state.name_of(cfg, lang, target);
    match cswap::run_task(target, &cfg.prewarm.task, &out) {
        Err(e) => notify(lang.prewarm_failed(), &e),
        Ok(mut child) => {
            notify(&lang.task_started_title(&name), lang.task_started_body());
            // Claude takes a while; wait off the UI thread and report back.
            std::thread::spawn(move || {
                let _ = child.wait();
                notify(lang.task_done_title(), lang.task_done_body());
            });
        }
    }
}

/// With the prewarm armed: check whether the message landed (or the wait ran
/// out) and undo the switch.
fn follow_up_prewarm(cfg: &config::Config, state: &mut State, lang: Lang, cmd_tx: &Sender<Cmd>) {
    let Some(armed) = &state.armed else {
        return;
    };
    let landed = state
        .accounts()
        .iter()
        .find(|a| a.number == armed.target)
        .is_some_and(|a| prewarm::prewarm_landed(a, armed.had_window, armed.base_pct));
    let expired = armed.since.elapsed() >= Duration::from_secs(cfg.prewarm.arm_timeout_minutes * 60);
    if !landed && !expired {
        return;
    }

    let back_to = armed.back_to;
    state.armed = None;
    state.cold_notified = false;
    let _ = cmd_tx.send(Cmd::Switch(Some(back_to)));

    let back_name = state.name_of(cfg, lang, back_to);
    if landed {
        notify(lang.clock_started_title(), &lang.clock_started_body(&back_name));
    } else {
        notify(
            lang.prewarm_cancelled_title(),
            &lang.prewarm_cancelled_body(&back_name),
        );
    }
}

fn make_icon(cfg: &config::Config, state: &State, size: u32) -> Icon {
    let alert = state.accounts().iter().any(|a| !a.is_ok());
    let spec = match state.active() {
        Some(acc) => {
            let (_, letter, color) = config::presentation(cfg, &acc.email, acc.number);
            icon::IconSpec {
                letter,
                color,
                five_hour: acc.pct(cswap::Win::FiveHour),
                seven_day: acc.pct(cswap::Win::SevenDay),
                stale: !acc.is_ok(),
                alert,
            }
        }
        None => icon::IconSpec {
            letter: if state.error.is_some() { '!' } else { '?' },
            color: [200, 200, 200],
            five_hour: None,
            seven_day: None,
            stale: true,
            alert,
        },
    };
    Icon::from_rgba(icon::render(&spec, size), size, size).expect("invalid icon buffer")
}

fn build_menu(cfg: &config::Config, state: &State, lang: Lang) -> Menu {
    let menu = Menu::new();

    if let Some(err) = &state.error {
        let _ = menu.append(&MenuItem::new(format!("⚠ {}", truncate(err, 60)), false, None));
        let _ = menu.append(&PredefinedMenuItem::separator());
    }

    if state.accounts().is_empty() && state.error.is_none() {
        let _ = menu.append(&MenuItem::new(lang.no_accounts(), false, None));
    }

    for acc in state.accounts() {
        let (name, letter, _) = config::presentation(cfg, &acc.email, acc.number);
        let mark = if acc.active { "●" } else { "○" };
        // A dead account is not worth switching to; what it needs is the
        // re-login, so its entry opens the guide instead.
        let (id, label, enabled) = if acc.is_ok() {
            (
                format!("switch:{}", acc.number),
                format!("{mark} {name} ({letter}) — {}", usage_long(acc)),
                !acc.active,
            )
        } else {
            (
                format!("relogin:{}", acc.number),
                format!(
                    "{mark} {name} ({letter}) — ⚠ {} → {}",
                    lang.relogin_tag(),
                    lang.relogin_action()
                ),
                true,
            )
        };
        let _ = menu.append(&MenuItem::with_id(id, label, enabled, None));
    }

    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&auto_submenu(cfg, state, lang));
    let _ = menu.append(&prewarm_submenu(cfg, state, lang));
    let _ = menu.append(&legend_submenu(lang));
    let _ = menu.append(&PredefinedMenuItem::separator());
    if state.accounts().len() > 2 {
        let _ = menu.append(&MenuItem::with_id("rotate", lang.rotate(), true, None));
    }
    let _ = menu.append(&MenuItem::with_id("refresh", lang.refresh(), true, None));
    let _ = menu.append(&MenuItem::with_id("tui", lang.dashboard(), true, None));
    let _ = menu.append(&MenuItem::with_id("config", lang.edit_appearance(), true, None));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("quit", lang.quit(), true, None));
    menu
}

/// Auto-switch submenu: the toggle, where it returns to, and the rule spelled
/// out in plain words.
fn auto_submenu(cfg: &config::Config, state: &State, lang: Lang) -> Submenu {
    let a = &cfg.auto;
    let pref_name = a
        .preferred
        .as_deref()
        .and_then(|email| {
            state
                .accounts()
                .iter()
                .find(|acc| acc.email == email)
                .map(|acc| config::presentation(cfg, &acc.email, acc.number).0)
        })
        .unwrap_or_else(|| lang.not_set().to_string());

    let title = if a.enabled {
        lang.auto_on(&pref_name)
    } else {
        lang.auto_off().to_string()
    };
    let sub = Submenu::new(title, true);

    let toggle = if a.enabled { lang.disable() } else { lang.enable() };
    let _ = sub.append(&MenuItem::with_id("auto:toggle", toggle, true, None));
    let _ = sub.append(&PredefinedMenuItem::separator());

    // Help text as disabled entries: Windows greys them out and they cannot be
    // clicked. It is the way to put explanations inside a native menu.
    for line in lang.auto_help(
        &pref_name,
        a.switch_at_pct,
        a.return_below_pct,
        a.cooldown_seconds / 60,
    ) {
        let _ = sub.append(&MenuItem::new(format!("   {line}"), false, None));
    }

    if state.accounts().len() > 1 {
        let _ = sub.append(&PredefinedMenuItem::separator());
        let _ = sub.append(&MenuItem::new(lang.preferred_account(), false, None));
        for acc in state.accounts() {
            let (name, _, _) = config::presentation(cfg, &acc.email, acc.number);
            let chosen = a.preferred.as_deref() == Some(acc.email.as_str());
            let mark = if chosen { "✓" } else { "  " };
            let _ = sub.append(&MenuItem::with_id(
                format!("prefer:{}", acc.number),
                format!("{mark} {name}"),
                !chosen,
                None,
            ));
        }
    }
    sub
}

/// Prewarm submenu: start the 5h clock of whichever account does not have one
/// running, plus the reason why that is worth doing.
fn prewarm_submenu(cfg: &config::Config, state: &State, lang: Lang) -> Submenu {
    let cold = prewarm::cold_reserve(state.accounts());
    let title = match (&state.armed, cold) {
        (Some(_), _) => lang.prewarm_title_armed(),
        (None, Some(_)) => lang.prewarm_title_cold(),
        (None, None) => lang.prewarm_title(),
    };
    let sub = Submenu::new(title, true);

    if let Some(armed) = &state.armed {
        let name = state.name_of(cfg, lang, armed.target);
        let _ = sub.append(&MenuItem::new(
            format!("   {}", lang.prewarm_armed_line(&name)),
            false,
            None,
        ));
        let _ = sub.append(&MenuItem::with_id(
            "prewarm:cancel",
            lang.cancel_and_return(),
            true,
            None,
        ));
    } else {
        for acc in state.accounts().iter().filter(|a| !a.active) {
            let (name, _, _) = config::presentation(cfg, &acc.email, acc.number);
            let running = acc.window(cswap::Win::FiveHour).is_some();
            let suffix = if running { lang.already_running() } else { "" };
            let _ = sub.append(&MenuItem::with_id(
                format!("prewarm:arm:{}", acc.number),
                lang.prewarm_arm(&name, suffix),
                !running,
                None,
            ));
            // Running a task is worth it even if the clock is already going:
            // it is work you wanted done, on the account you wanted it on.
            let has_task = !cfg.prewarm.task.trim().is_empty();
            let _ = sub.append(&MenuItem::with_id(
                format!("prewarm:task:{}", acc.number),
                if has_task {
                    lang.prewarm_task_run(&name)
                } else {
                    lang.prewarm_task_unset().to_string()
                },
                has_task,
                None,
            ));
        }
        if config::prewarm_output_path().exists() {
            let _ = sub.append(&MenuItem::with_id(
                "prewarm:output",
                lang.open_last_output(),
                true,
                None,
            ));
        }
    }

    let _ = sub.append(&PredefinedMenuItem::separator());
    for line in lang.prewarm_help() {
        let _ = sub.append(&MenuItem::new(format!("   {line}"), false, None));
    }
    sub
}

/// Icon legend, so nobody has to remember what each bar means.
fn legend_submenu(lang: Lang) -> Submenu {
    let sub = Submenu::new(lang.legend_title(), true);
    for line in lang.legend() {
        let _ = sub.append(&MenuItem::new(format!("   {line}"), false, None));
    }
    sub
}

fn pct_of(w: Option<&cswap::Window>) -> String {
    match w {
        Some(w) => format!("{:.0}%", w.effective_pct()),
        None => "—".to_string(),
    }
}

/// "5h 100% · 7d 63%" — compact form, for the tooltip (127-char limit).
fn usage_short(acc: &cswap::Account) -> String {
    format!(
        "5h {} · 7d {}",
        pct_of(acc.window(cswap::Win::FiveHour)),
        pct_of(acc.window(cswap::Win::SevenDay))
    )
}

/// Menu form: adds when each window resets.
fn usage_long(acc: &cswap::Account) -> String {
    let detail = |w: Option<&cswap::Window>| match w {
        Some(w) => match (w.clock.as_deref(), w.countdown.as_deref()) {
            (Some(clock), Some(left)) => format!(" (→{clock}, {left})"),
            (Some(clock), None) => format!(" (→{clock})"),
            _ => String::new(),
        },
        None => String::new(),
    };
    let five = acc.window(cswap::Win::FiveHour);
    let seven = acc.window(cswap::Win::SevenDay);
    format!(
        "5h {}{} · 7d {}{}",
        pct_of(five),
        detail(five),
        pct_of(seven),
        detail(seven)
    )
}

fn tooltip(cfg: &config::Config, state: &State, lang: Lang) -> String {
    if let Some(err) = &state.error {
        return truncate(&format!("cswap-tray: {err}"), TOOLTIP_MAX);
    }
    let mut lines: Vec<String> = Vec::new();
    // Active account first: it is the one the icon stands for.
    let mut ordered: Vec<&cswap::Account> = state.accounts().iter().collect();
    ordered.sort_by_key(|a| !a.active);
    for acc in ordered {
        let (name, _, _) = config::presentation(cfg, &acc.email, acc.number);
        let mark = if acc.active { "●" } else { "○" };
        lines.push(format!("{mark} {name}: {}", usage_short(acc)));
    }
    if lines.is_empty() {
        return lang.no_data().to_string();
    }
    truncate(&lines.join("\n"), TOOLTIP_MAX)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// Opens a file with its default application. Goes through explorer rather
/// than `cmd /c start` so no shell parses the path.
fn open_path(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

fn open_config() {
    let path = config::path();
    if !path.exists() {
        config::save(&config::load());
    }
    open_path(&path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_limit() {
        let long = "x".repeat(300);
        assert_eq!(truncate(&long, TOOLTIP_MAX).chars().count(), TOOLTIP_MAX);
        assert_eq!(truncate("short", TOOLTIP_MAX), "short");
    }
}
