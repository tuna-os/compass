//! The filesystem boundary an extension worker runs behind.
//!
//! Phase 4 confines each worker with **Landlock** rather than with the spec's
//! read-only bind mounts: Landlock needs no privilege and no mount namespace,
//! which is what makes it work inside a Flatpak. `vicinae spike sandbox`
//! established that it does work, nested, and this is that finding turned into
//! something a host can use.
//!
//! # A ruleset restricts the process that applies it, so a child needs a helper
//!
//! `restrict_self` confines the calling thread and everything it later spawns;
//! there is no "apply this to that process". Confining a child therefore means
//! running code *in* the child between fork and exec — which is `pre_exec`, and
//! `pre_exec` is `unsafe`, which this workspace forbids.
//!
//! The way out is [`compass-sandbox-exec`](../../compass-sandbox/src/bin/exec.rs):
//! a small program that applies a policy to *itself* and then `exec`s the real
//! worker. `std::os::unix::process::CommandExt::exec` is a safe function, the
//! restriction survives `execve` because that is what Landlock guarantees, and
//! the host spawns the launcher instead of the worker. [`Policy::command`]
//! builds that invocation.
//!
//! # Fail closed, and say which kind of closed
//!
//! A ruleset that applies and enforces nothing looks like a sandbox in every
//! log and confines nothing, so [`Enforcement`] distinguishes the three answers
//! the kernel can give and [`Mode::Strict`] refuses anything short of the whole
//! ruleset. The default is [`Mode::Strict`]: a worker running unconfined
//! because of an old kernel is a decision, not a default.

use std::path::{Path, PathBuf};

use landlock::{
    ABI, Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    path_beneath_rules,
};

/// The Landlock ABI this crate asks for.
///
/// Fixed rather than detected, for the reason the `landlock` crate gives for
/// offering no runtime query: choosing rights from the running kernel makes the
/// boundary differ between machines, and between boots. V1 is the ABI that
/// introduced the filesystem rights used here. Raising it is a deliberate
/// change with its own tests, not something to discover at runtime.
pub const TARGET_ABI: ABI = ABI::V1;

/// How much of a ruleset the kernel enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enforcement {
    /// Every rule, as written.
    Full,
    /// The subset an older kernel understood.
    Partial,
    /// None of it. The call succeeded and nothing is confined.
    None,
}

impl Enforcement {
    fn from_status(status: RulesetStatus) -> Self {
        match status {
            RulesetStatus::FullyEnforced => Self::Full,
            RulesetStatus::PartiallyEnforced => Self::Partial,
            RulesetStatus::NotEnforced => Self::None,
        }
    }
}

/// What to do when the kernel will not enforce the whole ruleset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Refuse to continue. The default.
    #[default]
    Strict,
    /// Continue, and let the caller decide what a partial boundary is worth.
    ///
    /// Only for a host that reports the degradation somewhere a person will
    /// see it. Silently accepting it is how a sandbox becomes decorative.
    BestEffort,
}

/// Why a policy could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The kernel refused, or the paths could not be opened.
    #[error("applying the Landlock ruleset failed: {0}")]
    Landlock(#[from] landlock::RulesetError),

    /// The ruleset applied, but the kernel enforced less than all of it.
    #[error(
        "the kernel enforced {enforced:?} of this ruleset, not all of it. A worker behind a \
         partial boundary is not behind the boundary this policy describes."
    )]
    NotFullyEnforced {
        /// What the kernel did enforce.
        enforced: Enforcement,
    },

    /// A path in the policy does not exist.
    ///
    /// Landlock rules are opened at apply time, and a missing path would
    /// otherwise silently contribute nothing — a policy that names a directory
    /// that is not there grants access to nothing and *looks* like it grants
    /// access to something.
    #[error("the policy names {0}, which does not exist")]
    MissingPath(PathBuf),
}

/// What a worker may reach.
///
/// Everything not named here is denied: there is no "allow the rest". A policy
/// with no paths at all is a process that can open nothing, which is a valid
/// and occasionally useful thing to ask for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Directories and files that may be read.
    pub read: Vec<PathBuf>,
    /// Directories and files that may be read and written.
    pub write: Vec<PathBuf>,
    /// Directories and files that may be executed.
    ///
    /// Separate from `read` because a worker that may read its own source is
    /// not thereby allowed to run any binary it finds.
    pub execute: Vec<PathBuf>,
    /// What to do about a partially-enforced ruleset.
    pub mode: Mode,
}

impl Policy {
    /// A policy that allows nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a readable path.
    #[must_use]
    pub fn read(mut self, path: impl Into<PathBuf>) -> Self {
        self.read.push(path.into());
        self
    }

    /// Adds a writable path. Writable does not imply readable; add both if both
    /// are wanted, as the C++ bind-mount design would have had to.
    #[must_use]
    pub fn write(mut self, path: impl Into<PathBuf>) -> Self {
        self.write.push(path.into());
        self
    }

    /// Adds an executable path.
    #[must_use]
    pub fn execute(mut self, path: impl Into<PathBuf>) -> Self {
        self.execute.push(path.into());
        self
    }

    /// Sets the mode.
    #[must_use]
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Every path the policy names, whatever the right.
    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.read
            .iter()
            .chain(self.write.iter())
            .chain(self.execute.iter())
    }

    /// Applies this policy to the calling process, for good.
    ///
    /// There is no way to undo it, which is why the launcher applies it to
    /// itself immediately before `exec` rather than the host applying it to
    /// itself and hoping.
    ///
    /// # Errors
    ///
    /// [`Error::MissingPath`] if a named path is not there, [`Error::Landlock`]
    /// if the kernel refuses, and [`Error::NotFullyEnforced`] in
    /// [`Mode::Strict`] when the kernel enforces less than the whole ruleset.
    pub fn apply(&self) -> Result<Enforcement, Error> {
        for path in self.paths() {
            if !path.exists() {
                return Err(Error::MissingPath(path.clone()));
            }
        }

        let abi = TARGET_ABI;
        let ruleset = Ruleset::default()
            .handle_access(AccessFs::from_all(abi))?
            .create()?
            .add_rules(path_beneath_rules(&self.read, AccessFs::from_read(abi)))?
            .add_rules(path_beneath_rules(
                &self.write,
                AccessFs::from_read(abi) | AccessFs::from_write(abi),
            ))?
            .add_rules(path_beneath_rules(&self.execute, AccessFs::Execute))?;

        let enforced = Enforcement::from_status(ruleset.restrict_self()?.ruleset);
        if self.mode == Mode::Strict && enforced != Enforcement::Full {
            return Err(Error::NotFullyEnforced { enforced });
        }
        Ok(enforced)
    }

    /// The arguments that hand this policy to the launcher.
    ///
    /// Paths go one per flag rather than in one separated list: a path may
    /// contain any byte but NUL, including the separator anyone would pick.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for (flag, paths) in [
            ("--read", &self.read),
            ("--write", &self.write),
            ("--execute", &self.execute),
        ] {
            for path in paths {
                args.push(flag.to_owned());
                args.push(path.to_string_lossy().into_owned());
            }
        }
        if self.mode == Mode::BestEffort {
            args.push("--best-effort".to_owned());
        }
        args
    }

    /// A command that runs `program` behind this policy.
    ///
    /// `launcher` is the path to `compass-sandbox-exec`. The host passes it
    /// explicitly rather than this crate guessing: where it lives is a
    /// packaging question, and a guess that is wrong fails at spawn time with
    /// a confusing error.
    #[must_use]
    pub fn command(
        &self,
        launcher: &Path,
        program: &Path,
        program_args: &[String],
    ) -> std::process::Command {
        let mut command = std::process::Command::new(launcher);
        command.args(self.args());
        command.arg("--");
        command.arg(program);
        command.args(program_args);
        command
    }
}

/// Reads a policy back out of [`Policy::args`].
///
/// Used by the launcher, and public so the round trip can be tested from one
/// place rather than asserted twice.
///
/// # Errors
///
/// Returns the offending argument if a flag has no value or is not known.
pub fn parse_args<I, S>(args: I) -> Result<(Policy, Vec<String>), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut policy = Policy::new();
    let mut rest = Vec::new();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        let mut value = || {
            args.next()
                .map(|v| v.as_ref().to_owned())
                .ok_or_else(|| format!("{arg} needs a path"))
        };
        match arg {
            "--read" => policy.read.push(value()?.into()),
            "--write" => policy.write.push(value()?.into()),
            "--execute" => policy.execute.push(value()?.into()),
            "--best-effort" => policy.mode = Mode::BestEffort,
            "--" => {
                rest.extend(args.map(|a| a.as_ref().to_owned()));
                break;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    Ok((policy, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_survives_being_turned_into_arguments() {
        let policy = Policy::new()
            .read("/usr/share")
            .read("/etc/hosts")
            .write("/tmp/work")
            .execute("/usr/bin/node")
            .mode(Mode::BestEffort);

        let (parsed, rest) = parse_args(policy.args()).expect("the round trip parses");
        assert_eq!(parsed, policy);
        assert!(rest.is_empty());
    }

    #[test]
    fn a_path_with_a_separator_in_it_survives() {
        // The reason paths go one per flag. A colon, a comma and a newline are
        // all legal in a path, and any of them would be a plausible separator.
        let awkward = "/tmp/a:b,c\nd";
        let policy = Policy::new().read(awkward);
        let (parsed, _) = parse_args(policy.args()).expect("parse");
        assert_eq!(parsed.read, vec![PathBuf::from(awkward)]);
    }

    #[test]
    fn the_program_is_whatever_follows_the_separator() {
        let policy = Policy::new().read("/usr");
        let mut args = policy.args();
        args.push("--".to_owned());
        args.extend(["node".to_owned(), "--read".to_owned(), "x".to_owned()]);

        let (parsed, rest) = parse_args(args).expect("parse");
        assert_eq!(parsed.read, vec![PathBuf::from("/usr")]);
        assert_eq!(
            rest,
            vec!["node".to_owned(), "--read".to_owned(), "x".to_owned()],
            "arguments after `--` belong to the program, even when they look like ours"
        );
    }

    #[test]
    fn an_unknown_argument_is_refused_rather_than_ignored() {
        // An ignored flag is a policy that is quietly weaker than it reads.
        assert!(parse_args(["--allow-everything"]).is_err());
        assert!(parse_args(["--read"]).is_err(), "a flag with no value");
    }

    #[test]
    fn the_default_mode_is_strict() {
        assert_eq!(Policy::new().mode, Mode::Strict);
        assert!(
            !Policy::new().args().contains(&"--best-effort".to_owned()),
            "strict is the default on both sides of the argument list"
        );
    }

    #[test]
    fn a_missing_path_is_refused_before_anything_is_applied() {
        // Applying the ruleset first would confine this test process for good.
        // That it returns before that is the property under test.
        let policy = Policy::new().read("/definitely/not/here");
        match policy.apply() {
            Err(Error::MissingPath(path)) => assert_eq!(path, Path::new("/definitely/not/here")),
            other => panic!("a missing path must be refused, got {other:?}"),
        }
    }

    #[test]
    fn the_command_puts_the_program_after_the_separator() {
        let policy = Policy::new().read("/usr");
        let command = policy.command(
            Path::new("/usr/libexec/compass-sandbox-exec"),
            Path::new("/usr/bin/node"),
            &["worker.js".to_owned()],
        );
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--read".to_owned(),
                "/usr".to_owned(),
                "--".to_owned(),
                "/usr/bin/node".to_owned(),
                "worker.js".to_owned(),
            ]
        );
        assert_eq!(
            command.get_program(),
            std::ffi::OsStr::new("/usr/libexec/compass-sandbox-exec")
        );
    }
}
