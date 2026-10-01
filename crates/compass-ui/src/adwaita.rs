//! Adwaita-style controls for the Settings page (#252) and the first-run
//! flow.
//!
//! Iced's built-in styles are its own: square, bordered buttons filled with
//! the primary colour, a switch with a dark knob, inputs with a hairline frame.
//! Next to GNOME's own preferences windows they look foreign. These follow
//! libadwaita 1.x instead: 6 px corners, flat neutral fills that are the text
//! colour at a low opacity, accent fills only where Adwaita uses them (the
//! suggested action, a switch that is on), a 2 px accent ring on a focused
//! entry, and rows grouped in boxed lists with 12 px corners and separators.
//!
//! Every colour comes from the [`Palette`], so light and dark and the user's
//! theme still apply: `selection` is Adwaita's `accent_bg_color` (fills) and
//! `selection_text` its `accent_fg_color`, while `accent` is `accent_color`,
//! the one for focus rings. The launcher's own rows keep their styles in
//! [`crate::design`]; nothing here is used outside Settings and onboarding.

use iced::widget::button as btn;
use iced::widget::{container, pick_list, rule, text_input, toggler};
use iced::{Background, Border, Color, Shadow};

use crate::design::{Palette, Rgb};

/// A button's and an entry's corner radius.
pub const CONTROL_RADIUS: f32 = 6.0;
/// The padding that makes a 13 px label a 34 px tall button or dropdown.
pub const CONTROL_PADDING: [f32; 2] = [8.5, 14.0];
/// The same height for an entry, with Adwaita's 9 px inset for its text.
pub const ENTRY_PADDING: [f32; 2] = [8.5, 9.0];
/// A switch's height; Iced draws it twice as wide.
pub const SWITCH_SIZE: f32 = 26.0;
/// A boxed list's corner radius.
pub const LIST_RADIUS: f32 = 12.0;
/// The padding inside a boxed list's row.
pub const ROW_PADDING: [f32; 2] = [8.0, 12.0];
/// The tallest control in a row, which sets the row's least height: 34 px
/// plus the row's padding is Adwaita's 50 px action row.
pub const ROW_MIN_CONTENT: f32 = 34.0;
/// The gap between controls, and between a group's title and its list.
pub const SPACING: f32 = 12.0;

/// `alpha(currentColor, a)`: the palette's text at opacity `a`.
fn ink(palette: Palette, a: f32) -> Color {
    Color {
        a,
        ..palette.text.to_iced()
    }
}

/// `a` moved `t` of the way to `b`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// A flat button: no border, a neutral fill that darkens as it is hovered
/// and pressed (`button` in libadwaita).
#[must_use]
pub fn button(palette: Palette, status: btn::Status) -> btn::Style {
    let (fill, text) = match status {
        btn::Status::Active => (0.10, 1.0),
        btn::Status::Hovered => (0.15, 1.0),
        btn::Status::Pressed => (0.30, 1.0),
        btn::Status::Disabled => (0.10, 0.5),
    };
    btn::Style {
        background: Some(Background::Color(ink(palette, fill))),
        text_color: ink(palette, text),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// A `.flat` button: no fill until it is hovered or pressed. Disabled, it is
/// only its faded label, which is how a done state (Installed) reads.
#[must_use]
pub fn flat_button(palette: Palette, status: btn::Status) -> btn::Style {
    let (fill, text) = match status {
        btn::Status::Active => (0.0, 1.0),
        btn::Status::Hovered => (0.07, 1.0),
        btn::Status::Pressed => (0.16, 1.0),
        btn::Status::Disabled => (0.0, 0.5),
    };
    btn::Style {
        background: (fill > 0.0).then(|| Background::Color(ink(palette, fill))),
        text_color: ink(palette, text),
        ..button(palette, btn::Status::Active)
    }
}

/// A small pill label beside a title (a store's name, say): the neutral
/// fill with fully rounded ends.
#[must_use]
pub fn badge(palette: Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(ink(palette, 0.08))),
        border: Border {
            radius: 9.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The one button a page asks the user to press (`.suggested-action`): the
/// accent fill, lighter on hover and darker when pressed.
#[must_use]
pub fn suggested_button(palette: Palette, status: btn::Status) -> btn::Style {
    let accent = palette.selection.to_iced();
    let on_accent = palette.selection_text.to_iced();
    let (background, text) = match status {
        btn::Status::Active => (accent, on_accent),
        btn::Status::Hovered => (mix(accent, on_accent, 0.1), on_accent),
        btn::Status::Pressed => (mix(accent, Color::BLACK, 0.2), on_accent),
        btn::Status::Disabled => (
            Color { a: 0.5, ..accent },
            Color {
                a: 0.5,
                ..on_accent
            },
        ),
    };
    btn::Style {
        background: Some(Background::Color(background)),
        text_color: text,
        ..button(palette, btn::Status::Active)
    }
}

/// A switch: a pill-shaped track, the neutral fill when off and the accent
/// when on, with a round knob in the colour drawn on the accent.
#[must_use]
pub fn switch(palette: Palette, status: toggler::Status) -> toggler::Style {
    let (on, hovered, disabled) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let accent = palette.selection.to_iced();
    let knob = palette.selection_text.to_iced();
    let track = match (on, hovered) {
        (true, false) => accent,
        (true, true) => mix(accent, knob, 0.1),
        (false, false) => ink(palette, 0.15),
        (false, true) => ink(palette, 0.20),
    };
    let fade = |colour: Color| {
        if disabled {
            Color {
                a: colour.a * 0.5,
                ..colour
            }
        } else {
            colour
        }
    };
    toggler::Style {
        background: Background::Color(fade(track)),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: Background::Color(fade(knob)),
        foreground_border_width: 1.0,
        foreground_border_color: fade(Color {
            a: 0.12,
            ..Color::BLACK
        }),
        text_color: Some(palette.text.to_iced()),
        border_radius: None,
        padding_ratio: 3.0 / SWITCH_SIZE,
    }
}

/// An entry: the neutral fill, no frame, and a 2 px accent ring while it has
/// the focus.
#[must_use]
pub fn entry(palette: Palette, status: text_input::Status) -> text_input::Style {
    let (fill, focused) = match status {
        text_input::Status::Active | text_input::Status::Disabled => (0.10, false),
        text_input::Status::Hovered => (0.13, false),
        text_input::Status::Focused { .. } => (0.10, true),
    };
    let accent = palette.accent.to_iced();
    text_input::Style {
        background: Background::Color(ink(palette, fill)),
        border: Border {
            color: Color { a: 0.5, ..accent },
            width: if focused { 2.0 } else { 0.0 },
            radius: CONTROL_RADIUS.into(),
        },
        icon: palette.muted.to_iced(),
        placeholder: ink(palette, 0.5),
        value: palette.text.to_iced(),
        selection: Color { a: 0.3, ..accent },
    }
}

/// A dropdown: drawn as a button, with its chevron in the text colour.
#[must_use]
pub fn dropdown(palette: Palette, status: pick_list::Status) -> pick_list::Style {
    let fill = match status {
        pick_list::Status::Active => 0.10,
        pick_list::Status::Hovered => 0.15,
        pick_list::Status::Opened { .. } => 0.30,
    };
    pick_list::Style {
        text_color: palette.text.to_iced(),
        placeholder_color: ink(palette, 0.5),
        handle_color: palette.text.to_iced(),
        background: Background::Color(ink(palette, fill)),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
    }
}

/// A boxed list's fill: Adwaita's `card_bg_color`.
///
/// White over the light window, and a lift of the text colour over the dark
/// one. A palette's `field` is the first, but in a dark palette it is the
/// sunken search field, darker than the card it sits on, which would read as
/// a hole rather than a card.
#[must_use]
pub fn card(palette: Palette) -> Color {
    let lighter = |c: Rgb| u16::from(c.r) + u16::from(c.g) + u16::from(c.b);
    if lighter(palette.field) >= lighter(palette.surface) {
        palette.field.to_iced()
    } else {
        mix(palette.surface.to_iced(), palette.text.to_iced(), 0.08)
    }
}

/// A boxed list (`.boxed-list`, an `AdwPreferencesGroup`'s rows): the card
/// fill, a hairline, and 12 px corners.
#[must_use]
pub fn boxed_list(palette: Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(card(palette))),
        border: Border {
            color: palette.border.to_iced(),
            width: 1.0,
            radius: LIST_RADIUS.into(),
        },
        ..container::Style::default()
    }
}

/// The separator between a boxed list's rows.
#[must_use]
pub fn separator(palette: Palette) -> rule::Style {
    rule::Style {
        color: palette.border.to_iced(),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

/// A sidebar row (`.navigation-sidebar`): the selected one takes the neutral
/// fill, as in GNOME Settings, and the rest are bare.
#[must_use]
pub fn sidebar_row(palette: Palette, selected: bool) -> container::Style {
    container::Style {
        background: selected.then(|| Background::Color(ink(palette, 0.10))),
        border: Border {
            radius: CONTROL_RADIUS.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{DARK, LIGHT};

    #[test]
    fn the_card_is_white_over_light_and_lifted_over_dark() {
        assert_eq!(card(LIGHT), LIGHT.field.to_iced());
        let dark = card(DARK);
        assert!(dark.r > DARK.surface.to_iced().r, "{dark:?}");
        assert!(dark.r < 0.25, "a lift, not a light card: {dark:?}");
    }

    #[test]
    fn only_the_suggested_action_and_an_on_switch_take_the_accent() {
        let accent = Some(Background::Color(LIGHT.selection.to_iced()));
        assert_ne!(button(LIGHT, btn::Status::Active).background, accent);
        assert_eq!(
            suggested_button(LIGHT, btn::Status::Active).background,
            accent
        );
        let on = switch(LIGHT, toggler::Status::Active { is_toggled: true });
        let off = switch(LIGHT, toggler::Status::Active { is_toggled: false });
        assert_eq!(Some(on.background), accent);
        assert_ne!(Some(off.background), accent);
    }

    #[test]
    fn a_flat_button_is_bare_until_hovered() {
        assert_eq!(flat_button(DARK, btn::Status::Active).background, None);
        assert_eq!(flat_button(DARK, btn::Status::Disabled).background, None);
        assert!(flat_button(DARK, btn::Status::Hovered).background.is_some());
        assert!(button(DARK, btn::Status::Active).background.is_some());
    }

    #[test]
    fn only_a_focused_entry_has_a_ring() {
        assert!(entry(DARK, text_input::Status::Active).border.width.abs() < f32::EPSILON);
        let focused = entry(DARK, text_input::Status::Focused { is_hovered: false });
        assert!((focused.border.width - 2.0).abs() < f32::EPSILON);
        assert_eq!(
            Color {
                a: 1.0,
                ..focused.border.color
            },
            DARK.accent.to_iced()
        );
    }
}
