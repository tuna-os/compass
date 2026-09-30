//! `figura compile <file.fig> --output <out.ts> (--client | --server) typescript`
//!
//! The same invocation the C++ `figura` took, TypeScript only. `figura
//! regenerate [ROOT]` rewrites every binding in [`compass_figura::COMMITTED`].

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compass_figura::{COMMITTED, Side, compile};

const USAGE: &str = "usage:\n  figura compile <file.fig> --output <out.ts> (--client|--server) typescript\n  figura regenerate [REPOSITORY_ROOT]";

fn generate(fig: &Path, side: Side, out: &Path) -> Result<(), String> {
    let source = std::fs::read_to_string(fig)
        .map_err(|e| format!("could not read {}: {e}", fig.display()))?;
    let generated =
        compile(&source, side).map_err(|e| format!("Failed to parse {}\n{e}", fig.display()))?;
    std::fs::write(out, generated).map_err(|e| format!("could not write {}: {e}", out.display()))
}

fn compile_command(args: &[String]) -> Result<(), String> {
    let mut proto = None;
    let mut output = None;
    let mut side = None;
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--output" | "-o" => output = iter.next().map(PathBuf::from),
            "--client" | "--server" => {
                let language = iter.next().map(String::as_str);
                if language != Some("typescript") {
                    return Err(format!(
                        "{arg} {}: only the typescript generator exists",
                        language.unwrap_or("")
                    ));
                }
                side = Some(if arg == "--client" {
                    Side::Client
                } else {
                    Side::Server
                });
            }
            other if proto.is_none() && !other.starts_with('-') => {
                proto = Some(PathBuf::from(other));
            }
            other => return Err(format!("unexpected argument {other}\n{USAGE}")),
        }
    }

    let (Some(proto), Some(output), Some(side)) = (proto, output, side) else {
        return Err(USAGE.to_owned());
    };
    generate(&proto, side, &output)?;
    let which = if side == Side::Client {
        "client"
    } else {
        "server"
    };
    println!("generated typescript {which} at {}", output.display());
    Ok(())
}

fn regenerate(root: &Path) -> Result<(), String> {
    for (fig, side, out) in COMMITTED {
        let out = root.join(out);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        generate(&root.join(fig), *side, &out)?;
        println!("{}", out.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("compile") => compile_command(&args[1..]),
        Some("regenerate") => regenerate(Path::new(args.get(1).map_or(".", String::as_str))),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
