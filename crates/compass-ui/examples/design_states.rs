//! Dumps the design tokens and a set of launcher states as JSON.
//!
//! `tools/design/` renders what this prints. The states are built from the
//! *real* view models -- `root_list::build` and `action_panel::flatten`, the
//! same functions the Iced app calls -- so the browser page cannot drift from
//! the ported logic. Only the painting is mirrored.
//!
//! Run: `cargo run -p compass-ui --example design_states > tools/design/states.json`

use compass_core::root_items::{RootItem, RootItemMeta};
use compass_ui::action_panel::{self, Action, PanelSection};
use compass_ui::design::{self, Appearance};
use compass_ui::preset;
use compass_ui::root_list;

/// A fixed clock, so frecency ordering is the same on every run and a
/// screenshot diff means a design change rather than the time of day.
const NOW: i64 = 1_700_000_000;

/// `RootItem` carries no icon -- the real launcher resolves one through the
/// icon theme, which a browser cannot do. The surrogate is handed an icon
/// *name* alongside the item and draws a glyph for it, which is enough to
/// judge the row's proportions and is labelled as a stand-in on the page.
fn app(id: &str, title: &str, subtitle: &str, icon: &str) -> (RootItem, String) {
    let item = RootItem {
        id: id.to_owned(),
        title: title.to_owned(),
        subtitle: subtitle.to_owned(),
        meta: RootItemMeta {
            // `RootItemMeta::default()` leaves this false, and the search
            // drops disabled items -- so a fixture built from `..default()`
            // matches nothing at all. The design page showed "No results"
            // for a query with three obvious matches, which is how this was
            // found.
            enabled: true,
            provider_id: "applications".to_owned(),
            ..RootItemMeta::default()
        },
        ..RootItem::default()
    };
    (item, icon.to_owned())
}

fn corpus() -> Vec<(RootItem, String)> {
    vec![
        app("firefox", "Firefox", "Browse the web", "web-browser"),
        app("files", "Files", "Access and organize files", "folder"),
        app(
            "terminal",
            "Terminal",
            "Use the command line",
            "utilities-terminal",
        ),
        app(
            "settings",
            "Settings",
            "Configure the system",
            "preferences-system",
        ),
        app(
            "text-editor",
            "Text Editor",
            "Edit text files",
            "accessories-text-editor",
        ),
        app(
            "calculator",
            "Calculator",
            "Perform calculations",
            "accessories-calculator",
        ),
        app(
            "software",
            "Software",
            "Add or remove applications",
            "system-software-install",
        ),
        app(
            "firewall",
            "Firewall",
            "Configure the firewall",
            "security-high",
        ),
    ]
}

fn panel_sections() -> Vec<PanelSection> {
    vec![
        PanelSection {
            name: String::new(),
            actions: vec![
                Action::new("Open").with_shortcut("Enter"),
                Action::new("Open in New Window").with_shortcut("Shift+Enter"),
            ],
        },
        PanelSection {
            name: "Copy".to_owned(),
            actions: vec![
                Action::new("Copy Name").with_shortcut("Ctrl+C"),
                Action::new("Copy Path").with_shortcut("Ctrl+Shift+C"),
            ],
        },
        PanelSection {
            name: "Manage".to_owned(),
            actions: vec![
                Action::new("Add to Favorites").with_shortcut("Ctrl+D"),
                Action::new("Hide Application"),
            ],
        },
    ]
}

/// Escapes a string for JSON.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list_state(name: &str, description: &str, query: &str, selected: usize) -> String {
    list_state_with_corpus(name, description, query, selected, &corpus())
}

fn list_state_with_corpus(
    name: &str,
    description: &str,
    query: &str,
    selected: usize,
    corpus: &[(RootItem, String)],
) -> String {
    let items: Vec<RootItem> = corpus.iter().map(|(item, _)| item.clone()).collect();
    let list = root_list::build(&items, query, &[], NOW);

    let mut sections = Vec::new();
    let mut position = 0usize;
    for section in &list.sections {
        let mut rows = Vec::new();
        for index in &section.rows {
            let (item, icon) = &corpus[*index];
            rows.push(format!(
                r#"{{"title":{},"subtitle":{},"icon":{},"selected":{}}}"#,
                quote(&item.title),
                quote(&item.subtitle),
                quote(icon),
                position == selected
            ));
            position += 1;
        }
        sections.push(format!(
            r#"{{"heading":{},"rows":[{}]}}"#,
            quote(section.kind.heading()),
            rows.join(",")
        ));
    }

    format!(
        r#"{{"name":{},"description":{},"kind":"list","query":{},"sections":[{}],"panel":null}}"#,
        quote(name),
        quote(description),
        quote(query),
        sections.join(",")
    )
}

/// A panel as it looks *when it opens*, which is the only way it is drawn here.
///
/// The selection is asked of `action_panel::selection_after_filter` rather
/// than passed in. It used to be a hard-coded `1`, which drew the highlight on
/// "Open in New Window" -- a state the launcher never opens in, since the
/// selection goes to the first selectable row. The VM tier's own frame shows
/// the caret on "Open", so the surrogate was the thing that was wrong, and a
/// surrogate that disagrees with the real launcher about which row is selected
/// is worse than no surrogate.
///
/// A future state showing the selection moved should call `next_selectable`
/// for the same reason: the answer comes from the ported logic, never from a
/// number typed here.
fn panel_state(name: &str, description: &str, filter: &str) -> String {
    let sections = panel_sections();
    let rows = action_panel::flatten(&sections, filter);
    let selected = action_panel::selection_after_filter(&rows);

    let drawn: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let kind = match row.kind {
                action_panel::RowKind::Divider => "divider",
                action_panel::RowKind::Header => "header",
                action_panel::RowKind::Item => "item",
            };
            let (title, shortcut) = match (row.kind, row.action) {
                (action_panel::RowKind::Header, _) => {
                    (sections[row.section].name.clone(), String::new())
                }
                (action_panel::RowKind::Item, Some(action)) => {
                    let a = &sections[row.section].actions[action];
                    (a.title.clone(), a.shortcut.clone().unwrap_or_default())
                }
                _ => (String::new(), String::new()),
            };
            format!(
                r#"{{"kind":{},"title":{},"shortcut":{},"selected":{}}}"#,
                quote(kind),
                quote(&title),
                quote(&shortcut),
                isize::try_from(index).unwrap_or(isize::MAX) == selected
            )
        })
        .collect();

    // The list behind the panel is the same one the launcher was showing.
    let behind = list_state("", "", "fi", 0);
    let behind_sections = behind
        .split_once(r#""sections":"#)
        .and_then(|(_, rest)| rest.rsplit_once(r#","panel""#))
        .map_or_else(|| "[]".to_owned(), |(s, _)| s.to_owned());

    format!(
        r#"{{"name":{},"description":{},"kind":"panel","query":{},"sections":{},"panel":{{"filter":{},"rows":[{}]}}}}"#,
        quote(name),
        quote(description),
        quote("fi"),
        behind_sections,
        quote(filter),
        drawn.join(",")
    )
}

fn main() {
    let long_corpus: Vec<_> = (0..40)
        .map(|i| {
            app(
                &format!("application-{i:02}"),
                &format!("Application {i:02}"),
                "",
                "application",
            )
        })
        .collect();
    let states = [
        list_state_with_corpus(
            "long-results",
            "Forty results; the query stays above the scrollable list.",
            "Application",
            0,
            &long_corpus,
        ),
        list_state_with_corpus(
            "long-results-last",
            "The last application selected; scrolling reveals it without moving the query.",
            "Application",
            39,
            &long_corpus,
        ),
        list_state(
            "empty",
            "Nothing typed yet: the resting state, and the one most people see most often.",
            "",
            0,
        ),
        list_state(
            "typing",
            "A query with several matches, the first selected.",
            "fi",
            0,
        ),
        list_state(
            "selection-moved",
            "The same query with the selection two rows down, to check the highlight against a subtitle.",
            "fi",
            1,
        ),
        list_state("no-results", "A query nothing matches.", "zzzz", 0),
        panel_state(
            "panel",
            "The action panel over the list, opened with Ctrl+B.",
            "",
        ),
        panel_state(
            "panel-filtered",
            "The panel with a filter typed, which hides whole sections.",
            "copy",
        ),
        panel_state(
            "panel-no-actions",
            "The focused action filter has no matches. The preview field is read-only; choose a state to change its Rust-filtered rows.",
            "zzzz",
        ),
    ];

    let appearances: Vec<String> = Appearance::ALL
        .iter()
        .map(|appearance| {
            let p = design::palette(*appearance);
            format!(
                r#"{{"name":{},"surface":{},"field":{},"text":{},"muted":{},"selection":{},"selectionText":{},"border":{},"accent":{},"backdrop":{},"backdropAlpha":{}}}"#,
                quote(appearance.name()),
                quote(&p.surface.hex()),
                quote(&p.field.hex()),
                quote(&p.text.hex()),
                quote(&p.muted.hex()),
                quote(&p.selection.hex()),
                quote(&p.selection_text.hex()),
                quote(&p.border.hex()),
                quote(&p.accent.hex()),
                quote(&p.backdrop.hex()),
                p.backdrop_alpha
            )
        })
        .collect();

    let g = design::GEOMETRY;
    let fonts: Vec<String> = design::FONT_STACK.iter().map(|f| quote(f)).collect();

    // The appearance presets (#84), each with the geometry it resolves to.
    //
    // Emitted from `preset::resolve` rather than transcribed, for the reason
    // the whole surrogate exists: a picture drawn from a second copy of the
    // numbers shows what someone believed the presets were, not what they are.
    let presets: Vec<String> = preset::NAMES
        .iter()
        .map(|(name, _)| {
            let r = preset::resolve(Some(name), None, None);
            format!(
                r#"{{"name":{},"icons":{},"fieldRule":{},"subtitles":{},"geometry":{}}}"#,
                quote(name),
                r.icons,
                r.field_rule,
                r.subtitles,
                geometry_json(&r.geometry)
            )
        })
        .collect();

    println!(
        r#"{{"generatedBy":"cargo run -p compass-ui --example design_states","panelMetrics":{{{}}},"geometry":{},"presets":[{}],"fontStack":[{}],"appearances":[{}],"states":[{}]}}"#,
        design::PANEL_METRICS
            .iter()
            .map(|(name, value)| format!("{}:{value}", quote(name)))
            .collect::<Vec<_>>()
            .join(","),
        geometry_json(&g),
        presets.join(","),
        fonts.join(","),
        appearances.join(","),
        states.join(",")
    );
}

/// A `Geometry` as the surrogate's JSON object.
///
/// One function rather than two format strings, so the default geometry and a
/// preset's cannot describe themselves with different key sets -- which would
/// make the browser silently fall back on whichever keys it happened to find.
fn geometry_json(g: &design::Geometry) -> String {
    format!(
        r#"{{"cardWidth":{},"cardMaxHeight":{},"cardRadius":{},"cardTopFraction":{},"cardPadding":{},"fieldHeight":{},"fieldRadius":{},"rowHeight":{},"rowRadius":{},"rowSpacing":{},"iconSize":{},"titleSize":{},"subtitleSize":{},"headingSize":{},"querySize":{}}}"#,
        g.card_width,
        g.card_max_height,
        g.card_radius,
        g.card_top_fraction,
        g.card_padding,
        g.field_height,
        g.field_radius,
        g.row_height,
        g.row_radius,
        g.row_spacing,
        g.icon_size,
        g.title_size,
        g.subtitle_size,
        g.heading_size,
        g.query_size
    )
}
