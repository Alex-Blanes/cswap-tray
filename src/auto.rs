//! Auto-switch policy with a preferred account.
//!
//! Semantics: live on the preferred account, fall back to another when the
//! active one runs out, and return to the preferred one as soon as it
//! recovers. `cswap auto` does not do that last part (its `best` strategy
//! stays wherever there is most quota), so the decision is taken here and only
//! the switch itself is delegated.
//!
//! This is a pure function over the account snapshot: the cooldown and the
//! actual command are the caller's business.

use crate::config::AutoCfg;
use crate::cswap::Account;

/// Returns the account number worth switching to, or `None` to stay put.
pub fn decide(cfg: &AutoCfg, accounts: &[Account]) -> Option<u32> {
    if !cfg.enabled || accounts.len() < 2 {
        return None;
    }
    let active = accounts.iter().find(|a| a.active)?;
    let preferred = cfg
        .preferred
        .as_deref()
        .and_then(|p| accounts.iter().find(|a| a.email == p));

    // Go home: the preferred account recovered and we are not on it.
    if let Some(pref) = preferred {
        if !pref.active && usable(pref, cfg.return_below_pct) {
            return Some(pref.number);
        }
    }

    // Flee: the active account hit the threshold. Without a usage reading we
    // stay, because there would be no way to tell the target is any better.
    let active_pct = active.binding_pct()?;
    if active_pct < cfg.switch_at_pct {
        return None;
    }

    // Target: the preferred account if it qualifies, else the least spent one.
    let candidates = || {
        accounts
            .iter()
            .filter(|a| !a.active && usable(a, cfg.switch_at_pct))
    };
    if let Some(pref) = preferred {
        if candidates().any(|a| a.number == pref.number) {
            return Some(pref.number);
        }
    }
    candidates()
        .min_by(|a, b| {
            a.binding_pct()
                .unwrap_or(100.0)
                .total_cmp(&b.binding_pct().unwrap_or(100.0))
        })
        .map(|a| a.number)
}

/// An account works as a target when it reports reliable usage and sits below
/// the given limit.
fn usable(acc: &Account, below_pct: f32) -> bool {
    acc.is_ok() && acc.binding_pct().is_some_and(|p| p < below_pct)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds accounts with the same shape `cswap list --json` emits.
    fn accounts(spec: &[(u32, &str, bool, f32, f32)]) -> Vec<Account> {
        let items: Vec<String> = spec
            .iter()
            .map(|(n, email, active, five, seven)| {
                format!(
                    r#"{{"number":{n},"email":"{email}","active":{active},"usageStatus":"ok",
                        "usage":{{"fiveHour":{{"pct":{five}}},"sevenDay":{{"pct":{seven}}}}}}}"#
                )
            })
            .collect();
        serde_json::from_str(&format!("[{}]", items.join(","))).unwrap()
    }

    fn cfg() -> AutoCfg {
        AutoCfg {
            enabled: true,
            preferred: Some("personal@x.com".into()),
            switch_at_pct: 90.0,
            return_below_pct: 80.0,
            cooldown_seconds: 300,
        }
    }

    #[test]
    fn stays_put_while_preferred_has_room() {
        let accs = accounts(&[(1, "personal@x.com", true, 40.0, 20.0), (2, "work@x.com", false, 0.0, 0.0)]);
        assert_eq!(decide(&cfg(), &accs), None);
    }

    #[test]
    fn falls_back_when_preferred_is_exhausted() {
        let accs = accounts(&[(1, "personal@x.com", true, 100.0, 63.0), (2, "work@x.com", false, 2.0, 19.0)]);
        assert_eq!(decide(&cfg(), &accs), Some(2));
    }

    #[test]
    fn seven_day_window_also_binds() {
        // 5h has room but the weekly one is spent: we must leave anyway.
        let accs = accounts(&[(1, "personal@x.com", true, 10.0, 95.0), (2, "work@x.com", false, 2.0, 19.0)]);
        assert_eq!(decide(&cfg(), &accs), Some(2));
    }

    #[test]
    fn returns_home_once_preferred_recovers() {
        let accs = accounts(&[(1, "personal@x.com", false, 5.0, 30.0), (2, "work@x.com", true, 50.0, 40.0)]);
        assert_eq!(decide(&cfg(), &accs), Some(1));
    }

    #[test]
    fn hysteresis_prevents_bouncing_back_too_soon() {
        // The preferred one dropped below 90 but is still above 80: not yet.
        let accs = accounts(&[(1, "personal@x.com", false, 85.0, 30.0), (2, "work@x.com", true, 50.0, 40.0)]);
        assert_eq!(decide(&cfg(), &accs), None);
    }

    #[test]
    fn stays_when_every_account_is_exhausted() {
        let accs = accounts(&[(1, "personal@x.com", true, 100.0, 63.0), (2, "work@x.com", false, 99.0, 90.0)]);
        assert_eq!(decide(&cfg(), &accs), None);
    }

    #[test]
    fn disabled_never_switches() {
        let mut c = cfg();
        c.enabled = false;
        let accs = accounts(&[(1, "personal@x.com", true, 100.0, 63.0), (2, "work@x.com", false, 2.0, 19.0)]);
        assert_eq!(decide(&c, &accs), None);
    }

    #[test]
    fn without_preferred_it_picks_the_least_used() {
        let mut c = cfg();
        c.preferred = None;
        let accs = accounts(&[
            (1, "a@x.com", true, 95.0, 10.0),
            (2, "b@x.com", false, 60.0, 10.0),
            (3, "c@x.com", false, 20.0, 10.0),
        ]);
        assert_eq!(decide(&c, &accs), Some(3));
    }

    #[test]
    fn unreadable_usage_disqualifies_a_target() {
        let accs: Vec<Account> = serde_json::from_str(
            r#"[{"number":1,"email":"personal@x.com","active":true,"usageStatus":"ok",
                 "usage":{"fiveHour":{"pct":100.0},"sevenDay":{"pct":10.0}}},
                {"number":2,"email":"work@x.com","active":false,"usageStatus":"error"}]"#,
        )
        .unwrap();
        assert_eq!(decide(&cfg(), &accs), None);
    }
}
