//! The **Freshness contract**: the three cadences on which the open board asks
//! GitHub what exists (ADR 0008).
//!
//! | cadence | default | what it keeps fresh |
//! |---|---|---|
//! | [`HotInterval`] | 15s | every fact on a PR, and the **Cascade** a merge triggers |
//! | [`StructuralInterval`] | 60s | issues: new, retitled, relabelled, assigned, closed |
//! | [`SafetyNetInterval`] | 5 min | what GitHub did not stamp, deletes, transfers |
//!
//! This module is the only place the refresh rhythm is defined; the
//! presentation layer's loops read it through [`FreshnessContract`]. Each
//! cadence is a small clamped value object, locally-owned settings state (GitHub
//! is the source of truth for the board itself), so a later settings UI shares
//! one rule for what a valid interval is.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Defines a cadence: a whole number of seconds clamped to `[MIN_SECS, MAX_SECS]`
/// on construction, so a misconfiguration can neither hammer GitHub's rate limit
/// (too short) nor let the board drift past its contract (too long).
macro_rules! cadence {
    (
        $(#[$doc:meta])*
        $name:ident { min: $min:expr, max: $max:expr, default: $default:expr }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name {
            secs: u64,
        }

        impl $name {
            /// The shortest allowed interval.
            pub const MIN_SECS: u64 = $min;
            /// The longest allowed interval.
            pub const MAX_SECS: u64 = $max;
            /// The default interval, its tier of the Freshness contract.
            pub const DEFAULT_SECS: u64 = $default;

            /// An interval of `secs` seconds, clamped to `[MIN_SECS, MAX_SECS]`.
            pub fn from_secs(secs: u64) -> Self {
                Self {
                    secs: secs.clamp(Self::MIN_SECS, Self::MAX_SECS),
                }
            }

            /// The interval in whole seconds.
            pub fn as_secs(&self) -> u64 {
                self.secs
            }

            /// The interval as a [`Duration`], for handing to a timer.
            pub fn as_duration(&self) -> Duration {
                Duration::from_secs(self.secs)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::from_secs(Self::DEFAULT_SECS)
            }
        }
    };
}

cadence! {
    /// How long the board waits between **hot** refreshes: the open-PR **Sweep**
    /// that carries every fact on the critical path (a PR appearing or vanishing,
    /// a new unresolved review comment, CI turning red, a conflict, an approval).
    ///
    /// The facts nobody tells the user about, so the board must go and look. The
    /// floor keeps one board from spending the whole GraphQL budget; the ceiling
    /// keeps "within 15 seconds" from quietly becoming minutes.
    HotInterval { min: 5, max: 300, default: 15 }
}

cadence! {
    /// How long the board waits between **structural** refreshes: the open-issue
    /// index **Sweep**, plus a **Hydrate** of only what it reports new or moved.
    ///
    /// Grooming output (a new PRD or Slice), retitles, relabels, assignments and
    /// manual closes: change the user expects, so it may lag behind the hot tier.
    StructuralInterval { min: 10, max: 3_600, default: 60 }
}

cadence! {
    /// How long the board waits between **safety-net** refreshes: a full
    /// **Hydrate** of every open issue.
    ///
    /// The one path that assumes GitHub lied about everything else: it heals a
    /// relationship edit GitHub did not stamp, which no index sweep can see. The
    /// floor keeps it from duplicating the structural tier; the ceiling bounds how
    /// long such an edit can stay wrong.
    SafetyNetInterval { min: 60, max: 14_400, default: 300 }
}

/// The three cadences of the open board, together: the Freshness contract as one
/// value. [`FreshnessContract::default`] is the promise the app is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FreshnessContract {
    /// The open-PR sweep's cadence.
    pub hot: HotInterval,
    /// The issue-index sweep's cadence.
    pub structural: StructuralInterval,
    /// The full issue hydrate's cadence.
    pub safety_net: SafetyNetInterval,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_contract_is_fifteen_seconds_one_minute_and_five_minutes() {
        let contract = FreshnessContract::default();

        assert_eq!(contract.hot.as_duration(), Duration::from_secs(15));
        assert_eq!(contract.structural.as_duration(), Duration::from_secs(60));
        assert_eq!(contract.safety_net.as_duration(), Duration::from_secs(300));
    }

    #[test]
    fn every_cadence_is_clamped_to_its_allowed_band() {
        struct Case {
            name: &'static str,
            clamp: fn(u64) -> u64,
            secs: u64,
            expected: u64,
        }

        let hot = |secs| HotInterval::from_secs(secs).as_secs();
        let structural = |secs| StructuralInterval::from_secs(secs).as_secs();
        let safety_net = |secs| SafetyNetInterval::from_secs(secs).as_secs();

        let cases = [
            Case {
                name: "hot: zero is raised to the floor so the sweep never busy-loops",
                clamp: hot,
                secs: 0,
                expected: HotInterval::MIN_SECS,
            },
            Case {
                name: "hot: a value in range is kept as-is",
                clamp: hot,
                secs: 20,
                expected: 20,
            },
            Case {
                name: "hot: above the ceiling is lowered to it",
                clamp: hot,
                secs: HotInterval::MAX_SECS + 1,
                expected: HotInterval::MAX_SECS,
            },
            Case {
                name: "structural: below the floor is raised to it",
                clamp: structural,
                secs: StructuralInterval::MIN_SECS - 1,
                expected: StructuralInterval::MIN_SECS,
            },
            Case {
                name: "structural: a value in range is kept as-is",
                clamp: structural,
                secs: 90,
                expected: 90,
            },
            Case {
                name: "structural: above the ceiling is lowered to it",
                clamp: structural,
                secs: StructuralInterval::MAX_SECS + 1,
                expected: StructuralInterval::MAX_SECS,
            },
            Case {
                name: "safety net: below the floor is raised to it",
                clamp: safety_net,
                secs: SafetyNetInterval::MIN_SECS - 1,
                expected: SafetyNetInterval::MIN_SECS,
            },
            Case {
                name: "safety net: a value in range is kept as-is",
                clamp: safety_net,
                secs: 900,
                expected: 900,
            },
            Case {
                name: "safety net: above the ceiling is lowered to it",
                clamp: safety_net,
                secs: SafetyNetInterval::MAX_SECS + 1,
                expected: SafetyNetInterval::MAX_SECS,
            },
        ];

        for case in cases {
            assert_eq!((case.clamp)(case.secs), case.expected, "{}", case.name);
        }
    }
}
