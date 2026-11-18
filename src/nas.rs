//! The Claude Code containers running on the NAS, one per project.
//!
//! There is no agent on the other side: every container already carries the
//! same `cswap` CLI, so `docker exec` is the whole protocol. One SSH call
//! refreshes every container at once — a connection per project would make the
//! menu wait a second and a half for its first line — and the menu only paints
//! the cached answer.
//!
//! The NAS itself is one more target, [`HOST`]: the same commands run straight
//! in the SSH shell instead of through `docker exec`, so the native Claude Code
//! there gets the same menu as the containers.
//!
//! Switching pauses `cswap-preferida` first. That daemon puts the preferred
//! account back within a minute, so a manual switch that left it running would
//! look broken.

use crate::config::NasCfg;
use crate::cswap::{Account, Snapshot as CswapSnapshot};
use crate::deepseek;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The auto-switcher shipped to a container the first time its preferred
/// account is changed from the tray. It reads its preferred account from
/// `~/.config/cswap-preferida.json`, so later changes need no restart.
const PREFERIDA: &str = include_str!("../scripts/cswap-preferida");

/// The native Claude Code of the NAS, outside any container. Docker names
/// cannot hold a space or parentheses, so it never collides with a container.
pub const HOST: &str = "NAS (host)";

/// The daemon of the target the command runs in, matched as the interpreter
/// running that exact file. The full path keeps the NAS from matching the
/// containers' daemons, which it also sees under the same uid; the `^python3`
/// keeps an editor or a `tail` naming the file from being killed with it.
const DAEMON_RUNNING: &str =
    r#"pgrep -f "^python3 $HOME/.local/bin/[c]swap-preferida" >/dev/null 2>&1"#;
const DAEMON_STOP: &str =
    r#"pkill -f "^python3 $HOME/.local/bin/[c]swap-preferida" >/dev/null 2>&1 || true"#;

/// What every target reports: its accounts, whether its daemon runs, and that
/// daemon's settings, in the `===CSWAP` / `AUTO=` protocol [`parse`] reads.
const INFO: &str = r#"cswap list --json || echo '{}'
if pgrep -f "^python3 $HOME/.local/bin/[c]swap-preferida" >/dev/null 2>&1; then echo AUTO=1; else echo AUTO=0; fi
python3 -c 'import json,os
try: c=json.load(open(os.path.expanduser("~/.config/cswap-preferida.json")))
except Exception: c={}
c=c if isinstance(c,dict) else {}
print("PREF="+str(c.get("preferred") or ""))
print("AT="+str(c.get("switch_at") or ""))' 2>/dev/null || true"#;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub containers: Vec<Container>,
}

#[derive(Debug, Clone)]
pub struct Container {
    pub name: String,
    pub accounts: Vec<Account>,
    /// `cswap-preferida` is running inside: it moves the account on its own.
    pub auto: bool,
    /// What that daemon will return to, read from its own config file.
    pub preferred: Option<String>,
    /// Where that daemon leaves the active account. `None`: the script's default.
    pub switch_at: Option<f32>,
}

/// What `list` runs on the NAS. The filter is applied server-side so only the
/// matching containers pay a `docker exec`.
fn list_script(cfg: &NasCfg) -> String {
    let docker = docker(cfg);
    let filter = safe_filter(&cfg.filter);
    let only = if filter.is_empty() {
        String::new()
    } else {
        format!("case \"$name:$image\" in *\"{filter}\"*) ;; *) continue;; esac\n")
    };
    let info = shq(INFO);
    format!(
        "set -e\n\
         if command -v cswap >/dev/null 2>&1; then\n\
         echo '===CSWAP {HOST}'\n\
         sh -c {info} || true\n\
         fi\n\
         list=$({docker} ps --format '{{{{.Names}}}} {{{{.Image}}}}')\n\
         printf '%s\\n' \"$list\" | while read -r name image; do\n\
         {only}\
         echo \"===CSWAP $name\"\n\
         {docker} exec \"$name\" sh -c {info} || echo '{{}}'\n\
         done\n"
    )
}

/// `cmd` run inside `target`, as a command for the NAS shell.
fn run(cfg: &NasCfg, target: &str, cmd: &str) -> String {
    if target == HOST {
        format!("sh -c {}", shq(cmd))
    } else {
        format!("{} exec {} sh -c {}", docker(cfg), shq(target), shq(cmd))
    }
}

/// Same, reading the payload from the SSH session's stdin.
fn run_stdin(cfg: &NasCfg, target: &str, cmd: &str) -> String {
    if target == HOST {
        run(cfg, target, cmd)
    } else {
        format!("{} exec -i {} sh -c {}", docker(cfg), shq(target), shq(cmd))
    }
}

/// Starts the daemon in `target` and returns at once. On the NAS it is
/// detached from the SSH session, which would otherwise wait for it.
fn start_daemon(cfg: &NasCfg, target: &str) -> String {
    if target == HOST {
        "mkdir -p \"$HOME/.cache\"; setsid sh -c cswap-preferida \
         >>\"$HOME/.cache/cswap-preferida.log\" 2>&1 </dev/null &"
            .to_string()
    } else {
        format!("{} exec -d {} sh -c cswap-preferida", docker(cfg), shq(target))
    }
}

/// `sudo -n`: a prompt would block a tray process forever, so it must fail
/// instead. Docker is the one command the NAS lets this user run without a
/// password.
fn docker(cfg: &NasCfg) -> &'static str {
    if cfg.sudo { "sudo -n docker" } else { "docker" }
}

/// Keeps the filter a substring and never shell syntax: it is spliced into a
/// `case` pattern. Anything outside the safe set is dropped, and an empty
/// result means "every container".
fn safe_filter(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect()
}

/// Docker names cannot contain quotes, but the name comes from a remote
/// listing all the same.
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn ssh_cmd(cfg: &NasCfg, script: &str) -> Command {
    // A non-interactive SSH shell leaves ~/.local/bin, where cswap lives on the
    // NAS itself, out of PATH.
    let script = format!("PATH=\"$HOME/.local/bin:$PATH\"; export PATH\n{script}");
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-o", "BatchMode=yes",
        "-o", "ConnectTimeout=6",
        // A remote command that hangs stops the refresher, not the tray: the
        // worker is its own thread, and these bound how long it sleeps.
        "-o", "ServerAliveInterval=5",
        "-o", "ServerAliveCountMax=2",
        cfg.ssh.trim(),
        &script,
    ])
    .creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn failure(cfg: &NasCfg, status: std::process::ExitStatus, stderr: &[u8]) -> String {
    let err = String::from_utf8_lossy(stderr);
    let last = err.lines().filter(|l| !l.trim().is_empty()).next_back().unwrap_or("");
    if last.is_empty() {
        format!("ssh {} failed ({status})", cfg.ssh.trim())
    } else {
        last.to_string()
    }
}

fn ssh(cfg: &NasCfg, script: &str) -> Result<String, String> {
    let out = ssh_cmd(cfg, script).output().map_err(|e| format!("ssh: {e}"))?;
    if !out.status.success() {
        return Err(failure(cfg, out.status, &out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Same connection, with the payload on stdin. The DeepSeek key and the remote
/// files go this way so they never appear in an argument list.
fn ssh_stdin(cfg: &NasCfg, script: &str, input: &str) -> Result<(), String> {
    let mut child = ssh_cmd(cfg, script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(input.as_bytes()).map_err(|e| format!("ssh stdin: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
    if !out.status.success() {
        return Err(failure(cfg, out.status, &out.stderr));
    }
    Ok(())
}

pub fn list(cfg: &NasCfg) -> Result<Snapshot, String> {
    Ok(parse(&ssh(cfg, &list_script(cfg))?))
}

pub fn switch(cfg: &NasCfg, container: &str, account: u32) -> Result<(), String> {
    ssh(cfg, &run(cfg, container, &format!("{DAEMON_STOP}\ncswap switch {account}")))
        .map(|_| ())
}

pub fn set_auto(cfg: &NasCfg, container: &str, on: bool) -> Result<(), String> {
    let script = if on {
        start_daemon(cfg, container)
    } else {
        run(cfg, container, DAEMON_STOP)
    };
    ssh(cfg, &script).map(|_| ())
}

/// Sets one key (`preferred`, `switch_at`) of a container's auto-switcher config.
///
/// Installs the version that reads its settings from a file — the original
/// had them hard-coded — merges the key into that file, keeping the others,
/// and restarts the daemon only if it was running. The old script is left
/// beside it as `.bak`.
pub fn set_setting(
    cfg: &NasCfg,
    container: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    ssh_stdin(
        cfg,
        &run_stdin(
            cfg,
            container,
            r#"b="$HOME/.local/bin/cswap-preferida"
cp -f "$b" "$b.bak" 2>/dev/null; cat > "$b" && chmod 755 "$b""#,
        ),
        PREFERIDA,
    )?;
    // The update travels on stdin as JSON, so neither key nor value is ever
    // spliced into shell or Python source.
    ssh_stdin(
        cfg,
        &run_stdin(
            cfg,
            container,
            r#"python3 -c 'import json,os,sys
p=os.path.expanduser("~/.config/cswap-preferida.json")
try: c=json.load(open(p))
except Exception: c={}
c=c if isinstance(c,dict) else {}
c.update(json.load(sys.stdin))
os.makedirs(os.path.dirname(p),exist_ok=True)
json.dump(c,open(p,"w"))'"#,
        ),
        &serde_json::json!({ key: value }).to_string(),
    )?;
    // The process in memory still runs the old code, so the switch to the
    // file-backed preference needs this one restart. Off stays off.
    ssh(
        cfg,
        &format!(
            "if {}; then\n{}\nsleep 1\n{}\nfi",
            run(cfg, container, DAEMON_RUNNING),
            run(cfg, container, DAEMON_STOP),
            start_daemon(cfg, container),
        ),
    )
    .map(|_| ())
}

/// Provisions the DeepSeek launcher inside a container and opens a session.
///
/// The key travels on stdin, never in an argument: argv is visible to every
/// process in the container. On disk it is mode 600 and read by the helper
/// Claude Code calls when the session starts.
pub fn open_deepseek(cfg: &NasCfg, container: &str, key: &str) -> Result<(), String> {
    if container == HOST {
        // The launcher and its settings assume the container's /home/claude.
        return Err("DeepSeek runs only inside the containers".to_string());
    }
    if key.is_empty()
        || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err("unexpected characters in the DeepSeek key".to_string());
    }
    let script = format!(
        "umask 077\n\
         mkdir -p \"$HOME/.config\" \"$HOME/.local/bin\"\n\
         cat > \"$HOME/.config/deepseek.key\" <<'KEY'\n{key}\nKEY\n\
         cat > \"$HOME/.local/bin/deepseek-key\" <<'SH'\n#!/bin/sh\ncat \"$HOME/.config/deepseek.key\"\nSH\n\
         chmod 700 \"$HOME/.local/bin/deepseek-key\"\n\
         cat > \"$HOME/.config/deepseek-settings.json\" <<'JSON'\n{}\nJSON\n\
         cat > \"$HOME/.local/bin/claude-ds\" <<'SH'\n#!/bin/sh\n\
         exec /home/claude/.local/bin/claude --settings \"$HOME/.config/deepseek-settings.json\" -n DeepSeek \"$@\"\nSH\n\
         chmod 700 \"$HOME/.local/bin/claude-ds\"\n",
        deepseek::remote_settings_json()
    );
    ssh_stdin(
        cfg,
        &format!("{} exec -i {} sh -s", docker(cfg), shq(container)),
        &script,
    )?;
    open_terminal(cfg, container)
}

/// A local console running `ssh -t` into the container: the container's Claude
/// Code is a TUI and needs that TTY.
fn open_terminal(cfg: &NasCfg, container: &str) -> Result<(), String> {
    let mut args: Vec<String> = vec![
        "/c".into(),
        "start".into(),
        "".into(),
        "cmd".into(),
        "/k".into(),
        "ssh".into(),
        "-t".into(),
        cfg.ssh.trim().into(),
    ];
    if cfg.sudo {
        args.extend(["sudo".into(), "-n".into()]);
    }
    args.extend([
        "docker".into(),
        "exec".into(),
        "-it".into(),
        container.into(),
        "/home/claude/.local/bin/claude-ds".into(),
    ]);
    Command::new("cmd")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open a terminal: {e}"))
}

/// Reads the `===CSWAP` / `AUTO=` protocol the script prints. A container whose
/// `cswap list` failed still appears, with no accounts, so the menu can say so
/// instead of pretending the project vanished.
fn parse(text: &str) -> Snapshot {
    let mut containers = Vec::new();
    let mut name: Option<String> = None;
    let mut buf = String::new();
    let mut auto = false;
    let mut preferred: Option<String> = None;
    let mut switch_at: Option<f32> = None;

    let flush = |name: &mut Option<String>,
                 buf: &str,
                 auto: bool,
                 preferred: &mut Option<String>,
                 switch_at: &mut Option<f32>,
                 out: &mut Vec<Container>| {
        if let Some(name) = name.take() {
            let accounts = serde_json::from_str::<CswapSnapshot>(buf)
                .map(|s| s.accounts)
                .unwrap_or_default();
            out.push(Container {
                name,
                accounts,
                auto,
                preferred: preferred.take(),
                switch_at: switch_at.take(),
            });
        }
    };

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("===CSWAP ") {
            flush(&mut name, &buf, auto, &mut preferred, &mut switch_at, &mut containers);
            name = Some(rest.trim().to_string());
            buf.clear();
            auto = false;
        } else if let Some(v) = line.trim().strip_prefix("AUTO=") {
            auto = v == "1";
        } else if let Some(v) = line.trim().strip_prefix("PREF=") {
            preferred = Some(v.trim().to_string()).filter(|s| !s.is_empty());
        } else if let Some(v) = line.trim().strip_prefix("AT=") {
            switch_at = v.trim().parse().ok();
        } else if name.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&mut name, &buf, auto, &mut preferred, &mut switch_at, &mut containers);
    Snapshot { containers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_block_per_container_with_its_auto_state() {
        let text = "\
===CSWAP claude-code
{\"schemaVersion\":1,\"accounts\":[{\"number\":2,\"email\":\"a@b.c\",\"active\":true,\"usageStatus\":\"ok\"}]}
AUTO=1
PREF=a@b.c
AT=100.0
===CSWAP proyecto-x
{}
AUTO=0
";
        let snap = parse(text);
        assert_eq!(snap.containers.len(), 2);
        assert_eq!(snap.containers[0].name, "claude-code");
        assert_eq!(snap.containers[0].accounts.len(), 1);
        assert_eq!(snap.containers[0].accounts[0].email, "a@b.c");
        assert!(snap.containers[0].accounts[0].active);
        assert!(snap.containers[0].auto);
        assert_eq!(snap.containers[0].preferred.as_deref(), Some("a@b.c"));
        assert_eq!(snap.containers[0].switch_at, Some(100.0));
        assert_eq!(snap.containers[1].name, "proyecto-x");
        assert!(snap.containers[1].accounts.is_empty());
        assert!(!snap.containers[1].auto);
        assert!(snap.containers[1].preferred.is_none());
        assert!(snap.containers[1].switch_at.is_none());
    }

    #[test]
    fn the_host_runs_in_place_and_containers_through_docker() {
        let cfg = NasCfg::default();
        assert_eq!(run(&cfg, HOST, "cswap list"), "sh -c 'cswap list'");
        assert!(run(&cfg, "claude-code", "cswap list")
            .ends_with("exec 'claude-code' sh -c 'cswap list'"));
        assert!(start_daemon(&cfg, HOST).starts_with("mkdir -p"));
        assert!(start_daemon(&cfg, HOST).ends_with('&'));
        assert!(start_daemon(&cfg, "claude-code").contains("exec -d 'claude-code'"));
        // The NAS block comes first and only when the NAS has its own cswap.
        let script = list_script(&cfg);
        assert!(script.find(HOST).unwrap() < script.find("ps --format").unwrap());
        assert!(script.contains("command -v cswap"));
    }

    #[test]
    fn the_host_parses_like_any_container() {
        let snap = parse("===CSWAP NAS (host)\n{\"schemaVersion\":1,\"accounts\":[]}\nAUTO=1\n");
        assert_eq!(snap.containers[0].name, HOST);
        assert!(snap.containers[0].auto);
    }

    #[test]
    fn the_shipped_daemon_reads_its_preference_from_a_file() {
        assert!(PREFERIDA.contains("cswap-preferida.json"));
        assert!(PREFERIDA.contains("def settings()"));
    }

    #[test]
    fn a_key_with_shell_syntax_never_leaves() {
        // Refused before any SSH is attempted.
        assert!(open_deepseek(&NasCfg::default(), "c", "sk-x; rm -rf /").is_err());
        assert!(open_deepseek(&NasCfg::default(), "c", "").is_err());
        assert!(open_deepseek(&NasCfg::default(), HOST, "sk-ok").is_err());
    }

    #[test]
    fn a_broken_block_still_names_its_container() {
        let snap = parse("===CSWAP roto\nnot json\nAUTO=0\n");
        assert_eq!(snap.containers.len(), 1);
        assert!(snap.containers[0].accounts.is_empty());
    }

    #[test]
    fn the_filter_cannot_smuggle_shell_syntax() {
        assert_eq!(safe_filter("claude-code"), "claude-code");
        assert_eq!(safe_filter("x'; rm -rf /"), "xrm-rf");
        assert_eq!(safe_filter("a b|c"), "abc");
    }
}
