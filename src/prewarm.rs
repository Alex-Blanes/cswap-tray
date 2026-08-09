//! Prewarming the reserve account.
//!
//! An account's 5h window **is opened by its first message**, not by the login
//! or the credential switch. If you always work on the same account, the other
//! one has its clock stopped: the day you need it, its window starts right
//! then and lasts five hours longer than you would like.
//!
//! Prewarming does not reserve quota, it **moves the clock forward**: an early
//! message on the reserve makes its window close (and renew) sooner.
//!
//! The pure decisions live here; the notice and the switch are `main`'s job.

use crate::cswap::{Account, Win};

/// The reserve is "cold": the active account already has its 5h window running
/// (that is, you are working) and some other account has none open. Returns
/// that account's number.
pub fn cold_reserve(accounts: &[Account]) -> Option<u32> {
    let active = accounts.iter().find(|a| a.active)?;
    // No window on the active account means there is nothing to infer: you are
    // not using Claude right now.
    active.window(Win::FiveHour)?;
    accounts
        .iter()
        .find(|a| !a.active && a.is_ok() && a.window(Win::FiveHour).is_none())
        .map(|a| a.number)
}

/// Evidence that a message landed on the prewarmed account: either it opened a
/// 5h window, or its usage went up compared to when the prewarm was armed.
pub fn prewarm_landed(acc: &Account, had_window: bool, base_pct: f32) -> bool {
    match acc.window(Win::FiveHour) {
        None => false,
        Some(w) => !had_window || w.pct > base_pct + 0.01,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Vec<Account> {
        serde_json::from_str(json).unwrap()
    }

    const WORKING_AND_COLD: &str = r#"[
        {"number":1,"email":"p@x.com","active":true,"usageStatus":"ok",
         "usage":{"fiveHour":{"pct":40.0},"sevenDay":{"pct":20.0}}},
        {"number":2,"email":"w@x.com","active":false,"usageStatus":"ok",
         "usage":{"sevenDay":{"pct":19.0}}}]"#;

    #[test]
    fn detects_a_cold_reserve_while_working() {
        assert_eq!(cold_reserve(&parse(WORKING_AND_COLD)), Some(2));
    }

    #[test]
    fn no_alert_when_the_reserve_clock_is_already_running() {
        let accs = parse(
            r#"[{"number":1,"email":"p@x.com","active":true,"usageStatus":"ok",
                 "usage":{"fiveHour":{"pct":40.0}}},
                {"number":2,"email":"w@x.com","active":false,"usageStatus":"ok",
                 "usage":{"fiveHour":{"pct":2.0}}}]"#,
        );
        assert_eq!(cold_reserve(&accs), None);
    }

    #[test]
    fn no_alert_when_you_are_not_working() {
        // The active account has no window either: no session to piggyback on.
        let accs = parse(
            r#"[{"number":1,"email":"p@x.com","active":true,"usageStatus":"ok",
                 "usage":{"sevenDay":{"pct":20.0}}},
                {"number":2,"email":"w@x.com","active":false,"usageStatus":"ok",
                 "usage":{"sevenDay":{"pct":19.0}}}]"#,
        );
        assert_eq!(cold_reserve(&accs), None);
    }

    #[test]
    fn an_unreadable_account_is_not_reported_as_cold() {
        let accs = parse(
            r#"[{"number":1,"email":"p@x.com","active":true,"usageStatus":"ok",
                 "usage":{"fiveHour":{"pct":40.0}}},
                {"number":2,"email":"w@x.com","active":false,"usageStatus":"error"}]"#,
        );
        assert_eq!(cold_reserve(&accs), None);
    }

    #[test]
    fn landing_is_detected_by_a_brand_new_window() {
        let accs = parse(
            r#"[{"number":2,"email":"w@x.com","active":true,"usageStatus":"ok",
                 "usage":{"fiveHour":{"pct":1.0}}}]"#,
        );
        assert!(prewarm_landed(&accs[0], false, 0.0));
    }

    #[test]
    fn landing_is_detected_by_usage_going_up() {
        let accs = parse(
            r#"[{"number":2,"email":"w@x.com","active":true,"usageStatus":"ok",
                 "usage":{"fiveHour":{"pct":3.0}}}]"#,
        );
        assert!(prewarm_landed(&accs[0], true, 2.0));
        // Same reading as when armed: nothing has landed yet.
        assert!(!prewarm_landed(&accs[0], true, 3.0));
    }

    #[test]
    fn no_window_means_nothing_landed() {
        let accs = parse(
            r#"[{"number":2,"email":"w@x.com","active":true,"usageStatus":"ok",
                 "usage":{"sevenDay":{"pct":19.0}}}]"#,
        );
        assert!(!prewarm_landed(&accs[0], false, 0.0));
    }
}
