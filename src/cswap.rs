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
        Some(w.pct)
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
    /// Time left, already formatted by cswap, e.g. "4h 20m".
    #[serde(default)]
    pub countdown: Option<String>,
    /// Reset time, already formatted, e.g. "20:49".
    #[serde(default)]
    pub clock: Option<String>,
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

fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(exe())
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
    Command::new(exe())
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
