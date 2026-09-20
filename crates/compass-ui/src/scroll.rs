use iced_winit::core::{
    Rectangle, Vector,
    widget::{
        Id, Operation,
        operation::{Outcome, Scrollable, scrollable},
    },
};

pub(crate) const PANEL_RESULTS: &str = "panel-results";
pub(crate) const PANEL_SELECTION: &str = "panel-selection";

#[derive(Default)]
struct RevealSelection {
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
        if id == Some(&Id::new(PANEL_RESULTS)) {
            self.viewport = Some((bounds, translation.y));
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&Id::new(PANEL_SELECTION)) {
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
            Id::new(PANEL_RESULTS),
            scrollable::AbsoluteOffset {
                x: None,
                y: Some(next.max(0.0)),
            },
        )))
    }
}

pub(crate) fn reveal_panel_selection<T>() -> iced::Task<T> {
    iced_winit::runtime::task::effect(iced_winit::runtime::Action::widget(
        RevealSelection::default(),
    ))
}
