//! cswap-tray — switch Claude Code accounts from the Windows tray.
//!
//! A thin layer over the claude-swap CLI: `cswap` stays the owner of the
//! credentials, the quota readings and the switch itself. This only paints the
//! state and forwards orders.
//!
//! - Left click: rotate to the next account (`cswap switch`).
//! - Right click: menu with the accounts, their usage and the actions.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod auto;
mod clipboard;
mod config;
mod cswap;
mod deepseek;
mod i18n;
mod icon;
mod nas;
mod prewarm;

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use i18n::Lang;
use tauri_winrt_notification::{Duration as ToastDuration, Toast};
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
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
    NasRefresh,
    NasSwitch {
        container: String,
        account: u32,
    },
    NasAuto {
        container: String,
        on: bool,
    },
    /// One key of a container's `cswap-preferida.json`.
    NasSet {
        container: String,
        key: &'static str,
        value: serde_json::Value,
    },
    NasDeepseek {
        container: String,
    },
}

type SnapResult = Result<cswap::Snapshot, String>;
type NasResult = Result<nas::Snapshot, String>;

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
    // An older version switched DeepSeek on by patching the user's settings.
    // Undo that once, before it can poison a session, and lay down the
    // per-session launcher this version uses instead.
    deepseek::disarm();
    if cfg.deepseek.available() {
        let _ = deepseek::install(&cfg.deepseek);
    } else {
        deepseek::uninstall();
    }
    let lang = Lang::resolve(&cfg.language);
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (snap_tx, snap_rx) = channel::<SnapResult>();
    let (nas_tx, nas_rx) = channel::<NasResult>();
    spawn_worker(
        cfg.refresh_seconds,
        cfg.nas.clone(),
        cfg.deepseek.clone(),
        lang,
        cmd_rx,
        snap_tx,
        nas_tx,
    );

    let icon_size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    let mut state = State {
        snapshot: None,
        error: None,
        nas: None,
        nas_error: None,
        last_auto_switch: None,
        armed: None,
        cold_notified: false,
        relogin_notified: std::collections::BTreeSet::new(),
        deepseek_offered: false,
    };

    let tray = TrayIconBuilder::new()
        .with_id("local")
        .with_menu(Box::new(Menu::new()))
        .with_menu_on_left_click(false)
        // The menu is built when it is about to be shown, so the tray opens it
        // itself rather than on the shell's right click. See `show_fresh_menu`.
        .with_menu_on_right_click(false)
        .with_tooltip(lang.loading())
        .with_icon(make_icon(&cfg, &state, icon_size))
        .build()
        .expect("could not create the tray icon");
    // The NAS gets an icon of its own: you may be working on both machines at
    // once, each on a different subscription, and both are worth a glance.
    let nas_tray = cfg.nas.enabled().then(|| {
        TrayIconBuilder::new()
            .with_id("nas")
            .with_menu(Box::new(Menu::new()))
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(false)
            .with_tooltip(nas_tooltip(&cfg, &state, lang))
            .with_icon(make_nas_icon(&cfg, &state, icon_size))
            .build()
            .expect("could not create the NAS tray icon")
    });

    let _ = cmd_tx.send(Cmd::Refresh);
    run_message_loop(
        &tray,
        nas_tray.as_ref(),
        &mut cfg,
        &mut state,
        lang,
        icon_size,
        &cmd_tx,
        &snap_rx,
        &nas_rx,
    );
}

struct State {
    snapshot: Option<cswap::Snapshot>,
    error: Option<String>,
    /// Accounts and auto state of the Claude Code containers on the NAS.
    nas: Option<nas::Snapshot>,
    /// Why the NAS could not be read. Kept apart from `error`: a NAS outage
    /// must not blank the local accounts.
    nas_error: Option<String>,
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
    /// DeepSeek already offered during this dry spell, so the fallback copies
    /// the command once and not again every cooldown. It is re-armed as soon
    /// as an account has quota again.
    deepseek_offered: bool,
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

fn spawn_worker(
    refresh_seconds: u64,
    nas_cfg: config::NasCfg,
    deepseek_cfg: config::DeepSeekCfg,
    lang: Lang,
    cmd_rx: Receiver<Cmd>,
    snap_tx: Sender<SnapResult>,
    nas_tx: Sender<NasResult>,
) {
    let period = Duration::from_secs(refresh_seconds.clamp(5, 3600));
    let nas_period = Duration::from_secs(nas_cfg.refresh_seconds.clamp(10, 3600));
    std::thread::spawn(move || {
        let mut last_nas: Option<Instant> = None;
        loop {
            let cmd = match cmd_rx.recv_timeout(period) {
                Ok(c) => Some(c),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Some(Cmd::Refresh),
                // The main thread is gone: follow it out.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let mut refresh_nas = false;
            match cmd {
                Some(Cmd::Switch(target)) => {
                    if let Err(e) = cswap::switch(target) {
                        if snap_tx.send(Err(e)).is_err() {
                            break;
                        }
                        continue;
                    }
                }
                Some(Cmd::NasRefresh) => refresh_nas = true,
                Some(Cmd::NasSwitch { container, account }) => {
                    match nas::switch(&nas_cfg, &container, account) {
                        Ok(()) => notify(
                            &lang.nas_switched_title(&container),
                            &lang.nas_switched_body(account),
                        ),
                        Err(e) => {
                            notify(lang.nas_failed(), &container);
                            if nas_tx.send(Err(e)).is_err() {
                                break;
                            }
                        }
                    }
                    refresh_nas = true;
                }
                Some(Cmd::NasAuto { container, on }) => {
                    if let Err(e) = nas::set_auto(&nas_cfg, &container, on) {
                        if nas_tx.send(Err(e)).is_err() {
                            break;
                        }
                    }
                    refresh_nas = true;
                }
                Some(Cmd::NasSet { container, key, value }) => {
                    if let Err(e) = nas::set_setting(&nas_cfg, &container, key, &value) {
                        notify(lang.nas_failed(), &container);
                        if nas_tx.send(Err(e)).is_err() {
                            break;
                        }
                    }
                    refresh_nas = true;
                }
                Some(Cmd::NasDeepseek { container }) => {
                    match deepseek::key(deepseek_cfg.python(), deepseek_cfg.script.trim()) {
                        Some(key) => {
                            if let Err(e) = nas::open_deepseek(&nas_cfg, &container, &key) {
                                notify(lang.nas_deepseek_failed(), &container);
                                let _ = nas_tx.send(Err(e));
                            }
                        }
                        None => notify(
                            lang.deepseek_no_key_title(),
                            lang.deepseek_no_key_body(),
                        ),
                    }
                }
                Some(Cmd::Refresh) | None => {}
            }
            // Its own cadence: the NAS round trip must not set the pace of the
            // local account polling, and vice versa.
            if nas_cfg.enabled()
                && (refresh_nas || last_nas.is_none_or(|t| t.elapsed() >= nas_period))
            {
                let _ = nas_tx.send(nas::list(&nas_cfg));
                last_nas = Some(Instant::now());
            }
            if snap_tx.send(cswap::list()).is_err() {
                break;
            }
        }
    });
}

fn run_message_loop(
    tray: &TrayIcon,
    nas_tray: Option<&TrayIcon>,
    cfg: &mut config::Config,
    state: &mut State,
    lang: Lang,
    icon_size: u32,
    cmd_tx: &Sender<Cmd>,
    snap_rx: &Receiver<SnapResult>,
    nas_rx: &Receiver<NasResult>,
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
        while let Ok(result) = nas_rx.try_recv() {
            match result {
                Ok(snap) => {
                    state.nas = Some(snap);
                    state.nas_error = None;
                }
                Err(e) => state.nas_error = Some(e),
            }
            dirty = true;
        }

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
                    capture_relogin(cfg, state, lang);
                    check_relogin(cfg, state, lang);
                    if state.armed.is_some() {
                        follow_up_prewarm(cfg, state, lang, cmd_tx);
                    } else {
                        apply_auto_policy(cfg, state, lang, cmd_tx);
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
                "rotate" => order_switch(cmd_tx, None),
                "deepseek:copy" => {
                    if let Some(command) = copy_deepseek_command(cfg, state, lang, false) {
                        notify(lang.deepseek_copied_title(), &lang.deepseek_copied_body(&command));
                    }
                    dirty = true;
                }
                "deepseek:copy:resume" => {
                    if let Some(command) = copy_deepseek_command(cfg, state, lang, true) {
                        notify(lang.deepseek_copied_title(), &lang.deepseek_copied_body(&command));
                    }
                    dirty = true;
                }
                "tui" => cswap::open_tui(),
                "config" => open_config(),
                "nas:refresh" => {
                    let _ = cmd_tx.send(Cmd::NasRefresh);
                }
                "auto:toggle" => {
                    cfg.auto.enabled = !cfg.auto.enabled;
                    config::save(cfg);
                    // Once switched on, let it act right away if it applies.
                    state.last_auto_switch = None;
                    state.deepseek_offered = false;
                    apply_auto_policy(cfg, state, lang, cmd_tx);
                    dirty = true;
                }
                "prewarm:output" => open_path(&config::prewarm_output_path()),
                "prewarm:cancel" => {
                    if let Some(armed) = state.armed.take() {
                        order_switch(cmd_tx, Some(armed.back_to));
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
                        order_switch(cmd_tx, Some(n));
                        dirty = true;
                    } else if let Some(n) = id.strip_prefix("relogin:").and_then(|n| n.parse::<u32>().ok())
                    {
                        if let Some(acc) = state.accounts().iter().find(|a| a.number == n) {
                            start_relogin_guide(n, &acc.email, lang.code());
                        }
                    } else if let Some(n) = id.strip_prefix("prefer:").and_then(|n| n.parse::<u32>().ok())
                    {
                        if let Some(acc) = state.accounts().iter().find(|a| a.number == n) {
                            cfg.auto.preferred = Some(acc.email.clone());
                            config::save(cfg);
                            dirty = true;
                        }
                    } else if let Some(rest) = id.strip_prefix("nas:switch:") {
                        if let Some((container, n)) = rest.rsplit_once(':') {
                            if let Ok(account) = n.parse::<u32>() {
                                let _ = cmd_tx.send(Cmd::NasSwitch {
                                    container: container.to_string(),
                                    account,
                                });
                                dirty = true;
                            }
                        }
                    } else if let Some(rest) = id.strip_prefix("nas:auto:") {
                        if let Some((container, state)) = rest.rsplit_once(':') {
                            if let Some(on) = match state {
                                "on" => Some(true),
                                "off" => Some(false),
                                _ => None,
                            } {
                                let _ = cmd_tx.send(Cmd::NasAuto {
                                    container: container.to_string(),
                                    on,
                                });
                                dirty = true;
                            }
                        }
                    } else if let Some(rest) = id.strip_prefix("nas:prefer:") {
                        if let Some((container, n)) = rest.rsplit_once(':') {
                            if let Ok(number) = n.parse::<u32>() {
                                let email = state
                                    .nas
                                    .as_ref()
                                    .and_then(|s| {
                                        s.containers.iter().find(|c| c.name == container)
                                    })
                                    .and_then(|c| {
                                        c.accounts.iter().find(|a| a.number == number)
                                    })
                                    .map(|a| a.email.clone());
                                if let Some(email) = email {
                                    let _ = cmd_tx.send(Cmd::NasSet {
                                        container: container.to_string(),
                                        key: "preferred",
                                        value: email.into(),
                                    });
                                    dirty = true;
                                }
                            }
                        }
                    } else if let Some(rest) = id.strip_prefix("nas:at:") {
                        if let Some((container, p)) = rest.rsplit_once(':') {
                            if let Ok(pct) = p.parse::<f32>() {
                                let _ = cmd_tx.send(Cmd::NasSet {
                                    container: container.to_string(),
                                    key: "switch_at",
                                    value: pct.into(),
                                });
                                dirty = true;
                            }
                        }
                    } else if let Some(p) = id.strip_prefix("auto:at:").and_then(|p| p.parse().ok()) {
                        cfg.auto.switch_at_pct = p;
                        config::save(cfg);
                        apply_auto_policy(cfg, state, lang, cmd_tx);
                        dirty = true;
                    } else if let Some(container) = id.strip_prefix("nas:deepseek:") {
                        let _ = cmd_tx.send(Cmd::NasDeepseek {
                            container: container.to_string(),
                        });
                        dirty = true;
                    }
                }
            }
        }

        while let Ok(event) = tray_events.try_recv() {
            if event.id().0 == "nas" {
                if let Some(nas_tray) = nas_tray {
                    handle_nas_click(event, nas_tray, cfg, state, lang, cmd_tx);
                }
                continue;
            }
            match event {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
                    // With two accounts this toggles; with more, it advances in order.
                    if state.accounts().len() > 1 {
                        order_switch(cmd_tx, None);
                        dirty = true;
                    }
                }
                TrayIconEvent::Click { button: MouseButton::Right, button_state: MouseButtonState::Up, .. } => {
                    // Asynchronous: the menu paints the cached answer and this
                    // refresh lands for the next opening. Waiting here would
                    // stall the menu behind an SSH round trip.
                    show_fresh_menu(tray, cfg, state, lang);
                }
                _ => {}
            }
        }

        if dirty {
            let _ = tray.set_icon(Some(make_icon(cfg, state, icon_size)));
            let _ = tray.set_tooltip(Some(tooltip(cfg, state, lang)));
            if let Some(nas_tray) = nas_tray {
                let _ = nas_tray.set_icon(Some(make_nas_icon(cfg, state, icon_size)));
                let _ = nas_tray.set_tooltip(Some(nas_tooltip(cfg, state, lang)));
            }
        }
    }
}

/// Left click moves the NAS host to its next account; right click opens the
/// NAS menu. Like the local one, the menu paints the cached reading and the
/// refresh it triggers lands for the next opening.
fn handle_nas_click(
    event: TrayIconEvent,
    nas_tray: &TrayIcon,
    cfg: &config::Config,
    state: &State,
    lang: Lang,
    cmd_tx: &Sender<Cmd>,
) {
    let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, .. } = event else {
        return;
    };
    match button {
        MouseButton::Left => {
            if let Some(account) = nas_host(state).and_then(|h| next_account(&h.accounts)) {
                let _ = cmd_tx.send(Cmd::NasSwitch { container: nas::HOST.to_string(), account });
            }
        }
        MouseButton::Right => {
            let _ = cmd_tx.send(Cmd::NasRefresh);
            nas_tray.set_menu(Some(Box::new(build_nas_menu(cfg, state, lang))));
            nas_tray.show_menu();
        }
        _ => {}
    }
}

/// Evaluates the auto-switch policy against the latest snapshot and orders the
/// switch when it applies. The cooldown lives here so the decision itself
/// (`auto::decide`) stays a pure, testable function.
fn apply_auto_policy(cfg: &config::Config, state: &mut State, lang: Lang, cmd_tx: &Sender<Cmd>) {
    if !cfg.auto.enabled {
        return;
    }
    let cooldown = Duration::from_secs(cfg.auto.cooldown_seconds);
    if state.last_auto_switch.is_some_and(|t| t.elapsed() < cooldown) {
        return;
    }
    let Some(target) = auto::decide(&cfg.auto, state.accounts()) else {
        // Nowhere left to go on the subscriptions. DeepSeek is the last
        // resort, and this is the only place the tray acts on its own — hence
        // the `fallback` setting guarding it, and hence `everything_spent`
        // refusing to act on a usage reading it could not trust. It copies
        // the command and nothing else: pasting it spends nothing, and the
        // other sessions keep their own account.
        if cfg.deepseek.available()
            && cfg.deepseek.fallback
            && !state.deepseek_offered
            && auto::everything_spent(&cfg.auto, state.accounts())
        {
            state.last_auto_switch = Some(Instant::now());
            if let Some(command) = copy_deepseek_command(cfg, state, lang, false) {
                notify(
                    lang.deepseek_fallback_title(),
                    &lang.deepseek_fallback_body(&command),
                );
            }
        }
        return;
    };
    state.deepseek_offered = false;
    state.last_auto_switch = Some(Instant::now());
    // Worked out before the order goes out: a policy that moves you around
    // unannounced reads as one that does nothing at all.
    let note = state.active().map(|from| {
        let pct = from.binding_pct().unwrap_or(0.0);
        let name = config::presentation(cfg, &from.email, from.number).0;
        let to = state.name_of(cfg, lang, target);
        (lang.auto_switched_title(&name, &to), lang.auto_switched_body(&name, pct))
    });
    order_switch(cmd_tx, Some(target));
    if let Some((title, body)) = note {
        notify(&title, &body);
    }
}

/// Every account change goes through here, so the policy has one door.
fn order_switch(cmd_tx: &Sender<Cmd>, target: Option<u32>) {
    let _ = cmd_tx.send(Cmd::Switch(target));
}

/// Copies the DeepSeek command, reporting it when it landed on the clipboard.
/// The caller does the talking, because a click and the fallback do not say the
/// same thing.
///
/// Refused without a key rather than copied: the helper would print nothing,
/// and Claude Code would have no credential of its own to use. The launcher is
/// rewritten first, so `claude-ds` always matches the current config.
fn copy_deepseek_command(
    cfg: &config::Config,
    state: &mut State,
    lang: Lang,
    resume: bool,
) -> Option<String> {
    state.deepseek_offered = true;
    if !deepseek::key_available(cfg.deepseek.python(), cfg.deepseek.script.trim()) {
        notify(lang.deepseek_no_key_title(), lang.deepseek_no_key_body());
        return None;
    }
    if let Err(e) = deepseek::install(&cfg.deepseek) {
        notify(lang.deepseek_failed_title(), &e);
        state.error = Some(e);
        return None;
    }
    let command = deepseek::command(resume);
    match clipboard::copy(&command) {
        Ok(()) => Some(command),
        Err(e) => {
            notify(lang.deepseek_failed_title(), &e);
            state.error = Some(e);
            None
        }
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
        .filter(|a| a.needs_relogin())
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
    // Nothing dead left to repair, so a watcher still armed is a guide window
    // that was closed without logging in. Disarm it, or the next ordinary
    // token refresh would look like our landing.
    if dead.is_empty() {
        disarm_relogin();
    }
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
            start_relogin_guide(number, &mail, code);
            Ok(())
        })
        .show();
}

/// Armed when a guided re-login opens, holding the credential store's stamp
/// from that moment. The outer `None` means "not watching"; `Some(None)` means
/// "watching, and the store was unreadable then" — which a first login turns
/// into a landing by creating the file.
///
/// A static rather than a field on `State` because the toast's `on_activated`
/// fires on Windows' own thread, not the poll loop that owns `State`.
static RELOGIN_PENDING: Mutex<Option<Option<SystemTime>>> = Mutex::new(None);

/// Opens the guided re-login and starts watching for the login in one move, so
/// no call site can open the guide and forget to record the "before" stamp.
///
/// The stamp is read *before* the window opens: anything written from here on
/// counts as the login we asked for.
fn start_relogin_guide(number: u32, email: &str, lang_code: &'static str) {
    if let Ok(mut pending) = RELOGIN_PENDING.lock() {
        *pending = Some(cswap::login_stamp());
    }
    cswap::open_relogin_guide(&config::dir(), number, email, lang_code);
}

fn disarm_relogin() {
    if let Ok(mut pending) = RELOGIN_PENDING.lock() {
        *pending = None;
    }
}

/// Saves the new credential the moment a login lands, so the repair finishes
/// without you quitting Claude Code first.
///
/// The guide runs `cswap add` too, but only after `claude` exits — and nobody
/// quits Claude Code just to bank a token. This gets there first; the guide's
/// own call then finds the work already done and simply confirms it.
///
/// The evidence is the credential store being written after the guide opened.
/// That also fires on an ordinary token refresh, which is harmless: `cswap add`
/// matches by email, so the worst case is re-capturing a credential that was
/// already correct, and an account still broken is flagged again next poll.
fn capture_relogin(cfg: &config::Config, state: &mut State, lang: Lang) {
    let armed = match RELOGIN_PENDING.lock() {
        Ok(pending) => *pending,
        Err(_) => return,
    };
    let Some(before) = armed else {
        return;
    };
    if !login_landed(before, cswap::login_stamp()) {
        return;
    }
    disarm_relogin();
    match cswap::add_current() {
        Ok(()) => {
            let name = match state.accounts().iter().find(|a| a.active) {
                Some(a) => config::presentation(cfg, &a.email, a.number).0,
                None => String::new(),
            };
            // `relogin_notified` is deliberately left alone: the snapshot in
            // hand still shows the account as dead, and clearing it here would
            // make `check_relogin` fire a second toast on this very poll. The
            // next snapshot drops the number on its own.
            notify(&lang.relogin_done_title(&name), lang.relogin_done_body());
        }
        Err(e) => state.error = Some(e),
    }
}

/// Whether the credential store was written since the guide opened.
///
/// A store that cannot be read now proves nothing, so it is not a landing. One
/// that was unreadable before and is readable now is: a first login writes the
/// file that was not there.
fn login_landed(before: Option<SystemTime>, now: Option<SystemTime>) -> bool {
    now.is_some() && now != before
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
        .and_then(|a| a.window(cswap::Win::FiveHour).map(|w| (true, w.pct.unwrap_or(0.0))))
        .unwrap_or((false, 0.0));
    let name = state.name_of(cfg, lang, target);

    state.armed = Some(Armed { target, back_to, had_window, base_pct, since: Instant::now() });
    order_switch(cmd_tx, Some(target));
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
    order_switch(cmd_tx, Some(back_to));

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
    account_icon(cfg, state.accounts(), state.error.is_some(), false, size)
}

fn make_nas_icon(cfg: &config::Config, state: &State, size: u32) -> Icon {
    let accounts = nas_host(state).map(|c| c.accounts.as_slice()).unwrap_or(&[]);
    account_icon(cfg, accounts, state.nas_error.is_some(), true, size)
}

/// Letter and bars of whichever account is active in `accounts`.
fn account_icon(
    cfg: &config::Config,
    accounts: &[cswap::Account],
    failed: bool,
    remote: bool,
    size: u32,
) -> Icon {
    let alert = accounts.iter().any(|a| a.needs_relogin());
    let spec = match accounts.iter().find(|a| a.active) {
        Some(acc) => {
            let (_, letter, color) = config::presentation(cfg, &acc.email, acc.number);
            icon::IconSpec {
                letter,
                color,
                five_hour: acc.pct(cswap::Win::FiveHour),
                seven_day: acc.pct(cswap::Win::SevenDay),
                stale: !acc.is_ok(),
                alert,
                remote,
            }
        }
        None => icon::IconSpec {
            letter: if failed { '!' } else { '?' },
            color: [200, 200, 200],
            five_hour: None,
            seven_day: None,
            stale: true,
            alert,
            remote,
        },
    };
    Icon::from_rgba(icon::render(&spec, size), size, size).expect("invalid icon buffer")
}

/// Builds the menu from the latest snapshot and opens it.
///
/// Building on demand rather than on every poll is not a refinement, it is the
/// fix for a tray whose menu went dead after half a day up. `muda` hands each
/// item it creates an id from a counter that never reuses a value, and Windows
/// delivers only the low 16 bits of that id in `WM_COMMAND`. Rebuilding the
/// whole menu twice a minute burned through the 65 535 usable ids in about
/// twelve hours, after which no click could be matched to its item and every
/// entry silently did nothing.
///
/// ponytail: still finite — roughly a thousand openings per run. If that is
/// ever reached, the real cure is updating the existing items in place instead
/// of recreating them, which costs stable handles for every dynamic label.
fn show_fresh_menu(tray: &TrayIcon, cfg: &config::Config, state: &State, lang: Lang) {
    tray.set_menu(Some(Box::new(build_menu(cfg, state, lang))));
    tray.show_menu();
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
        let (id, label, enabled) = if !acc.needs_relogin() {
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

    if cfg.deepseek.available() {
        let _ = menu.append(&deepseek_submenu(lang));
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

/// DeepSeek submenu: the two commands the user may want on the clipboard, and
/// the one thing about them that is not obvious from the labels — the session
/// name that makes a DeepSeek conversation recognisable in the `-r` picker.
fn deepseek_submenu(lang: Lang) -> Submenu {
    let sub = Submenu::new(lang.deepseek_label(), true);
    let _ = sub.append(&MenuItem::with_id(
        "deepseek:copy",
        lang.deepseek_copy_new(),
        true,
        None,
    ));
    let _ = sub.append(&MenuItem::with_id(
        "deepseek:copy:resume",
        lang.deepseek_copy_resume(),
        true,
        None,
    ));
    let _ = sub.append(&PredefinedMenuItem::separator());
    for line in lang.deepseek_help() {
        let _ = sub.append(&MenuItem::new(format!("   {line}"), false, None));
    }
    sub
}

/// The NAS's own Claude Code, the one the second icon stands for.
fn nas_host(state: &State) -> Option<&nas::Container> {
    state.nas.as_ref()?.containers.iter().find(|c| c.name == nas::HOST)
}

/// Menu of the NAS icon: the host's entries at the top level, then one
/// submenu per container. The containers are discovered on the NAS, not
/// configured by hand: a new project appears on the next refresh.
fn build_nas_menu(cfg: &config::Config, state: &State, lang: Lang) -> Menu {
    let menu = Menu::new();
    if let Some(err) = &state.nas_error {
        let _ = menu.append(&MenuItem::new(format!("⚠ {}", truncate(err, 60)), false, None));
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    let containers = state.nas.as_ref().map(|s| s.containers.as_slice()).unwrap_or(&[]);
    if containers.is_empty() && state.nas_error.is_none() {
        let _ = menu.append(&MenuItem::new(lang.nas_waiting(), false, None));
    }
    if let Some(host) = nas_host(state) {
        for item in container_items(cfg, lang, host) {
            let _ = menu.append(item.as_ref());
        }
    }
    let others: Vec<&nas::Container> = containers.iter().filter(|c| c.name != nas::HOST).collect();
    if !others.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
    }
    for c in &others {
        let active = c
            .accounts
            .iter()
            .find(|a| a.active)
            .map(|a| config::presentation(cfg, &a.email, a.number).0)
            .unwrap_or_else(|| lang.not_set().to_string());
        let csub = Submenu::new(format!("{} — {}", c.name, active), true);
        for item in container_items(cfg, lang, c) {
            let _ = csub.append(item.as_ref());
        }
        let _ = menu.append(&csub);
    }
    if !containers.is_empty() {
        let _ = menu.append(&PredefinedMenuItem::separator());
        for line in lang.nas_help() {
            let _ = menu.append(&MenuItem::new(format!("   {line}"), false, None));
        }
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("nas:refresh", lang.refresh(), true, None));
    menu
}

/// Accounts and controls of one NAS target, for the host's top level or a
/// container's submenu alike.
fn container_items(cfg: &config::Config, lang: Lang, c: &nas::Container) -> Vec<Box<dyn IsMenuItem>> {
    let mut items: Vec<Box<dyn IsMenuItem>> = Vec::new();
    for acc in &c.accounts {
        let (name, letter, _) = config::presentation(cfg, &acc.email, acc.number);
        let mark = if acc.active { "●" } else { "○" };
        items.push(Box::new(MenuItem::with_id(
            format!("nas:switch:{}:{}", c.name, acc.number),
            format!("{mark} {name} ({letter}) — {}", usage_long(acc)),
            !acc.active,
            None,
        )));
    }
    if c.accounts.is_empty() {
        items.push(Box::new(MenuItem::new(lang.nas_no_data(), false, None)));
    }

    items.push(Box::new(PredefinedMenuItem::separator()));
    // Clicking toggles: the label says what a click will do, and the id
    // carries the state to land in.
    let (label, next) = if c.auto {
        (lang.nas_auto_on(), "off")
    } else {
        (lang.nas_auto_off(), "on")
    };
    items.push(Box::new(MenuItem::with_id(
        format!("nas:auto:{}:{}", c.name, next),
        label,
        true,
        None,
    )));
    if !c.accounts.is_empty() {
        let pref_name = c
            .preferred
            .as_deref()
            .and_then(|email| {
                c.accounts
                    .iter()
                    .find(|a| a.email == email)
                    .map(|a| config::presentation(cfg, &a.email, a.number).0)
            })
            .unwrap_or_else(|| lang.nas_preferred_unset().to_string());
        let psub = Submenu::new(lang.nas_preferred_title(&pref_name), true);
        for acc in &c.accounts {
            let (name, _, _) = config::presentation(cfg, &acc.email, acc.number);
            let chosen = c.preferred.as_deref() == Some(acc.email.as_str());
            let mark = if chosen { "✓" } else { "  " };
            let _ = psub.append(&MenuItem::with_id(
                format!("nas:prefer:{}:{}", c.name, acc.number),
                format!("{mark} {name}"),
                !chosen,
                None,
            ));
        }
        items.push(Box::new(psub));
        // Unset means the script's default, which mirrors the tray's.
        let at = c.switch_at.unwrap_or_else(config::default_switch_at);
        items.push(Box::new(switch_at_submenu(lang, at, &format!("nas:at:{}:", c.name))));
    }
    if cfg.deepseek.available() {
        items.push(Box::new(MenuItem::with_id(
            format!("nas:deepseek:{}", c.name),
            lang.nas_deepseek(),
            true,
            None,
        )));
    }
    items
}

/// "Switch at: 95%" with one entry per choice; the id is `prefix` + the number.
fn switch_at_submenu(lang: Lang, current: f32, prefix: &str) -> Submenu {
    let sub = Submenu::new(lang.switch_at_title(current), true);
    for p in config::SWITCH_AT_CHOICES {
        let chosen = (p - current).abs() < 0.5;
        let mark = if chosen { "✓" } else { "  " };
        let _ = sub.append(&MenuItem::with_id(format!("{prefix}{p}"), format!("{mark} {p:.0}%"), !chosen, None));
    }
    sub
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
    let _ = sub.append(&switch_at_submenu(lang, a.switch_at_pct, "auto:at:"));
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
    match w.and_then(|w| w.effective_pct()) {
        Some(p) => format!("{p:.0}%"),
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

fn nas_tooltip(cfg: &config::Config, state: &State, lang: Lang) -> String {
    if let Some(err) = &state.nas_error {
        return truncate(&format!("{}: {err}", lang.nas_unreachable()), TOOLTIP_MAX);
    }
    let Some(host) = nas_host(state) else {
        return format!("{} · {}", lang.nas_label(), lang.loading());
    };
    let mut lines = vec![lang.nas_label().to_string()];
    let mut ordered: Vec<&cswap::Account> = host.accounts.iter().collect();
    ordered.sort_by_key(|a| !a.active);
    for acc in ordered {
        let (name, _, _) = config::presentation(cfg, &acc.email, acc.number);
        let mark = if acc.active { "●" } else { "○" };
        lines.push(format!("{mark} {name}: {}", usage_short(acc)));
    }
    truncate(&lines.join("\n"), TOOLTIP_MAX)
}

/// The account after the active one, wrapping: what a left click moves to.
fn next_account(accounts: &[cswap::Account]) -> Option<u32> {
    if accounts.len() < 2 {
        return None;
    }
    let i = accounts.iter().position(|a| a.active).unwrap_or(accounts.len() - 1);
    Some(accounts[(i + 1) % accounts.len()].number)
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
    fn a_left_click_moves_to_the_next_account_and_wraps() {
        let accs = |active: u32| {
            let json = format!(
                r#"{{"schemaVersion":1,"accounts":[
                    {{"number":1,"email":"a@x.com","active":{}}},
                    {{"number":2,"email":"b@x.com","active":{}}},
                    {{"number":3,"email":"c@x.com","active":{}}}]}}"#,
                active == 1,
                active == 2,
                active == 3
            );
            serde_json::from_str::<cswap::Snapshot>(&json).unwrap().accounts
        };
        assert_eq!(next_account(&accs(1)), Some(2));
        assert_eq!(next_account(&accs(3)), Some(1));
        assert_eq!(next_account(&accs(1)[..1]), None, "a single account has nowhere to go");
    }

    #[test]
    fn truncate_respects_limit() {
        let long = "x".repeat(300);
        assert_eq!(truncate(&long, TOOLTIP_MAX).chars().count(), TOOLTIP_MAX);
        assert_eq!(truncate("short", TOOLTIP_MAX), "short");
    }

    #[test]
    fn a_login_lands_only_when_the_store_is_written() {
        let before = SystemTime::UNIX_EPOCH;
        let after = before + Duration::from_secs(1);
        assert!(login_landed(Some(before), Some(after)));
        assert!(login_landed(None, Some(after)), "a first login creates the file");
        assert!(!login_landed(Some(before), Some(before)), "untouched is not a landing");
        assert!(!login_landed(Some(before), None), "an unreadable store proves nothing");
        assert!(!login_landed(None, None));
    }
}
