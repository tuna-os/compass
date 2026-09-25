//! Script commands: finding them, and running them in their output modes.
//!
//! The header parser, the scan's rules, the command line and the output
//! tokenizer are `compass_core::script_command`, `script_scan` and
//! `script_output`; this is the engine's side — which directories are
//! scanned, the processes, and what each mode does with what they print, as
//! `ScriptExecutorAction::execute` decides it:
//!
//! * `fullOutput` streams stdout and stderr to a view (with `FORCE_COLOR=1`);
//! * `compact` and `inline` take the first line of stdout within ten seconds;
//!   an inline script's line becomes its subtitle in root search;
//! * `silent` does the same and says the line in a transient notification,
//!   the launcher having hidden;
//! * `terminal` runs in the terminal emulator.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use compass_core::script_command::{ArgumentType, OutputMode};
use compass_core::script_scan::{ScriptCommandFile, ScriptItem, ScriptMetadataStore};
use compass_ipc::{ScriptArgumentEntry, ScriptEntry};
use tokio::io::AsyncReadExt;

/// How long a one-line mode may run before it is killed, as `executeOneLine`
/// allows.
pub const ONE_LINE_TIMEOUT: Duration = Duration::from_secs(10);

/// Where inline scripts' last output lines are kept, in Compass's data
/// directory.
pub const METADATA_FILE: &str = "compass-script-metadata.json";

/// The provider id whose preferences carry `customDirs`.
pub const PREFERENCES_PROVIDER_ID: &str = "scripts";

/// The default script directories, `Omnicast::dataSearchPaths("scripts")`:
/// `vicinae/scripts` under the data home, then under each data directory.
#[must_use]
pub fn default_directories() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = compass_core::xdg_dirs::data_home() {
        dirs.push(home.join("vicinae/scripts"));
    }
    for dir in compass_core::xdg_dirs::data_dirs() {
        let path = dir.join("vicinae/scripts");
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    }
    dirs
}

/// The `customDirs` preference, a list of directories scanned before the
/// default ones.
#[must_use]
pub fn custom_directories(
    preferences: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<PathBuf> {
    preferences
        .and_then(|preferences| preferences.get("customDirs"))
        .and_then(serde_json::Value::as_array)
        .map(|dirs| {
            dirs.iter()
                .filter_map(serde_json::Value::as_str)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// The scanned scripts and the record of inline runs.
#[derive(Debug, Default)]
pub struct Scripts {
    /// The directories scanned, custom ones first.
    pub directories: Vec<PathBuf>,
    /// The scripts the last scan found.
    pub files: Vec<ScriptCommandFile>,
    metadata: ScriptMetadataStore,
    metadata_path: Option<PathBuf>,
}

impl Scripts {
    /// Scripts over `directories`, with inline output kept at
    /// `metadata_path`.
    #[must_use]
    pub fn new(directories: Vec<PathBuf>, metadata_path: Option<PathBuf>) -> Self {
        let metadata = metadata_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| ScriptMetadataStore::from_json(&text))
            .unwrap_or_default();
        Self {
            directories,
            files: Vec::new(),
            metadata,
            metadata_path,
        }
    }

    /// Scans the directories again.
    pub fn rescan(&mut self) {
        self.files = compass_core::script_scan::scan(&self.directories);
    }

    /// The items root search lists.
    #[must_use]
    pub fn items(&self) -> Vec<ScriptItem> {
        self.files
            .iter()
            .map(|file| ScriptItem::new(file, self.metadata.last_run_data(&file.id).as_deref()))
            .collect()
    }

    /// The script with this id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&ScriptCommandFile> {
        self.files.iter().find(|file| file.id == id)
    }

    /// Records an inline script's newest line, as `saveRun` does.
    pub fn save_run(&mut self, id: &str, line: &str) {
        let now = crate::shortcuts::now();
        self.metadata
            .save_run(id, line, i64::try_from(now).unwrap_or_default());
        if let Some(path) = &self.metadata_path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::write(path, self.metadata.to_json()) {
                tracing::warn!(%error, "could not save an inline script's output");
            }
        }
    }
}

/// A script as the wire carries it.
#[must_use]
pub fn entry(item: &ScriptItem) -> ScriptEntry {
    ScriptEntry {
        id: item.id.clone(),
        title: item.title.clone(),
        subtitle: item.subtitle.clone(),
        keywords: item.keywords.clone(),
        mode: item.mode.as_str().to_owned(),
        needs_confirmation: item.needs_confirmation,
        path: item.path.clone(),
        arguments: item
            .arguments
            .iter()
            .map(|argument| ScriptArgumentEntry {
                kind: match argument.argument_type {
                    ArgumentType::Text => "text",
                    ArgumentType::Password => "password",
                    ArgumentType::Dropdown => "dropdown",
                }
                .to_owned(),
                placeholder: argument.placeholder.clone(),
                optional: argument.optional,
                options: argument
                    .data
                    .iter()
                    .map(|option| (option.title.clone(), option.value.clone()))
                    .collect(),
            })
            .collect(),
    }
}

/// Where `name` is on `$PATH`, as `QStandardPaths::findExecutable` finds it.
fn find_executable(name: &str) -> Option<String> {
    if name.contains('/') {
        return Path::new(name).is_file().then(|| name.to_owned());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .map(|found| found.to_string_lossy().into_owned())
}

/// The argv that runs `script` with `arguments`, as `createCommandLine`
/// builds it.
///
/// # Errors
///
/// The interpreter the script names cannot be found.
pub fn command_line(
    script: &ScriptCommandFile,
    arguments: &[String],
) -> Result<Vec<String>, String> {
    let interpreter = if script.data.exec.is_empty() {
        compass_core::script_command::resolve_interpreter(
            &script.data,
            &script.path,
            &|path| Path::new(path).is_file(),
            &find_executable,
        )?
    } else {
        Vec::new()
    };
    Ok(script.command_line(&interpreter, arguments))
}

/// The directory a script runs in: its `currentDirectoryPath`, else its own.
#[must_use]
pub fn working_directory(script: &ScriptCommandFile) -> PathBuf {
    script
        .data
        .current_directory_path
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| script.path.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// One run the launcher follows.
#[derive(Debug, Default)]
pub struct Run {
    /// What it printed so far.
    pub output: Vec<u8>,
    /// Whether it has ended.
    pub finished: bool,
    /// Its exit code, when it exited normally.
    pub exit_code: Option<i32>,
    started: Option<Instant>,
    ended: Option<Instant>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Run {
    /// How long it has run, or ran.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        match (self.started, self.ended) {
            (Some(started), Some(ended)) => ended.duration_since(started),
            (Some(started), None) => started.elapsed(),
            _ => Duration::ZERO,
        }
    }

    /// The first line of what it printed, as the one-line modes read it.
    #[must_use]
    pub fn first_line(&self) -> String {
        String::from_utf8_lossy(&self.output)
            .split('\n')
            .next()
            .unwrap_or_default()
            .to_owned()
    }
}

/// A run that started: its session, its state, and the task that ends it.
pub type Started = (u64, Arc<std::sync::Mutex<Run>>, tokio::task::JoinHandle<()>);

/// The runs in progress or recently ended, by session.
#[derive(Debug, Default)]
pub struct Runs {
    next: std::sync::atomic::AtomicU64,
    runs: std::sync::Mutex<HashMap<u64, Arc<std::sync::Mutex<Run>>>>,
}

impl Runs {
    /// A run by session.
    #[must_use]
    pub fn get(&self, session: u64) -> Option<Arc<std::sync::Mutex<Run>>> {
        self.runs.lock().ok()?.get(&session).cloned()
    }

    /// Stops a run, if it is still going.
    pub fn stop(&self, session: u64) {
        if let Some(run) = self.get(session)
            && let Ok(mut run) = run.lock()
            && let Some(stop) = run.stop.take()
        {
            let _ = stop.send(());
        }
    }

    /// Starts `argv` in `cwd` and returns its session and a handle to its
    /// run. `combined` interleaves stderr with stdout and sets
    /// `FORCE_COLOR=1`, as the full-output view does; `timeout` kills it.
    ///
    /// # Errors
    ///
    /// The process did not start.
    pub fn start(
        &self,
        argv: &[String],
        cwd: &Path,
        combined: bool,
        timeout: Option<Duration>,
    ) -> Result<Started, String> {
        let (program, args) = argv.split_first().ok_or("the script has no command line")?;
        let mut command =
            tokio::process::Command::from(compass_platform_linux::host_command(program));
        command
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(if combined {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .kill_on_drop(true);
        if combined {
            command.env("FORCE_COLOR", "1");
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("Script execution failed: {error}"))?;
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let run = Arc::new(std::sync::Mutex::new(Run {
            started: Some(Instant::now()),
            stop: Some(stop),
            ..Run::default()
        }));
        let session = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if let Ok(mut runs) = self.runs.lock() {
            // Ended runs are only kept until the next one starts: a view
            // follows one run at a time.
            runs.retain(|_, run| run.lock().is_ok_and(|run| !run.finished));
            runs.insert(session, Arc::clone(&run));
        }
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let followed = Arc::clone(&run);
        let task = tokio::spawn(async move {
            let pump = |pipe: Option<tokio::process::ChildStdout>| {
                let run = Arc::clone(&followed);
                async move {
                    let Some(mut pipe) = pipe else { return };
                    let mut buffer = [0u8; 4096];
                    while let Ok(read) = pipe.read(&mut buffer).await {
                        if read == 0 {
                            break;
                        }
                        if let Ok(mut run) = run.lock() {
                            run.output.extend_from_slice(&buffer[..read]);
                        }
                    }
                }
            };
            let pump_err = |pipe: Option<tokio::process::ChildStderr>| {
                let run = Arc::clone(&followed);
                async move {
                    let Some(mut pipe) = pipe else { return };
                    let mut buffer = [0u8; 4096];
                    while let Ok(read) = pipe.read(&mut buffer).await {
                        if read == 0 {
                            break;
                        }
                        if let Ok(mut run) = run.lock() {
                            run.output.extend_from_slice(&buffer[..read]);
                        }
                    }
                }
            };
            let waited = async {
                let (_, _, status) = tokio::join!(pump(stdout), pump_err(stderr), child.wait());
                status.ok().and_then(|status| status.code())
            };
            let limit = async {
                match timeout {
                    Some(timeout) => tokio::time::sleep(timeout).await,
                    None => std::future::pending().await,
                }
            };
            let exit_code = tokio::select! {
                code = waited => code,
                _ = stopped => None,
                () = limit => None,
            };
            if let Ok(mut run) = followed.lock() {
                run.finished = true;
                run.exit_code = exit_code;
                run.ended = Some(Instant::now());
                run.stop = None;
            }
        });
        Ok((session, run, task))
    }
}

/// What a one-line run's result says, as the modes phrase it: the line, or
/// the mode's own sentence when the script printed nothing.
#[must_use]
pub fn one_line_message(mode: OutputMode, ok: bool, line: &str) -> String {
    if !line.is_empty() {
        return line.to_owned();
    }
    match (mode, ok) {
        (OutputMode::Silent, true) => "Script executed",
        (OutputMode::Silent, false) => "Failed to execute script",
        (OutputMode::Inline, false) => "Script exited with error code",
        (_, true) => "Script executed",
        (_, false) => "Script execution failed",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn custom_directories_come_from_the_preference() {
        let prefs = serde_json::json!({"customDirs": ["/a", 3, "/b"]});
        assert_eq!(
            custom_directories(prefs.as_object()),
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert!(custom_directories(None).is_empty());
    }

    #[tokio::test]
    async fn a_run_collects_output_and_its_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            "hello.sh",
            "#!/bin/sh\n# @raycast.schemaVersion 1\n# @raycast.title Hello\n\
             # @raycast.mode fullOutput\n# @raycast.argument1 {\"type\":\"text\",\"placeholder\":\"who\"}\n\
             echo \"hello $1 $FORCE_COLOR\"; pwd; echo oops >&2; exit 3\n",
        );
        let mut scripts = Scripts::new(vec![dir.path().to_owned()], None);
        scripts.rescan();
        let script = scripts.find("hello.sh").expect("scanned");
        let argv = command_line(script, &["world".to_owned()]).unwrap();
        assert!(argv.last().is_some_and(|a| a == "world"), "{argv:?}");
        let runs = Runs::default();
        let (session, run, task) = runs
            .start(&argv, &working_directory(script), true, None)
            .unwrap();
        task.await.unwrap();
        let run = run.lock().unwrap();
        assert!(run.finished);
        assert_eq!(run.exit_code, Some(3));
        let text = String::from_utf8_lossy(&run.output);
        assert!(text.contains("hello world 1"), "{text}");
        assert!(
            text.contains(&dir.path().to_string_lossy().into_owned()),
            "{text}"
        );
        assert!(
            text.contains("oops"),
            "stderr is shown in full output: {text}"
        );
        assert!(runs.get(session).is_some());
    }

    #[tokio::test]
    async fn a_one_line_run_times_out_and_a_stop_ends_a_run() {
        let runs = Runs::default();
        let argv = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "echo first; sleep 30".to_owned(),
        ];
        let started = Instant::now();
        let (_, run, task) = runs
            .start(
                &argv,
                Path::new("/"),
                false,
                Some(Duration::from_millis(300)),
            )
            .unwrap();
        task.await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(run.lock().unwrap().first_line(), "first");
        assert_eq!(run.lock().unwrap().exit_code, None);

        let (session, run, task) = runs.start(&argv, Path::new("/"), false, None).unwrap();
        runs.stop(session);
        task.await.unwrap();
        assert!(run.lock().unwrap().finished);
    }

    #[test]
    fn the_one_line_sentences_are_the_modes() {
        assert_eq!(
            one_line_message(OutputMode::Silent, true, ""),
            "Script executed"
        );
        assert_eq!(
            one_line_message(OutputMode::Inline, false, ""),
            "Script exited with error code"
        );
        assert_eq!(
            one_line_message(OutputMode::Compact, false, ""),
            "Script execution failed"
        );
        assert_eq!(one_line_message(OutputMode::Compact, true, "42"), "42");
    }
}
