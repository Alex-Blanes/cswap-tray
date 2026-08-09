//! Presentation config: how each account looks in the tray, plus the knobs for
//! auto-switching and prewarming.
//!
//! It lives in `%APPDATA%\cswap-tray\config.json` and is generated on first run
//! from whatever accounts cswap reports. Accounts themselves are still added
//! and removed with `cswap` — this file only decides how they are shown and
//! how the tray behaves.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Consumer mail domains: assumed to be a personal account.
const FREE_PROVIDERS: &[&str] = &[
    "hotmail.com", "outlook.com", "outlook.es", "live.com", "gmail.com",
    "googlemail.com", "yahoo.com", "yahoo.es", "icloud.com", "me.com",
    "proton.me", "protonmail.com", "gmx.com", "aol.com",
];

/// Default colours, in assignment order.
const PALETTE: &[&str] = &["#3B82F6", "#F59E0B", "#10B981", "#A855F7", "#EC4899", "#14B8A6"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCfg {
    /// Readable name in the menu.
    pub name: String,
    /// Character drawn on the icon (A-Z, 0-9).
    pub letter: String,
    /// Colour of the letter, "#RRGGBB".
    pub color: String,
}

/// Auto-switch policy with a preferred account.
///
/// `cswap auto` has no notion of a preferred account: its `best` strategy jumps
/// to whichever has the most quota and stays there. The semantics here are
/// "live on the preferred one, fall back when it runs out, and return as soon
/// as it recovers". That is why the two engines must not run at the same time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoCfg {
    #[serde(default)]
    pub enabled: bool,
    /// Email of the account you want to be on by default.
    #[serde(default)]
    pub preferred: Option<String>,
    /// The active account is abandoned when its binding window reaches this.
    #[serde(default = "default_switch_at")]
    pub switch_at_pct: f32,
    /// Return to the preferred account once it drops below this. The gap with
    /// `switch_at_pct` is the hysteresis that prevents constant flapping.
    #[serde(default = "default_return_below")]
    pub return_below_pct: f32,
    /// Minimum wait between automatic switches.
    #[serde(default = "default_cooldown")]
    pub cooldown_seconds: u64,
}

fn default_switch_at() -> f32 {
    90.0
}
fn default_return_below() -> f32 {
    80.0
}
fn default_cooldown() -> u64 {
    300
}

impl Default for AutoCfg {
    fn default() -> Self {
        AutoCfg {
            enabled: false,
            preferred: None,
            switch_at_pct: default_switch_at(),
            return_below_pct: default_return_below(),
            cooldown_seconds: default_cooldown(),
        }
    }
}

/// Prewarming the reserve account. See `prewarm.rs` for the why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrewarmCfg {
    /// Warn when you are working and the reserve has its clock stopped.
    #[serde(default = "yes")]
    pub notify: bool,
    /// If no message arrives within this time after arming "send my next
    /// message through the reserve", the switch is undone and you are told.
    #[serde(default = "default_arm_timeout")]
    pub arm_timeout_minutes: u64,
    /// A prompt of your own to run on the reserve account. Empty by default,
    /// and deliberately so: the point is to send that account **work you
    /// actually want done** and whose answer you read — the output is written
    /// to `last-prewarm-output.md` and reachable from the menu. A canned
    /// prompt nobody reads would just be a hollow request dressed up as work.
    #[serde(default)]
    pub task: String,
}

fn yes() -> bool {
    true
}
fn default_arm_timeout() -> u64 {
    15
}

impl Default for PrewarmCfg {
    fn default() -> Self {
        PrewarmCfg {
            notify: yes(),
            arm_timeout_minutes: default_arm_timeout(),
            task: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// UI language: "auto" (follow Windows), "en" or "es".
    #[serde(default = "default_language")]
    pub language: String,
    /// How often `cswap list --json` is re-read. cswap caches usage on disk, so
    /// polling does not imply API calls.
    #[serde(default = "default_refresh")]
    pub refresh_seconds: u64,
    #[serde(default)]
    pub auto: AutoCfg,
    #[serde(default)]
    pub prewarm: PrewarmCfg,
    /// Key: the account email exactly as cswap reports it.
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountCfg>,
}

fn default_language() -> String {
    "auto".to_string()
}
fn default_refresh() -> u64 {
    30
}

impl Default for Config {
    fn default() -> Self {
        Config {
            language: default_language(),
            refresh_seconds: default_refresh(),
            auto: AutoCfg::default(),
            prewarm: PrewarmCfg::default(),
            accounts: BTreeMap::new(),
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("cswap-tray").join("config.json")
}

/// Where the answer to a prewarm task is written, so it is work you receive
/// rather than a request into the void.
pub fn prewarm_output_path() -> PathBuf {
    path().with_file_name("last-prewarm-output.md")
}

pub fn load() -> Config {
    match std::fs::read_to_string(path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(p, s);
    }
}

/// Fills in accounts the config does not know yet. Returns `true` when
/// something changed (and therefore it is worth saving).
pub fn ensure_accounts(cfg: &mut Config, emails: &[String]) -> bool {
    let mut changed = false;
    for email in emails {
        if cfg.accounts.contains_key(email) {
            continue;
        }
        let idx = cfg.accounts.len();
        let personal = is_personal(email);
        let work_taken = cfg.accounts.values().any(|a| a.letter == "W");
        let (name, letter) = if personal {
            (String::from("Personal"), String::from("P"))
        } else if !work_taken {
            (domain_label(email), String::from("W"))
        } else {
            let l = domain_label(email).chars().next().unwrap_or('?').to_ascii_uppercase();
            (domain_label(email), l.to_string())
        };
        // The palette's blue is reserved for the personal account; the rest
        // follow behind.
        let color = if personal {
            PALETTE[0].to_string()
        } else {
            PALETTE[1 + (idx % (PALETTE.len() - 1))].to_string()
        };
        cfg.accounts.insert(email.clone(), AccountCfg { name, letter, color });
        changed = true;
    }
    changed
}

/// Default preferred account: the first personal one detected. That is the
/// usual setup — you work on the personal one and the corporate acts as
/// reserve — and it can always be changed from the menu.
pub fn default_preferred(emails: &[String]) -> Option<String> {
    emails.iter().find(|e| is_personal(e)).cloned()
}

fn domain(email: &str) -> &str {
    email.split('@').nth(1).unwrap_or("")
}

fn is_personal(email: &str) -> bool {
    let d = domain(email).to_ascii_lowercase();
    FREE_PROVIDERS.contains(&d.as_str())
}

/// "someone@acme-corp.com" -> "Acme-corp"
fn domain_label(email: &str) -> String {
    let d = domain(email);
    let base = d.split('.').next().unwrap_or(d);
    let mut chars = base.chars();
    match chars.next() {
        Some(f) => f.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::from("Work"),
    }
}

/// How an account should be shown, with a sane fallback when it is not in the
/// config yet (an account added while the tray was already running).
pub fn presentation(cfg: &Config, email: &str, number: u32) -> (String, char, [u8; 3]) {
    match cfg.accounts.get(email) {
        Some(a) => (
            a.name.clone(),
            a.letter.chars().next().unwrap_or('?'),
            parse_color(&a.color).unwrap_or([200, 200, 200]),
        ),
        None => {
            let letter = char::from_digit(number % 10, 10).unwrap_or('?');
            (email.to_string(), letter, [200, 200, 200])
        }
    }
}

pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_detection() {
        assert!(is_personal("someone@hotmail.com"));
        assert!(!is_personal("someone@acme-corp.com"));
    }

    #[test]
    fn defaults_assign_p_and_w() {
        let mut cfg = Config::default();
        let emails = vec![
            "someone@acme-corp.com".to_string(),
            "someone@hotmail.com".to_string(),
        ];
        assert!(ensure_accounts(&mut cfg, &emails));
        assert_eq!(cfg.accounts["someone@acme-corp.com"].letter, "W");
        assert_eq!(cfg.accounts["someone@acme-corp.com"].name, "Acme-corp");
        assert_eq!(cfg.accounts["someone@hotmail.com"].letter, "P");
        assert_eq!(cfg.accounts["someone@hotmail.com"].color, PALETTE[0]);
        // Idempotent: a second pass changes nothing.
        assert!(!ensure_accounts(&mut cfg, &emails));
    }

    #[test]
    fn color_parsing() {
        assert_eq!(parse_color("#3B82F6"), Some([59, 130, 246]));
        assert_eq!(parse_color("3B82F6"), Some([59, 130, 246]));
        assert_eq!(parse_color("nope"), None);
    }
}
