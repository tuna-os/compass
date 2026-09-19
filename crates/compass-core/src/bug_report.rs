//! The bug-report link and the fallback manager.
//!
//! Ports `src/server/src/builtins/vicinae/bug-report-url.hpp` and
//! `manage-fallback-*.cpp` — the pre-filled issue the Report Bug command
//! opens, and the two lists of commands that can answer a search nothing else
//! claimed.

/// Where an issue about the launcher is filed.
pub const CREATE_ISSUE_URL: &str = "https://github.com/vicinaehq/vicinae/issues/new";

/// Where an issue about an extension is filed.
///
/// A different repository *and* a different path — `/issues/new/choose`
/// rather than `/issues/new` — because that repository offers templates and
/// the launcher's does not. Sending an extension report to the launcher's
/// tracker would file it against the wrong project.
pub const CREATE_EXTENSION_ISSUE_URL: &str =
    "https://github.com/vicinaehq/extensions/issues/new/choose";

/// The label GitHub applies to a report opened this way.
pub const ISSUE_TYPE: &str = "bug";

/// The body a bug report is pre-filled with.
///
/// Copied out of the C++ raw string literal mechanically rather than retyped.
/// The seven fields are the ones nobody remembers to include and everybody is
/// asked for, so pre-filling them is most of what this command is for.
pub const ISSUE_TEMPLATE: &str = r##"**System information**

- Version: {version} ({commit})
- Build info: {build_info}
- Provenance: {provenance}
- OS: {os}
- QT Platform: {qt_platform}
- DE: {desktop}

**Describe the bug**

A clear and concise description of what the bug is.

**To Reproduce**

Steps to reproduce the behavior.

**Expected behavior**

A clear and concise description of what you expected to happen.

**Screenshots**

If applicable, add screenshots to help explain your problem.

**Additional context**

Add any other context about the problem here.
"##;

/// What the launcher knows about itself, for a bug report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemInfo {
    /// The released version.
    pub version: String,
    /// The commit it was built from.
    pub commit: String,
    /// How it was built.
    pub build_info: String,
    /// Where the build came from.
    pub provenance: String,
    /// The operating system.
    pub os: String,
    /// Which Qt platform plugin is in use.
    pub qt_platform: String,
    /// Which desktop environment.
    pub desktop: String,
}

/// How the operating system is described.
///
/// `/etc/os-release` when it can be read — which gives the distribution's own
/// name and version, the thing a maintainer actually needs — and the kernel's
/// product name with the CPU architecture otherwise. The two formats differ:
/// a dash between the os-release fields, parentheses around the architecture.
#[must_use]
pub fn os_description(
    os_release: Option<(&str, &str)>,
    product_name: &str,
    architecture: &str,
) -> String {
    match os_release {
        Some((pretty_name, version)) => format!("{pretty_name} - {version}"),
        None => format!("{product_name} ({architecture})"),
    }
}

/// Fill the issue template in.
#[must_use]
pub fn issue_body(info: &SystemInfo) -> String {
    ISSUE_TEMPLATE
        .replace("{version}", &info.version)
        .replace("{commit}", &info.commit)
        .replace("{build_info}", &info.build_info)
        .replace("{provenance}", &info.provenance)
        .replace("{os}", &info.os)
        .replace("{qt_platform}", &info.qt_platform)
        .replace("{desktop}", &info.desktop)
}

/// The query parameters of a bug-report link, in the order they are added.
///
/// A title is included only when one was given: an empty `title=` would leave
/// GitHub's own placeholder unused and the field looking filled in.
#[must_use]
pub fn issue_query(title: Option<&str>, body: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(title) = title.filter(|t| !t.is_empty()) {
        out.push(("title", title.to_owned()));
    }
    out.push(("body", body.to_owned()));
    out.push(("type", ISSUE_TYPE.to_owned()));
    out
}

/// Where a command sits in the fallback manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackSection {
    /// Already a fallback.
    Enabled,
    /// Could be one.
    Available,
}

/// Split the searchable commands into the two lists.
///
/// A command that is not suitable as a fallback appears in **neither** — the
/// manager is not a list of everything with a switch next to it, and showing
/// commands that cannot be turned on would be showing switches that do
/// nothing.
#[must_use]
pub fn fallback_sections(
    items: &[(String, bool)],
    enabled_ids: &[String],
) -> Vec<(String, FallbackSection)> {
    items
        .iter()
        .filter(|(_, suitable)| *suitable)
        .map(|(id, _)| {
            let section = if enabled_ids.contains(id) {
                FallbackSection::Enabled
            } else {
                FallbackSection::Available
            };
            (id.clone(), section)
        })
        .collect()
}

/// Order the enabled list by the stored fallback order.
///
/// The enabled list is **not** sorted by relevance like the available one: a
/// fallback's position is what decides which of them answers a query first, so
/// showing them in any other order would show a ranking that is not the one in
/// force. Anything missing from the order sorts to the end, together, keeping
/// its relative order.
#[must_use]
pub fn order_enabled(ids: &[String], fallback_order: &[String]) -> Vec<String> {
    let mut out = ids.to_vec();
    let position = |id: &String| {
        fallback_order
            .iter()
            .position(|f| f == id)
            .unwrap_or(fallback_order.len())
    };
    out.sort_by_key(position);
    out
}

/// What each list's only action is called.
#[must_use]
pub const fn fallback_action_label(section: FallbackSection) -> &'static str {
    match section {
        FallbackSection::Enabled => "Disable fallback",
        FallbackSection::Available => "Enable fallback",
    }
}
