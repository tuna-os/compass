//! Snippets: the engine's store, and expanding one for copying or pasting.
//!
//! The store is `compass_core::snippet_store` and the expansion rules are
//! `compass_core::snippet_expander`; this is the machine's side of them — the
//! clock for `{date}`, the clipboard for `{clipboard}`, and the processes a
//! `{shell}` placeholder runs.
//!
//! Compass keeps its own `compass-snippets.json` (ADR-0017 decision 3), and
//! the first start without one copies Vicinae's `snippets/snippets.json`,
//! which has the same shape.
//!
//! Keyword expansion as you type is not here: it needs the input server's
//! keyboard hook, which is not ported (PARITY, Snippets).

use std::path::Path;
use std::time::Duration;

use compass_core::snippet_expander::{self, ExpansionContext, ShellCommand};
use compass_core::snippet_store::{SerializedSnippet, SnippetData, SnippetStore};
use compass_ipc::SnippetEntry;

/// Compass's snippet file, in its data directory.
pub const FILE_NAME: &str = "compass-snippets.json";

/// Vicinae's snippet file, relative to the shared data directory.
pub const VICINAE_FILE: &str = "snippets/snippets.json";

/// Opens the store in `dir`, bringing Vicinae's snippets across first when
/// Compass has none of its own yet.
#[must_use]
pub fn open(dir: &Path) -> SnippetStore {
    let path = dir.join(FILE_NAME);
    import_from_vicinae(&path, &dir.join(VICINAE_FILE));
    SnippetStore::open(&path)
}

/// Copies Vicinae's snippet file to `compass` when `compass` does not exist
/// and `vicinae` holds a list that parses. Returns whether it did.
pub fn import_from_vicinae(compass: &Path, vicinae: &Path) -> bool {
    if compass.exists() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(vicinae) else {
        return false;
    };
    if serde_json::from_str::<Vec<SerializedSnippet>>(&text).is_err() {
        tracing::warn!(path = %vicinae.display(), "Vicinae's snippets do not parse; not importing them");
        return false;
    }
    if let Some(parent) = compass.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    match std::fs::write(compass, text) {
        Ok(()) => {
            tracing::info!(from = %vicinae.display(), "imported Vicinae's snippets");
            true
        }
        Err(error) => {
            tracing::warn!(%error, "could not import Vicinae's snippets");
            false
        }
    }
}

/// A snippet as the wire carries it.
#[must_use]
pub fn entry(snippet: &SerializedSnippet) -> SnippetEntry {
    let (text, file) = match &snippet.data {
        SnippetData::Text { text } => (Some(text.clone()), None),
        SnippetData::File { file } => (None, Some(file.clone())),
    };
    SnippetEntry {
        id: snippet.id.clone(),
        name: snippet.name.clone(),
        text,
        file,
        created_at: snippet.created_at,
        updated_at: snippet.updated_at,
        keyword: snippet.keyword().map(str::to_owned),
        word: snippet.expansion.as_ref().is_some_and(|e| e.word),
        apps: snippet
            .expansion
            .as_ref()
            .map(|e| e.apps.clone())
            .unwrap_or_default(),
    }
}

/// The machine's answers for one expansion, gathered before it runs: the
/// expander is synchronous, and reading the clipboard and running commands
/// are not.
#[derive(Debug, Default)]
pub struct Context {
    /// The clipboard's text, or empty.
    pub clipboard: String,
    /// One UUID for the whole expansion, as the C++ expander keeps one.
    pub uuid: String,
    /// The local time now.
    pub now: Option<compass_core::qt_date::DateTime>,
    /// Each `{shell}` placeholder's output, in order.
    pub shell: Vec<String>,
}

impl ExpansionContext for Context {
    fn clipboard_text(&self) -> String {
        self.clipboard.clone()
    }

    fn uuid(&self) -> String {
        self.uuid.clone()
    }

    fn formatted_date(&self, format: &str) -> String {
        self.now
            .as_ref()
            .map(|now| compass_core::qt_date::format(format, now))
            .unwrap_or_default()
    }

    fn run_shell_commands(&self, _commands: &[ShellCommand]) -> Vec<String> {
        self.shell.clone()
    }
}

/// The local time, broken down for a Qt date format.
#[must_use]
pub fn local_now() -> compass_core::qt_date::DateTime {
    let now = jiff::Zoned::now();
    let small = |value: i8| u8::try_from(value).unwrap_or_default();
    compass_core::qt_date::DateTime {
        year: i32::from(now.year()),
        month: small(now.month()),
        day: small(now.day()),
        hour: small(now.hour()),
        minute: small(now.minute()),
        second: small(now.second()),
        millisecond: u16::try_from(now.millisecond()).unwrap_or_default(),
        weekday: small(now.weekday().to_monday_one_offset()),
        zone: now.strftime("%Z").to_string(),
    }
}

/// How long a `{shell}` placeholder may run: `SHELL_TIMEOUT_MS`.
pub const SHELL_TIMEOUT: Duration = Duration::from_millis(snippet_expander::SHELL_TIMEOUT_MS);

/// Runs one `{shell}` placeholder as `executeShellPlaceholdersSync` does:
/// `exec -c code`, or the login shell (`$SHELL`, else `/bin/sh`) when no
/// `exec=` is given, on the host. Its trimmed output when it exits 0 within
/// [`SHELL_TIMEOUT`], nothing otherwise.
pub async fn run_shell(command: &ShellCommand) -> String {
    let program = if command.exec.is_empty() {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())
    } else {
        command.exec.clone()
    };
    let mut process = tokio::process::Command::from(compass_platform_linux::host_command(&program));
    process
        .args(["-c", &command.code])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(SHELL_TIMEOUT, process.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
        Ok(Ok(output)) => {
            tracing::warn!(code = %command.code, status = %output.status, "a snippet's shell placeholder failed");
            String::new()
        }
        Ok(Err(error)) => {
            tracing::warn!(code = %command.code, %error, "a snippet's shell placeholder did not start");
            String::new()
        }
        Err(_) => {
            tracing::warn!(code = %command.code, "a snippet's shell placeholder timed out");
            String::new()
        }
    }
}

/// Expands `text` with `arguments`: its shell placeholders run concurrently,
/// as the C++ starts them together, and `clipboard` stands for
/// `{clipboard}`.
pub async fn expand(
    text: &str,
    arguments: &[(String, String)],
    clipboard: Option<String>,
) -> snippet_expander::Expansion {
    let parts = compass_core::shortcut::parse_link(text).parts;
    let commands = snippet_expander::shell_commands(&parts);
    let shell = futures_util::future::join_all(commands.iter().map(run_shell)).await;
    let context = Context {
        clipboard: clipboard.unwrap_or_default(),
        uuid: uuid::Uuid::new_v4().hyphenated().to_string(),
        now: Some(local_now()),
        shell,
    };
    snippet_expander::expand(
        &parts,
        arguments,
        snippet_expander::Options::default(),
        &context,
    )
}

/// Whether expanding `text` reads the clipboard.
#[must_use]
pub fn needs_clipboard(text: &str) -> bool {
    compass_core::shortcut::parse_link(text)
        .placeholders
        .iter()
        .any(|placeholder| placeholder.id == snippet_expander::CLIPBOARD_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vicinae_snippets_come_across_once() {
        let dir = tempfile::tempdir().unwrap();
        let vicinae = dir.path().join(VICINAE_FILE);
        std::fs::create_dir_all(vicinae.parent().unwrap()).unwrap();
        std::fs::write(
            &vicinae,
            r#"[{"id":"snp-aaaaaaaaaaaa","name":"Sig","data":{"text":"Best"},"createdAt":1,
                "expansion":{"keyword":";sig","apps":[],"word":true}}]"#,
        )
        .unwrap();
        let store = open(dir.path());
        assert_eq!(store.snippets().len(), 1);
        assert_eq!(entry(&store.snippets()[0]).keyword.as_deref(), Some(";sig"));
        assert!(!import_from_vicinae(&dir.path().join(FILE_NAME), &vicinae));
    }

    #[tokio::test]
    async fn shell_placeholders_run_and_the_rest_expand() {
        let expanded = expand(
            "{shell code=\"echo hi\"} {name} {shell exec=\"/bin/sh\" code=\"exit 3\"}|{clipboard}|{date format=\"yyyy\"}",
            &[("name".to_owned(), "Zoë".to_owned())],
            Some("clip".to_owned()),
        )
        .await;
        let text = expanded.to_text();
        let year = local_now().year.to_string();
        assert_eq!(text, format!("hi Zoë |clip|{year}"));
    }

    #[tokio::test]
    async fn a_slow_shell_placeholder_is_cut_off() {
        let started = std::time::Instant::now();
        let out = run_shell(&ShellCommand {
            code: "sleep 5; echo late".into(),
            exec: "/bin/sh".into(),
        })
        .await;
        assert_eq!(out, "");
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn the_clock_is_local_and_sane() {
        let now = local_now();
        assert!((1..=12).contains(&now.month));
        assert!((1..=7).contains(&now.weekday));
        assert!(now.year >= 2024);
    }
}
