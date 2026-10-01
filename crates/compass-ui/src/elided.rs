//! One line of text that ends in an ellipsis when it does not fit.
//!
//! A list row is a fixed height, and Iced's `text` wraps: a long subtitle
//! (LibreOffice's comment, an extension's description) took a second line
//! and ran into the row below (P-07). Iced 0.14 can turn wrapping off but has
//! no ellipsis, and a line cut off at the edge reads as a rendering fault, so
//! this widget measures the line with the renderer's own shaping at layout,
//! when the width is known, and draws the longest prefix that fits with `…`.
//!
//! The decision of where to cut is [`elide`], which takes the measurement as
//! a function so it can be tested without a renderer.

use iced::{Color, Element, Length, Pixels, Rectangle, Size};
use iced_core::layout::{self, Layout};
use iced_core::renderer;
use iced_core::text::{self, Paragraph as _};
use iced_core::widget::{self, Tree, Widget};
use iced_core::{Text, mouse};

/// The mark a cut line ends in.
pub const ELLIPSIS: char = '…';

/// The longest prefix of `content` that, with [`ELLIPSIS`] after it, is at
/// most `width` wide as `measure` measures it; `content` itself when it fits.
/// The prefix ends on a character boundary and loses trailing spaces, so the
/// mark sits against the last word. When not even the mark fits, the mark.
#[must_use]
pub fn elide(content: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(content) <= width {
        return content.to_owned();
    }
    let boundaries: Vec<usize> = content.char_indices().map(|(index, _)| index).collect();
    let cut = |count: usize| {
        let end = boundaries.get(count).copied().unwrap_or(content.len());
        let mut line = content[..end].trim_end().to_owned();
        line.push(ELLIPSIS);
        line
    };
    // The most characters whose cut still fits, by bisection: `low` fits
    // (zero characters is the bare mark, accepted whatever its width), and
    // everything above `high` does not.
    let (mut low, mut high) = (0, boundaries.len().saturating_sub(1));
    while low < high {
        let middle = (low + high).div_ceil(2);
        if measure(&cut(middle)) <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    cut(low)
}

/// A single line of `content` in `size` and `color`, elided to its width.
pub fn elided<'a, Message, Theme, Renderer>(
    content: impl Into<String>,
    size: f32,
    font: Option<Renderer::Font>,
    color: Color,
) -> Element<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer + 'a,
{
    Element::new(Elided::<Renderer> {
        content: content.into(),
        size: Pixels(size),
        font,
        color,
    })
}

struct Elided<Renderer: text::Renderer> {
    content: String,
    size: Pixels,
    font: Option<Renderer::Font>,
    color: Color,
}

/// What the last layout decided, kept so an unchanged line at an unchanged
/// width is not shaped again on every relayout.
struct State<P: text::Paragraph> {
    paragraph: text::paragraph::Plain<P>,
    content: String,
    width: f32,
}

impl<P: text::Paragraph> Default for State<P> {
    fn default() -> Self {
        Self {
            paragraph: text::paragraph::Plain::default(),
            content: String::new(),
            width: -1.0,
        }
    }
}

impl<Renderer: text::Renderer> Elided<Renderer> {
    fn text<'b>(
        &self,
        content: &'b str,
        bounds: Size,
        renderer: &Renderer,
    ) -> Text<&'b str, Renderer::Font> {
        Text {
            content,
            bounds,
            size: self.size,
            line_height: text::LineHeight::default(),
            font: self.font.unwrap_or_else(|| renderer.default_font()),
            align_x: text::Alignment::Default,
            align_y: iced::alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for Elided<Renderer>
where
    Renderer: text::Renderer,
{
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::<Renderer::Paragraph>::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer::Paragraph>>();
        layout::sized(limits, Length::Fill, Length::Shrink, |limits| {
            let bounds = limits.max();
            if state.content != self.content || state.width != bounds.width {
                let unbounded = Size::new(f32::INFINITY, f32::INFINITY);
                let shown = elide(&self.content, bounds.width, |line| {
                    Renderer::Paragraph::with_text(self.text(line, unbounded, renderer)).min_width()
                });
                let _ = state.paragraph.update(self.text(&shown, bounds, renderer));
                state.content.clone_from(&self.content);
                state.width = bounds.width;
            }
            state.paragraph.min_bounds()
        })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let bounds = layout.bounds();
        renderer.fill_paragraph(
            state.paragraph.raw(),
            bounds.position(),
            self.color,
            *viewport,
        );
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        // The whole line, so a find by text matches what the row says.
        operation.text(None, layout.bounds(), &self.content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten pixels a character, the mark included.
    fn monospace(line: &str) -> f32 {
        line.chars().count() as f32 * 10.0
    }

    #[test]
    fn a_line_that_fits_is_left_alone() {
        assert_eq!(elide("Vim", 100.0, monospace), "Vim");
        assert_eq!(elide("exactly ten", 110.0, monospace), "exactly ten");
    }

    #[test]
    fn a_long_line_keeps_what_fits_and_ends_in_the_mark() {
        let line = "Launch applications to create text documents";
        let shown = elide(line, 200.0, monospace);
        assert_eq!(shown, "Launch applications…");
        assert!(monospace(&shown) <= 200.0);
        // One more character would not have fitted.
        assert!(monospace("Launch applications t…") > 200.0);
    }

    #[test]
    fn the_mark_sits_against_the_last_word() {
        assert_eq!(elide("Edit text files", 60.0, monospace), "Edit…");
    }

    #[test]
    fn characters_are_never_split() {
        let shown = elide("ééééééééé", 50.0, monospace);
        assert_eq!(shown, "éééé…");
    }

    #[test]
    fn with_no_room_the_mark_alone() {
        assert_eq!(elide("Anything", 3.0, monospace), "…");
        assert_eq!(elide("", 0.0, monospace), "");
    }
}
