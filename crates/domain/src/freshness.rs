//! Board freshness: how often to poll and how to label the last refresh.
//!
//! Both pieces are pure so the composition root can read configuration / the
//! clock and hand the raw values in, keeping the policy testable and offline.

use std::time::Duration;

/// The background-poll cadence. Configurable, with a ~60s default and a small
/// floor so a misconfiguration cannot hammer GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollInterval(Duration);

impl PollInterval {
    /// The default cadence when nothing is configured (~60s).
    pub const DEFAULT_SECS: u64 = 60;
    /// The smallest cadence we honour, to protect GitHub's rate limit.
    pub const MIN_SECS: u64 = 5;

    /// Build from a number of seconds, clamped up to [`PollInterval::MIN_SECS`].
    pub fn from_secs(secs: u64) -> Self {
        Self(Duration::from_secs(secs.max(Self::MIN_SECS)))
    }

    /// Resolve a configured value (e.g. an environment variable).
    ///
    /// Absent, blank, or unparseable input falls back to the default; a value
    /// below the floor is clamped up to it.
    pub fn from_config(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => value
                .parse::<u64>()
                .map(Self::from_secs)
                .unwrap_or_default(),
            None => Self::default(),
        }
    }

    /// The interval as a [`Duration`], for sleeping between polls.
    pub fn as_duration(self) -> Duration {
        self.0
    }

    /// The interval in whole seconds, for display.
    pub fn as_secs(self) -> u64 {
        self.0.as_secs()
    }
}

impl Default for PollInterval {
    fn default() -> Self {
        Self::from_secs(Self::DEFAULT_SECS)
    }
}

/// Format a UNIX timestamp (seconds since the epoch) as a `HH:MM:SS UTC`
/// wall-clock label for the "last updated" indicator. Pure, so the UI can pass
/// the current time and the result stays testable without a real clock.
pub fn format_last_updated(secs_since_epoch: u64) -> String {
    let secs_of_day = secs_since_epoch % 86_400;
    let hours = secs_of_day / 3_600;
    let minutes = (secs_of_day % 3_600) / 60;
    let seconds = secs_of_day % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_sixty_seconds() {
        assert_eq!(PollInterval::default().as_secs(), 60);
        assert_eq!(
            PollInterval::default().as_duration(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn config_parsing_covers_the_edges() {
        struct Case {
            name: &'static str,
            raw: Option<&'static str>,
            expected_secs: u64,
        }

        let cases = [
            Case {
                name: "unset uses the default",
                raw: None,
                expected_secs: 60,
            },
            Case {
                name: "blank uses the default",
                raw: Some("   "),
                expected_secs: 60,
            },
            Case {
                name: "non-numeric uses the default",
                raw: Some("soon"),
                expected_secs: 60,
            },
            Case {
                name: "a valid value is honoured",
                raw: Some("30"),
                expected_secs: 30,
            },
            Case {
                name: "surrounding whitespace is trimmed",
                raw: Some(" 90 "),
                expected_secs: 90,
            },
            Case {
                name: "below the floor is clamped up",
                raw: Some("1"),
                expected_secs: PollInterval::MIN_SECS,
            },
        ];

        for case in cases {
            assert_eq!(
                PollInterval::from_config(case.raw).as_secs(),
                case.expected_secs,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn from_secs_clamps_to_the_floor() {
        assert_eq!(PollInterval::from_secs(0).as_secs(), PollInterval::MIN_SECS);
        assert_eq!(PollInterval::from_secs(120).as_secs(), 120);
    }

    #[test]
    fn formats_the_last_updated_clock_in_utc() {
        assert_eq!(format_last_updated(0), "00:00:00 UTC");
        // 1_700_000_000 -> 2023-11-14 22:13:20 UTC.
        assert_eq!(format_last_updated(1_700_000_000), "22:13:20 UTC");
        // Wraps across day boundaries (only the time-of-day is shown).
        assert_eq!(format_last_updated(86_400 + 3_661), "01:01:01 UTC");
    }
}
