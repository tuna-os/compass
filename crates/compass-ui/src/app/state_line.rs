//! The launcher's one-line account of what it shows, logged after every
//! message; see [`LauncherApp::state_line`] for why it exists.

use super::{AppItem, LauncherApp, Page, RootRow, release_check};

impl LauncherApp {
    /// One line saying what the launcher is showing, for a log.
    ///
    /// # Why this exists, and why it is a function rather than a `tracing!`
    ///
    /// The VM tier asserts what the launcher did by counting changed pixels.
    /// That is the only way to prove something reached the screen, and it is
    /// terrible at everything else: "5.44% of pixels in a box changed" cannot
    /// say *which* row is selected, what the query matched, or what the panel
    /// contains, and it moves with the font, the theme and the card geometry.
    /// Two of this tier's runs were spent on a containment box that had gone
    /// stale, asserting nothing about the launcher at all.
    ///
    /// So the state machine says what it did, in a line a grep can check
    /// exactly, and the pixels go back to proving only the thing they are
    /// uniquely good for: that a window was drawn.
    ///
    /// It is a pure function returning a `String` rather than a `tracing`
    /// macro at the call site so that its content is unit-testable. A log line
    /// asserted only inside a 25-minute VM run is a log line nobody can
    /// control.
    ///
    /// The shape is `key=value`, space separated, with titles quoted, because
    /// that is what survives being grepped out of a log interleaved with
    /// wgpu's debug output.
    #[must_use]
    pub fn state_line(&self) -> String {
        let mut line = format!(
            "query={:?} results={} selected={}",
            self.query,
            self.results.len(),
            self.selected
        );
        // The title, not just the index: an index is only meaningful against a
        // corpus the reader cannot see, and "selected=0" is equally true of a
        // right and a wrong first row.
        match self.selected_row() {
            Some(RootRow::App(_)) => {
                let name = self.selected_item().map_or("", AppItem::name);
                line.push_str(&format!(" selected_title={name:?}"));
            }
            Some(RootRow::Command(command)) => {
                line.push_str(&format!(" selected_title={:?}", command.title));
            }
            Some(RootRow::Extension(index)) => {
                let title = self
                    .app_index
                    .extensions()
                    .get(index)
                    .map_or("", |command| command.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Calculator) => {
                let answer = self.calculator.as_ref().map_or("", |a| a.answer.as_str());
                line.push_str(&format!(" selected_title={answer:?}"));
            }
            Some(RootRow::Update) => {
                let title = self
                    .update
                    .as_ref()
                    .map(release_check::title)
                    .unwrap_or_default();
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Shortcut(index)) => {
                let title = self
                    .app_index
                    .shortcuts()
                    .get(index)
                    .map_or("", crate::shortcuts_page::display_name);
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Script(index)) => {
                let title = self
                    .app_index
                    .scripts()
                    .get(index)
                    .map_or("", |script| script.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::RhaiScript(index)) => {
                let title = self
                    .app_index
                    .rhai_scripts()
                    .get(index)
                    .map_or("", |script| script.title.as_str());
                line.push_str(&format!(" selected_title={title:?}"));
            }
            Some(RootRow::Fallback(fallback)) => {
                let title = self.fallback_title(fallback).unwrap_or_default();
                line.push_str(&format!(" selected_title={title:?} fallback"));
            }
            None => line.push_str(" selected_title=none"),
        }
        if let Page::Themes(page) = &self.page {
            line.push_str(&format!(
                " page=themes themes_query={:?} themes_rows={} themes_selected={} theme={}",
                page.query,
                page.rows.len(),
                page.selected,
                self.view.theme_choice.name()
            ));
        }
        if let Page::Dmenu(page) = &self.page {
            line.push_str(&format!(
                " page=dmenu dmenu_token={} dmenu_query={:?} dmenu_shown={} dmenu_selected={}",
                page.token,
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Programs(page) = &self.page {
            line.push_str(&format!(
                " page=programs programs_query={:?} programs_rows={} programs_selected={}",
                page.query,
                page.rows.len(),
                page.selected
            ));
        }
        if let Page::ScriptOutput(page) = &self.page {
            line.push_str(&format!(
                " page=script_output script_session={} script_finished={} script_exit={:?}",
                page.session, page.state.finished, page.state.exit_code
            ));
        }
        if let Page::Snippets(page) = &self.page {
            line.push_str(&format!(
                " page=snippets snippets_query={:?} snippets_shown={} snippets_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Shortcuts(page) = &self.page {
            line.push_str(&format!(
                " page=shortcuts shortcuts_query={:?} shortcuts_shown={} shortcuts_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Windows(page) = &self.page {
            line.push_str(&format!(
                " page=windows windows_query={:?} windows_shown={} windows_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Workspaces(page) = &self.page {
            line.push_str(&format!(
                " page=workspaces workspaces_query={:?} workspaces_shown={} workspaces_selected={}",
                page.query,
                page.shown.len(),
                page.selected
            ));
        }
        if let Page::Clipboard(page) = &self.page {
            line.push_str(&format!(
                " page=clipboard clipboard_query={:?} clipboard_rows={} clipboard_selected={}",
                page.query,
                page.rows.len(),
                page.selected
            ));
        }
        if let Page::Files(page) = &self.page {
            line.push_str(&format!(
                " page=files files_query={:?} files_heading={:?} files_rows={} files_selected={}",
                page.query,
                page.heading,
                page.rows.len(),
                page.selected
            ));
        }
        match &self.panel {
            None => line.push_str(" panel=closed"),
            Some(panel) => {
                line.push_str(&format!(
                    " panel=open panel_filter={:?} panel_rows={} panel_selected={}",
                    panel.filter,
                    panel.rows.len(),
                    panel.selected
                ));
                let title = usize::try_from(panel.selected)
                    .ok()
                    .and_then(|index| panel.rows.get(index))
                    .and_then(|row| {
                        let action = row.action?;
                        Some(
                            panel
                                .sections
                                .get(row.section)?
                                .actions
                                .get(action)?
                                .title
                                .as_str(),
                        )
                    });
                match title {
                    Some(title) => line.push_str(&format!(" panel_title={title:?}")),
                    None => line.push_str(" panel_title=none"),
                }
            }
        }
        if let Some(error) = &self.error {
            line.push_str(&format!(" error={error:?}"));
        }
        line.push_str(if self.window.is_some() {
            " window=open"
        } else {
            " window=hidden"
        });
        line
    }
}
