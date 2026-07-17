use serde::{Deserialize, Serialize};

/// The user-selected app theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreference {
    Light,
    Dark,
}

impl ThemePreference {
    /// The daisyUI theme name for this preference.
    pub const fn as_data_theme(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Parse a persisted daisyUI theme name.
    pub fn from_data_theme(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}
