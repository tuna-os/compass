//! Script commands: a shell script with a header the launcher reads.
//!
//! Ports `src/lib/script-command/`. A script command is an ordinary
//! executable whose leading comments carry metadata:
//!
//! ```bash
//! #!/usr/bin/env bash
//! # @raycast.schemaVersion 1
//! # @raycast.title Say hello
//! # @raycast.mode compact
//! ```
//!
//! Two scopes exist, `@raycast` and `@vicinae`, and a file may use one or the
//! other but not both. Four comment markers are recognised — `//`, `--`, `#`
//! and `;` — because the header has to work in whatever language the script is
//! written in.
//!
//! # Quirks reproduced on purpose
//!
//! These are not improvements waiting to happen; a script that works with the
//! C++ engine has to work here.
//!
//! * **Only spaces are trimmed from a line, not tabs.** The C++ `trim` takes a
//!   single character and is called with `' '`, so a tab-indented `# @…` line
//!   is not recognised as a comment at all. Inside the line, after the marker,
//!   whitespace *is* skipped with `isspace`, so tabs there are fine.
//! * **`data` for a dropdown is one object, not a list.** The struct is a
//!   single `{title, value}`, and glaze would refuse an array.
//! * **`secure: true` forces `password`**, whatever `type` said. It is the
//!   legacy Raycast spelling.
//! * **An empty time unit means seconds.** `refreshTime 30` is thirty seconds.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// What the launcher does with the script's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutputMode {
    /// Show everything, in a detail view.
    #[default]
    Full,
    /// A compact view.
    Compact,
    /// Inline in the results list.
    Inline,
    /// Nothing.
    Silent,
    /// Hand it to a terminal.
    Terminal,
}

impl OutputMode {
    /// The spelling used in the header.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "fullOutput",
            Self::Compact => "compact",
            Self::Inline => "inline",
            Self::Silent => "silent",
            Self::Terminal => "terminal",
        }
    }

    /// The mode a header value names.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fullOutput" => Some(Self::Full),
            "compact" => Some(Self::Compact),
            "inline" => Some(Self::Inline),
            "silent" => Some(Self::Silent),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

/// What kind of input an argument asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgumentType {
    /// Free text.
    Text,
    /// Free text, not echoed.
    Password,
    /// One of a fixed set.
    Dropdown,
}

/// One option of a dropdown argument.
///
/// Singular, not a list: the C++ struct is one `{title, value}` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgumentDataOption {
    /// What the user sees.
    pub title: String,
    /// What the script receives.
    pub value: String,
}

/// An argument as it is written in the header, before validation.
#[derive(Debug, Clone, Default, Deserialize)]
struct ParsedArgument {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default)]
    optional: Option<bool>,
    #[serde(default, rename = "percentEncoded")]
    percent_encoded: Option<bool>,
    #[serde(default)]
    data: Option<ArgumentDataOption>,
    /// Legacy Raycast spelling; forces [`ArgumentType::Password`].
    #[serde(default)]
    secure: Option<bool>,
}

/// A validated argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptArgument {
    /// What kind of input.
    pub argument_type: ArgumentType,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Whether it may be left empty.
    pub optional: bool,
    /// Whether the value is percent-encoded before it reaches the script.
    pub percent_encoded: bool,
    /// The dropdown's option.
    pub data: Option<ArgumentDataOption>,
}

/// How a terminal-mode command wants its terminal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOptions {
    /// Keep the terminal open after the command exits.
    #[serde(default)]
    pub hold: Option<bool>,
    /// Window title.
    #[serde(default)]
    pub title: Option<String>,
    /// Which terminal application.
    #[serde(default, rename = "appId")]
    pub app_id: Option<String>,
    /// Where to run.
    #[serde(default, rename = "workingDirectory")]
    pub working_directory: Option<String>,
}

/// A parsed script command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ScriptCommand {
    /// `@vicinae.exec`, when given.
    pub exec: Vec<String>,
    /// The words of the `#!` line, if there is one.
    pub shebang: Vec<String>,
    /// Always `"1"`; anything else is refused.
    pub schema_version: String,
    /// Display name.
    pub title: String,
    /// What to do with the output.
    pub mode: OutputMode,
    /// Grouping name.
    pub package_name: Option<String>,
    /// Icon.
    pub icon: Option<String>,
    /// Icon for dark themes.
    pub icon_dark: Option<String>,
    /// Working directory.
    pub current_directory_path: Option<String>,
    /// Whether to ask before running.
    pub needs_confirmation: bool,
    /// Re-run interval, in seconds. Only valid with [`OutputMode::Inline`].
    pub refresh_time: Option<u64>,
    /// Who wrote it.
    pub author: Option<String>,
    /// Where to find them.
    pub author_url: Option<String>,
    /// What it does.
    pub description: Option<String>,
    /// `@vicinae.keywords`, extra search terms.
    pub keywords: Vec<String>,
    /// Up to three arguments.
    pub arguments: Vec<ScriptArgument>,
    /// `@vicinae.terminal`.
    pub terminal: Option<TerminalOptions>,
}

/// The comment markers a header line may start with.
///
/// Order matters, and the C++ says so in a comment: the longer markers are
/// checked first so that `--` is not read as one `-`.
const COMMENT_MARKERS: &[&str] = &["//", "--", "#", ";"];

/// The two scopes a key may be in.
const SCOPES: &[&str] = &["@vicinae", "@raycast"];

/// Trims a single character from both ends, as the C++ `trim` does.
fn trim_char(text: &str, c: char) -> &str {
    text.trim_matches(c)
}

/// `@scope.key value`, if the line is one.
fn parse_kv(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.trim_start_matches(char::is_whitespace);
    if !rest.starts_with('@') {
        return None;
    }
    let dot = rest.find('.')?;
    let scope = &rest[..dot];
    if !SCOPES.contains(&scope) {
        return None;
    }

    let after_dot = &rest[dot + 1..];
    let key_end = after_dot
        .find(char::is_whitespace)
        .unwrap_or(after_dot.len());
    let key = &after_dot[..key_end];
    let value = after_dot[key_end..].trim_start_matches(char::is_whitespace);

    Some((scope, key, value))
}

/// `"5s"` → 5, `"2m"` → 120, `"30"` → 30.
///
/// # Errors
///
/// A message naming what was wrong, as the C++ does.
fn parse_time_to_seconds(text: &str) -> Result<u64, String> {
    if text.is_empty() {
        return Err("Time string is empty".to_owned());
    }

    let unit_pos = text
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(text.len(), |(i, _)| i);

    if unit_pos == 0 {
        return Err(format!("Invalid time format: {text}"));
    }

    let value: u64 = text[..unit_pos]
        .parse()
        .map_err(|_| format!("Invalid number in time string: {text}"))?;

    let multiplier = match trim_char(&text[unit_pos..], ' ') {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        unit => return Err(format!("Unknown time unit: {unit}")),
    };

    Ok(value * multiplier)
}

fn parse_argument(json: &str) -> Result<ScriptArgument, String> {
    let parsed: ParsedArgument =
        serde_json::from_str(json).map_err(|error| format!("Failed to parse JSON: {error}"))?;

    let mut argument_type = match parsed.r#type.as_str() {
        "text" => ArgumentType::Text,
        "password" => ArgumentType::Password,
        "dropdown" => ArgumentType::Dropdown,
        other => return Err(format!("Unknown argument type: \"{other}\"")),
    };

    if parsed.secure == Some(true) {
        argument_type = ArgumentType::Password;
    }

    if argument_type == ArgumentType::Dropdown && parsed.data.is_none() {
        return Err("Dropdown argument must have a 'data' field".to_owned());
    }

    Ok(ScriptArgument {
        argument_type,
        placeholder: parsed.placeholder,
        optional: parsed.optional.unwrap_or(false),
        percent_encoded: parsed.percent_encoded.unwrap_or(false),
        data: parsed.data,
    })
}

impl ScriptCommand {
    /// Parses a script's text.
    ///
    /// # Errors
    ///
    /// A message naming what was wrong, in the C++'s words where it has any.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut data = Self::default();
        let mut scope: Option<String> = None;

        if let Some(after) = text.strip_prefix("#!") {
            let first_line = after.split('\n').next().unwrap_or(after);
            data.shebang = first_line
                .split(' ')
                .map(|word| trim_char(word, '\r'))
                .filter(|word| !word.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }

        for raw in text.split('\n') {
            // Space-only, exactly as the C++'s `trim(s)` -- see the module
            // docs. A tab-indented header line is invisible in both engines.
            let line = trim_char(trim_char(raw, '\r'), ' ');

            let Some(marker) = COMMENT_MARKERS.iter().find(|m| line.starts_with(**m)) else {
                continue;
            };
            let Some((kv_scope, key, value)) = parse_kv(&line[marker.len()..]) else {
                continue;
            };

            match &scope {
                None => scope = Some(kv_scope.to_owned()),
                Some(seen) if seen != kv_scope => {
                    return Err("Mixing @vicinae and @raycast keys is not allowed".to_owned());
                }
                Some(_) => {}
            }

            let vicinae_only = |field: &str| -> Result<(), String> {
                if kv_scope == "@vicinae" {
                    Ok(())
                } else {
                    Err(format!("{field} field is only supported in @vicinae scope"))
                }
            };

            match key {
                "schemaVersion" => data.schema_version = value.to_owned(),
                "title" => data.title = value.to_owned(),
                "mode" => {
                    data.mode = OutputMode::parse(value)
                        .ok_or_else(|| format!("Invalid mode: \"{value}\""))?;
                }
                "icon" => data.icon = Some(value.to_owned()),
                "iconDark" => data.icon_dark = Some(value.to_owned()),
                "packageName" => data.package_name = Some(value.to_owned()),
                "currentDirectoryPath" => data.current_directory_path = Some(value.to_owned()),
                "needsConfirmation" => {
                    data.needs_confirmation = match value {
                        "true" => true,
                        "false" => false,
                        _ => {
                            return Err(
                                "needsConfirmation needs to be either true or false".to_owned()
                            );
                        }
                    };
                }
                "author" => data.author = Some(value.to_owned()),
                "authorURL" => data.author_url = Some(value.to_owned()),
                "description" => data.description = Some(value.to_owned()),
                "refreshTime" => {
                    data.refresh_time = Some(
                        parse_time_to_seconds(value)
                            .map_err(|error| format!("Failed to parse refreshTime: {error}"))?,
                    );
                }
                "keywords" => {
                    vicinae_only("keywords")?;
                    data.keywords = serde_json::from_str(value)
                        .map_err(|error| format!("Failed to parse keywords: {error}"))?;
                }
                "exec" => {
                    vicinae_only("exec")?;
                    data.exec = serde_json::from_str(value)
                        .map_err(|error| format!("Failed to parse exec: {error}"))?;
                }
                "terminal" => {
                    vicinae_only("terminal")?;
                    data.terminal = Some(
                        serde_json::from_str(value)
                            .map_err(|error| format!("Failed to parse terminal: {error}"))?,
                    );
                }
                "argument1" | "argument2" | "argument3" => {
                    let argument = parse_argument(value)
                        .map_err(|error| format!("Failed to parse {key}: {error}"))?;
                    data.arguments.push(argument);
                }
                _ => {}
            }
        }

        if data.schema_version != "1" {
            return Err("Invalid schema version, expected 1".to_owned());
        }
        if data.title.is_empty() {
            return Err("Title should not be empty".to_owned());
        }
        if data.refresh_time.is_some() && data.mode != OutputMode::Inline {
            return Err("refreshTime is only allowed when output mode is inline".to_owned());
        }

        Ok(data)
    }

    /// Reads and parses a file.
    ///
    /// # Errors
    ///
    /// The read error, or whatever [`parse`](Self::parse) refused.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|error| format!("could not read {}: {error}", path.as_ref().display()))?;
        Self::parse(&text)
    }
}

/// The interpreter a `#!` line names, as an argv.
///
/// `env` and `env -S` are unwrapped, and the program is reduced to its file
/// name so that it can be looked up on `$PATH`.
#[must_use]
pub fn shebang_interpreter(shebang: &[String]) -> Vec<String> {
    const ENV_WRAPPER: &str = "env";
    const ENV_SPLIT_FLAG: &str = "-S";

    let mut words = shebang.iter();
    let Some(first) = words.next() else {
        return Vec::new();
    };

    let mut program = first;
    if file_name(first) == ENV_WRAPPER {
        let Some(next) = words.next() else {
            return Vec::new();
        };
        program = if next == ENV_SPLIT_FLAG {
            match words.next() {
                Some(after) => after,
                None => return Vec::new(),
            }
        } else {
            next
        };
    }

    let mut argv = vec![file_name(program).to_owned()];
    argv.extend(words.cloned());
    argv
}

fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

/// The interpreter a file extension implies.
#[must_use]
pub fn interpreter_for_extension(extension: &str) -> Vec<String> {
    let argv: &[&str] = match extension {
        ".py" => &["python"],
        ".js" | ".mjs" | ".cjs" => &["node"],
        ".sh" | ".bash" => &["bash"],
        ".ps1" => &["pwsh", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"],
        ".rb" => &["ruby"],
        ".pl" => &["perl"],
        ".lua" => &["lua"],
        ".php" => &["php"],
        _ => &[],
    };
    argv.iter().map(|s| (*s).to_owned()).collect()
}

/// Names that are worth a second look on `$PATH`.
const ALIASES: &[(&str, &str)] = &[
    ("python3", "python"),
    ("pwsh", "powershell"),
    ("sh", "bash"),
];

/// The argv that runs `script`, resolved against `$PATH`.
///
/// `lookup` answers where a program is, or `None`. `exists` answers whether a
/// path is a regular file — the shebang's own program is used verbatim when it
/// is one, which is how a script pinning an absolute interpreter keeps it.
///
/// An empty result means "no interpreter is needed": the script is executable
/// itself.
///
/// # Errors
///
/// When an interpreter is named and cannot be found.
pub fn resolve_interpreter(
    command: &ScriptCommand,
    script: &Path,
    exists: &impl Fn(&str) -> bool,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Vec<String>, String> {
    if let Some(first) = command.shebang.first()
        && exists(first)
    {
        return Ok(command.shebang.clone());
    }

    let mut argv = shebang_interpreter(&command.shebang);
    if argv.is_empty() {
        let extension = script
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        argv = interpreter_for_extension(&extension);
    }
    if argv.is_empty() {
        return Ok(argv);
    }

    let name = argv[0].clone();
    let program = lookup(&name).or_else(|| {
        ALIASES
            .iter()
            .find(|(from, _)| *from == name)
            .and_then(|(_, to)| lookup(to))
    });

    let Some(program) = program else {
        return Err(format!(
            "No interpreter found for {} (wanted {name})",
            script.display()
        ));
    };

    argv[0] = program;
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "#!/usr/bin/env bash\n# @vicinae.schemaVersion 1\n# @vicinae.title T\n";

    #[test]
    fn the_four_comment_markers_all_carry_a_header() {
        // `//`, `--`, `#` and `;`, because the header has to work in whatever
        // language the script is written in.
        for marker in ["//", "--", "#", ";"] {
            let source =
                format!("{marker} @vicinae.schemaVersion 1\n{marker} @vicinae.title Marker\n");
            let parsed = ScriptCommand::parse(&source)
                .unwrap_or_else(|e| panic!("{marker} should carry a header: {e}"));
            assert_eq!(parsed.title, "Marker");
        }
    }

    #[test]
    fn a_tab_indented_header_line_is_invisible_to_both_engines() {
        // The C++ `trim` takes one character and is called with a space, so a
        // line starting with a tab never matches a comment marker. Reproduced
        // rather than fixed: a script that works there has to work here, and
        // "fixing" it would make this engine accept scripts the other refuses.
        let source = "\t# @vicinae.schemaVersion 1\n\t# @vicinae.title Tabbed\n";
        assert!(
            ScriptCommand::parse(source).is_err(),
            "a tab-indented header was read; the C++ does not read it"
        );

        // The control: the same lines with spaces.
        let spaced = "  # @vicinae.schemaVersion 1\n  # @vicinae.title Spaced\n";
        assert_eq!(
            ScriptCommand::parse(spaced)
                .expect("spaces are trimmed")
                .title,
            "Spaced"
        );
    }

    #[test]
    fn whitespace_after_the_marker_may_be_a_tab() {
        // `parseKV` skips with `isspace`, so inside the line a tab is fine.
        let source = "#\t@vicinae.schemaVersion 1\n#\t@vicinae.title Inner\n";
        assert_eq!(
            ScriptCommand::parse(source)
                .expect("a tab after the marker")
                .title,
            "Inner"
        );
    }

    #[test]
    fn mixing_the_two_scopes_is_refused() {
        let source = "# @vicinae.schemaVersion 1\n# @raycast.title Mixed\n";
        assert_eq!(
            ScriptCommand::parse(source).unwrap_err(),
            "Mixing @vicinae and @raycast keys is not allowed"
        );
    }

    #[test]
    fn three_fields_belong_to_the_vicinae_scope_alone() {
        for field in ["keywords", "exec", "terminal"] {
            let source =
                format!("# @raycast.schemaVersion 1\n# @raycast.title T\n# @raycast.{field} []\n");
            assert_eq!(
                ScriptCommand::parse(&source).unwrap_err(),
                format!("{field} field is only supported in @vicinae scope")
            );
        }
    }

    #[test]
    fn the_time_units_are_seconds_minutes_hours_and_days() {
        for (value, seconds) in [
            ("30", 30),
            ("30s", 30),
            ("2m", 120),
            ("1h", 3600),
            ("1d", 86400),
        ] {
            let source = format!(
                "# @vicinae.schemaVersion 1\n# @vicinae.title T\n# @vicinae.mode inline\n# @vicinae.refreshTime {value}\n"
            );
            assert_eq!(
                ScriptCommand::parse(&source)
                    .unwrap_or_else(|e| panic!("{value}: {e}"))
                    .refresh_time,
                Some(seconds),
                "{value} should be {seconds} seconds"
            );
        }
    }

    #[test]
    fn a_refresh_time_outside_inline_mode_is_refused() {
        let source = format!("{HEADER}# @vicinae.mode compact\n# @vicinae.refreshTime 30s\n");
        assert_eq!(
            ScriptCommand::parse(&source).unwrap_err(),
            "refreshTime is only allowed when output mode is inline"
        );
    }

    #[test]
    fn the_legacy_secure_flag_forces_a_password_argument() {
        let source =
            format!("{HEADER}# @vicinae.argument1 {{ \"type\": \"text\", \"secure\": true }}\n");
        let parsed = ScriptCommand::parse(&source).expect("parses");
        assert_eq!(parsed.arguments[0].argument_type, ArgumentType::Password);
    }

    #[test]
    fn a_dropdown_without_data_is_refused_and_one_with_it_is_not() {
        let without = format!("{HEADER}# @vicinae.argument1 {{ \"type\": \"dropdown\" }}\n");
        assert!(
            ScriptCommand::parse(&without)
                .unwrap_err()
                .contains("Dropdown argument must have a 'data' field")
        );

        let with = format!(
            "{HEADER}# @vicinae.argument1 {{ \"type\": \"dropdown\", \"data\": {{ \"title\": \"A\", \"value\": \"a\" }} }}\n"
        );
        let parsed = ScriptCommand::parse(&with).expect("parses");
        assert_eq!(
            parsed.arguments[0].data,
            Some(ArgumentDataOption {
                title: "A".to_owned(),
                value: "a".to_owned()
            }),
            "`data` is one object, not a list -- the C++ struct is a single pair"
        );
    }

    #[test]
    fn the_percent_encoded_flag_is_read_by_its_camel_case_name() {
        // The JSON key is `percentEncoded`. A Rust field name that reached the
        // wire unrenamed would read `false` for every script that sets it, and
        // the script would receive an unencoded value.
        let source = format!(
            "{HEADER}# @vicinae.argument1 {{ \"type\": \"text\", \"percentEncoded\": true }}\n"
        );
        let parsed = ScriptCommand::parse(&source).expect("parses");
        assert!(parsed.arguments[0].percent_encoded);
    }

    #[test]
    fn the_shebang_is_split_into_words() {
        let parsed = ScriptCommand::parse(
            "#!/usr/bin/env -S python3 -u\n# @vicinae.schemaVersion 1\n# @vicinae.title S\n",
        )
        .expect("parses");
        assert_eq!(
            parsed.shebang,
            vec![
                "/usr/bin/env".to_owned(),
                "-S".to_owned(),
                "python3".to_owned(),
                "-u".to_owned()
            ]
        );
    }

    #[test]
    fn env_and_env_dash_s_are_unwrapped() {
        let words =
            |line: &str| -> Vec<String> { line.split(' ').map(ToOwned::to_owned).collect() };
        assert_eq!(
            shebang_interpreter(&words("/usr/bin/env python3")),
            vec!["python3".to_owned()]
        );
        assert_eq!(
            shebang_interpreter(&words("/usr/bin/env -S python3 -u")),
            vec!["python3".to_owned(), "-u".to_owned()]
        );
        assert_eq!(
            shebang_interpreter(&words("/bin/bash")),
            vec!["bash".to_owned()],
            "the program is reduced to its file name so it can be found on PATH"
        );
        assert!(shebang_interpreter(&[]).is_empty());
        assert!(
            shebang_interpreter(&words("/usr/bin/env")).is_empty(),
            "`env` with nothing after it names no interpreter"
        );
    }

    #[test]
    fn an_extension_names_an_interpreter_when_the_shebang_does_not() {
        assert_eq!(interpreter_for_extension(".py"), vec!["python".to_owned()]);
        assert_eq!(interpreter_for_extension(".mjs"), vec!["node".to_owned()]);
        assert_eq!(
            interpreter_for_extension(".ps1"),
            vec![
                "pwsh".to_owned(),
                "-NoProfile".to_owned(),
                "-ExecutionPolicy".to_owned(),
                "Bypass".to_owned(),
                "-File".to_owned()
            ]
        );
        assert!(interpreter_for_extension(".zzz").is_empty());
    }

    #[test]
    fn an_absolute_interpreter_that_exists_is_used_verbatim() {
        let command = ScriptCommand::parse(
            "#!/opt/weird/python -u\n# @vicinae.schemaVersion 1\n# @vicinae.title S\n",
        )
        .expect("parses");
        let argv = resolve_interpreter(
            &command,
            Path::new("/tmp/s.py"),
            &|path| path == "/opt/weird/python",
            &|_| None,
        )
        .expect("resolves");
        assert_eq!(
            argv,
            vec!["/opt/weird/python".to_owned(), "-u".to_owned()],
            "a script pinning an interpreter keeps it"
        );
    }

    #[test]
    fn an_alias_is_tried_when_the_name_is_not_on_the_path() {
        let command = ScriptCommand::parse(
            "#!/usr/bin/env python3\n# @vicinae.schemaVersion 1\n# @vicinae.title S\n",
        )
        .expect("parses");
        let argv = resolve_interpreter(&command, Path::new("/tmp/s.py"), &|_| false, &|name| {
            (name == "python").then(|| "/usr/bin/python".to_owned())
        })
        .expect("resolves through the python3 -> python alias");
        assert_eq!(argv, vec!["/usr/bin/python".to_owned()]);
    }

    #[test]
    fn an_interpreter_that_cannot_be_found_is_an_error_naming_it() {
        let command = ScriptCommand::parse(
            "#!/usr/bin/env ruby\n# @vicinae.schemaVersion 1\n# @vicinae.title S\n",
        )
        .expect("parses");
        let error = resolve_interpreter(&command, Path::new("/tmp/s.rb"), &|_| false, &|_| None)
            .unwrap_err();
        assert!(error.contains("ruby"), "{error}");
    }

    #[test]
    fn a_script_with_no_shebang_and_no_known_extension_needs_no_interpreter() {
        // An empty argv is "run it directly", not an error.
        let command = ScriptCommand::parse("# @vicinae.schemaVersion 1\n# @vicinae.title S\n")
            .expect("parses");
        let argv = resolve_interpreter(&command, Path::new("/tmp/s"), &|_| false, &|_| None)
            .expect("no interpreter needed");
        assert!(argv.is_empty());
    }

    #[test]
    fn the_output_modes_round_trip_through_their_names() {
        for mode in [
            OutputMode::Full,
            OutputMode::Compact,
            OutputMode::Inline,
            OutputMode::Silent,
            OutputMode::Terminal,
        ] {
            assert_eq!(OutputMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(OutputMode::parse("nonsense"), None);
        assert_eq!(
            OutputMode::default(),
            OutputMode::Full,
            "a script that names no mode gets full output"
        );
    }
}
