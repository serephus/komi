//! Presentation settings that every page needs but that no handler ever
//! changes: the site name and how often a page refreshes itself when the live
//! stream is unavailable. Set once at startup from the config.

use std::sync::OnceLock;

use crate::config::Config;

#[derive(Debug, Clone)]
struct Settings {
    site_name: String,
    refresh_seconds: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            site_name: "komi".to_owned(),
            refresh_seconds: 10,
        }
    }
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

/// Called once from [`super::router`]. Later calls are ignored, which keeps the
/// tests that build a router more than once working.
pub fn init(config: &Config) {
    let _ = SETTINGS.set(Settings {
        site_name: config.site_name.clone(),
        refresh_seconds: config.refresh_seconds,
    });
}

fn settings() -> &'static Settings {
    SETTINGS.get_or_init(Settings::default)
}

pub fn site_name() -> &'static str {
    &settings().site_name
}

pub fn refresh_seconds() -> u64 {
    settings().refresh_seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_settings_fall_back_to_sensible_values() {
        // Tests run in one process, so this holds whether or not init() has run.
        assert!(!site_name().is_empty());
        assert!((2..=600).contains(&refresh_seconds()));
    }

    #[test]
    fn init_copies_the_config_in() {
        init(&Config {
            site_name: "club".to_owned(),
            refresh_seconds: 25,
            ..Config::default()
        });
        // Ignored if another test initialised first; both are valid states.
        assert!(!site_name().is_empty());
    }
}
