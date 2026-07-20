//! Freshness settings: how often the board re-polls GitHub in the background.
//!
//! The cadence is locally-owned settings state (GitHub is the source of truth
//! for the board itself), modelled here as a small validated value object so the
//! presentation layer's poll loop and any future settings UI share one rule for
//! what a valid interval is.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long the board waits between background refreshes.
///
/// Constructed through [`PollInterval::from_secs`], which clamps to a sane range
/// so a misconfiguration can neither hammer GitHub (too short) nor let the board
/// drift stale for too long (too long). The default is one minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollInterval {
    secs: u64,
}

impl PollInterval {
    /// The shortest allowed interval. A floor protects the GitHub rate limit
    /// from an over-eager configuration.
    pub const MIN_SECS: u64 = 10;
    /// The longest allowed interval, so the board cannot drift arbitrarily stale.
    pub const MAX_SECS: u64 = 3600;
    /// The default cadence: one minute.
    pub const DEFAULT_SECS: u64 = 60;

    /// A poll interval of `secs` seconds, clamped to `[MIN_SECS, MAX_SECS]`.
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

impl Default for PollInterval {
    fn default() -> Self {
        Self::from_secs(Self::DEFAULT_SECS)
    }
}

/// How long the board waits between full-load reconcile passes.
///
/// Constructed through [`ReconcileInterval::from_secs`], which clamps to a sane
/// range so reconcile runs often enough to heal cache drift, but not so often
/// that it duplicates the fast delta poll path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileInterval {
    secs: u64,
}

impl ReconcileInterval {
    /// Shortest reconcile cadence.
    pub const MIN_SECS: u64 = 60;
    /// Longest reconcile cadence.
    pub const MAX_SECS: u64 = 14_400;
    /// Default cadence: five minutes.
    pub const DEFAULT_SECS: u64 = 300;

    /// A reconcile interval of `secs` seconds, clamped to `[MIN_SECS, MAX_SECS]`.
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

impl Default for ReconcileInterval {
    fn default() -> Self {
        Self::from_secs(Self::DEFAULT_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_one_minute() {
        assert_eq!(PollInterval::default().as_secs(), 60);
        assert_eq!(
            PollInterval::default().as_duration(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn clamps_out_of_range_intervals_into_the_allowed_band() {
        struct Case {
            name: &'static str,
            secs: u64,
            expected: u64,
        }

        let cases = [
            Case {
                name: "zero is raised to the minimum so the poll never busy-loops",
                secs: 0,
                expected: PollInterval::MIN_SECS,
            },
            Case {
                name: "below the floor is raised to the minimum",
                secs: PollInterval::MIN_SECS - 1,
                expected: PollInterval::MIN_SECS,
            },
            Case {
                name: "a value in range is kept as-is",
                secs: 90,
                expected: 90,
            },
            Case {
                name: "above the ceiling is lowered to the maximum",
                secs: PollInterval::MAX_SECS + 1,
                expected: PollInterval::MAX_SECS,
            },
        ];

        for case in cases {
            assert_eq!(
                PollInterval::from_secs(case.secs).as_secs(),
                case.expected,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn reconcile_default_is_five_minutes() {
        assert_eq!(ReconcileInterval::default().as_secs(), 300);
        assert_eq!(
            ReconcileInterval::default().as_duration(),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn reconcile_interval_is_clamped_to_its_allowed_band() {
        struct Case {
            secs: u64,
            expected: u64,
        }

        let cases = [
            Case {
                secs: 0,
                expected: ReconcileInterval::MIN_SECS,
            },
            Case {
                secs: ReconcileInterval::MIN_SECS - 1,
                expected: ReconcileInterval::MIN_SECS,
            },
            Case {
                secs: 900,
                expected: 900,
            },
            Case {
                secs: ReconcileInterval::MAX_SECS + 1,
                expected: ReconcileInterval::MAX_SECS,
            },
        ];

        for case in cases {
            assert_eq!(
                ReconcileInterval::from_secs(case.secs).as_secs(),
                case.expected
            );
        }
    }
}
