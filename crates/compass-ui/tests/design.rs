//! The launcher's visual vocabulary.

use compass_ui::design::{self, Appearance, ColorScheme, Rgb};

#[test]
fn the_portal_reports_dark_as_one_and_light_as_two() {
    // The order nobody guesses, and getting it backwards produces a launcher
    // that is dark for exactly the users who asked for light.
    assert_eq!(ColorScheme::from_portal(1), ColorScheme::PreferDark);
    assert_eq!(ColorScheme::from_portal(2), ColorScheme::PreferLight);
}

#[test]
fn zero_is_no_preference() {
    assert_eq!(ColorScheme::from_portal(0), ColorScheme::NoPreference);
}

#[test]
fn a_value_the_specification_gains_later_reads_as_no_preference() {
    // Reserved room to grow. A desktop reporting something newer should get a
    // readable launcher rather than none.
    assert_eq!(ColorScheme::from_portal(3), ColorScheme::NoPreference);
    assert_eq!(
        ColorScheme::from_portal(u32::MAX),
        ColorScheme::NoPreference
    );
}

#[test]
fn no_preference_draws_light_because_that_is_adwaitas_default() {
    assert_eq!(ColorScheme::NoPreference.appearance(), Appearance::Light);
}

#[test]
fn each_preference_draws_what_it_asked_for() {
    assert_eq!(ColorScheme::PreferDark.appearance(), Appearance::Dark);
    assert_eq!(ColorScheme::PreferLight.appearance(), Appearance::Light);
}

#[test]
fn hex_round_trips_the_values_as_they_are_written_in_adwaita() {
    // These are transcribed from GNOME's published palette, which is written
    // in hex. A test in hex is one someone can check against the source.
    assert_eq!(Rgb::new(0x35, 0x84, 0xe4).hex(), "#3584e4");
    assert_eq!(design::LIGHT.surface.hex(), "#fafafa");
    assert_eq!(design::DARK.surface.hex(), "#242424");
    assert_eq!(design::LIGHT.accent.hex(), "#3584e4");
}

#[test]
fn a_component_below_sixteen_keeps_its_leading_zero() {
    // `#{:x}` without the width would print `#f0f0f` for this and every
    // consumer would read a five-digit colour.
    assert_eq!(Rgb::new(0x0f, 0x00, 0x0f).hex(), "#0f000f");
}

#[test]
fn the_two_palettes_are_not_the_same() {
    assert_ne!(design::LIGHT.surface, design::DARK.surface);
    assert_ne!(design::LIGHT.text, design::DARK.text);
}

#[test]
fn text_and_surface_are_far_apart_in_both() {
    // Not a contrast-ratio check -- that belongs with the accessibility work
    // -- but a guard against a palette edit that makes text the same shade as
    // what it sits on, which is the way a colour change goes catastrophically
    // rather than subtly wrong.
    for appearance in Appearance::ALL {
        let p = design::palette(appearance);
        let distance = i32::from(p.text.r) - i32::from(p.surface.r);
        assert!(
            distance.abs() > 100,
            "{} text and surface are too close",
            appearance.name()
        );
    }
}

#[test]
fn the_selection_is_readable_against_its_own_fill() {
    for appearance in Appearance::ALL {
        let p = design::palette(appearance);
        assert_ne!(p.selection, p.selection_text);
    }
}

#[test]
fn the_backdrop_dims_rather_than_hides() {
    // Fully opaque would make the launcher a full-screen application; zero
    // would leave the card floating on an undimmed desktop and hard to read.
    for appearance in Appearance::ALL {
        let alpha = design::palette(appearance).backdrop_alpha;
        assert!(alpha > 0.0 && alpha < 0.6, "{alpha} is not a dimming");
    }
}

#[test]
fn the_card_is_a_fixed_width_rather_than_a_share_of_the_screen() {
    // A launcher that grows with the monitor is unreadable on a wide one.
    let g = design::GEOMETRY;
    assert_eq!(g.card_width, 720);
}

#[test]
fn the_card_sits_above_the_middle() {
    // The eye starts above centre and the results grow downward, so a card
    // centred vertically ends up low once it fills.
    const { assert!(design::GEOMETRY.card_top_fraction < 0.5) };
}

#[test]
fn a_row_is_tall_enough_for_a_title_and_a_subtitle() {
    let g = design::GEOMETRY;
    assert!(g.row_height > g.title_size + g.subtitle_size);
}

#[test]
fn the_icon_fits_inside_a_row() {
    let g = design::GEOMETRY;
    assert!(g.icon_size < g.row_height);
}

#[test]
fn both_appearances_have_a_theme() {
    for appearance in Appearance::ALL {
        let theme = design::theme(appearance);
        assert_eq!(
            theme.palette().background,
            design::palette(appearance).surface.to_iced()
        );
    }
}

#[test]
fn the_font_stack_starts_with_gnomes_interface_font() {
    assert_eq!(design::FONT_STACK.first().copied(), Some("Cantarell"));
}

#[test]
fn a_dropdown_reads_as_part_of_the_card() {
    // #239: the kind filter wore Iced's default pick-list chrome — square
    // corners, a full border, generic colors — and its menu read as a foreign
    // box. Both now come from the palette, in both appearances.
    use iced::widget::{overlay::menu, pick_list};

    for appearance in Appearance::ALL {
        let palette = design::palette(appearance);
        for status in [
            pick_list::Status::Active,
            pick_list::Status::Hovered,
            pick_list::Status::Opened { is_hovered: false },
            pick_list::Status::Opened { is_hovered: true },
        ] {
            let closed = design::dropdown(palette, status);
            assert_eq!(
                closed.background,
                iced::Background::Color(palette.field.to_iced())
            );
            assert_eq!(closed.text_color, palette.text.to_iced());
            assert_eq!(
                closed.border.radius,
                f32::from(design::GEOMETRY.row_radius).into()
            );
            assert_eq!(closed.border.width, 0.0);
        }
        let open: menu::Style = design::dropdown_menu(palette);
        assert_eq!(
            open.background,
            iced::Background::Color(palette.surface.to_iced())
        );
        assert_eq!(
            open.selected_background,
            iced::Background::Color(palette.selection.to_iced())
        );
        assert_eq!(open.selected_text_color, palette.selection_text.to_iced());
        assert_eq!(open.text_color, palette.text.to_iced());
    }
}
