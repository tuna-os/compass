//! The input server behind snippet keyword expansion.
//!
//! Part of [`super`]; see that module for the purity rule. The facts that
//! need the machine — whether `/dev/uinput` opens, what capabilities the
//! helper carries, what the running engine says — are gathered by the caller
//! into [`InputServerFacts`].

use std::path::{Path, PathBuf};

use compass_ipc::{DoctorCheck, DoctorStatus, InputServerStatus};

use super::check;
use super::sandbox::FLATPAK_INFO_PATH;
use crate::doctor::fs::FsProbe;

/// What the input-server check reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputServerFacts {
    /// `input_server.enabled` in `compass.json`.
    pub enabled: bool,
    /// The helper binary found, if any.
    pub helper: Option<PathBuf>,
    /// Whether the helper carries `cap_dac_override` (from `getcap`); `None`
    /// when that could not be read.
    pub capability: Option<bool>,
    /// Whether this user can open `/dev/uinput` for writing.
    pub uinput_writable: bool,
    /// Whether this user can read some `/dev/input/event*` node.
    pub input_readable: bool,
    /// What the running engine says about its helper, when one answered.
    pub engine: Option<InputServerStatus>,
}

/// The remedy for a helper without the capability.
fn setcap(helper: &Path) -> String {
    format!(
        "grant it the capability the C++ helper gets at install: \
         `sudo setcap cap_dac_override+ep {}` (NixOS: the compass input-server module's \
         security.wrappers, then point COMPASS_INPUT_SERVER_BIN at /run/wrappers/bin/compass-input-server)",
        helper.display()
    )
}

/// Whether snippet keywords can expand here, and what to do if not.
///
/// Never a failure: the launcher, and copying and pasting snippets, work
/// without it. What is lost is typing a keyword anywhere.
pub fn input_server<F: FsProbe>(fs: &F, facts: &InputServerFacts) -> DoctorCheck {
    const NAME: &str = "input-server";

    if fs.exists(Path::new(FLATPAK_INFO_PATH)) {
        return check(
            NAME,
            DoctorStatus::Warn,
            "inside a Flatpak: snippet keyword expansion is unavailable. The sandbox has no \
             /dev/input to read typing from and no /dev/uinput to type with, and no Flatpak \
             permission grants either. Snippets can still be copied and pasted from the \
             launcher; install a native package for keyword expansion",
        );
    }
    if !facts.enabled {
        return check(
            NAME,
            DoctorStatus::Ok,
            "disabled (input_server.enabled is false): snippet keywords do not expand. \
             `compass input-server enable` turns it on",
        );
    }
    if let Some(status) = &facts.engine
        && status.running
        && status.injection
    {
        return check(
            NAME,
            DoctorStatus::Ok,
            format!(
                "running and able to type; watching {} snippet keyword(s)",
                status.keywords
            ),
        );
    }
    let Some(helper) = &facts.helper else {
        return check(
            NAME,
            DoctorStatus::Warn,
            "compass-input-server is not installed beside the engine (or at \
             $COMPASS_INPUT_SERVER_BIN): snippet keywords will not expand",
        );
    };

    let mut problems = Vec::new();
    if let Some(problem) = facts.engine.as_ref().and_then(|s| s.problem.as_deref()) {
        problems.push(format!("the engine reports: {problem}"));
    }
    let privileged = facts.capability == Some(true);
    let reachable = facts.uinput_writable && facts.input_readable;
    if !(privileged || reachable) {
        let missing = match (facts.input_readable, facts.uinput_writable) {
            (false, false) => "/dev/input/event* and /dev/uinput are",
            (false, true) => "/dev/input/event* is",
            _ => "/dev/uinput is",
        };
        let capability = match facts.capability {
            Some(false) => "does not carry cap_dac_override",
            _ => "carries no capability this check could read (is getcap installed?)",
        };
        problems.push(format!(
            "{} {capability}, and {missing} not open to this user; {}",
            helper.display(),
            setcap(helper)
        ));
    }

    if problems.is_empty() {
        let state = match &facts.engine {
            Some(status) if status.running => "running, but cannot type yet",
            Some(_) => "installed but not running yet",
            None => "installed with the access it needs; the engine is not running to ask",
        };
        return check(
            NAME,
            DoctorStatus::Ok,
            format!("{} {state}", helper.display()),
        );
    }
    check(
        NAME,
        DoctorStatus::Warn,
        format!("snippet keywords will not expand: {}", problems.join("; ")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::fs::FakeFs;

    fn facts() -> InputServerFacts {
        InputServerFacts {
            enabled: true,
            helper: Some("/usr/libexec/compass/compass-input-server".into()),
            ..InputServerFacts::default()
        }
    }

    #[test]
    fn a_flatpak_is_told_keyword_expansion_cannot_work_there() {
        let fs = FakeFs::new().with_file(FLATPAK_INFO_PATH);
        let check = input_server(&fs, &facts());
        assert_eq!(check.status, DoctorStatus::Warn);
        assert!(check.detail.unwrap().contains("Flatpak"));
    }

    #[test]
    fn a_disabled_helper_is_fine_and_says_how_to_enable_it() {
        let check = input_server(
            &FakeFs::new(),
            &InputServerFacts {
                enabled: false,
                ..facts()
            },
        );
        assert_eq!(check.status, DoctorStatus::Ok);
        assert!(check.detail.unwrap().contains("input-server enable"));
    }

    #[test]
    fn a_missing_helper_is_named() {
        let check = input_server(
            &FakeFs::new(),
            &InputServerFacts {
                helper: None,
                ..facts()
            },
        );
        assert_eq!(check.status, DoctorStatus::Warn);
        assert!(check.detail.unwrap().contains("not installed"));
    }

    #[test]
    fn a_helper_without_access_gets_the_setcap_remedy() {
        let check = input_server(
            &FakeFs::new(),
            &InputServerFacts {
                capability: Some(false),
                ..facts()
            },
        );
        assert_eq!(check.status, DoctorStatus::Warn);
        let detail = check.detail.unwrap();
        assert!(detail.contains("setcap cap_dac_override+ep"), "{detail}");
        assert!(detail.contains("/dev/uinput"), "{detail}");
    }

    #[test]
    fn a_capable_helper_or_a_running_one_is_ok() {
        let capable = input_server(
            &FakeFs::new(),
            &InputServerFacts {
                capability: Some(true),
                ..facts()
            },
        );
        assert_eq!(capable.status, DoctorStatus::Ok);

        let running = input_server(
            &FakeFs::new(),
            &InputServerFacts {
                engine: Some(InputServerStatus {
                    enabled: true,
                    running: true,
                    injection: true,
                    keywords: 2,
                    ..InputServerStatus::default()
                }),
                ..facts()
            },
        );
        assert_eq!(running.status, DoctorStatus::Ok);
        assert!(running.detail.unwrap().contains("2 snippet keyword"));
    }

    #[test]
    fn the_engines_problem_is_passed_on() {
        let check = input_server(
            &FakeFs::new(),
            &InputServerFacts {
                capability: Some(true),
                engine: Some(InputServerStatus {
                    enabled: true,
                    running: true,
                    problem: Some("Failed to open /dev/uinput: Permission denied".into()),
                    ..InputServerStatus::default()
                }),
                ..facts()
            },
        );
        assert_eq!(check.status, DoctorStatus::Warn);
        assert!(check.detail.unwrap().contains("Permission denied"));
    }
}
