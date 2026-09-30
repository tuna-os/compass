//! `compass script template`: a new script command's source, in one of ten
//! languages.
//!
//! A port of the C++ CLI's `ScriptCommandGenerator`: the same languages under
//! the same names, the same templates byte for byte, and `{title}` and
//! `{mode}` substituted everywhere they appear.

use crate::script_command::OutputMode;

/// A language a template can be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// `bash`.
    Bash,
    /// `python`.
    Python,
    /// `javascript`.
    Javascript,
    /// `typescript`.
    TypeScript,
    /// `ruby`.
    Ruby,
    /// `perl`.
    Perl,
    /// `php`.
    Php,
    /// `lua`.
    Lua,
    /// `go`.
    Golang,
    /// `swift`.
    Swift,
}

/// Every language with the name `--lang` takes, in the order the C++ lists
/// them in its error.
pub const LANGUAGES: [(&str, Language); 10] = [
    ("bash", Language::Bash),
    ("python", Language::Python),
    ("javascript", Language::Javascript),
    ("typescript", Language::TypeScript),
    ("ruby", Language::Ruby),
    ("perl", Language::Perl),
    ("php", Language::Php),
    ("lua", Language::Lua),
    ("go", Language::Golang),
    ("swift", Language::Swift),
];

/// The modes `--mode` takes, as the C++ error lists them.
pub const MODES: &str = "fullOutput, compact, inline, silent, terminal";

impl Language {
    /// The language `--lang` names; exact, as the C++ map lookup is.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        LANGUAGES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, language)| *language)
    }

    /// The names `--lang` takes, comma separated.
    #[must_use]
    pub fn supported() -> String {
        LANGUAGES
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }

    const fn template(self) -> &'static str {
        match self {
            Self::Bash => concat!(
                "#!/bin/bash\n",
                "# @vicinae.schemaVersion 1\n",
                "# @vicinae.title {title}\n",
                "# @vicinae.mode {mode}\n",
                "# @vicinae.exec [\"/bin/bash\"]\n",
                "\n",
                "echo \"Hello world!\"",
            ),
            Self::Python => concat!(
                "#!/usr/bin/env python3\n",
                "# @vicinae.schemaVersion 1\n",
                "# @vicinae.title {title}\n",
                "# @vicinae.mode {mode}\n",
                "# @vicinae.exec [\"/usr/bin/env\", \"python3\"]\n",
                "\n",
                "print(\"Hello world!\")",
            ),
            Self::Javascript => concat!(
                "#!/usr/bin/env node\n",
                "// @vicinae.schemaVersion 1\n",
                "// @vicinae.title {title}\n",
                "// @vicinae.mode {mode}\n",
                "// @vicinae.exec [\"/usr/bin/env\", \"node\"]\n",
                "\n",
                "console.log('Hello world!');",
            ),
            Self::TypeScript => concat!(
                "#!/usr/bin/env ts-node\n",
                "// @vicinae.schemaVersion 1\n",
                "// @vicinae.title {title}\n",
                "// @vicinae.mode {mode}\n",
                "// @vicinae.exec [\"/usr/bin/env\", \"ts-node\"]\n",
                "\n",
                "console.log('Hello world!');",
            ),
            Self::Ruby => concat!(
                "#!/usr/bin/env ruby\n",
                "# @vicinae.schemaVersion 1\n",
                "# @vicinae.title {title}\n",
                "# @vicinae.mode {mode}\n",
                "# @vicinae.exec [\"/usr/bin/env\", \"ruby\"]\n",
                "\n",
                "puts 'Hello world!'",
            ),
            Self::Perl => concat!(
                "#!/usr/bin/env perl\n",
                "# @vicinae.schemaVersion 1\n",
                "# @vicinae.title {title}\n",
                "# @vicinae.mode {mode}\n",
                "# @vicinae.exec [\"/usr/bin/env\", \"perl\"]\n",
                "\n",
                "print \"Hello world!\\n\";",
            ),
            Self::Php => concat!(
                "#!/usr/bin/env php\n",
                "<?php\n",
                "// @vicinae.schemaVersion 1\n",
                "// @vicinae.title {title}\n",
                "// @vicinae.mode {mode}\n",
                "// @vicinae.exec [\"/usr/bin/env\", \"php\"]\n",
                "\n",
                "echo \"Hello world!\\n\";",
            ),
            Self::Lua => concat!(
                "#!/usr/bin/env lua\n",
                "-- @vicinae.schemaVersion 1\n",
                "-- @vicinae.title {title}\n",
                "-- @vicinae.mode {mode}\n",
                "-- @vicinae.exec [\"/usr/bin/env\", \"lua\"]\n",
                "\n",
                "print(\"Hello world!\")",
            ),
            Self::Golang => concat!(
                "#!/usr/bin/env go\n",
                "// @vicinae.schemaVersion 1\n",
                "// @vicinae.title {title}\n",
                "// @vicinae.mode {mode}\n",
                "// @vicinae.exec [\"/usr/bin/env\", \"go\", \"run\"]\n",
                "\n",
                "package main\n",
                "\n",
                "import \"fmt\"\n",
                "\n",
                "func main() {\n",
                "\tfmt.Println(\"Hello world!\")\n",
                "}",
            ),
            Self::Swift => concat!(
                "#!/usr/bin/env swift\n",
                "// @vicinae.schemaVersion 1\n",
                "// @vicinae.title {title}\n",
                "// @vicinae.mode {mode}\n",
                "// @vicinae.exec [\"/usr/bin/env\", \"swift\"]\n",
                "\n",
                "print(\"Hello world!\")",
            ),
        }
    }
}

/// The script `compass script template` prints, without its trailing
/// newline (the CLI adds one, as `std::endl` does).
#[must_use]
pub fn generate(title: &str, language: Language, mode: OutputMode) -> String {
    language
        .template()
        .replace("{title}", title)
        .replace("{mode}", mode.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script_command::ScriptCommand;

    #[test]
    fn the_bash_template_is_the_cpps_exactly() {
        assert_eq!(
            generate("Say Hello", Language::Bash, OutputMode::Compact),
            "#!/bin/bash\n# @vicinae.schemaVersion 1\n# @vicinae.title Say Hello\n\
             # @vicinae.mode compact\n# @vicinae.exec [\"/bin/bash\"]\n\necho \"Hello world!\""
        );
    }

    #[test]
    fn every_language_parses_back_as_a_script_command_with_its_title_and_mode() {
        for (name, language) in LANGUAGES {
            for mode in [
                OutputMode::Full,
                OutputMode::Compact,
                OutputMode::Inline,
                OutputMode::Silent,
                OutputMode::Terminal,
            ] {
                let text = generate("Tëst {x}", language, mode);
                let parsed = ScriptCommand::parse(&text)
                    .unwrap_or_else(|error| panic!("{name} {mode:?}: {error}\n{text}"));
                assert_eq!(parsed.title, "Tëst {x}", "{name}");
                assert_eq!(parsed.mode, mode, "{name}");
                assert!(!parsed.exec.is_empty(), "{name} has an exec");
            }
        }
    }

    #[test]
    fn languages_are_named_exactly_and_listed_in_the_cpps_order() {
        assert_eq!(Language::parse("go"), Some(Language::Golang));
        assert_eq!(Language::parse("golang"), None);
        assert_eq!(Language::parse("Bash"), None, "the C++ lookup is exact");
        assert_eq!(
            Language::supported(),
            "bash, python, javascript, typescript, ruby, perl, php, lua, go, swift"
        );
    }

    #[test]
    fn php_keeps_its_open_tag_and_go_its_program() {
        let php = generate("x", Language::Php, OutputMode::Silent);
        assert!(php.starts_with("#!/usr/bin/env php\n<?php\n// @vicinae.schemaVersion 1"));
        let go = generate("x", Language::Golang, OutputMode::Silent);
        assert!(go.ends_with("func main() {\n\tfmt.Println(\"Hello world!\")\n}"));
        assert!(go.contains("[\"/usr/bin/env\", \"go\", \"run\"]"));
    }
}
