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

struct RevealSelection {
    scroll_id: &'static str,
    selection_id: &'static str,
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
        let top = selected.y - viewport.y;
        let bottom = top + selected.height;
        let next = if top < offset || selected.height > viewport.height {
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

fn reveal_selection<T>(scroll_id: &'static str, selection_id: &'static str) -> iced::Task<T> {
    iced_winit::runtime::task::effect(iced_winit::runtime::Action::widget(RevealSelection {
        scroll_id,
        selection_id,
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
