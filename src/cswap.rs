//! Bridge to the claude-swap CLI.
//!
//! All the account, credential and quota logic lives in `cswap`. Here we only
//! invoke `cswap list --json` (a versioned contract, schemaVersion 1) and
//! `cswap switch`, and deserialize what the tray needs.

use serde::Deserialize;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Keeps a console window from flashing when the CLI is invoked from a
/// windowed app.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub number: u32,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub usage_status: String,
    #[serde(default)]
    pub usage: Option<Usage>,
    /// cswap keeps the last good reading when a refresh fails.
    #[serde(default)]
    pub last_good_usage: Option<Usage>,
}

impl Account {
    /// Current usage, falling back to the last good reading if the refresh
    /// failed.
    pub fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref().or(self.last_good_usage.as_ref())
    }

    pub fn pct(&self, window: Win) -> Option<f32> {
        let u = self.usage()?;
        let w = match window {
            Win::FiveHour => u.five_hour.as_ref(),
            Win::SevenDay => u.seven_day.as_ref(),
        }?;
        Some(w.effective_pct())
    }

    pub fn window(&self, window: Win) -> Option<&Window> {
        let u = self.usage()?;
        match window {
            Win::FiveHour => u.five_hour.as_ref(),
            Win::SevenDay => u.seven_day.as_ref(),
        }
    }

    /// Usage of the window that binds first: the one deciding whether the
    /// account is spent, be it the 5h or the 7d one.
    pub fn binding_pct(&self) -> Option<f32> {
        match (self.pct(Win::FiveHour), self.pct(Win::SevenDay)) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.usage_status.is_empty() || self.usage_status == "ok"
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Win {
    FiveHour,
    SevenDay,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(default)]
    pub five_hour: Option<Window>,
    #[serde(default)]
    pub seven_day: Option<Window>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    #[serde(default)]
    pub pct: f32,
    /// When this window rolls over, RFC3339 as cswap emits it.
    #[serde(default)]
    pub resets_at: Option<String>,
    /// Time left, already formatted by cswap, e.g. "4h 20m".
    #[serde(default)]
    pub countdown: Option<String>,
    /// Reset time, already formatted, e.g. "20:49".
    #[serde(default)]
    pub clock: Option<String>,
}

impl Window {
    /// The reading belongs to a cycle that already closed. cswap only refreshes
    /// usage when something asks the API, so after hitting the cap the last
    /// reading stays pinned at its old percentage long past the reset — which
    /// is exactly when the tray must know the quota is back.
    pub fn expired(&self) -> bool {
        self.resets_at
            .as_deref()
            .and_then(epoch_of)
            .is_some_and(|t| t <= now_epoch())
    }

    /// Percentage to act on and to show: zero once the window has rolled over.
    pub fn effective_pct(&self) -> f32 {
        if self.expired() { 0.0 } else { self.pct }
    }
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Epoch seconds of an RFC3339 stamp as cswap emits it
/// ("2026-08-19T20:30:00.386497+00:00", or with a "Z"). Anything it cannot
/// fully parse yields `None`, which every caller reads as "no idea" and never
/// as "expired" — a misread must not fake free quota.
fn epoch_of(ts: &str) -> Option<i64> {
    let num = |a: usize, z: usize| ts.get(a..z)?.parse::<i64>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, min, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // days_from_civil (Howard Hinnant): calendar date -> days since 1970-01-01.
    let y = if month <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let mut secs = days * 86_400 + hour * 3_600 + min * 60 + sec;

    // Trailing zone: "Z", "+HH:MM" or "-HH:MM", after any fractional seconds.
    let tail = ts[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if !tail.is_empty() && tail != "Z" {
        let sign = match tail.as_bytes()[0] {
            b'+' => -1,
            b'-' => 1,
            _ => return None,
        };
        secs += sign * (num_at(tail, 1, 3)? * 3_600 + num_at(tail, 4, 6)? * 60);
    }
    Some(secs)
}

fn num_at(s: &str, a: usize, z: usize) -> Option<i64> {
    s.get(a..z)?.parse().ok()
}

/// Locates the executable once. `cswap` is usually on PATH, but a tray app
/// launched at startup can inherit a different one, so the paths where
/// `uv tool` and `pipx` drop it are tried too.
fn exe() -> &'static str {
    static EXE: OnceLock<String> = OnceLock::new();
    EXE.get_or_init(|| {
        let mut candidates: Vec<String> = vec!["cswap".into()];
        if let Ok(home) = std::env::var("USERPROFILE") {
            for sub in [".local/bin/cswap.exe", ".local/bin/claude-swap.exe"] {
                candidates.push(PathBuf::from(&home).join(sub).to_string_lossy().into_owned());
            }
        }
        for c in &candidates {
            if Command::new(c)
                .arg("--version")
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .is_ok()
            {
                return c.clone();
            }
        }
        "cswap".into()
    })
}

/// Argv prefix for the capture calls. `CREATE_NO_WINDOW` only suppresses the
/// console of the process we spawn: `cswap.exe` is a console-subsystem shim
/// that re-launches the interpreter, and that grandchild gets a fresh console
/// of its own, which flashes on screen. Driving the module through `pythonw`
/// (GUI subsystem) keeps every link of the chain windowless. Falls back to the
/// shim when the interpreter cannot be located.
fn quiet() -> &'static Vec<String> {
    static QUIET: OnceLock<Vec<String>> = OnceLock::new();
    QUIET.get_or_init(|| {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(appdata) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(appdata).join("uv/tools/claude-swap"));
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            roots.push(PathBuf::from(&home).join("pipx/venvs/claude-swap"));
        }
        for root in roots {
            let py = root.join("Scripts/pythonw.exe");
            if py.is_file() {
                return vec![
                    py.to_string_lossy().into_owned(),
                    "-m".into(),
                    "claude_swap".into(),
                ];
            }
        }
        vec![exe().to_string()]
    })
}

fn run(args: &[&str]) -> Result<String, String> {
    let prefix = quiet();
    let out = Command::new(&prefix[0])
        .args(&prefix[1..])
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("could not run cswap: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().filter(|l| !l.trim().is_empty()).next_back().unwrap_or("");
        return Err(if last.is_empty() {
            format!("cswap {} failed ({})", args.join(" "), out.status)
        } else {
            last.to_string()
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn list() -> Result<Snapshot, String> {
    let stdout = run(&["list", "--json"])?;
    let snap: Snapshot =
        serde_json::from_str(&stdout).map_err(|e| format!("unreadable cswap JSON: {e}"))?;
    if snap.schema_version != 1 {
        // Not fatal: the fields used here are the basic ones, and showing data
        // beats going blank over a version bump.
        eprintln!("warning: unknown schemaVersion {}", snap.schema_version);
    }
    Ok(snap)
}

/// `None` rotates to the next account (`cswap switch`'s own semantics).
pub fn switch(target: Option<u32>) -> Result<(), String> {
    match target {
        Some(n) => run(&["switch", &n.to_string()]).map(|_| ()),
        None => run(&["switch"]).map(|_| ()),
    }
}

/// Runs one of your own prompts on another account, which opens that account's
/// 5h window as a side effect of doing actual work.
///
/// `cswap run` applies the credential **to that process only**, so the system's
/// active account is untouched. The answer is written to `out` so it is work
/// you receive, not a request into the void. The child is returned instead of
/// awaited: Claude takes a while and the tray must not block.
pub fn run_task(number: u32, task: &str, out: &std::path::Path) -> Result<std::process::Child, String> {
    if task.trim().is_empty() {
        return Err("no prewarm task configured".to_string());
    }
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::File::create(out).map_err(|e| format!("could not write the output: {e}"))?;
    let prefix = quiet();
    Command::new(&prefix[0])
        .args(&prefix[1..])
        .args(["run", &number.to_string(), "--", "claude", "-p", task])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run the prewarm task: {e}"))
}

/// Opens the interactive dashboard in a new console.
pub fn open_tui() {
    let _ = Command::new("cmd")
        .args(["/c", "start", "", "cmd", "/k", exe(), "tui"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_launcher_never_goes_through_a_console_shim() {
        let prefix = quiet();
        assert!(!prefix.is_empty());
        // Either we found the interpreter and drive the module directly, or we
        // fell back to the shim. Anything else means the resolver is broken.
        if prefix.len() > 1 {
            assert!(prefix[0].to_lowercase().ends_with("pythonw.exe"));
            assert_eq!(prefix[1..], ["-m", "claude_swap"]);
        } else {
            assert_eq!(prefix[0], exe());
        }
    }

    fn win(resets_at: &str, pct: f32) -> Window {
        Window { pct, resets_at: Some(resets_at.into()), countdown: None, clock: None }
    }

    #[test]
    fn parses_the_shapes_cswap_emits() {
        assert_eq!(epoch_of("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_of("2026-08-19T20:30:00.386497+00:00"), Some(1_787_171_400));
        // Same instant written in a +02:00 zone.
        assert_eq!(
            epoch_of("2026-08-19T22:30:00+02:00"),
            epoch_of("2026-08-19T20:30:00Z")
        );
        // Leap day, and a rejection.
        assert_eq!(epoch_of("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(epoch_of("not a date"), None);
        assert_eq!(epoch_of("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn a_window_past_its_reset_reads_as_empty() {
        let spent = win("2020-01-01T00:00:00Z", 100.0);
        assert!(spent.expired());
        assert_eq!(spent.effective_pct(), 0.0);
    }

    #[test]
    fn a_live_window_keeps_its_reading() {
        let live = win("2999-01-01T00:00:00Z", 100.0);
        assert!(!live.expired());
        assert_eq!(live.effective_pct(), 100.0);
    }

    #[test]
    fn an_unreadable_reset_never_fakes_free_quota() {
        let unknown = Window { pct: 100.0, resets_at: None, countdown: None, clock: None };
        assert!(!unknown.expired());
        assert_eq!(unknown.effective_pct(), 100.0);
    }

    #[test]
    fn account_pct_follows_the_expiry() {
        let acc: Account = serde_json::from_str(
            r#"{"number":1,"email":"a@x.com","active":true,"usageStatus":"ok",
                "usage":{"fiveHour":{"pct":100.0,"resetsAt":"2020-01-01T00:00:00Z"},
                         "sevenDay":{"pct":11.0,"resetsAt":"2999-01-01T00:00:00Z"}}}"#,
        )
        .unwrap();
        assert_eq!(acc.pct(Win::FiveHour), Some(0.0));
        assert_eq!(acc.pct(Win::SevenDay), Some(11.0));
        assert_eq!(acc.binding_pct(), Some(11.0));
    }
}
