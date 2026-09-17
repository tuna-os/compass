//! The VM tier's client invocations, checked against the real CLI parser.
//!
//! # Why this exists
//!
//! `packaging/vmtest/checks.sh` drives the engine by shelling out to the
//! Flatpak, whose entrypoint **is** `vicinae`. So the arguments it passes are
//! subcommands, with no `vicinae` in front of them.
//!
//! A version of it wrote `compass_cli vicinae ping`, which runs
//! `vicinae vicinae ping`. The parser rejects that, always — so the engine
//! readiness check could never succeed, and the failure presented as *"timed
//! out after 120s waiting for: the engine to answer a ping"* with a perfectly
//! healthy engine running beside it. It cost a 30-minute VM run to see, and
//! would have cost another to confirm any fix.
//!
//! The parser is right here. Nothing about catching that needed a VM.
//!
//! # What it checks
//!
//! Every `compass_cli …` call site in `checks.sh`, parsed with the same clap
//! definition the binary uses. It is not a spelling check against a list: a
//! list would have to be kept in step with `Command` by hand, which is the
//! failure mode one layer up.

use clap::Parser;
use vicinae::cli::Cli;

/// Where the guest checks live, relative to this crate.
const CHECKS: &str = "../../packaging/vmtest/checks.sh";

/// How many call sites there must be at least.
///
/// Without a floor this test passes vacuously the moment the helper is renamed
/// or the call sites move: "found nothing, nothing was wrong". The number is
/// deliberately lower than the real count so ordinary edits do not trip it,
/// and any change that removes most of them does.
const MINIMUM_CALL_SITES: usize = 4;

/// The arguments of every `flatpak run … <APP> …` invocation, in file order.
///
/// These are the long-running ones -- `serve`, `ui`, `spike` -- started inside
/// `setsid bash -c` blocks rather than through `compass_cli`. They carry real
/// flags (`serve --no-hotkey`, `doctor --json`) and are just as able to be
/// wrong, but a mistake in one costs a 25-minute VM run to find.
///
/// The app is named either by `"$APP"` or by a positional `"$5"`, depending on
/// whether the invocation is inside a `bash -c` string; both forms appear and
/// both are matched, because matching only one would silently cover half of
/// them.
fn flatpak_call_sites(script: &str) -> Vec<(usize, Vec<String>)> {
    let mut found = Vec::new();
    let lines: Vec<&str> = script.lines().collect();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || !trimmed.starts_with("flatpak run ") {
            continue;
        }

        // Gather the logical invocation: this line plus any continuations.
        let mut text = String::new();
        let mut cursor = index;
        loop {
            let piece = lines[cursor].trim_end();
            text.push(' ');
            text.push_str(piece.strip_suffix('\\').unwrap_or(piece));
            if !piece.ends_with('\\') || cursor + 1 >= lines.len() {
                break;
            }
            cursor += 1;
        }

        // Everything after the app name is the CLI's argv.
        let Some(rest) = text
            .split_once("\"$APP\"")
            .or_else(|| text.split_once("\"$5\""))
        else {
            continue;
        };

        let args: Vec<String> = rest
            .1
            .split_whitespace()
            .take_while(|token| {
                !token.starts_with('>')
                    && !token.starts_with('<')
                    && !token.starts_with("2>")
                    && !token.starts_with('|')
                    && !token.starts_with('&')
                    && !token.starts_with(';')
            })
            .map(|token| token.trim_matches('"').to_owned())
            .collect();

        // `compass_cli`'s own body is `flatpak run … "$APP" "$@"` -- a
        // pass-through with no arguments of its own. Its call sites are
        // checked by `every_vm_tier_invocation_parses_as_a_real_command`;
        // checking the forwarder would just be asking whether `$@` is a
        // subcommand.
        if args == ["$@"] {
            continue;
        }

        found.push((index + 1, args));
    }

    found
}

/// The arguments of every `compass_cli` call site, in file order.
///
/// Shell operators end an invocation: everything from the first redirection,
/// pipe, `&&`, `;` or closing paren onward belongs to the shell, not to the
/// CLI. Comment lines and the helper's own definition are skipped.
fn call_sites(script: &str) -> Vec<(usize, Vec<String>)> {
    let mut found = Vec::new();

    for (number, line) in script.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with("compass_cli()") {
            continue;
        }

        let Some(rest) = line.split_once("compass_cli ") else {
            continue;
        };

        let args: Vec<String> = rest
            .1
            .split_whitespace()
            .take_while(|token| {
                !token.starts_with('>')
                    && !token.starts_with('<')
                    && !token.starts_with("2>")
                    && !token.starts_with('|')
                    && !token.starts_with('&')
                    && !token.starts_with(';')
                    && !token.starts_with(')')
            })
            .map(ToOwned::to_owned)
            .collect();

        found.push((number + 1, args));
    }

    found
}

#[test]
fn every_vm_tier_invocation_parses_as_a_real_command() {
    let script = std::fs::read_to_string(CHECKS).expect("read checks.sh");
    let sites = call_sites(&script);

    assert!(
        sites.len() >= MINIMUM_CALL_SITES,
        "found only {} `compass_cli` call sites in checks.sh, expected at least {}. \
         Either the helper was renamed -- in which case this test now guards nothing \
         and should be pointed at the new name -- or the summon checks were removed.",
        sites.len(),
        MINIMUM_CALL_SITES
    );

    for (line, args) in &sites {
        assert!(
            !args.is_empty(),
            "checks.sh:{line}: `compass_cli` with no arguments"
        );

        // `vicinae` as argv[0] and the rest after it, exactly as the Flatpak
        // entrypoint receives them.
        let argv: Vec<&str> = std::iter::once("vicinae")
            .chain(args.iter().map(String::as_str))
            .collect();

        if let Err(err) = Cli::try_parse_from(&argv) {
            panic!(
                "checks.sh:{line}: `{}` is not something this CLI accepts.\n\
                 The Flatpak entrypoint IS `vicinae`, so these arguments are \
                 subcommands -- writing `vicinae` in front of them runs \
                 `vicinae vicinae …`.\n\n{err}",
                args.join(" ")
            );
        }
    }
}

#[test]
fn every_long_running_vm_tier_invocation_parses_too() {
    let script = std::fs::read_to_string(CHECKS).expect("read checks.sh");
    let sites = flatpak_call_sites(&script);

    // `serve`, `ui`, `spike sandbox`, `spike global-shortcut`, `doctor`.
    const MINIMUM: usize = 4;
    assert!(
        sites.len() >= MINIMUM,
        "found only {} `flatpak run` invocations in checks.sh, expected at least {}. \
         If they moved, point this test at where they went rather than deleting it.",
        sites.len(),
        MINIMUM
    );

    for (line, args) in &sites {
        let argv: Vec<&str> = std::iter::once("vicinae")
            .chain(args.iter().map(String::as_str))
            .collect();

        if let Err(err) = Cli::try_parse_from(&argv) {
            panic!(
                "checks.sh:{line}: `{}` is not something this CLI accepts.\n\n{err}",
                args.join(" ")
            );
        }
    }
}

#[test]
fn the_parser_would_reject_the_mistake_this_guards_against() {
    // The control. Without it, the test above could be passing because the
    // parser accepts anything -- which is exactly what a `try_parse_from` that
    // silently ignored unknown arguments would look like.
    let doubled = ["vicinae", "vicinae", "ping"];
    assert!(
        Cli::try_parse_from(doubled).is_err(),
        "the parser accepts `vicinae vicinae ping`, so the test above proves nothing"
    );

    // And the shape that must keep working, so a parser that rejected
    // everything would not look like a passing guard either.
    assert!(
        Cli::try_parse_from(["vicinae", "ping"]).is_ok(),
        "the parser rejects a plain `vicinae ping`"
    );
}
