// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! The design tokens: every UI color (with alpha) in a dark and a light
//! palette, plus the layout and motion scales. Pages read them through
//! `theme::tokens(cx)` and never write color literals.
//!
//! The material changes only the window/chrome fill (`window`). Under glass
//! it is the palette's window color at the least alpha that keeps chrome text
//! legible over any desktop (`contrast::min_glass_alpha`); the content plane
//! and everything painted on it stay opaque.

use super::contrast;
use super::material::{Material, Mode};
use domain::ProjectStatus;
use gpui::{hsla, px, Hsla, Pixels};
use std::time::Duration;

/// Fills of the four document processing states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StatusColors {
    pub(crate) imported: Hsla,
    pub(crate) processing: Hsla,
    pub(crate) ready: Hsla,
    pub(crate) failed: Hsla,
}

/// The fills and text of one button variant. Hover and pressed fills
/// replace `fill` while the pointer is over or pressing the button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ButtonColors {
    pub(crate) fill: Hsla,
    pub(crate) hover: Hsla,
    pub(crate) pressed: Hsla,
    pub(crate) text: Hsla,
    /// The fill of a disabled button.
    pub(crate) disabled: Hsla,
}

/// The button variants: the one primary action of a group, secondary
/// actions and unselected choices, borderless actions on panels, and
/// destructive actions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Buttons {
    pub(crate) primary: ButtonColors,
    pub(crate) secondary: ButtonColors,
    pub(crate) ghost: ButtonColors,
    pub(crate) danger: ButtonColors,
    /// The text of every disabled button.
    pub(crate) disabled_text: Hsla,
}

/// Corner radii: controls, cards, panels, sheets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Radius {
    pub(crate) sm: Pixels,
    pub(crate) md: Pixels,
    pub(crate) lg: Pixels,
    pub(crate) xl: Pixels,
}

/// The spacing ladder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Spacing {
    pub(crate) xs: Pixels,
    pub(crate) sm: Pixels,
    pub(crate) md: Pixels,
    pub(crate) lg: Pixels,
    pub(crate) xl: Pixels,
}

/// Heights of buttons, selects and single-line inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ControlHeight {
    pub(crate) sm: Pixels,
    pub(crate) md: Pixels,
    pub(crate) lg: Pixels,
}

/// The type scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FontSize {
    pub(crate) xs: Pixels,
    pub(crate) sm: Pixels,
    pub(crate) md: Pixels,
    pub(crate) lg: Pixels,
    pub(crate) xl: Pixels,
    pub(crate) xxl: Pixels,
}

/// Animation durations. Open/close animations change only opacity and
/// position, never blur radius or overlay size. All zero under reduced
/// motion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Motion {
    pub(crate) hover: Duration,
    pub(crate) menu: Duration,
    pub(crate) dialog: Duration,
    pub(crate) layout: Duration,
}

impl Motion {
    pub(crate) fn new(reduce_motion: bool) -> Self {
        let duration = |milliseconds| {
            if reduce_motion {
                Duration::ZERO
            } else {
                Duration::from_millis(milliseconds)
            }
        };
        Self {
            hover: duration(150),
            menu: duration(140),
            dialog: duration(180),
            layout: duration(200),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tokens {
    pub(crate) mode: Mode,
    pub(crate) material: Material,
    /// Window glass tint behind the title bar and sidebar chrome; opaque
    /// under the opaque material.
    pub(crate) window: Hsla,
    /// The content plane: reading, transcripts, Detail, forms. Always opaque.
    pub(crate) content: Hsla,
    /// Cards and fields on the content plane.
    pub(crate) surface: Hsla,
    /// Surfaces raised above a card: nested cards and the Detail panel.
    pub(crate) raised: Hsla,
    /// Floating surfaces (menus, dropdowns, dialog sheets) under the opaque
    /// material.
    pub(crate) float: Hsla,
    /// `float` at the least alpha that keeps text on it legible over any
    /// backdrop: the tint of a frosted menu under glass.
    pub(crate) frost: Hsla,
    /// The tint of a frosted dialog sheet under glass. The sheet carries no
    /// text (the dialog's text sits on an opaque panel on it), so the tint
    /// only has to let the blurred page show through.
    pub(crate) sheet_frost: Hsla,
    /// The drop shadow of a floating surface; only the opaque fallback casts
    /// one.
    pub(crate) shadow: Hsla,
    /// 1px dividers.
    pub(crate) hairline: Hsla,
    /// Control and card borders.
    pub(crate) border: Hsla,
    /// Translucent hover wash.
    pub(crate) wash: Hsla,
    /// Selected rows and text selection.
    pub(crate) selection: Hsla,
    /// Modal backdrop.
    pub(crate) scrim: Hsla,
    pub(crate) text: Hsla,
    pub(crate) text_muted: Hsla,
    /// Hints and secondary notices.
    pub(crate) text_subtle: Hsla,
    /// Focus borders, rings and informational fills.
    pub(crate) accent: Hsla,
    /// Accent-colored text on the content plane.
    pub(crate) accent_text: Hsla,
    /// Text on an `accent` fill.
    pub(crate) accent_foreground: Hsla,
    pub(crate) caret: Hsla,
    pub(crate) danger: Hsla,
    pub(crate) warning: Hsla,
    /// A warning card's fill; `warning_text` is legible on it.
    pub(crate) warning_surface: Hsla,
    pub(crate) warning_text: Hsla,
    pub(crate) success: Hsla,
    pub(crate) status: StatusColors,
    pub(crate) button: Buttons,
    /// The ring around a control that has keyboard focus.
    pub(crate) focus_ring: Hsla,
    /// A scroll area's scrollbar thumb, and the thumb under the pointer or
    /// being dragged.
    pub(crate) scrollbar_thumb: Hsla,
    pub(crate) scrollbar_thumb_hover: Hsla,
    pub(crate) drag_ghost: Hsla,
    /// The UI font: the system font (SF, with PingFang for Chinese).
    pub(crate) font_family: &'static str,
    pub(crate) radius: Radius,
    pub(crate) spacing: Spacing,
    pub(crate) control_height: ControlHeight,
    pub(crate) font_size: FontSize,
    pub(crate) motion: Motion,
}

/// The alpha of `sheet_frost`.
const SHEET_FROST_ALPHA: f32 = 0.5;

const RADIUS: Radius = Radius {
    sm: px(6.),
    md: px(8.),
    lg: px(12.),
    xl: px(16.),
};

const SPACING: Spacing = Spacing {
    xs: px(4.),
    sm: px(8.),
    md: px(12.),
    lg: px(16.),
    xl: px(24.),
};

const CONTROL_HEIGHT: ControlHeight = ControlHeight {
    sm: px(24.),
    md: px(32.),
    lg: px(40.),
};

const FONT_FAMILY: &str = ".SystemUIFont";

const FONT_SIZE: FontSize = FontSize {
    xs: px(12.),
    sm: px(14.),
    md: px(16.),
    lg: px(18.),
    xl: px(20.),
    xxl: px(24.),
};

impl Tokens {
    pub(crate) fn new(mode: Mode, material: Material, reduce_motion: bool) -> Self {
        let mut tokens = match mode {
            Mode::Dark => dark(),
            Mode::Light => light(),
        };
        tokens.motion = Motion::new(reduce_motion);
        tokens.frost = tokens.float.alpha(contrast::min_glass_alpha(
            tokens.float.into(),
            tokens.text.into(),
            tokens.text_muted.into(),
        ));
        tokens.sheet_frost = tokens.float.alpha(SHEET_FROST_ALPHA);
        if material == Material::Glass {
            tokens.material = Material::Glass;
            tokens.window = tokens.window.alpha(glass_alpha(&tokens));
        }
        tokens
    }

    pub(crate) fn status(&self, status: ProjectStatus) -> Hsla {
        match status {
            ProjectStatus::Imported => self.status.imported,
            ProjectStatus::Processing => self.status.processing,
            ProjectStatus::Ready => self.status.ready,
            ProjectStatus::Failed => self.status.failed,
        }
    }

    /// The fill the window root paints under everything: clear under
    /// glass, so the chrome's own tint over the blurred desktop shows.
    pub(crate) fn root_background(&self) -> Hsla {
        match self.material {
            Material::Glass => self.window.alpha(0.),
            Material::Opaque => self.window,
        }
    }
}

/// The least glass alpha that keeps the palette's chrome text legible over
/// any desktop.
fn glass_alpha(opaque: &Tokens) -> f32 {
    contrast::min_glass_alpha(
        opaque.window.into(),
        opaque.text.into(),
        opaque.text_muted.into(),
    )
}

fn hex(value: u32) -> Hsla {
    gpui::rgb(value).into()
}

/// A palette's button colors: `[fill, hover, pressed]` of the solid
/// variants, `[hover, pressed]` of the ghost variant, the danger hue and
/// its text, and the palette's muted text.
struct ButtonPalette {
    primary: [Hsla; 3],
    primary_text: Hsla,
    secondary: [Hsla; 3],
    secondary_text: Hsla,
    ghost: [Hsla; 2],
    danger: Hsla,
    danger_text: Hsla,
    text_muted: Hsla,
}

/// The fill alphas of the danger wash: resting, hovered and pressed.
const DANGER_WASH: [f32; 3] = [0.2, 0.3, 0.4];
/// How much of a solid fill a disabled primary or danger button keeps.
const DISABLED_FILL: f32 = 0.15;

fn buttons(palette: ButtonPalette) -> Buttons {
    let solid = |[fill, hover, pressed]: [Hsla; 3], text: Hsla, disabled: Hsla| ButtonColors {
        fill,
        hover,
        pressed,
        text,
        disabled,
    };
    let [primary, ..] = palette.primary;
    let [secondary, ..] = palette.secondary;
    let clear = gpui::transparent_black();
    let wash = DANGER_WASH.map(|alpha| palette.danger.alpha(alpha));
    Buttons {
        primary: solid(
            palette.primary,
            palette.primary_text,
            primary.opacity(DISABLED_FILL),
        ),
        // A disabled secondary button keeps its plate; only its text dims.
        secondary: solid(palette.secondary, palette.secondary_text, secondary),
        ghost: solid(
            [clear, palette.ghost[0], palette.ghost[1]],
            palette.secondary_text,
            clear,
        ),
        danger: solid(wash, palette.danger_text, wash[0].opacity(DISABLED_FILL)),
        disabled_text: palette.text_muted.opacity(0.5),
    }
}

/// The opaque dark palette.
fn dark() -> Tokens {
    Tokens {
        mode: Mode::Dark,
        material: Material::Opaque,
        window: hex(0x111318),
        content: hex(0x111318),
        surface: hex(0x171b22),
        raised: hex(0x202733),
        float: hex(0x1c222c),
        frost: hex(0x1c222c),
        sheet_frost: hex(0x1c222c),
        shadow: hsla(0., 0., 0., 0.5),
        hairline: hex(0x2b303b),
        border: hex(0x303744),
        wash: hsla(0., 0., 1., 0.06),
        selection: hex(0x27344a),
        scrim: hsla(0., 0., 0., 0.6),
        text: hex(0xe9edf5),
        text_muted: hex(0x8992a3),
        text_subtle: hex(0x667085),
        accent: hex(0x3b82f6),
        accent_text: hex(0x60a5fa),
        accent_foreground: hex(0xffffff),
        caret: hex(0x6699cc),
        danger: hex(0xf87171),
        warning: hex(0xf59e0b),
        warning_surface: hex(0x3a2d16),
        warning_text: hex(0xfbbf24),
        success: hex(0x4ade80),
        status: StatusColors {
            imported: hex(0x374151),
            processing: hex(0x92400e),
            ready: hex(0x166534),
            failed: hex(0x991b1b),
        },
        button: buttons(ButtonPalette {
            primary: [hex(0xf9f9f9), hex(0xf5f5f5), hex(0xe4e4e4)],
            primary_text: hex(0x161616),
            secondary: [hex(0x252525), hex(0x292929), hex(0x212121)],
            secondary_text: hex(0xf9f9f9),
            ghost: [hsla(0., 0., 0.164, 0.8), hsla(0., 0., 0.179, 0.8)],
            danger: hex(0xf87171),
            danger_text: hex(0xf87171),
            text_muted: hex(0x8992a3),
        }),
        focus_ring: hex(0x3b82f6),
        scrollbar_thumb: hsla(0., 0., 0.32, 0.9),
        scrollbar_thumb_hover: hsla(0., 0., 0.32, 1.),
        drag_ghost: hex(0x263142),
        font_family: FONT_FAMILY,
        radius: RADIUS,
        spacing: SPACING,
        control_height: CONTROL_HEIGHT,
        font_size: FONT_SIZE,
        motion: Motion::new(false),
    }
}

/// The opaque light palette. Chrome recedes to grey while the content plane
/// is white; accents move down the scale to keep their contrast on white.
fn light() -> Tokens {
    Tokens {
        mode: Mode::Light,
        material: Material::Opaque,
        window: hex(0xe8ebf0),
        content: hex(0xffffff),
        surface: hex(0xf4f6f9),
        raised: hex(0xffffff),
        float: hex(0xffffff),
        frost: hex(0xffffff),
        sheet_frost: hex(0xffffff),
        shadow: hsla(0., 0., 0., 0.18),
        hairline: hex(0xdde1e8),
        border: hex(0xc9d0db),
        wash: hsla(0., 0., 0., 0.05),
        selection: hex(0xdbe7fd),
        scrim: hsla(0., 0., 0., 0.32),
        text: hex(0x171a21),
        text_muted: hex(0x566072),
        text_subtle: hex(0x6b7587),
        accent: hex(0x2563eb),
        accent_text: hex(0x1d4ed8),
        accent_foreground: hex(0xffffff),
        caret: hex(0x2563eb),
        danger: hex(0xdc2626),
        warning: hex(0xd97706),
        warning_surface: hex(0xfef3c7),
        warning_text: hex(0x92400e),
        success: hex(0x16a34a),
        status: StatusColors {
            imported: hex(0xe5e7eb),
            processing: hex(0xfde68a),
            ready: hex(0xbbf7d0),
            failed: hex(0xfecaca),
        },
        button: buttons(ButtonPalette {
            primary: [hex(0x161616), hex(0x252525), hex(0x090909)],
            primary_text: hex(0xf9f9f9),
            secondary: [hex(0xe4e4e4), hex(0xe4e4e4), hex(0xd3d3d3)],
            secondary_text: hex(0x161616),
            ghost: [hsla(0., 0., 0.808, 0.8), hsla(0., 0., 0.718, 0.8)],
            danger: hex(0xdc2626),
            // Darker than `danger`: legible on its own wash.
            danger_text: hex(0xa51d1d),
            text_muted: hex(0x566072),
        }),
        focus_ring: hex(0x2563eb),
        scrollbar_thumb: hsla(0., 0., 0.64, 0.9),
        scrollbar_thumb_hover: hsla(0., 0., 0.64, 1.),
        drag_ghost: hex(0xdbe3ef),
        font_family: FONT_FAMILY,
        radius: RADIUS,
        spacing: SPACING,
        control_height: CONTROL_HEIGHT,
        font_size: FONT_SIZE,
        motion: Motion::new(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contrast::{
        contrast_ratio, glass_is_legible, min_glass_alpha, BODY_TEXT_CONTRAST, MUTED_TEXT_CONTRAST,
        WORST_CASE_BACKDROPS,
    };

    const MODES: [Mode; 2] = [Mode::Dark, Mode::Light];

    fn ratio(foreground: Hsla, background: Hsla) -> f32 {
        contrast_ratio(foreground.into(), background.into())
    }

    #[test]
    fn glass_uses_the_computed_alpha_over_black_and_white_desktops() {
        for mode in MODES {
            let opaque = Tokens::new(mode, Material::Opaque, false);
            let glass = Tokens::new(mode, Material::Glass, false);
            let required = min_glass_alpha(
                opaque.window.into(),
                opaque.text.into(),
                opaque.text_muted.into(),
            );

            assert!(required > 0. && required < 1., "{mode:?}: {required}");
            assert_eq!(glass.window.a, required, "{mode:?} glass alpha");
            assert!(glass_is_legible(
                glass.window.into(),
                glass.text.into(),
                glass.text_muted.into()
            ));
            for backdrop in WORST_CASE_BACKDROPS {
                let chrome = contrast::composite(glass.window.into(), backdrop);
                assert!(
                    contrast_ratio(glass.text.into(), chrome) >= BODY_TEXT_CONTRAST,
                    "{mode:?} text over {backdrop:?}"
                );
                assert!(
                    contrast_ratio(glass.text_muted.into(), chrome) >= MUTED_TEXT_CONTRAST,
                    "{mode:?} muted text over {backdrop:?}"
                );
            }
        }
    }

    #[test]
    fn frost_is_the_float_fill_at_the_least_legible_alpha() {
        for mode in MODES {
            for material in [Material::Glass, Material::Opaque] {
                let tokens = Tokens::new(mode, material, false);
                assert_eq!(tokens.float.a, 1., "{mode:?} float is opaque");
                assert!(
                    tokens.frost.a > 0. && tokens.frost.a < 1.,
                    "{mode:?} frost is translucent: {}",
                    tokens.frost.a
                );
                assert_eq!(tokens.frost.alpha(1.), tokens.float);
                assert_eq!(tokens.sheet_frost.alpha(1.), tokens.float);
                assert!(
                    tokens.sheet_frost.a < tokens.frost.a,
                    "{mode:?}: a sheet shows more of the page than a menu"
                );
                assert!(glass_is_legible(
                    tokens.frost.into(),
                    tokens.text.into(),
                    tokens.text_muted.into()
                ));
                assert!(ratio(tokens.text, tokens.float) >= 4.5);
                assert!(ratio(tokens.text_muted, tokens.float) >= 4.5);
            }
        }
    }

    #[test]
    fn the_material_changes_only_the_window_fill() {
        for mode in MODES {
            let opaque = Tokens::new(mode, Material::Opaque, false);
            let glass = Tokens::new(mode, Material::Glass, false);

            assert_eq!(opaque.window.a, 1.);
            assert_eq!(glass.content.a, 1., "the glass content plane is opaque");
            assert_eq!(opaque.root_background(), opaque.window);
            assert_eq!(glass.root_background().a, 0.);
            assert_eq!(
                Tokens {
                    material: Material::Opaque,
                    window: opaque.window,
                    ..glass
                },
                opaque,
                "{mode:?}: glass may differ from opaque only in the window fill"
            );
        }
    }

    #[test]
    fn palettes_keep_text_legible_on_the_content_plane() {
        for mode in MODES {
            let tokens = Tokens::new(mode, Material::Opaque, false);
            for (name, background) in [
                ("content", tokens.content),
                ("surface", tokens.surface),
                ("raised", tokens.raised),
            ] {
                assert!(
                    ratio(tokens.text, background) >= 4.5,
                    "{mode:?} text/{name}"
                );
                assert!(
                    ratio(tokens.text_muted, background) >= 4.5,
                    "{mode:?} muted/{name}"
                );
                assert!(
                    ratio(tokens.accent_text, background) >= 4.5,
                    "{mode:?} accent text/{name}"
                );
            }
            assert!(ratio(tokens.text_subtle, tokens.content) >= 3.);
            assert!(ratio(tokens.text, tokens.selection) >= 4.5);
            assert!(ratio(tokens.text, tokens.drag_ghost) >= 4.5);
            assert!(ratio(tokens.warning_text, tokens.warning_surface) >= 4.5);
            assert!(ratio(tokens.accent_foreground, tokens.accent) >= 3.);
            for status in [
                ProjectStatus::Imported,
                ProjectStatus::Processing,
                ProjectStatus::Ready,
                ProjectStatus::Failed,
            ] {
                assert!(
                    ratio(tokens.text, tokens.status(status)) >= 4.5,
                    "{mode:?} text on {status:?}"
                );
            }
        }
    }

    #[test]
    fn button_text_is_legible_on_every_resting_variant_and_the_ring_stands_out() {
        for mode in MODES {
            let tokens = Tokens::new(mode, Material::Opaque, false);
            let buttons = tokens.button;
            for (variant, colors) in [
                ("primary", buttons.primary),
                ("secondary", buttons.secondary),
                ("ghost", buttons.ghost),
                ("danger", buttons.danger),
            ] {
                for (plane, background) in
                    [("content", tokens.content), ("surface", tokens.surface)]
                {
                    let fill = contrast::composite(colors.fill.into(), background.into());
                    assert!(
                        contrast_ratio(colors.text.into(), fill) >= BODY_TEXT_CONTRAST,
                        "{mode:?} {variant} text on {plane}"
                    );
                }
            }
            // A focus indicator needs 3:1 against what it sits on.
            for background in [tokens.content, tokens.surface, tokens.window] {
                assert!(ratio(tokens.focus_ring, background) >= 3., "{mode:?} ring");
            }
        }
    }

    #[test]
    fn layout_scales_are_the_design_ladder() {
        let tokens = Tokens::new(Mode::Dark, Material::Opaque, false);
        let radius = tokens.radius;
        assert_eq!(
            [radius.sm, radius.md, radius.lg, radius.xl],
            [px(6.), px(8.), px(12.), px(16.)]
        );
        let spacing = tokens.spacing;
        assert_eq!(
            [spacing.xs, spacing.sm, spacing.md, spacing.lg, spacing.xl],
            [px(4.), px(8.), px(12.), px(16.), px(24.)]
        );
        let heights = tokens.control_height;
        assert_eq!(
            [heights.sm, heights.md, heights.lg],
            [px(24.), px(32.), px(40.)]
        );
        let font = tokens.font_size;
        assert_eq!(
            [font.xs, font.sm, font.md, font.lg, font.xl, font.xxl],
            [px(12.), px(14.), px(16.), px(18.), px(20.), px(24.)]
        );
        assert_eq!(
            Tokens::new(Mode::Light, Material::Glass, false).radius,
            radius
        );
    }

    #[test]
    fn reduced_motion_zeroes_every_duration() {
        let motion = |reduce| {
            let motion = Tokens::new(Mode::Dark, Material::Opaque, reduce).motion;
            [motion.hover, motion.menu, motion.dialog, motion.layout]
        };
        assert!(motion(false).iter().all(|duration| !duration.is_zero()));
        assert_eq!(motion(true), [Duration::ZERO; 4]);
    }
}
