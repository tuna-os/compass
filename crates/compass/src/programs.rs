//! Run Terminal Program: the executables on `PATH`, and finding one.
//!
//! A port of `ProgramDb` (`src/server/src/internal/program-db/`): every entry
//! of every `PATH` directory, and the lookup that decides whether a typed
//! command names a program. What the view offers for each is
//! `compass_core::system_run`.

use std::path::{Path, PathBuf};

/// The command's entrypoint, under the `commands` provider; its preferences
/// live at `providers.commands.entrypoints.run-program.preferences`.
pub const ENTRYPOINT: &str = "run-program";

/// The `default-action` preference's default: the C++ command declares
/// `run-in-terminal`.
pub const DEFAULT_ACTION: &str = "run-in-terminal";

/// The `PATH` directories, each once, in order: `Omnicast::systemPaths`.
#[must_use]
pub fn path_directories() -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty() && seen.insert(dir.clone()))
        .collect()
}

/// Every entry of every `PATH` directory, as `ProgramDb::scan` lists them.
#[must_use]
pub fn scan() -> Vec<String> {
    let mut programs = Vec::with_capacity(1000);
    for dir in path_directories() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        programs.extend(
            entries
                .flatten()
                .map(|entry| entry.path().to_string_lossy().into_owned()),
        );
    }
    programs
}

/// Where `name` is: itself when it is a file, else the first `PATH`
/// directory that has it, as `ProgramDb::programPath` finds it.
#[must_use]
pub fn program_path(name: &str) -> Option<PathBuf> {
    if Path::new(name).is_file() {
        return Some(PathBuf::from(name));
    }
    path_directories()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The `default-action` preference's value, or its default.
#[must_use]
pub fn default_action(preferences: Option<&serde_json::Map<String, serde_json::Value>>) -> String {
    preferences
        .and_then(|preferences| preferences.get("default-action"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(DEFAULT_ACTION)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_found_on_path_or_as_a_file() {
        assert!(program_path("sh").is_some(), "sh is on PATH");
        assert!(program_path("/bin/sh").is_some());
        assert!(program_path("certainly-not-a-program-here").is_none());
        assert!(scan().iter().any(|path| path.ends_with("/sh")));
    }

    #[test]
    fn the_default_action_is_the_preference_or_run_in_terminal() {
        assert_eq!(default_action(None), "run-in-terminal");
        let prefs = serde_json::json!({"default-action": "run"});
        assert_eq!(default_action(prefs.as_object()), "run");
    }
}
