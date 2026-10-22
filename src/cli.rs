//! Command line arguments. Two flags do not justify a dependency.

use std::path::PathBuf;

pub const USAGE: &str = "\
komi — correspondence Go, no clocks, no result

usage: komi [--config PATH]

  -c, --config PATH   read settings from a TOML file
                      (default: $KOMI_CONFIG, then /etc/komi/config.toml,
                      then ./komi.toml)
  -h, --help          show this help
  -V, --version       show the version

Settings are resolved as: built-in defaults, then the config file, then the
KOMI_BIND, KOMI_DB, KOMI_SECURE_COOKIES, KOMI_TRUST_PROXY and KOMI_SESSION_DAYS
environment variables. RUST_LOG overrides the log filter from the file.
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    pub config: Option<PathBuf>,
    pub help: bool,
    pub version: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("unknown argument “{0}”")]
    Unknown(String),
    #[error("--config needs a path")]
    MissingValue,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, CliError> {
    let mut cli = Cli::default();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "-c" | "--config" => {
                let value = args.next().ok_or(CliError::MissingValue)?;
                if value.is_empty() {
                    return Err(CliError::MissingValue);
                }
                cli.config = Some(PathBuf::from(value));
            }
            other => match other.strip_prefix("--config=") {
                Some(value) if !value.is_empty() => cli.config = Some(PathBuf::from(value)),
                Some(_) => return Err(CliError::MissingValue),
                None => return Err(CliError::Unknown(argument)),
            },
        }
    }
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Cli, CliError> {
        parse(args.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn no_arguments_means_defaults() {
        assert_eq!(parse_args(&[]).unwrap(), Cli::default());
    }

    #[test]
    fn the_config_path_can_be_given_in_both_spellings() {
        assert_eq!(
            parse_args(&["--config", "/etc/komi/config.toml"])
                .unwrap()
                .config,
            Some(PathBuf::from("/etc/komi/config.toml"))
        );
        assert_eq!(
            parse_args(&["--config=/etc/komi/config.toml"])
                .unwrap()
                .config,
            Some(PathBuf::from("/etc/komi/config.toml"))
        );
        assert_eq!(
            parse_args(&["-c", "komi.toml"]).unwrap().config,
            Some(PathBuf::from("komi.toml"))
        );
    }

    #[test]
    fn help_and_version_are_flags() {
        assert!(parse_args(&["--help"]).unwrap().help);
        assert!(parse_args(&["-h"]).unwrap().help);
        assert!(parse_args(&["--version"]).unwrap().version);
        assert!(parse_args(&["-V"]).unwrap().version);
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(matches!(
            parse_args(&["--config"]).unwrap_err(),
            CliError::MissingValue
        ));
        assert!(matches!(
            parse_args(&["--config="]).unwrap_err(),
            CliError::MissingValue
        ));
        assert!(matches!(
            parse_args(&["--verbose"]).unwrap_err(),
            CliError::Unknown(_)
        ));
    }

    #[test]
    fn the_usage_text_documents_the_search_order() {
        assert!(USAGE.contains("--config"));
        assert!(USAGE.contains("/etc/komi/config.toml"));
        assert!(USAGE.contains("KOMI_CONFIG"));
    }
}
