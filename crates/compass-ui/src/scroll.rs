use iced_winit::core::{
    Rectangle, Vector,
    widget::{
        Id, Operation,
        operation::{Outcome, Scrollable, scrollable},
    },
};

pub(crate) const PANEL_RESULTS: &str = "panel-results";
pub(crate) const PANEL_SELECTION: &str = "panel-selection";
pub(crate) const ROOT_RESULTS: &str = "root-results";
pub(crate) const ROOT_SELECTION: &str = "root-selection";
/// The settings page's scrollable body.
pub(crate) const SETTINGS_BODY: &str = "settings-body";
/// The control on a settings or onboarding page that has the keyboard.
pub(crate) const SETTINGS_FOCUS: &str = "settings-focus";
/// The settings sidebar's scrollable list.
pub(crate) const SETTINGS_SIDEBAR: &str = "settings-sidebar";
/// The selected row of the settings sidebar.
pub(crate) const SETTINGS_SIDEBAR_SELECTION: &str = "settings-sidebar-selection";

struct RevealSelection {
    scroll_id: &'static str,
    selection_id: &'static str,
    /// Room kept between the target and the viewport's edge.
    margin: f32,
    viewport: Option<(Rectangle, f32)>,
    selected: Option<Rectangle>,
}

impl Operation for RevealSelection {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _: Rectangle,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new(self.scroll_id)) {
            self.viewport = Some((bounds, translation.y));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&Id::new(self.selection_id)) {
            self.selected = Some(bounds);
        }
    }

    fn finish(&self) -> Outcome<()> {
        let (Some((viewport, offset)), Some(selected)) = (self.viewport, self.selected) else {
            return Outcome::None;
        };
        let top = selected.y - viewport.y - self.margin;
        let bottom = selected.y - viewport.y + selected.height + self.margin;
        let next = if top < offset || bottom - top > viewport.height {
            top
        } else if bottom > offset + viewport.height {
            bottom - viewport.height
        } else {
            return Outcome::None;
        };
        Outcome::Chain(Box::new(scrollable::scroll_to(
            Id::new(self.scroll_id),
            scrollable::AbsoluteOffset {
                x: None,
                y: Some(next.max(0.0)),
            },
        )))
    }
}

pub(crate) fn reveal_panel_selection<T>() -> iced::Task<T> {
    reveal_selection(PANEL_RESULTS, PANEL_SELECTION)
}

pub(crate) fn reveal_root_selection<T>() -> iced::Task<T> {
    reveal_selection(ROOT_RESULTS, ROOT_SELECTION)
}

/// How far an arrow key scrolls a page that is read rather than picked
/// from: about two lines of body text.
pub(crate) const LINE_STEP: f32 = 40.0;
/// How far Page Up and Page Down scroll it: most of the card, so a line
/// at the edge stays in view.
pub(crate) const PAGE_STEP: f32 = 320.0;

/// What a reading key does to a page of text (a store listing, a Markdown
/// detail): Up and Down scroll by a line, Page Up and Page Down by a page,
/// Home and End to either end. `None` for any other key.
pub(crate) fn reading_key<T>(key: iced::keyboard::Key<&str>) -> Option<iced::Task<T>> {
    use iced::keyboard::{Key, key::Named};
    use iced::widget::operation::{AbsoluteOffset, RelativeOffset, scroll_by, snap_to};
    let by = |y: f32| scroll_by(ROOT_RESULTS, AbsoluteOffset { x: 0.0, y });
    Some(match key {
        Key::Named(Named::ArrowDown) => by(LINE_STEP),
        Key::Named(Named::ArrowUp) => by(-LINE_STEP),
        Key::Named(Named::PageDown) => by(PAGE_STEP),
        Key::Named(Named::PageUp) => by(-PAGE_STEP),
        Key::Named(Named::Home) => snap_to(ROOT_RESULTS, RelativeOffset::START),
        Key::Named(Named::End) => snap_to(ROOT_RESULTS, RelativeOffset::END),
        _ => return None,
    })
}

/// Scrolls the settings page so the control with the keyboard is in view.
pub(crate) fn reveal_settings_focus<T>() -> iced::Task<T> {
    reveal(SETTINGS_BODY, SETTINGS_FOCUS, 12.0)
}

/// The onboarding's theme list, opened from the keyboard.
pub(crate) const ONBOARDING_MENU: &str = "onboarding-menu";

/// Scrolls the onboarding's theme list to its highlighted theme.
pub(crate) fn reveal_onboarding_option<T>() -> iced::Task<T> {
    reveal(ONBOARDING_MENU, SETTINGS_FOCUS, 4.0)
}

/// Scrolls the settings sidebar so its selected row is in view.
pub(crate) fn reveal_settings_page<T>() -> iced::Task<T> {
    reveal(SETTINGS_SIDEBAR, SETTINGS_SIDEBAR_SELECTION, 8.0)
}

fn reveal_selection<T>(scroll_id: &'static str, selection_id: &'static str) -> iced::Task<T> {
    reveal(scroll_id, selection_id, 0.0)
}

fn reveal<T>(scroll_id: &'static str, selection_id: &'static str, margin: f32) -> iced::Task<T> {
    iced_winit::runtime::task::effect(iced_winit::runtime::Action::widget(RevealSelection {
        scroll_id,
        selection_id,
        margin,
        viewport: None,
        selected: None,
    }))
}

/// The launcher's scrollable: `ViciScrollBar`'s thin bar, 6 px and rounded,
/// floating over the content with no rail, in the text colour at a quarter
/// opacity (`SemanticColor::ScrollBarBackground`). iced's own default is a
/// 10 px square bar on a filled rail.
pub(crate) fn scrollable<'a, Message: 'a>(
    content: impl Into<iced::Element<'a, Message>>,
) -> iced::widget::Scrollable<'a, Message> {
    use iced::widget::scrollable::{Direction, Scrollbar};
    iced::widget::scrollable(content)
        .direction(Direction::Vertical(
            Scrollbar::new()
                .width(SCROLLBAR_WIDTH)
                .scroller_width(SCROLLBAR_WIDTH)
                .margin(SCROLLBAR_MARGIN),
        ))
        .style(scrollbar_style)
}

/// `ViciScrollBar`'s width.
const SCROLLBAR_WIDTH: f32 = 6.0;
/// Room between the bar and the edge of what scrolls.
const SCROLLBAR_MARGIN: f32 = 2.0;

/// How opaque the bar is: faint at rest, the C++'s quarter opacity while the
/// pointer is over the list, and stronger under the pointer or a drag. The
/// C++ also fades the bar out a moment after scrolling stops; iced's style
/// has no clock, so at rest it stays faintly visible instead.
fn scrollbar_alpha(status: iced::widget::scrollable::Status) -> f32 {
    use iced::widget::scrollable::Status;
    match status {
        Status::Active { .. } => 0.15,
        Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        }
        | Status::Dragged {
            is_vertical_scrollbar_dragged: true,
            ..
        } => 0.45,
        Status::Hovered { .. } | Status::Dragged { .. } => 0.25,
    }
}

/// The style of [`scrollable`].
pub(crate) fn scrollbar_style(
    theme: &iced::Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    use iced::widget::scrollable::{Rail, Scroller};
    let text = theme.palette().text;
    let rail = Rail {
        background: None,
        border: iced::Border::default(),
        scroller: Scroller {
            background: iced::Background::Color(iced::Color {
                a: scrollbar_alpha(status),
                ..text
            }),
            border: iced::Border::default().rounded(SCROLLBAR_WIDTH / 2.0),
        },
    };
    iced::widget::scrollable::Style {
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        ..iced::widget::scrollable::default(theme, status)
    }
}

#[cfg(test)]
mod scrollbar_tests {
    use super::*;
    use iced::widget::scrollable::Status;

    #[test]
    fn the_bar_is_the_cpps_thin_rounded_bar_with_no_rail() {
        let style = scrollbar_style(
            &iced::Theme::Dark,
            Status::Hovered {
                is_horizontal_scrollbar_hovered: false,
                is_vertical_scrollbar_hovered: false,
                is_horizontal_scrollbar_disabled: true,
                is_vertical_scrollbar_disabled: false,
            },
        );
        assert_eq!(style.vertical_rail.background, None, "no rail behind it");
        assert_eq!(
            style.vertical_rail.scroller.border.radius,
            iced::border::Radius::from(SCROLLBAR_WIDTH / 2.0),
            "rounded ends"
        );
        let iced::Background::Color(color) = style.vertical_rail.scroller.background else {
            panic!("a flat colour");
        };
        assert!(
            (color.a - 0.25).abs() < f32::EPSILON,
            "the C++'s quarter opacity"
        );
    }

    #[test]
    fn the_reading_keys_scroll_and_the_rest_do_not() {
        use iced::keyboard::{Key, key::Named};
        for named in [
            Named::ArrowDown,
            Named::ArrowUp,
            Named::PageDown,
            Named::PageUp,
            Named::Home,
            Named::End,
        ] {
            assert!(reading_key::<()>(Key::Named(named)).is_some(), "{named:?}");
        }
        assert!(reading_key::<()>(Key::Named(Named::Enter)).is_none());
        assert!(reading_key::<()>(Key::Character("j")).is_none());
    }

    #[test]
    fn the_bar_is_faint_at_rest_and_strong_under_the_pointer() {
        let rest = scrollbar_alpha(Status::Active {
            is_horizontal_scrollbar_disabled: true,
            is_vertical_scrollbar_disabled: false,
        });
        let over = scrollbar_alpha(Status::Hovered {
            is_horizontal_scrollbar_hovered: false,
            is_vertical_scrollbar_hovered: true,
            is_horizontal_scrollbar_disabled: true,
            is_vertical_scrollbar_disabled: false,
        });
        assert!(rest < over);
    }
}
