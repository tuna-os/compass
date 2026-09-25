//! `vicinae config`: the path of `vicinae.json`, its schema, and migrating the C++ settings.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use compass_core::config::{self, default_config_path, json_schema_pretty};
use compass_core::config_migration::{Migration, legacy_config_path, migrate_file};

use crate::cli::ConfigCommand;
use crate::{EXIT_FAILURE, EXIT_OK};

/// Runs a `vicinae config` subcommand.
///
/// # Errors
///
/// When a path cannot be resolved, the settings file cannot be migrated, or the result cannot be
/// written.
pub fn run(command: ConfigCommand) -> Result<ExitCode> {
    match command {
        ConfigCommand::Path => {
            println!("{}", default_config_path()?.display());
            println!("{} (C++ engine)", legacy_config_path()?.display());
            Ok(ExitCode::from(EXIT_OK))
        }
        ConfigCommand::Schema => {
            print!("{}", json_schema_pretty());
            Ok(ExitCode::from(EXIT_OK))
        }
        ConfigCommand::Default => {
            println!(
                "{}",
                serde_json::to_string_pretty(&config::default_document())?
            );
            Ok(ExitCode::from(EXIT_OK))
        }
        ConfigCommand::Migrate {
            from,
            to,
            write,
            force,
            json,
        } => {
            let from = match from {
                Some(from) => from,
                None => legacy_config_path()?,
            };
            if !from.is_file() {
                eprintln!("no settings to migrate at {}", from.display());
                return Ok(ExitCode::from(EXIT_FAILURE));
            }
            let target = match (write, to) {
                (false, _) => None,
                (true, Some(to)) => Some(to),
                (true, None) => Some(default_config_path()?),
            };
            if let Some(to) = &target
                && to.exists()
                && !force
            {
                bail!(
                    "{} already exists; pass --force to replace it (it is kept as a .bak)",
                    to.display()
                );
            }
            let migration = migrate_file(&from)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&migration)?);
            } else {
                print!("{}", render(&migration));
            }

            if let Some(to) = target {
                write_config(&migration, &to)?;
                eprintln!("wrote {}", to.display());
            } else if !json {
                eprintln!("dry run: pass --write to save it");
            }
            Ok(ExitCode::from(EXIT_OK))
        }
    }
}

/// Writes the migrated config, keeping whatever was at `to` as `<to>.bak`.
fn write_config(migration: &Migration, to: &Path) -> Result<()> {
    if to.exists() {
        let mut backup = to.as_os_str().to_owned();
        backup.push(".bak");
        let backup = PathBuf::from(backup);
        std::fs::copy(to, &backup)
            .with_context(|| format!("could not back up {}", to.display()))?;
    }
    migration.config.save_to(to)?;
    Ok(())
}

/// The human report: the resulting file, then what moved and what did not.
#[must_use]
pub fn render(migration: &Migration) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "read:");
    for source in &migration.sources {
        let _ = writeln!(out, "  {}", source.display());
    }
    for missing in &migration.missing_imports {
        let _ = writeln!(out, "  {} (imported, not found)", missing.display());
    }
    let _ = writeln!(out, "carried across:");
    for mapped in &migration.mapped {
        let _ = writeln!(out, "  {} -> {}", mapped.from, mapped.to);
    }
    if !migration.skipped.is_empty() {
        let _ = writeln!(out, "left behind:");
        for skipped in &migration.skipped {
            let _ = writeln!(out, "  {}: {}", skipped.key, skipped.reason);
        }
    }
    let _ = writeln!(out, "{}:", config::CONFIG_RELATIVE_PATH);
    out.push_str(&migration.config.to_json_pretty().unwrap_or_default());
    out
}
