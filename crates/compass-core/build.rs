//! Turns the committed glyph table into a Rust one at build time.
//!
//! `glyph/glyph.cpp` is 41,879 generated lines holding every emoji and
//! curated symbol the launcher offers, produced from the Unicode Character
//! Database by `glyph/scripts/gen.ts` and committed. It is C++ syntax because
//! the generator was written for the C++ engine; it is data here, never
//! compiled. Regenerating it needs network access and a Node toolchain;
//! reading it needs neither.
//!
//! So this parses the committed file rather than duplicating the table or
//! re-running the generator. Two properties follow, and both are the point:
//! the two engines can never disagree about which emoji exist, and a change to
//! the generator's output shape fails this build loudly instead of silently
//! producing a shorter table.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/compass-core sits two levels below the repository root")
        .to_path_buf();

    let table = repo.join("crates/compass-core/glyph/glyph.cpp");
    let header = repo.join("crates/compass-core/glyph/glyph.hpp");
    println!("cargo:rerun-if-changed={}", table.display());
    println!("cargo:rerun-if-changed={}", header.display());
    println!("cargo:rerun-if-changed=build.rs");

    let source =
        std::fs::read_to_string(&table).unwrap_or_else(|e| panic!("read {}: {e}", table.display()));
    let header_source = std::fs::read_to_string(&header)
        .unwrap_or_else(|e| panic!("read {}: {e}", header.display()));

    let icons = repo.join("src/typescript/api/src/api/icon.ts");
    println!("cargo:rerun-if-changed={}", icons.display());
    let icon_source =
        std::fs::read_to_string(&icons).unwrap_or_else(|e| panic!("read {}: {e}", icons.display()));

    let generated = generate(&source, &header_source);
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let out = out_dir.join("glyph_table.rs");
    std::fs::write(&out, generated).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));

    // The generator stamps the Unicode, Emoji and CLDR versions into the first
    // line. Carried across rather than written down, so this build cannot
    // claim a release it does not hold.
    let versions = source.lines().next().unwrap_or_default();
    std::fs::write(out_dir.join("glyph_versions.txt"), versions).expect("write the version line");

    std::fs::write(
        out_dir.join("builtin_icon_table.rs"),
        generate_icons(&icon_source),
    )
    .expect("write the icon table");
}

/// The C++ string literals between `{{` and `}}` of a named array.
fn array_body<'a>(source: &'a str, declaration: &str) -> &'a str {
    let start = source
        .find(declaration)
        .unwrap_or_else(|| panic!("the glyph table no longer declares `{declaration}`"));
    let after = &source[start..];
    let open = after
        .find("{{")
        .unwrap_or_else(|| panic!("`{declaration}` is not an aggregate initialiser"));
    let body = &after[open + 2..];
    let close = body
        .find("}};")
        .unwrap_or_else(|| panic!("`{declaration}`'s initialiser is not closed"));
    &body[..close]
}

/// Every `"..."` literal in `text`, unescaped.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some((_, c)) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some((_, c)) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next().map(|(_, c)| c) {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('r') => value.push('\r'),
                    Some('0') => value.push('\0'),
                    Some(other) => value.push(other),
                    None => break,
                },
                other => value.push(other),
            }
        }
        out.push(value);
    }
    out
}

/// A parsed enum's variants, in declaration order.
fn enum_variants(header: &str, name: &str) -> Vec<String> {
    let start = header
        .find(&format!("enum class {name} :"))
        .unwrap_or_else(|| panic!("the glyph header no longer declares `enum class {name}`"));
    let body = header[start..]
        .split_once('{')
        .expect("the enum has a body")
        .1
        .split_once('}')
        .expect("the enum body is closed")
        .0;

    body.split(',')
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && !v.starts_with("//"))
        .collect()
}

fn generate(source: &str, header: &str) -> String {
    let kinds = enum_variants(header, "Kind");
    let categories = enum_variants(header, "Category");

    let keywords = string_literals(array_body(source, "kw_pool = "));
    assert!(
        keywords.len() > 10_000,
        "parsed only {} keywords; the table's shape has changed",
        keywords.len()
    );

    let mut items = String::new();
    let mut count = 0usize;
    for entry in split_entries(array_body(source, "g_items = ")) {
        let strings = string_literals(&entry);
        assert!(
            strings.len() >= 2,
            "an item with no character and name: {entry}"
        );
        let character = &strings[0];
        let name = &strings[1];

        let (offset, len) = keyword_span(&entry);
        let kind = qualified(&entry, "Kind::", &kinds);
        let category = qualified(&entry, "Category::", &categories);
        let skinnable = entry.contains("true");

        let _ = write!(
            items,
            "Glyph {{ character: {character:?}, name: {name:?}, keywords: {offset}..{}, \
             kind: Kind::{kind}, category: Category::{category}, skinnable: {skinnable} }}, ",
            offset + len
        );
        count += 1;
    }
    assert!(
        count > 4_000,
        "parsed only {count} glyphs; the table's shape has changed"
    );

    let mut sections = String::new();
    let mut section_count = 0usize;
    for entry in split_entries(array_body(source, "g_sections = ")) {
        let label = string_literals(&entry)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("a section with no label: {entry}"));
        let (offset, len) = item_span(&entry);
        let kind = qualified(&entry, "Kind::", &kinds);
        let category = qualified(&entry, "Category::", &categories);

        let _ = write!(
            sections,
            "Section {{ category: Category::{category}, kind: Kind::{kind}, \
             label: {label:?}, members: {offset}..{} }}, ",
            offset + len
        );
        section_count += 1;
    }
    assert!(
        section_count == categories.len(),
        "{section_count} sections for {} categories",
        categories.len()
    );

    // Documented, because the workspace denies missing docs and a generated
    // file is not exempt. The label is the one the C++ `categoryLabel` gives.
    let kind_variants: String = kinds
        .iter()
        .map(|k| format!("/// `Kind::{k}`, as the generated table spells it.\n    {k},\n    "))
        .collect::<Vec<_>>()
        .concat();
    let category_variants: String = categories
        .iter()
        .map(|c| format!("/// `Category::{c}`, as the generated table spells it.\n    {c},\n    "))
        .collect::<Vec<_>>()
        .concat();
    let keyword_literals: String = keywords
        .iter()
        .map(|k| format!("{k:?}, "))
        .collect::<Vec<_>>()
        .concat();

    format!(
        "// Generated by build.rs from crates/compass-core/glyph/glyph.cpp -- do not edit.\n\
         /// Which table an entry came from.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n\
         pub enum Kind {{\n    {kind_variants}}}\n\n\
         /// The section an entry belongs to.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n\
         pub enum Category {{\n    {category_variants}}}\n\n\
         pub(crate) static KEYWORDS: &[&str] = &[{keyword_literals}];\n\n\
         pub(crate) static GLYPHS: &[Glyph] = &[{items}];\n\n\
         pub(crate) static SECTIONS: &[Section] = &[{sections}];\n"
    )
}

/// Splits an aggregate initialiser into its top-level `{...}` entries.
fn split_entries(body: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;

    for c in body.chars() {
        if in_string {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                current.push(c);
            }
            '{' => {
                depth += 1;
                if depth > 1 {
                    current.push(c);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    entries.push(std::mem::take(&mut current));
                } else {
                    current.push(c);
                }
            }
            _ if depth > 0 => current.push(c),
            _ => {}
        }
    }
    entries
}

/// `{kw_pool.data() + 12, 3}` → `(12, 3)`.
fn keyword_span(entry: &str) -> (usize, usize) {
    span_after(entry, "kw_pool.data() + ")
}

/// `{g_items.data() + 0, 171}` → `(0, 171)`.
fn item_span(entry: &str) -> (usize, usize) {
    span_after(entry, "g_items.data() + ")
}

fn span_after(entry: &str, needle: &str) -> (usize, usize) {
    let at = entry
        .find(needle)
        .unwrap_or_else(|| panic!("no `{needle}` in {entry}"));
    let rest = &entry[at + needle.len()..];
    let (offset, tail) = rest
        .split_once(',')
        .unwrap_or_else(|| panic!("no span length in {entry}"));
    let len: String = tail
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (
        offset.trim().parse().expect("a span offset"),
        len.parse().expect("a span length"),
    )
}

/// The variant named after `prefix` in `entry`, checked against `known`.
fn qualified(entry: &str, prefix: &str, known: &[String]) -> String {
    let at = entry
        .find(prefix)
        .unwrap_or_else(|| panic!("no `{prefix}` in {entry}"));
    let name: String = entry[at + prefix.len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    assert!(
        known.contains(&name),
        "`{prefix}{name}` is not one of {known:?}"
    );
    name
}

/// The icon names, in order, out of the `Icon` enum extensions import from
/// `@vicinae/api`: one `Name = "name",` line per icon. `scripts/generate-icons.js`
/// writes that enum from the SVGs in `extra/builtin-icons`.
fn generate_icons(source: &str) -> String {
    let mut names = Vec::new();
    for line in source.lines() {
        let Some((_, value)) = line.split_once(" = \"") else {
            continue;
        };
        let Some((literal, _)) = value.split_once('"') else {
            continue;
        };
        names.push(literal.to_owned());
    }

    assert!(
        names.len() > 800,
        "parsed only {} icon names; the mapping's shape has changed",
        names.len()
    );

    let literals: String = names
        .iter()
        .map(|n| format!("{n:?}, "))
        .collect::<Vec<_>>()
        .concat();

    format!(
        "// Generated by build.rs from src/typescript/api/src/api/icon.ts.\n\
         pub(crate) static ICON_NAMES: &[&str] = &[{literals}];\n"
    )
}
