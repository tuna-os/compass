//! Suite 0 Parity Harness — Cross-implementation parity tests.
//!
//! This binary runs the same operations against both the C++ and Rust engines
//! and compares their JSON output to ensure they produce identical results.
//!
//! See `docs/rust-engine/PLAN.md` §8.1 for the full specification.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use compass_testkit::corpus::desktop_entries;

#[derive(Debug, Serialize, Deserialize)]
struct ParityConfig {
    /// Path to the C++ vicinae binary.
    cpp_engine: PathBuf,
    /// Path to the Rust vicinae binary.
    rust_engine: PathBuf,
    /// Corpus directory.
    corpus_dir: PathBuf,
    /// Output directory for diffs.
    output_dir: Option<PathBuf>,
    /// Drive the Rust binary on BOTH sides, to check the harness itself.
    ///
    /// See [`main`]'s `--selftest`.
    selftest: bool,
    /// Report regressions without failing on them.
    ///
    /// See [`main`]'s `--report-only`.
    report_only: bool,
}

/// One ranked hit, as an engine actually emits it.
///
/// This mirrors `compass_ipc::protocol::QueryHit` field for field, and did not
/// before: it was `{key, name, score, quality}` against an engine that emits
/// `{id, title, subtitle, score}`. Every name differed but `score`, and
/// `quality` does not exist at all, so `serde_json::from_str` would have failed
/// on every query — the fourth thing wrong with a harness written to §8.1's
/// prose and never run against a binary.
///
/// Deliberately NOT `compass_ipc::QueryHit` itself. Suite 0's whole job is to
/// notice when the two engines disagree, and sharing the Rust engine's own type
/// would make the harness track it silently through a rename. A separate
/// definition means a shape change shows up here as a parse failure, which is
/// the outcome that gets looked at.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct SearchResultItem {
    id: String,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    /// The engine's own score, and **not comparable across engines**.
    ///
    /// `f64` rather than `u32` because the C++ `rootQuery` emits a double: its
    /// `SearchableRootItem::fuzzyScore` returns `score.score +
    /// FRECENCY_WEIGHT * frecency()`, the value it *orders by*, while the Rust
    /// engine puts the 0..=100 match score on the wire. Parsing it as an
    /// integer would fail on every C++ hit with a fractional part, which is
    /// most of them.
    ///
    /// See [`compare_results`] for what this is and is not used for.
    score: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ParityReport {
    total: usize,
    identical: usize,
    known_divergence: usize,
    regression: usize,
    details: Vec<ParityDetail>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ParityDetail {
    query: String,
    status: ParityStatus,
    cpp_results: Vec<SearchResultItem>,
    rust_results: Vec<SearchResultItem>,
    divergence_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
enum ParityStatus {
    Identical,
    KnownDivergence,
    Regression,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut config = ParityConfig {
        cpp_engine: PathBuf::from("./build/bin/vicinae"),
        rust_engine: PathBuf::from("./target/release/vicinae"),
        corpus_dir: PathBuf::from("crates/compass-testkit/corpus/desktop-entries"),
        output_dir: None,
        selftest: false,
        report_only: false,
    };

    // Parse command line args
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--cpp" => {
                i += 1;
                config.cpp_engine = PathBuf::from(&args[i]);
            }
            "--rust" => {
                i += 1;
                config.rust_engine = PathBuf::from(&args[i]);
            }
            "--corpus" => {
                i += 1;
                config.corpus_dir = PathBuf::from(&args[i]);
            }
            "--output" => {
                i += 1;
                config.output_dir = Some(PathBuf::from(&args[i]));
            }
            // A HARNESS THAT HAS NEVER RUN PROVES NOTHING, and this one had
            // not: running it for the first time found a panic on the first
            // accented name in the corpus, before a single query was
            // compared. `--selftest` drives the Rust binary on both sides so
            // the pipeline -- stage, serve, poll, query, parse, compare -- is
            // exercised wherever the C++ engine is not available, which is
            // everywhere except the Bluefin container.
            //
            // It does NOT claim parity: an engine agrees with itself by
            // construction. It claims the harness works, and it refuses a run
            // in which nothing ranked, which is the only way self-parity can
            // pass while measuring nothing.
            "--selftest" => {
                config.selftest = true;
            }
            // RECORDED BEFORE GATED, per ADR-0010.
            //
            // The first differential between two independently written
            // engines will disagree somewhere, and nobody knows where yet. A
            // job that reddens on that first run teaches nothing and gets
            // muted; a job that PRINTS where they disagree turns an unknown
            // into a list.
            //
            // This suppresses only the exit code for ranking differences. A
            // harness that could not run, an engine that would not start, and
            // a run in which nothing ranked all still fail: those are not
            // findings about the engines, they are the harness measuring
            // nothing.
            "--report-only" => {
                config.report_only = true;
            }
            _ => {}
        }
        i += 1;
    }

    println!("Suite 0 Parity Harness");
    println!("  C++ engine:  {}", config.cpp_engine.display());
    println!("  Rust engine: {}", config.rust_engine.display());
    println!("  Corpus:      {}", config.corpus_dir.display());

    let report = run_parity(&config)?;

    println!("\nResults:");
    println!("  Total queries:     {}", report.total);
    println!("  Identical:         {}", report.identical);
    println!("  Known divergence:  {}", report.known_divergence);
    println!("  Regressions:       {}", report.regression);

    // BEFORE ANY EXIT. The report was written last, after the `exit(1)` on
    // regressions, so the run that most needed its diff on disk was the one
    // run that never wrote one.
    if let Some(output_dir) = &config.output_dir {
        std::fs::create_dir_all(output_dir)?;
        let report_path = output_dir.join("parity-report.json");
        std::fs::write(report_path, serde_json::to_string_pretty(&report)?)?;
        println!("  Report written to: {}", output_dir.display());
    }

    // SELF-PARITY OVER EMPTY RANKINGS IS THE FAILURE MODE THIS WHOLE SUITE
    // EXISTS TO CATCH. An engine compared with itself agrees on every query,
    // including every query that returned nothing, so "0 regressions" is
    // worthless unless something was actually ranked.
    if config.selftest {
        let ranked = report
            .details
            .iter()
            .filter(|detail| !detail.cpp_results.is_empty())
            .count();
        println!("  Queries that ranked:  {ranked}");
        if ranked == 0 {
            eprintln!(
                "\n❌ SELFTEST RANKED NOTHING across {} queries — the engine served but indexed \
                 no corpus, so this run compared nothing and agreed perfectly.",
                report.total
            );
            std::process::exit(1);
        }
    }

    if report.regression > 0 {
        let banner = if config.report_only {
            "⚠️  RANKING DIFFERENCES (recorded, not gated)"
        } else {
            "❌ REGRESSIONS DETECTED!"
        };
        eprintln!("\n{banner}");

        // Bounded: a first differential can disagree on hundreds of queries,
        // and a log nobody scrolls to the end of is a log nobody reads.
        const SHOWN: usize = 25;
        let differing = report
            .details
            .iter()
            .filter(|detail| detail.status == ParityStatus::Regression);
        for detail in differing.clone().take(SHOWN) {
            eprintln!("  Query: {}", detail.query);
            eprintln!("    C++:  {:?}", detail.cpp_results);
            eprintln!("    Rust: {:?}", detail.rust_results);
        }
        let total = differing.count();
        if total > SHOWN {
            eprintln!(
                "  … and {} more; the full list is in the report",
                total - SHOWN
            );
        }

        if !config.report_only {
            std::process::exit(1);
        }
    } else {
        println!("\n✅ All checks passed");
    }

    Ok(())
}

fn run_parity(config: &ParityConfig) -> Result<ParityReport> {
    // Generate test queries from corpus
    let queries = generate_test_queries(&config.corpus_dir)?;

    let mut report = ParityReport {
        total: 0,
        identical: 0,
        known_divergence: 0,
        regression: 0,
        details: Vec::new(),
    };

    // One engine each, started once and reused for every query. Starting them
    // per query would make the run time dominated by index builds and would say
    // nothing extra: Suite 0 compares ranking, not startup.
    // Both engines index THIS, and nothing the runner happens to have.
    let corpus = StagedCorpus::stage(&config.corpus_dir)?;
    println!(
        "  Staged {} entries at {}",
        corpus.entries,
        corpus.root.display()
    );

    let left_flavour = if config.selftest {
        Flavour::Rust
    } else {
        Flavour::Cpp
    };
    let cpp = RunningEngine::start(&config.cpp_engine, "cpp", left_flavour, &corpus)?;
    let rust = RunningEngine::start(&config.rust_engine, "rust", Flavour::Rust, &corpus)?;

    for query in &queries {
        report.total += 1;

        let cpp_results = run_search(&cpp, "cpp", query)?;
        let rust_results = run_search(&rust, "rust", query)?;

        let (status, reason) = compare_results(&cpp_results, &rust_results);

        match status {
            ParityStatus::Identical => report.identical += 1,
            ParityStatus::KnownDivergence => report.known_divergence += 1,
            ParityStatus::Regression => report.regression += 1,
        }

        report.details.push(ParityDetail {
            query: query.clone(),
            status,
            cpp_results,
            rust_results,
            divergence_reason: reason,
        });
    }

    Ok(report)
}

fn generate_test_queries(_corpus_dir: &Path) -> Result<Vec<String>> {
    let mut queries = Vec::new();

    // Add queries from the corpus
    for entry in desktop_entries() {
        if let Some(text) = entry.as_str() {
            // Extract potential search terms from the desktop entry
            for line in text.lines() {
                if let Some(name) = line.strip_prefix("Name=") {
                    let name = name.trim();
                    if name.chars().count() > 2 {
                        queries.push(name.to_owned());

                        // Prefixes by CHARACTER, not by byte. `name[..i]`
                        // panics the moment a corpus entry is not ASCII --
                        // "byte index 2 is not a char boundary; it is inside
                        // 'é' of `Déjà Dup Backups`" -- and the checked-in
                        // corpus has such entries, so the harness aborted
                        // before it compared anything at all. Found by
                        // running it, which nothing had done.
                        for (count, (offset, _)) in name.char_indices().enumerate().skip(1) {
                            if count > 4 {
                                break;
                            }
                            queries.push(name[..offset].to_owned());
                        }
                    }
                }
            }
        }
    }

    // Add some common search terms
    queries.extend([
        "firefox".to_owned(),
        "chrome".to_owned(),
        "terminal".to_owned(),
        "editor".to_owned(),
        "browser".to_owned(),
        "calc".to_owned(),
        "file".to_owned(),
        "settings".to_owned(),
        "system".to_owned(),
        "app".to_owned(),
    ]);

    // Deduplicate and sort
    queries.sort();
    queries.dedup();

    Ok(queries)
}

/// The corpus, laid out where an XDG engine will actually look for it.
///
/// # Why this exists
///
/// Without it both engines index THE HOST'S applications, and the comparison
/// is over whatever happens to be installed on the runner. That is not a
/// corpus: it is not checked in, it differs between machines, and it makes a
/// green run mean nothing in particular.
///
/// The Suite 0 spike settled the mechanics against a real C++ engine
/// (`.github/workflows/cpp-on-target.yaml`): `XDG_DATA_DIRS=<root>` with the
/// entries at `<root>/applications` is what both sides read —
/// `xdgpp::appDirs()` is `dataHome()/applications` plus each
/// `dataDirs()/applications`, and `compass_xdg` follows the same spec outside
/// a Flatpak sandbox.
///
/// `XDG_DATA_HOME` and `HOME` are pointed at empty directories for the same
/// reason: left alone, the runner's own `~/.local/share/applications` joins
/// the index on one side of a comparison that is supposed to be about the
/// engines.
#[derive(Debug)]
struct StagedCorpus {
    /// Set as `XDG_DATA_DIRS`. Entries live in `<root>/applications`.
    root: PathBuf,
    /// Set as `XDG_DATA_HOME`, empty.
    data_home: PathBuf,
    /// Set as `HOME`, empty.
    home: PathBuf,
    /// How many `.desktop` files were staged.
    entries: usize,
}

impl StagedCorpus {
    /// Copies every `.desktop` file under `corpus_dir` into a private tree.
    ///
    /// # Errors
    ///
    /// When the corpus yields no entries. AN EMPTY CORPUS IS NOT A PASS: two
    /// engines that rank nothing agree on everything, and the run would report
    /// perfect parity having compared nothing. The Suite 0 spike spent five
    /// commits diagnosing an empty ranking that turned out to be a query term
    /// absent from the corpus, which is the same mistake read from the other
    /// end.
    fn stage(corpus_dir: &Path) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!(
            "compass-parity-corpus-{}-{nanos}",
            std::process::id()
        ));

        let root = base.join("share");
        let applications = root.join("applications");
        let data_home = base.join("data-home");
        let home = base.join("home");
        for dir in [&applications, &data_home, &home] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }

        let mut entries = 0usize;
        let mut pending = vec![corpus_dir.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let listing = std::fs::read_dir(&dir)
                .with_context(|| format!("reading the corpus at {}", dir.display()))?;
            for entry in listing {
                let path = entry?.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "desktop") {
                    let Some(name) = path.file_name() else {
                        continue;
                    };
                    std::fs::copy(&path, applications.join(name))
                        .with_context(|| format!("staging {}", path.display()))?;
                    entries += 1;
                }
            }
        }

        anyhow::ensure!(
            entries > 0,
            "no .desktop entries under {} — an empty corpus makes both engines rank nothing and \
             agree perfectly, which is not a pass",
            corpus_dir.display()
        );

        Ok(Self {
            root,
            data_home,
            home,
            entries,
        })
    }

    /// The environment both engines are given, so neither can see the other's
    /// index or the runner's.
    fn env(&self) -> [(&'static str, PathBuf); 3] {
        [
            ("XDG_DATA_DIRS", self.root.clone()),
            ("XDG_DATA_HOME", self.data_home.clone()),
            ("HOME", self.home.clone()),
        ]
    }
}

impl Drop for StagedCorpus {
    fn drop(&mut self) {
        if let Some(base) = self.root.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

/// An engine process started for the duration of a parity run.
///
/// `query` asks a RUNNING engine over its IPC socket — it is not a one-shot
/// ranking command. The harness used to exec the binary once per query with
/// nothing listening, so every call returned
///
///   error: no Compass engine is listening on /tmp/vicinae-default/ipc.sock
///
/// and Suite 0 could never have produced a single comparison. Each engine now
/// gets its own `serve` on its own socket, so the two cannot reach each other's
/// index and a stale engine from a previous run cannot be mistaken for this
/// one's.
struct RunningEngine {
    child: Child,
    binary: PathBuf,
    socket: PathBuf,
    flavour: Flavour,
    /// The private `XDG_RUNTIME_DIR` a C++ engine was given, and `None` for a
    /// Rust one, which takes `--socket` instead.
    runtime_dir: Option<PathBuf>,
    /// The staged corpus environment, applied to the server AND to every
    /// client call: the C++ CLI resolves paths the same way its server does,
    /// and a client reading the runner's real `HOME` is a difference between
    /// the two sides that has nothing to do with either engine.
    corpus_env: Vec<(String, PathBuf)>,
}

/// Which engine a binary is, because the two are started, asked and stopped
/// differently and neither accepts the other's argv.
///
/// # How the C++ engine gets its own socket, without a C++ change
///
/// The Rust engine takes `--socket <path>`. The C++ engine has no such flag --
/// `vicinae::serverSocketName()` is `runtimeDir() / "vicinae.sock"`, and
/// `runtimeDir()` reads **`XDG_RUNTIME_DIR`**. So giving the C++ side a private
/// runtime directory gives it a private socket, and the CLI that talks to it
/// derives the same path from the same variable. Adding a `--socket` flag to a
/// tree we are deleting would have been the obvious move and was not necessary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavour {
    Cpp,
    Rust,
}

impl Flavour {
    /// Environment this flavour needs before it will start at all.
    ///
    /// The C++ engine is a Qt application and tries to open a display even
    /// when serving headlessly. Without this it aborts before binding its
    /// socket, which the Suite 0 spike measured directly:
    ///
    /// ```text
    /// === rung: bare ===
    /// FATAL - This application failed to start because no Qt platform
    ///         plugin could be initialized.
    /// === rung: offscreen ===
    /// ANSWERED after 400ms
    /// ```
    ///
    /// `offscreen` is enough: no compositor, no session bus, no
    /// `dbus-run-session`. The Rust engine needs nothing.
    fn env(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Cpp => &[("QT_QPA_PLATFORM", "offscreen")],
            Self::Rust => &[],
        }
    }

    /// The argv that starts a server.
    ///
    /// `serve` on the Rust side; `server` on the C++ side, which is a
    /// different word and was one of the reasons the old harness could not
    /// have started a C++ engine even with the socket sorted out.
    fn serve_args(self, socket: &Path) -> Vec<String> {
        match self {
            Self::Rust => vec![
                "--socket".to_owned(),
                socket.to_string_lossy().into_owned(),
                "serve".to_owned(),
            ],
            Self::Cpp => vec!["server".to_owned()],
        }
    }

    /// The argv that asks a running server to rank `query`.
    fn query_args(self, socket: &Path, query: &str) -> Vec<String> {
        let mut args = match self {
            Self::Rust => vec!["--socket".to_owned(), socket.to_string_lossy().into_owned()],
            Self::Cpp => Vec::new(),
        };
        args.push("query".to_owned());
        args.push("--json".to_owned());
        args.push(query.to_owned());
        args
    }

    /// The argv that asks whether a server is answering yet.
    fn ping_args(self, socket: &Path) -> Vec<String> {
        match self {
            Self::Rust => vec![
                "--socket".to_owned(),
                socket.to_string_lossy().into_owned(),
                "ping".to_owned(),
            ],
            Self::Cpp => vec!["ping".to_owned()],
        }
    }

    /// The argv that asks a server to stop, if it has one.
    ///
    /// The C++ CLI has no `shutdown` subcommand, so there is nothing to ask and
    /// the caller goes straight to killing the child. `vicinae server` `exec`s
    /// the server binary rather than forking it, so the PID we spawned *is* the
    /// server -- which is the opposite of the Flatpak case that caught out
    /// `prove-smoke.sh`, and worth stating because the two look alike.
    fn shutdown_args(self, socket: &Path) -> Option<Vec<String>> {
        match self {
            Self::Rust => Some(vec![
                "--socket".to_owned(),
                socket.to_string_lossy().into_owned(),
                "shutdown".to_owned(),
            ]),
            Self::Cpp => None,
        }
    }
}

impl RunningEngine {
    /// Start `<binary> --socket <socket> serve` and wait until it answers.
    ///
    /// READINESS IS A STATE, NOT A MOMENT. This polls `ping` until it succeeds
    /// rather than sleeping a plausible number of seconds or waiting for the
    /// socket file to appear: the file exists from the moment the listener
    /// binds, which is before the index is built, and a sleep is only ever
    /// correct on the machine it was tuned on. ADR-0010 records the VM tier
    /// learning the same lesson the expensive way — three runs screenshotted a
    /// launcher that had not painted yet because the check waited for the
    /// process instead of the renderer.
    fn start(binary: &Path, name: &str, flavour: Flavour, corpus: &StagedCorpus) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "compass-parity-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating the {name} engine's socket directory"))?;

        // The C++ engine derives its socket from XDG_RUNTIME_DIR rather than
        // taking a flag, so the private directory IS the isolation. See
        // [`Flavour`].
        let (socket, runtime_dir) = match flavour {
            Flavour::Rust => (dir.join("ipc.sock"), None),
            Flavour::Cpp => (dir.join("vicinae").join("vicinae.sock"), Some(dir.clone())),
        };

        let corpus_env: Vec<(String, PathBuf)> = corpus
            .env()
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();

        let mut command = Command::new(binary);
        command
            .args(flavour.serve_args(&socket))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        if let Some(runtime) = &runtime_dir {
            command.env("XDG_RUNTIME_DIR", runtime);
        }
        for (key, value) in &corpus_env {
            command.env(key, value);
        }
        for (key, value) in flavour.env() {
            command.env(key, value);
        }

        let child = command
            .spawn()
            .with_context(|| format!("spawning the {name} engine at {}", binary.display()))?;

        let engine = Self {
            child,
            binary: binary.to_path_buf(),
            socket,
            flavour,
            runtime_dir,
            corpus_env,
        };
        engine.wait_until_answering(name)?;
        Ok(engine)
    }

    /// A command against this engine, with its environment already applied.
    fn command(&self, args: Vec<String>) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(args);
        if let Some(runtime) = &self.runtime_dir {
            command.env("XDG_RUNTIME_DIR", runtime);
        }
        for (key, value) in &self.corpus_env {
            command.env(key, value);
        }
        for (key, value) in self.flavour.env() {
            command.env(key, value);
        }
        command
    }

    fn wait_until_answering(&self, name: &str) -> Result<()> {
        const LIMIT: Duration = Duration::from_secs(30);
        let started = Instant::now();

        while started.elapsed() < LIMIT {
            let ping = self.command(self.flavour.ping_args(&self.socket)).output();

            if matches!(&ping, Ok(out) if out.status.success()) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        anyhow::bail!(
            "the {name} engine did not answer `ping` on {} within {LIMIT:?} — it may have failed to \
             start, or it may be indexing something enormous",
            self.socket.display()
        )
    }

    fn shutdown(&mut self) {
        // Ask first: a clean shutdown lets the engine release its socket, and
        // an engine that ignores this is itself worth knowing about. Kill only
        // if asking did not work.
        if let Some(args) = self.flavour.shutdown_args(&self.socket) {
            let _ = self.command(args).output();
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();

        // The Rust engine's socket sits directly in the private directory;
        // the C++ engine's sits one level down, in `<dir>/vicinae/`. Remove
        // the directory we made, not whichever one happens to be the parent.
        if let Some(dir) = self
            .runtime_dir
            .clone()
            .or_else(|| self.socket.parent().map(Path::to_path_buf))
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Drop for RunningEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Invoke one engine for one query.
///
/// THE ARGV HERE IS LOAD-BEARING, and it was wrong in three separate ways until
/// each was measured against a real binary rather than read off §8.1's prose.
///
/// First, `--json` came before the subcommand, which clap rejects outright:
///
/// ```text
/// error: unexpected argument '--json' found
///   tip: 'query --json' exists
/// ```
///
/// `json` is a flag on `query`, not a global.
///
/// Second, there was no running engine. `query` asks one over its IPC socket,
/// so every call returned "no Compass engine is listening". `RunningEngine`
/// above now starts one per side.
///
/// Third, it passed `--engine <name>`, and that is wrong in principle rather
/// than in spelling. Until the Phase 7 cutover (§5) THE BINARY IS THE ENGINE:
/// the Rust binary refuses `--engine cpp` by design —
///
/// ```text
/// --engine cpp was selected, but this binary is the Rust engine and
/// cannot dispatch to the C++ one
/// ```
///
/// — and the C++ binary has no such flag at all, so passing it can only turn a
/// working invocation into a failing one. Suite 0 selects an engine by choosing
/// which path to exec, which is what `--cpp` and `--rust` are for.
///
/// `crates/vicinae/src/cli.rs` pins this argv, because an argv assembled in one
/// crate and parsed in another has no compiler between the two.
fn run_search(
    engine: &RunningEngine,
    engine_name: &str,
    query: &str,
) -> Result<Vec<SearchResultItem>> {
    let output = engine
        .command(engine.flavour.query_args(&engine.socket, query))
        .output()
        .with_context(|| format!("Failed to run {} engine", engine_name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{} engine failed: {}", engine_name, stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let results: Vec<SearchResultItem> = serde_json::from_str(&stdout)
        .with_context(|| format!("Failed to parse {} engine output", engine_name))?;

    Ok(results)
}

/// Compare two rankings.
///
/// # The verdict is the ORDER OF IDS, and the score is diagnosis
///
/// This used to compare whole structs, `score` included. Against two Rust
/// engines that was right and told us nothing; against the real C++ engine it
/// would report a regression on **every query with a match**, because the two
/// numbers are not on one scale and never were:
///
/// * Rust puts `QueryHit.score` on the wire -- the 0..=100 match score, with
///   the frecency boost deliberately excluded.
/// * C++ `rootQuery` emits the value `RootItemManager::search` *orders by*,
///   which is `score.score + FRECENCY_WEIGHT * frecency()`.
///
/// Reproducing either number on the other side means duplicating the other
/// engine's weighting at a second site, which manufactures divergences instead
/// of removing one. And the gate Phase 1 actually names is **ranking**, not
/// scoring -- §8.1a already measures scores differing on 20.8% of queries while
/// the ranking absorbs it.
///
/// So: same ids in the same order is [`ParityStatus::Identical`]. Same ids in
/// the same order with different scores is [`ParityStatus::KnownDivergence`]
/// with the scale named -- declared, per §8.1, not discovered. Anything that
/// changes *which* items appear or *in what order* is a
/// [`ParityStatus::Regression`], which is the thing Suite 0 exists to catch.
fn compare_results(
    cpp: &[SearchResultItem],
    rust: &[SearchResultItem],
) -> (ParityStatus, Option<String>) {
    let cpp_ids: Vec<&str> = cpp.iter().map(|hit| hit.id.as_str()).collect();
    let rust_ids: Vec<&str> = rust.iter().map(|hit| hit.id.as_str()).collect();

    if cpp_ids != rust_ids {
        return (
            ParityStatus::Regression,
            Some(format!(
                "ranking differs\n  cpp:  {cpp_ids:?}\n  rust: {rust_ids:?}"
            )),
        );
    }

    let scores_differ = cpp
        .iter()
        .zip(rust)
        .any(|(a, b)| (a.score - b.score).abs() > f64::EPSILON);

    if scores_differ {
        return (
            ParityStatus::KnownDivergence,
            Some(
                "same ranking, different scores: the C++ engine reports the value it orders by \
                 (match score plus frecency) and the Rust engine reports the 0..=100 match score. \
                 Declared in PARITY.md; the ranking is what is gated."
                    .to_owned(),
            ),
        );
    }

    (ParityStatus::Identical, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, score: f64) -> SearchResultItem {
        SearchResultItem {
            id: id.to_owned(),
            title: id.to_owned(),
            subtitle: None,
            score,
        }
    }

    #[test]
    fn identical_results_compare_identical() {
        let a = vec![hit("firefox", 100.0), hit("files", 80.0)];
        let b = vec![hit("firefox", 100.0), hit("files", 80.0)];
        assert_eq!(compare_results(&a, &b).0, ParityStatus::Identical);
    }

    // The control. A comparison harness that cannot report a difference is
    // worth nothing, and this one reported 378/378 identical the first time it
    // ever ran — which looks like success and is also exactly what a broken
    // comparison looks like. Each case below is a way the two engines could
    // really diverge.
    #[test]
    fn a_difference_is_a_regression() {
        let base = vec![hit("firefox", 100.0), hit("files", 80.0)];

        // Different ranking order — the thing Suite 0 exists to catch.
        let reordered = vec![hit("files", 80.0), hit("firefox", 100.0)];
        assert_eq!(
            compare_results(&base, &reordered).0,
            ParityStatus::Regression
        );

        // A hit one engine found and the other did not.
        let truncated = vec![hit("firefox", 100.0)];
        assert_eq!(
            compare_results(&base, &truncated).0,
            ParityStatus::Regression
        );

        // And one side empty, which is what a silently failing engine produces.
        assert_eq!(compare_results(&base, &[]).0, ParityStatus::Regression);

        // A swap deeper in the list, which a comparison that only looked at
        // the top hit would miss. §8.1a reports full-order parity at 84.4%
        // against top-1 at 100%, so this is where the disagreements live.
        let deep = vec![
            hit("firefox", 100.0),
            hit("files", 80.0),
            hit("a", 10.0),
            hit("b", 9.0),
        ];
        let deep_swapped = vec![
            hit("firefox", 100.0),
            hit("files", 80.0),
            hit("b", 9.0),
            hit("a", 10.0),
        ];
        assert_eq!(
            compare_results(&deep, &deep_swapped).0,
            ParityStatus::Regression
        );
    }

    /// A non-ASCII name must not abort the run.
    ///
    /// `name[..i]` on byte indices panicked on the first accented entry in the
    /// checked-in corpus:
    ///
    /// ```text
    /// byte index 2 is not a char boundary; it is inside 'é' (bytes 1..3)
    /// of `Déjà Dup Backups`
    /// ```
    ///
    /// The harness died before comparing a single query, and no test noticed
    /// because nothing had ever run it against the real corpus.
    #[test]
    fn prefixes_are_taken_by_character_so_an_accented_name_does_not_abort_the_run() {
        let corpus = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            corpus.path().join("deja-dup.desktop"),
            "[Desktop Entry]\nType=Application\nName=Déjà Dup Backups\nExec=/bin/true\n",
        )
        .expect("write");

        let queries = generate_test_queries(corpus.path()).expect("queries");

        assert!(
            queries.iter().any(|q| q == "Déjà Dup Backups"),
            "the full name must be queried"
        );
        for prefix in ["D", "Dé", "Déj", "Déjà"] {
            assert!(
                queries.iter().any(|q| q == prefix),
                "the {prefix:?} prefix must be generated, split on characters"
            );
        }
    }

    /// CONTROL. An empty corpus must be refused, not run.
    ///
    /// Two engines that rank nothing agree on everything, so a run over an
    /// empty corpus reports perfect parity having compared nothing. That is
    /// the exact shape of green Suite 0 exists to catch, and it would be the
    /// worst possible place to produce one.
    #[test]
    fn staging_refuses_a_corpus_with_no_entries() {
        let empty = tempfile::tempdir().expect("tempdir");
        std::fs::write(empty.path().join("README.md"), "not a desktop entry").expect("write");

        let error = StagedCorpus::stage(empty.path()).expect_err("an empty corpus must not pass");
        let message = format!("{error}");
        assert!(
            message.contains("agree perfectly"),
            "the refusal must say why an empty corpus is not a pass: {message}"
        );
    }

    /// Entries land where an XDG engine looks: `$XDG_DATA_DIRS/applications`.
    ///
    /// Nested directories are flattened on purpose — the checked-in corpus
    /// keeps entries in `real/`, and `$XDG_DATA_DIRS` names the parent of
    /// `applications`, not of `real`.
    #[test]
    fn staging_collects_every_entry_into_the_applications_directory() {
        let source = tempfile::tempdir().expect("tempdir");
        let nested = source.path().join("real");
        std::fs::create_dir_all(&nested).expect("mkdir");
        for name in ["one", "two"] {
            std::fs::write(
                nested.join(format!("{name}.desktop")),
                format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true\n"),
            )
            .expect("write");
        }
        std::fs::write(source.path().join("notes.txt"), "ignored").expect("write");

        let staged = StagedCorpus::stage(source.path()).expect("staging");
        assert_eq!(staged.entries, 2, "only .desktop files are staged");
        for name in ["one", "two"] {
            assert!(
                staged
                    .root
                    .join("applications")
                    .join(format!("{name}.desktop"))
                    .exists(),
                "{name} must be staged under applications/"
            );
        }
    }

    /// Both engines are pointed at the staged corpus and away from the
    /// runner's own.
    ///
    /// Without `XDG_DATA_HOME` and `HOME`, the machine's
    /// `~/.local/share/applications` joins one side of a comparison that is
    /// supposed to be about the engines. Asserted on the environment the
    /// harness builds, because that is the thing that was missing.
    #[test]
    fn the_staged_corpus_replaces_every_data_root_an_engine_would_read() {
        let source = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            source.path().join("one.desktop"),
            "[Desktop Entry]\nType=Application\nName=One\nExec=/bin/true\n",
        )
        .expect("write");

        let staged = StagedCorpus::stage(source.path()).expect("staging");
        let env = staged.env();
        let keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, ["XDG_DATA_DIRS", "XDG_DATA_HOME", "HOME"]);

        assert_eq!(
            env[0].1, staged.root,
            "entries are read from the staged root"
        );
        for (key, value) in &env[1..] {
            assert!(
                std::fs::read_dir(value)
                    .expect("the directory exists")
                    .next()
                    .is_none(),
                "{key} must point at an empty directory, or the runner's own apps leak in"
            );
        }
    }

    /// The two engines' argv, pinned.
    ///
    /// Neither CLI accepts the other's. `crates/vicinae/src/cli.rs` pins the
    /// Rust half from its own side; this pins both from here, because the
    /// C++ half has no Rust parser to check it and its every earlier version
    /// was assembled from §8.1's prose and was wrong.
    ///
    /// What the C++ CLI actually offers (`src/cli/src/cli.cpp`): `server`, not
    /// `serve`; a bare `ping`; `query --json`, added for rung 2; and **no**
    /// `--socket` and no `shutdown`.
    #[test]
    fn each_engine_is_invoked_the_way_its_own_cli_parses() {
        let socket = Path::new("/tmp/x/ipc.sock");

        assert_eq!(
            Flavour::Rust.serve_args(socket),
            ["--socket", "/tmp/x/ipc.sock", "serve"]
        );
        assert_eq!(Flavour::Cpp.serve_args(socket), ["server"]);

        assert_eq!(
            Flavour::Rust.query_args(socket, "fire"),
            ["--socket", "/tmp/x/ipc.sock", "query", "--json", "fire"]
        );
        assert_eq!(
            Flavour::Cpp.query_args(socket, "fire"),
            ["query", "--json", "fire"]
        );

        assert_eq!(
            Flavour::Rust.ping_args(socket),
            ["--socket", "/tmp/x/ipc.sock", "ping"]
        );
        assert_eq!(Flavour::Cpp.ping_args(socket), ["ping"]);

        // The C++ CLI has no `shutdown`, so asking would print an error and
        // the caller must go straight to killing the child.
        assert!(Flavour::Rust.shutdown_args(socket).is_some());
        assert!(Flavour::Cpp.shutdown_args(socket).is_none());

        // No C++ invocation may carry `--socket`: that flag does not exist on
        // that side, and passing it turns a working call into a failing one.
        // The C++ engine is isolated by XDG_RUNTIME_DIR instead.
        for args in [
            Flavour::Cpp.serve_args(socket),
            Flavour::Cpp.query_args(socket, "fire"),
            Flavour::Cpp.ping_args(socket),
        ] {
            assert!(
                !args.iter().any(|arg| arg == "--socket"),
                "the C++ CLI has no --socket flag, but {args:?} passes one"
            );
        }
    }

    /// Scores on two scales are a DECLARED divergence, not a regression.
    ///
    /// Without this the harness reports a regression on every query with a
    /// match, because the two engines put different numbers on the wire by
    /// design — see [`compare_results`].
    #[test]
    fn same_ranking_different_scores_is_a_known_divergence() {
        let cpp = vec![hit("firefox", 137.4), hit("files", 91.2)];
        let rust = vec![hit("firefox", 100.0), hit("files", 80.0)];

        let (status, reason) = compare_results(&cpp, &rust);
        assert_eq!(status, ParityStatus::KnownDivergence);
        assert!(
            reason
                .expect("a divergence cites its reason")
                .contains("frecency"),
            "§8.1 requires a divergence to cite a rationale, not just a verdict"
        );

        // And it must not swallow a reordering that happens to share scores:
        // a known divergence that absorbs a regression is worse than no check.
        let swapped = vec![hit("files", 91.2), hit("firefox", 137.4)];
        assert_eq!(compare_results(&swapped, &rust).0, ParityStatus::Regression);
    }

    // The shape is the engine's, not §8.1's prose. This deserialises output in
    // `compass_ipc::protocol::QueryHit`'s spelling; the previous struct was
    // `{key, name, score, quality}` and would have failed on every query.
    #[test]
    fn the_engines_json_deserialises() {
        let json =
            r#"[{"id":"firefox.desktop","title":"Firefox","subtitle":"Web Browser","score":100}]"#;
        let parsed: Vec<SearchResultItem> = serde_json::from_str(json).expect("engine JSON parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "firefox.desktop");
        assert_eq!(parsed[0].title, "Firefox");
        assert!((parsed[0].score - 100.0).abs() < f64::EPSILON);

        // And the C++ engine's fractional score parses, which `u32` could not.
        let cpp_shape = r#"[{"id":"apps:firefox","title":"Firefox","score":137.42}]"#;
        let cpp: Vec<SearchResultItem> =
            serde_json::from_str(cpp_shape).expect("the C++ rootQuery shape parses");
        assert!((cpp[0].score - 137.42).abs() < f64::EPSILON);
        assert_eq!(cpp[0].subtitle, None);

        // And the old shape is genuinely rejected, so this test would have
        // caught the original mismatch rather than passing either way.
        let old_shape = r#"[{"key":"firefox","name":"Firefox","score":100,"quality":1}]"#;
        assert!(
            serde_json::from_str::<Vec<SearchResultItem>>(old_shape).is_err(),
            "the pre-measurement shape must not parse, or this test proves nothing"
        );
    }
}
