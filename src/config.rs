//! Configuration.
//!
//! Three layers, lowest precedence first: built-in defaults, an optional TOML
//! file, then environment variables. The environment only overrides the handful
//! of keys that were environment variables before the file existed, so a
//! container can still be pointed at a database without mounting anything.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where a config file is looked for when none is given explicitly.
pub const DEFAULT_CONFIG_PATHS: [&str; 2] = ["/etc/komi/config.toml", "komi.toml"];

/// Environment variable naming a config file, checked before the defaults.
pub const CONFIG_ENV: &str = "KOMI_CONFIG";

#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on.
    pub bind: SocketAddr,
    /// redb database file.
    pub database: PathBuf,
    /// Name shown in the header and the page title.
    pub site_name: String,
    /// Default log filter, when `RUST_LOG` is not set.
    pub log: String,
    /// Mark the session cookie `Secure`. Defaults to on unless we bind to a
    /// loopback address, so local plain-HTTP testing just works.
    pub secure_cookies: bool,
    /// Trust `X-Forwarded-For` / `X-Real-IP` when rate limiting.
    pub trust_proxy: bool,
    /// Whether new accounts may be created.
    pub allow_registration: bool,
    /// Language used when the visitor has no preference.
    pub default_language: String,
    /// How often a page refreshes itself when the live event stream is not
    /// available. With a stream, this is never used.
    pub refresh_seconds: u64,
    pub session: Session,
    pub limits: Limits,
    pub lobby: Lobby,
}

#[derive(Debug, Clone, Copy)]
pub struct Session {
    /// How long a session lives without being used.
    pub days: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct Lobby {
    /// Days without activity after which a game with a free seat drops off the
    /// lobby. It stays reachable by its link. 0 keeps them forever.
    pub stale_open_days: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Failed logins allowed from one address before it is locked out.
    pub login_attempts: u32,
    pub login_window_minutes: i64,
    /// Accounts creatable from one address per window.
    pub registration_attempts: u32,
    pub registration_window_minutes: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "127.0.0.1:8080".parse().expect("a valid literal address"),
            database: PathBuf::from("komi.redb"),
            site_name: "komi".to_owned(),
            log: "info".to_owned(),
            secure_cookies: false,
            trust_proxy: false,
            allow_registration: true,
            default_language: "en".to_owned(),
            refresh_seconds: 30,
            session: Session { days: 30 },
            limits: Limits {
                login_attempts: 10,
                login_window_minutes: 15,
                registration_attempts: 5,
                registration_window_minutes: 60,
            },
            lobby: Lobby {
                stale_open_days: 14,
            },
        }
    }
}

impl Config {
    pub fn session_ttl_seconds(&self) -> i64 {
        self.session.days * 24 * 60 * 60
    }

    /// How old an untouched game with a free seat may get before the lobby
    /// stops advertising it. `None` means never.
    pub fn stale_open_seconds(&self) -> Option<i64> {
        (self.lobby.stale_open_days > 0).then(|| self.lobby.stale_open_days * 24 * 60 * 60)
    }

    /// Read defaults, then a config file (chosen by `explicit`, `$KOMI_CONFIG`
    /// or [`DEFAULT_CONFIG_PATHS`]), then the environment.
    pub fn load(explicit: Option<PathBuf>) -> Result<Loaded, ConfigError> {
        let requested = explicit.or_else(|| env_value(CONFIG_ENV).map(PathBuf::from));

        let file = match requested {
            // An explicitly named file that cannot be read is an error: silently
            // falling back to defaults would be a nasty surprise.
            Some(path) => Some((path.clone(), read(&path)?)),
            None => {
                let mut found = None;
                for candidate in DEFAULT_CONFIG_PATHS {
                    let path = PathBuf::from(candidate);
                    if path.is_file() {
                        found = Some((path.clone(), read(&path)?));
                        break;
                    }
                }
                found
            }
        };

        let (path, text) = match &file {
            Some((path, text)) => (Some(path.as_path()), Some(text.as_str())),
            None => (None, None),
        };
        let config = Config::build(path, text, |key| std::env::var(key).ok())?;
        Ok(Loaded {
            config,
            source: file.map(|(path, _)| path),
        })
    }

    /// The merge itself, isolated from the file system and the environment so
    /// that the tests can drive every combination.
    pub fn build(
        path: Option<&Path>,
        text: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Config, ConfigError> {
        let file = match (path, text) {
            (Some(path), Some(text)) => {
                toml::from_str::<FileConfig>(text).map_err(|error| ConfigError::Toml {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })?
            }
            _ => FileConfig::default(),
        };

        let defaults = Config::default();

        let bind_text = env("KOMI_BIND").or(file.bind);
        let bind: SocketAddr = bind_text
            .as_deref()
            .unwrap_or("127.0.0.1:8080")
            .parse()
            .map_err(|_| ConfigError::Invalid {
                field: "bind",
                value: bind_text.unwrap_or_default(),
            })?;

        let database = env("KOMI_DB")
            .map(PathBuf::from)
            .or(file.database)
            .unwrap_or(defaults.database);

        let secure_cookies = env_bool(&env, "KOMI_SECURE_COOKIES")?
            .or(file.secure_cookies)
            .unwrap_or_else(|| !bind.ip().is_loopback());

        let trust_proxy = env_bool(&env, "KOMI_TRUST_PROXY")?
            .or(file.trust_proxy)
            .unwrap_or(defaults.trust_proxy);

        let session_days = match env("KOMI_SESSION_DAYS") {
            Some(value) => value.parse().map_err(|_| ConfigError::Invalid {
                field: "KOMI_SESSION_DAYS",
                value,
            })?,
            None => file.session.days.unwrap_or(defaults.session.days),
        };

        let config = Config {
            bind,
            database,
            site_name: file.site_name.unwrap_or(defaults.site_name),
            log: file.log.unwrap_or(defaults.log),
            secure_cookies,
            trust_proxy,
            allow_registration: file
                .allow_registration
                .unwrap_or(defaults.allow_registration),
            default_language: file.default_language.unwrap_or(defaults.default_language),
            refresh_seconds: file.refresh_seconds.unwrap_or(defaults.refresh_seconds),
            session: Session { days: session_days },
            limits: Limits {
                login_attempts: file
                    .limits
                    .login_attempts
                    .unwrap_or(defaults.limits.login_attempts),
                login_window_minutes: file
                    .limits
                    .login_window_minutes
                    .unwrap_or(defaults.limits.login_window_minutes),
                registration_attempts: file
                    .limits
                    .registration_attempts
                    .unwrap_or(defaults.limits.registration_attempts),
                registration_window_minutes: file
                    .limits
                    .registration_window_minutes
                    .unwrap_or(defaults.limits.registration_window_minutes),
            },
            lobby: Lobby {
                stale_open_days: file
                    .lobby
                    .stale_open_days
                    .unwrap_or(defaults.lobby.stale_open_days),
            },
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.database.as_os_str().is_empty() {
            return Err(ConfigError::Invalid {
                field: "database",
                value: String::new(),
            });
        }
        if self.site_name.trim().is_empty() || self.site_name.chars().count() > 40 {
            return Err(ConfigError::Invalid {
                field: "site_name",
                value: self.site_name.clone(),
            });
        }
        if self.site_name.chars().any(char::is_control) {
            return Err(ConfigError::Invalid {
                field: "site_name",
                value: "contains control characters".to_owned(),
            });
        }
        if crate::i18n::Lang::parse(&self.default_language).is_none() {
            return Err(ConfigError::Invalid {
                field: "default_language",
                value: self.default_language.clone(),
            });
        }
        if self.log.trim().is_empty() {
            return Err(ConfigError::Invalid {
                field: "log",
                value: String::new(),
            });
        }
        range("session.days", self.session.days, 1, 3650)?;
        // Only used as a fallback, so a slow interval is the sensible default.
        range(
            "refresh_seconds",
            i64::try_from(self.refresh_seconds).unwrap_or(i64::MAX),
            5,
            3600,
        )?;
        range(
            "limits.login_attempts",
            i64::from(self.limits.login_attempts),
            1,
            10_000,
        )?;
        range(
            "limits.login_window_minutes",
            self.limits.login_window_minutes,
            1,
            1440,
        )?;
        range(
            "limits.registration_attempts",
            i64::from(self.limits.registration_attempts),
            1,
            10_000,
        )?;
        range(
            "limits.registration_window_minutes",
            self.limits.registration_window_minutes,
            1,
            1440,
        )?;
        // 0 is meaningful here: keep stale open seats in the lobby forever.
        range("lobby.stale_open_days", self.lobby.stale_open_days, 0, 3650)?;
        Ok(())
    }
}

fn range(field: &'static str, value: i64, min: i64, max: i64) -> Result<(), ConfigError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::OutOfRange {
            field,
            min,
            max,
            value,
        })
    }
}

fn read(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|error| ConfigError::Read {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn env_bool(
    env: &impl Fn(&str) -> Option<String>,
    key: &'static str,
) -> Result<Option<bool>, ConfigError> {
    match env(key) {
        Some(value) => parse_bool(&value)
            .map(Some)
            .ok_or(ConfigError::Invalid { field: key, value }),
        None => Ok(None),
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// The file is deliberately strict about its shape: a misspelled key is a
/// mistake worth reporting, not a setting worth ignoring.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: Option<String>,
    database: Option<PathBuf>,
    site_name: Option<String>,
    log: Option<String>,
    secure_cookies: Option<bool>,
    trust_proxy: Option<bool>,
    allow_registration: Option<bool>,
    default_language: Option<String>,
    refresh_seconds: Option<u64>,
    #[serde(default)]
    session: FileSession,
    #[serde(default)]
    lobby: FileLobby,
    #[serde(default)]
    limits: FileLimits,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSession {
    days: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLimits {
    login_attempts: Option<u32>,
    login_window_minutes: Option<i64>,
    registration_attempts: Option<u32>,
    registration_window_minutes: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLobby {
    stale_open_days: Option<i64>,
}

/// A loaded configuration plus where it came from, for logging.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    pub source: Option<PathBuf>,
}

impl Loaded {
    pub fn source_label(&self) -> String {
        match &self.source {
            Some(path) => path.display().to_string(),
            None => "defaults".to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("{path}: {message}")]
    Toml { path: PathBuf, message: String },
    #[error("setting “{field}” is not valid: {value}")]
    Invalid { field: &'static str, value: String },
    #[error("setting “{field}” must be between {min} and {max}, got {value}")]
    OutOfRange {
        field: &'static str,
        min: i64,
        max: i64,
        value: i64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(text: Option<&str>, pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        let path = Path::new("komi.toml");
        Config::build(text.map(|_| path), text, |key| {
            pairs
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.clone())
        })
    }

    #[test]
    fn defaults_are_local_and_insecure_for_dev() {
        let config = build(None, &[]).unwrap();
        assert_eq!(config.bind.to_string(), "127.0.0.1:8080");
        assert_eq!(config.database, PathBuf::from("komi.redb"));
        assert_eq!(config.site_name, "komi");
        assert_eq!(config.log, "info");
        assert!(!config.secure_cookies, "loopback defaults to plain http");
        assert!(!config.trust_proxy);
        assert!(config.allow_registration);
        assert_eq!(config.refresh_seconds, 30);
        assert_eq!(config.session.days, 30);
        assert_eq!(config.limits.login_attempts, 10);
        assert_eq!(config.limits.registration_attempts, 5);
        assert_eq!(config.lobby.stale_open_days, 14);
        assert_eq!(config.stale_open_seconds(), Some(14 * 24 * 60 * 60));
        assert_eq!(config.session_ttl_seconds(), 30 * 24 * 60 * 60);
    }

    #[test]
    fn a_config_file_sets_everything_it_mentions() {
        let toml = r#"
            bind = "0.0.0.0:9000"
            database = "/var/lib/komi/games.redb"
            site_name = "go club"
            log = "komi=debug"
            secure_cookies = false
            trust_proxy = true
            allow_registration = false
            refresh_seconds = 30

            [session]
            days = 7

            [limits]
            login_attempts = 3
            login_window_minutes = 5
            registration_attempts = 1
            registration_window_minutes = 120

            [lobby]
            stale_open_days = 3
        "#;
        let config = build(Some(toml), &[]).unwrap();
        assert_eq!(config.bind.to_string(), "0.0.0.0:9000");
        assert_eq!(config.database, PathBuf::from("/var/lib/komi/games.redb"));
        assert_eq!(config.site_name, "go club");
        assert_eq!(config.log, "komi=debug");
        assert!(!config.secure_cookies);
        assert!(config.trust_proxy);
        assert!(!config.allow_registration);
        assert_eq!(config.refresh_seconds, 30);
        assert_eq!(config.session.days, 7);
        assert_eq!(config.limits.login_attempts, 3);
        assert_eq!(config.limits.registration_window_minutes, 120);
        assert_eq!(config.lobby.stale_open_days, 3);
    }

    #[test]
    fn a_partial_file_keeps_the_defaults() {
        let config = build(Some("refresh_seconds = 20\n"), &[]).unwrap();
        assert_eq!(config.refresh_seconds, 20);
        assert_eq!(config.session.days, 30);
        assert_eq!(config.bind.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn the_environment_wins_over_the_file() {
        let toml = "bind = \"0.0.0.0:9000\"\ndatabase = \"/from/file.redb\"\n";
        let config = build(
            Some(toml),
            &[
                ("KOMI_BIND", "127.0.0.1:1234"),
                ("KOMI_DB", "/from/env.redb"),
            ],
        )
        .unwrap();
        assert_eq!(config.bind.to_string(), "127.0.0.1:1234");
        assert_eq!(config.database, PathBuf::from("/from/env.redb"));
    }

    #[test]
    fn secure_cookies_follow_the_bind_address_unless_told_otherwise() {
        assert!(!build(None, &[]).unwrap().secure_cookies);
        assert!(
            !build(Some("bind = \"127.0.0.1:8080\"\n"), &[])
                .unwrap()
                .secure_cookies
        );
        assert!(
            build(Some("bind = \"0.0.0.0:8080\"\n"), &[])
                .unwrap()
                .secure_cookies
        );
        assert!(
            !build(
                Some("bind = \"0.0.0.0:8080\"\nsecure_cookies = false\n"),
                &[]
            )
            .unwrap()
            .secure_cookies
        );
        assert!(
            build(Some("secure_cookies = true\n"), &[])
                .unwrap()
                .secure_cookies
        );
        // The environment can override the file as well.
        assert!(
            build(
                Some("secure_cookies = false\n"),
                &[("KOMI_SECURE_COOKIES", "yes")]
            )
            .unwrap()
            .secure_cookies
        );
    }

    #[test]
    fn misspelled_keys_are_reported_with_the_file_name() {
        let error = build(Some("site_nmae = \"oops\"\n"), &[]).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("komi.toml"), "got {message}");
        assert!(message.contains("site_nmae"), "got {message}");
    }

    #[test]
    fn misspelled_section_keys_are_reported_too() {
        let error = build(Some("[limits]\nlogin_attempt = 3\n"), &[]).unwrap_err();
        assert!(error.to_string().contains("login_attempt"));
    }

    #[test]
    fn nonsense_values_are_refused() {
        for toml in [
            "bind = \"nonsense\"\n",
            "bind = \"127.0.0.1\"\n",
            "refresh_seconds = 1\n",
            "refresh_seconds = 100_000\n",
            "refresh_seconds = 100_000\n",
            "session.days = 0\n",
            "site_name = \"\"\n",
            "site_name = \"   \"\n",
            "log = \"\"\n",
            "database = \"\"\n",
            "[limits]\nlogin_attempts = 0\n",
            "[limits]\nlogin_window_minutes = 0\n",
            "[limits]\nregistration_window_minutes = 5000\n",
        ] {
            assert!(build(Some(toml), &[]).is_err(), "should reject {toml:?}");
        }
    }

    #[test]
    fn wrong_types_are_refused() {
        assert!(build(Some("refresh_seconds = \"ten\"\n"), &[]).is_err());
        assert!(build(Some("secure_cookies = \"maybe\"\n"), &[]).is_err());
        assert!(build(Some("session = 3\n"), &[]).is_err());
    }

    #[test]
    fn bad_environment_values_are_refused() {
        assert!(build(None, &[("KOMI_SECURE_COOKIES", "maybe")]).is_err());
        assert!(build(None, &[("KOMI_TRUST_PROXY", "maybe")]).is_err());
        assert!(build(None, &[("KOMI_SESSION_DAYS", "lots")]).is_err());
        assert!(build(None, &[("KOMI_SESSION_DAYS", "0")]).is_err());
        assert!(build(None, &[("KOMI_BIND", "nope")]).is_err());
    }

    #[test]
    fn zero_never_means_unlimited() {
        // Closing sign-up is allow_registration = false, not a limit of zero.
        assert!(build(Some("[limits]\nregistration_attempts = 0\n"), &[]).is_err());
        assert!(build(Some("[limits]\nlogin_attempts = 0\n"), &[]).is_err());
        assert!(build(Some("[limits]\nregistration_attempts = 1\n"), &[]).is_ok());
    }

    #[test]
    fn stale_open_seats_can_be_kept_forever() {
        assert_eq!(
            build(Some("[lobby]\nstale_open_days = 0\n"), &[])
                .unwrap()
                .stale_open_seconds(),
            None
        );
        assert!(build(Some("[lobby]\nstale_open_days = 4000\n"), &[]).is_err());
    }

    #[test]
    fn reading_a_named_file_works_and_a_missing_one_is_an_error() {
        let dir = std::env::temp_dir().join(format!("komi-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "site_name = \"from disk\"\nrefresh_seconds = 15\n").unwrap();

        let loaded = Config::load(Some(path.clone())).unwrap();
        assert_eq!(loaded.config.site_name, "from disk");
        assert_eq!(loaded.config.refresh_seconds, 15);
        assert_eq!(loaded.source, Some(path));
        assert!(loaded.source_label().ends_with("config.toml"));

        let missing = dir.join("nope.toml");
        assert!(matches!(
            Config::load(Some(missing)).unwrap_err(),
            ConfigError::Read { .. }
        ));

        std::fs::remove_dir_all(&dir).ok();
    }
}
