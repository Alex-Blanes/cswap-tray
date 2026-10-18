//! DeepSeek as one more "account": the paid API, for when every subscription
//! is spent.
//!
//! Claude Code picks its credential by precedence, and `apiKeyHelper` beats the
//! OAuth login that `cswap` manages. So the switch is a patch to
//! `~/.claude/settings.json`: the helper that prints the key, plus the base URL
//! and the model names. Turning it off removes exactly those keys and the
//! subscription takes over again, with no need to log in anywhere.
//!
//! Two rules hold the whole thing together:
//!
//! - **The base URL and the key travel together, in one write.** If the URL
//!   pointed at DeepSeek while the credential was still Anthropic's OAuth
//!   token, Claude Code would ship that token to api.deepseek.com. Hence the
//!   atomic write, and hence `key_available` gating the way in.
//! - **Only our own keys are touched.** `settings.json` is the user's, with
//!   hooks, permissions and a statusline in it; an `apiKeyHelper` that is not
//!   ours is left alone.

use serde_json::{Map, Value};
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Marks an `apiKeyHelper` as ours. It is the helper script's own name, so a
/// helper the user set up for something else is never removed.
const HELPER_MARK: &str = "deepseek-key";

/// Written into `env` on the way in, removed on the way out. The model names
/// are DeepSeek's recommended setup for Claude Code: without them Claude Code
/// sends `claude-opus-*`, which DeepSeek maps to its Pro tier.
const ENV: &[(&str, &str)] = &[
    ("ANTHROPIC_BASE_URL", "https://api.deepseek.com/anthropic"),
    ("ANTHROPIC_MODEL", "deepseek-flash[1m]"),
    ("ANTHROPIC_DEFAULT_OPUS_MODEL", "deepseek-flash[1m]"),
    ("ANTHROPIC_DEFAULT_SONNET_MODEL", "deepseek-flash[1m]"),
    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "deepseek-flash"),
    ("CLAUDE_CODE_SUBAGENT_MODEL", "deepseek-flash"),
];

fn settings_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    Some(std::path::PathBuf::from(home).join(".claude").join("settings.json"))
}

fn read() -> Option<(std::path::PathBuf, Value)> {
    let p = settings_path()?;
    let v = match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(&s).ok()?,
        // No settings file yet: an empty object is the right starting point.
        Err(_) => Value::Object(Map::new()),
    };
    v.is_object().then_some((p, v))
}

/// Whether DeepSeek is the active credential right now.
///
/// Read from `settings.json` rather than remembered in our own config: that
/// file is the thing Claude Code obeys, and the user can edit it by hand.
pub fn is_on() -> bool {
    read().is_some_and(|(_, v)| {
        v.get("apiKeyHelper")
            .and_then(Value::as_str)
            .is_some_and(|h| h.contains(HELPER_MARK))
    })
}

/// Turns DeepSeek on or off. `helper` is the command that prints the key.
pub fn set(on: bool, helper: &str) -> Result<(), String> {
    let Some((path, mut v)) = read() else {
        return Err("could not read ~/.claude/settings.json".into());
    };
    patch(&mut v, on, helper);
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    // Atomic: a half-written settings.json is the one way to end up with
    // DeepSeek's URL and Anthropic's token at the same time.
    let tmp = path.with_extension("json.cswap-tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not replace settings.json: {e}"))
}

/// The edit itself, separated out so it can be checked without a real file.
fn patch(v: &mut Value, on: bool, helper: &str) {
    let obj = match v.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    if on {
        obj.insert("apiKeyHelper".into(), Value::String(helper.into()));
    } else if obj
        .get("apiKeyHelper")
        .and_then(Value::as_str)
        .is_some_and(|h| h.contains(HELPER_MARK))
    {
        obj.remove("apiKeyHelper");
    }
    let env = obj
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut();
    let Some(env) = env else { return };
    for (k, val) in ENV {
        if on {
            env.insert((*k).into(), Value::String((*val).into()));
        } else {
            env.remove(*k);
        }
    }
}

/// Whether the key is actually there. The gate on the way in: with no key the
/// helper prints nothing, Claude Code falls back to the OAuth token, and the
/// base URL would send it to DeepSeek.
pub fn key_available(python: &str, script: &str) -> bool {
    Command::new(python)
        .args([script, "get"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Value {
        serde_json::json!({
            "model": "opus",
            "hooks": {"SessionStart": []},
            "env": {"FORCE_HYPERLINK": "1"}
        })
    }

    #[test]
    fn on_and_off_leave_the_rest_of_the_file_alone() {
        let mut v = settings();
        patch(&mut v, true, "py deepseek-key.py get");
        assert_eq!(v["apiKeyHelper"], "py deepseek-key.py get");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://api.deepseek.com/anthropic");
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "deepseek-flash[1m]");

        patch(&mut v, false, "");
        assert!(v.get("apiKeyHelper").is_none());
        assert!(v["env"].get("ANTHROPIC_BASE_URL").is_none());
        // Untouched throughout.
        assert_eq!(v["model"], "opus");
        assert_eq!(v["env"]["FORCE_HYPERLINK"], "1");
        assert!(v["hooks"].is_object());
    }

    #[test]
    fn a_helper_that_is_not_ours_survives_turning_it_off() {
        let mut v = serde_json::json!({"apiKeyHelper": "/opt/corp-token.sh"});
        patch(&mut v, false, "");
        assert_eq!(v["apiKeyHelper"], "/opt/corp-token.sh");
    }

    #[test]
    fn the_url_never_goes_in_without_the_helper() {
        // Both halves come from the same call, so there is no ordering in which
        // the base URL is set while the credential is still the OAuth token.
        let mut v = Value::Object(Map::new());
        patch(&mut v, true, "py deepseek-key.py get");
        assert!(v["apiKeyHelper"].as_str().unwrap().contains(HELPER_MARK));
        assert!(v["env"]["ANTHROPIC_BASE_URL"].is_string());
    }
}
