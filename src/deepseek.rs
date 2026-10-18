//! DeepSeek as one more *session*, not one more credential.
//!
//! DeepSeek serves an Anthropic-compatible API, so a Claude Code session can
//! talk to it with nothing but a base URL, a key and the model names. The
//! catch is *when* each half is read: the credential when the session starts,
//! while the base URL from settings can reach a session already running.
//! Patching `~/.claude/settings.json` while a session is open therefore moves
//! that session's base URL to DeepSeek but leaves the subscription's OAuth
//! credential in place, so the subscription token is what ends up at
//! api.deepseek.com — a 401, and a token handed to a third party. Removing the
//! patch afterwards does not put the base URL back either: the process keeps
//! it until it exits.
//!
//! So this module never touches the user's settings. It writes a settings file
//! of its own and opens Claude Code with `--settings`, which scopes both the
//! endpoint and the credential to that one session:
//!
//! - `apiKeyHelper` prints the key from the Windows credential manager, so the
//!   key is never written to disk and never becomes an environment variable.
//! - `ENABLE_TOOL_SEARCH=false` is not a preference. Claude Code 2.1.286+
//!   emits `tool_definition` blocks in the message body, and DeepSeek's
//!   deserializer only accepts `tool_reference`; its 422 does not match the
//!   text Claude Code's own fallback looks for, so the session would be stuck
//!   (deepseek-ai/DeepSeek-V3#1701).
//!
//! `claude-ds.cmd`, installed next to `claude.exe`, hides the `--settings`
//! flag: a session is `claude-ds` in any terminal, or the menu entry, which
//! copies that same command to the clipboard for luvus.

use serde_json::{Map, Value};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{self, DeepSeekCfg};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Marks an `apiKeyHelper` as ours. It is the helper script's own name, so a
/// helper the user set up for something else is never removed.
const HELPER_MARK: &str = "deepseek-key";

/// The launcher lives next to `claude.exe`, which is on PATH by definition.
const LAUNCHER: &str = "claude-ds.cmd";
/// What the user types (or pastes): the launcher without its extension.
const COMMAND: &str = "claude-ds";
/// Inside the generated launcher, so it is only ever deleted or rewritten when
/// it is ours.
const LAUNCHER_MARK: &str = "cswap-tray deepseek";

/// Our settings file, handed to Claude Code with `--settings`.
const SETTINGS: &str = "deepseek-settings.json";

const BASE_URL: &str = "https://api.deepseek.com/anthropic";

/// The endpoint, the models and the two flags that keep a DeepSeek session
/// workable. Values from DeepSeek's Claude Code guide, which is also where
/// `deepseek-flash[1m]` comes from: without pinning the names Claude Code sends
/// `claude-opus-*`, which DeepSeek maps to its Pro tier.
const ENV: &[(&str, &str)] = &[
    ("ANTHROPIC_BASE_URL", BASE_URL),
    ("ANTHROPIC_MODEL", "deepseek-flash[1m]"),
    ("ANTHROPIC_DEFAULT_OPUS_MODEL", "deepseek-flash[1m]"),
    ("ANTHROPIC_DEFAULT_SONNET_MODEL", "deepseek-flash[1m]"),
    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "deepseek-flash"),
    ("CLAUDE_CODE_SUBAGENT_MODEL", "deepseek-flash"),
    ("CLAUDE_CODE_EFFORT_LEVEL", "max"),
    ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "786432"),
    // See the module comment: the whole reason this is written down.
    ("ENABLE_TOOL_SEARCH", "false"),
];

fn settings_path() -> PathBuf {
    config::dir().join(SETTINGS)
}

fn launcher_path() -> Option<PathBuf> {
    claude_exe().and_then(|claude| claude.parent().map(|dir| dir.join(LAUNCHER)))
}

/// The JSON Claude Code is given for one session. No secret in it: the helper
/// command reads the key from the credential manager every time a session
/// starts, and fails closed if it cannot.
fn session_settings(helper: &str) -> Value {
    let env: Map<String, Value> = ENV
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
        .collect();
    let mut obj = Map::new();
    obj.insert("apiKeyHelper".into(), Value::String(helper.into()));
    obj.insert("env".into(), Value::Object(env));
    Value::Object(obj)
}

/// Writes the per-session settings and the `claude-ds` launcher.
///
/// Called at startup and again before opening a session, so editing
/// `config.json` (the helper's interpreter or path) needs no restart.
pub fn install(cfg: &DeepSeekCfg) -> Result<(), String> {
    let claude = claude_exe().ok_or("claude.exe is not on PATH")?;
    let dir = config::dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

    let settings = settings_path();
    let text = serde_json::to_string_pretty(&session_settings(&cfg.helper()))
        .map_err(|e| e.to_string())?;
    write_if_changed(&settings, &text)?;

    let launcher = claude
        .parent()
        .map(|dir| dir.join(LAUNCHER))
        .ok_or("claude.exe has no directory")?;
    write_if_changed(&launcher, &launcher_text(&claude))
}

/// Removes what `install` wrote. Used when `script` is emptied: the menu entry
/// is already gone, and a stale `claude-ds` would keep pointing at a helper
/// that no longer exists.
pub fn uninstall() {
    if let Some(launcher) = launcher_path() {
        if std::fs::read_to_string(&launcher).is_ok_and(|s| s.contains(LAUNCHER_MARK)) {
            let _ = std::fs::remove_file(&launcher);
        }
    }
    let _ = std::fs::remove_file(settings_path());
}

/// A `.cmd` file, so the settings flag never has to be typed. Both paths are
/// expanded at run time (`%~dp0` is the launcher's own directory, `%APPDATA%`
/// the profile's) so nothing non-ASCII or profile-specific is baked in.
///
/// `-n` is Claude Code's own session name, and it is what puts "DeepSeek" in
/// the terminal title for the whole session; `title` only covers the instant
/// before Claude Code takes over. An explicit `-n` after ours still wins.
fn launcher_text(claude: &Path) -> String {
    let exe = claude
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("claude.exe");
    format!(
        "@echo off\r\n\
         rem {LAUNCHER_MARK} - generated by cswap-tray; edit config.json, not this file\r\n\
         title DeepSeek\r\n\
         \"%~dp0{exe}\" --settings \"%APPDATA%\\cswap-tray\\{SETTINGS}\" -n DeepSeek %*\r\n"
    )
}

fn write_if_changed(path: &Path, text: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return Ok(());
    }
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn claude_exe() -> Option<PathBuf> {
    let out = Command::new("where")
        .arg("claude")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

/// The command the tray copies to the clipboard. `claude-ds` is the installed
/// launcher; `-r DeepSeek` opens Claude Code's resume picker already filtered
/// to the sessions this launcher named, so a normal conversation is not
/// resumed under the paid endpoint by accident.
pub fn command(resume: bool) -> String {
    if resume {
        format!("{COMMAND} -r DeepSeek")
    } else {
        COMMAND.to_string()
    }
}

/// Whether the key is actually there. The gate on the way in: with no key the
/// helper fails, and Claude Code refuses to start rather than fall back to the
/// subscription's OAuth token.
pub fn key_available(python: &str, script: &str) -> bool {
    Command::new(python)
        .args([script, "get"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| {
            o.status.success() && !o.stdout.iter().all(|byte| byte.is_ascii_whitespace())
        })
}

/// Removes the keys the previous version wrote into `~/.claude/settings.json`.
///
/// That version switched DeepSeek on globally, and an upgrade must not leave
/// it on: every session would keep talking to api.deepseek.com. Harmless when
/// the file has nothing of ours.
pub fn disarm() {
    let home = match std::env::var("USERPROFILE") {
        Ok(home) => home,
        Err(_) => return,
    };
    let path = PathBuf::from(home).join(".claude").join("settings.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    if !value.is_object() || !strip(&mut value) {
        return;
    }
    let Ok(text) = serde_json::to_string_pretty(&value) else {
        return;
    };
    // Atomic, for the same reason the old version was: a half-written
    // settings.json is worse than either state.
    let tmp = path.with_extension("json.cswap-tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The edit itself, separated out so it can be checked without a real file.
/// Returns whether it touched anything.
fn strip(value: &mut Value) -> bool {
    let ours = value
        .get("apiKeyHelper")
        .and_then(Value::as_str)
        .is_some_and(|helper| helper.contains(HELPER_MARK));
    if !ours {
        return false;
    }
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    obj.remove("apiKeyHelper");
    if let Some(env) = obj.get_mut("env").and_then(Value::as_object_mut) {
        for (key, _) in ENV {
            env.remove(*key);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_session_carries_the_helper_the_endpoint_and_the_models() {
        let v = session_settings("py deepseek-key.py get");
        assert_eq!(v["apiKeyHelper"], "py deepseek-key.py get");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], BASE_URL);
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "deepseek-flash[1m]");
        assert_eq!(v["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "deepseek-flash");
        assert_eq!(v["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "deepseek-flash");
    }

    #[test]
    fn tool_search_is_turned_off_on_purpose() {
        let v = session_settings("py deepseek-key.py get");
        assert_eq!(v["env"]["ENABLE_TOOL_SEARCH"], "false");
    }

    #[test]
    fn the_settings_file_never_holds_the_key() {
        let text = serde_json::to_string(&session_settings("py deepseek-key.py get")).unwrap();
        assert!(!text.contains("sk-"));
    }

    #[test]
    fn the_launcher_forwards_arguments_and_names_the_window() {
        let text = launcher_text(Path::new(r"C:\bin\claude.exe"));
        assert!(text.contains(r#""%~dp0claude.exe" --settings"#));
        assert!(text.contains(r#"%APPDATA%\cswap-tray\deepseek-settings.json"#));
        assert!(text.contains("-n DeepSeek"));
        assert!(text.contains("%*"), "so claude-ds --continue works");
        assert!(text.contains("title DeepSeek"));
        assert!(text.contains(LAUNCHER_MARK));
    }

    #[test]
    fn strip_removes_our_keys_and_nothing_else() {
        let mut v = json!({
            "model": "opus",
            "hooks": {"SessionStart": []},
            "apiKeyHelper": "py deepseek-key.py get",
            "env": {
                "FORCE_HYPERLINK": "1",
                "ANTHROPIC_BASE_URL": BASE_URL,
                "ANTHROPIC_MODEL": "deepseek-flash[1m]",
                "ENABLE_TOOL_SEARCH": "false"
            }
        });
        assert!(strip(&mut v));
        assert!(v.get("apiKeyHelper").is_none());
        assert!(v["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert!(v["env"].get("ENABLE_TOOL_SEARCH").is_none());
        assert_eq!(v["model"], "opus");
        assert_eq!(v["env"]["FORCE_HYPERLINK"], "1");
        assert!(v["hooks"].is_object());
    }

    #[test]
    fn a_foreign_helper_keeps_its_environment_too() {
        let mut v = json!({
            "apiKeyHelper": "/opt/corp-token.sh",
            "env": {"ANTHROPIC_BASE_URL": BASE_URL}
        });
        assert!(!strip(&mut v));
        assert_eq!(v["apiKeyHelper"], "/opt/corp-token.sh");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], BASE_URL);
    }

    #[test]
    fn the_copied_command_is_the_launcher_and_can_resume() {
        assert_eq!(COMMAND, LAUNCHER.trim_end_matches(".cmd"));
        assert_eq!(command(false), "claude-ds");
        assert_eq!(command(true), "claude-ds -r DeepSeek");
    }
}
