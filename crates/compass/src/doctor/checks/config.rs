//! Whether `compass.json` says what its author meant.
//!
//! Part of [`super`]; see that module for the purity rule. Reading and
//! checking the file is the caller's ([`crate::config_watch::load_and_check`]),
//! so a test hands in what was found.

use std::path::PathBuf;

use compass_ipc::{DoctorCheck, DoctorStatus};

use super::check;

/// What reading `compass.json` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFacts {
    /// Where it is, when that could be worked out.
    pub path: Option<PathBuf>,
    /// Whether a file is there at all.
    pub exists: bool,
    /// Each problem that did not stop it loading, as a sentence; or why it
    /// could not be read.
    pub problems: Result<Vec<String>, String>,
}

impl Default for ConfigFacts {
    fn default() -> Self {
        Self {
            path: None,
            exists: false,
            problems: Ok(Vec::new()),
        }
    }
}

/// `config.file`: a file that does not parse fails, one that loads with
/// problems warns and names each, and a clean one (or none) passes.
#[must_use]
pub fn config_file(facts: &ConfigFacts) -> DoctorCheck {
    const NAME: &str = "config.file";
    let Some(path) = &facts.path else {
        return check(
            NAME,
            DoctorStatus::Warn,
            "the configuration directory cannot be found: neither XDG_CONFIG_HOME nor HOME is set",
        );
    };
    let path = path.display();
    if !facts.exists {
        return check(
            NAME,
            DoctorStatus::Ok,
            format!("{path} does not exist yet, so every setting is at its default"),
        );
    }
    match &facts.problems {
        Err(error) => check(
            NAME,
            DoctorStatus::Fail,
            format!("{error}. Until it is fixed, every setting is at its default"),
        ),
        Ok(problems) if problems.is_empty() => {
            check(NAME, DoctorStatus::Ok, format!("{path} is valid"))
        }
        Ok(problems) => {
            let count = if problems.len() == 1 {
                "1 problem".to_owned()
            } else {
                format!("{} problems", problems.len())
            };
            check(
                NAME,
                DoctorStatus::Warn,
                format!("{path} has {count}: {}", problems.join("; ")),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::checks::tests::detail;

    fn facts(problems: Result<Vec<String>, String>) -> ConfigFacts {
        ConfigFacts {
            path: Some("/home/u/.config/compass/compass.json".into()),
            exists: true,
            problems,
        }
    }

    #[test]
    fn a_clean_file_and_no_file_pass() {
        assert_eq!(config_file(&facts(Ok(Vec::new()))).status, DoctorStatus::Ok);
        let missing = ConfigFacts {
            exists: false,
            ..facts(Ok(Vec::new()))
        };
        assert!(detail(&config_file(&missing)).contains("default"));
    }

    #[test]
    fn problems_warn_and_are_each_named() {
        let c = config_file(&facts(Ok(vec![
            "lancher is not a Compass setting and is ignored. Did you mean launcher?".into(),
        ])));
        assert_eq!(c.status, DoctorStatus::Warn);
        assert!(detail(&c).contains("1 problem: lancher"), "{}", detail(&c));
    }

    #[test]
    fn a_file_that_does_not_parse_fails() {
        let c = config_file(&facts(Err(
            "invalid configuration at x: line 3 column 1".into()
        )));
        assert_eq!(c.status, DoctorStatus::Fail);
    }
}
