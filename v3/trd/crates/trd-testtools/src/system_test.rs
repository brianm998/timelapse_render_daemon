// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

//! The guard for system tests (overview §10 rule 1 and §15.2). A system test runs only on CI, and
//! only when its own switch is set; otherwise it is ignored by an ordinary run and fails, rather
//! than passing silently, when it is forced to run with `--ignored`.

/// The variable that marks a CI run.
const CI_VAR: &str = "CI";

/// True only when `CI` is `true` and the test's `switch` is `1`. `get` reads one environment
/// variable.
pub fn system_test_allowed(get: impl Fn(&str) -> Option<String>, switch: &str) -> bool {
    get(CI_VAR).as_deref() == Some("true") && get(switch).as_deref() == Some("1")
}

/// Panics unless the system test may run; the message names what to set.
pub fn require_system_test(switch: &str) {
    if !system_test_allowed(|key| std::env::var(key).ok(), switch) {
        panic!("refusing to run a system test outside CI: set CI=true and {switch}=1");
    }
}

/// Declares a system test: `system_test!(SWITCH, async fn name() { … })`. It expands to an async
/// test on a multi-threaded runtime, ignored by ordinary runs, whose body first calls
/// `require_system_test("SWITCH")`.
#[macro_export]
macro_rules! system_test {
    ($switch:ident, async fn $name:ident () $body:block) => {
        #[::tokio::test(flavor = "multi_thread")]
        #[ignore = "system test: run by v3-system-tests.yml"]
        async fn $name() {
            $crate::require_system_test(stringify!($switch));
            $body
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWITCH: &str = "TRD_TESTTOOLS_NEVER_SET";

    /// An environment with `CI` and the switch set to the given values.
    fn env(
        ci: Option<&'static str>,
        switch: Option<&'static str>,
    ) -> impl Fn(&str) -> Option<String> {
        move |key: &str| {
            if key == CI_VAR {
                ci.map(str::to_string)
            } else if key == SWITCH {
                switch.map(str::to_string)
            } else {
                None
            }
        }
    }

    #[test]
    fn system_test_guard_matrix() {
        let cases: [(Option<&str>, Option<&str>, bool); 10] = [
            (None, None, false),
            (None, Some("1"), false),
            (Some(""), Some("1"), false),
            (Some("false"), Some("1"), false),
            (Some("1"), Some("1"), false),
            (Some("True"), Some("1"), false),
            (Some("true"), None, false),
            (Some("true"), Some("true"), false),
            (Some("true"), Some("0"), false),
            (Some("true"), Some("1"), true),
        ];
        for (ci, switch, expected) in cases {
            assert_eq!(
                system_test_allowed(env(ci, switch), SWITCH),
                expected,
                "CI={ci:?} {SWITCH}={switch:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "refusing to run a system test outside CI")]
    fn require_system_test_refuses() {
        require_system_test(SWITCH);
    }
}
