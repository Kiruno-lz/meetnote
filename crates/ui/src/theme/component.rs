//! gpui-component follows the tokens through its `ThemeConfig`: `Theme::change`
//! resolves the config through `Theme::apply_config` and derives the
//! `gpui_base` theme, so buttons, collapsibles, separators, scrollbars and
//! `Root` paint token colors. Writing `ThemeColor` directly would leave `tokens.background`,
//! which `Root` paints over the whole window, opaque and hide the glass.
//!
//! gpui-component blends several unset hover fills onto `background`, keeping
//! its alpha; under glass that alpha is 0 and the fills would vanish. Those
//! fills are set here against the opaque content plane instead, so the
//! material changes nothing but `background`.

use super::material::Mode;
use super::tokens::Tokens;
use gpui::{App, Hsla, Rgba, SharedString, Window};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use std::rc::Rc;

/// `base` (the registry's default config for the mode) with every color
/// MeetNote owns replaced by its token.
pub(crate) fn component_config(tokens: &Tokens, base: &ThemeConfig) -> ThemeConfig {
    let mut config = base.clone();
    config.name = match tokens.mode {
        Mode::Dark => "MeetNote Dark",
        Mode::Light => "MeetNote Light",
    }
    .into();
    config.mode = component_mode(tokens.mode);
    config.font_size = Some(f32::from(tokens.font_size.md));
    config.radius = Some(f32::from(tokens.radius.sm) as usize);
    config.radius_lg = Some(f32::from(tokens.radius.md) as usize);

    // gpui-component's own hover formula, on the opaque content plane.
    let hover = |color: Hsla| tokens.content.blend(color.opacity(0.9));
    let colors = &mut config.colors;
    for (field, color) in [
        (&mut colors.background, tokens.root_background()),
        (&mut colors.foreground, tokens.text),
        (&mut colors.muted_foreground, tokens.text_muted),
        (&mut colors.border, tokens.hairline),
        (&mut colors.window_border, tokens.hairline),
        (&mut colors.input, tokens.border),
        (&mut colors.accent, tokens.raised),
        (&mut colors.accent_foreground, tokens.text),
        (&mut colors.popover, tokens.content),
        (&mut colors.popover_foreground, tokens.text),
        (&mut colors.list, tokens.content),
        (&mut colors.list_hover, tokens.wash),
        (&mut colors.list_active, tokens.selection),
        (&mut colors.list_active_border, tokens.accent),
        (&mut colors.selection, tokens.selection),
        (&mut colors.caret, tokens.caret),
        (&mut colors.ring, tokens.accent),
        (&mut colors.drag_border, tokens.accent),
        (&mut colors.link, tokens.accent_text),
        (&mut colors.link_hover, tokens.accent_text),
        (&mut colors.link_active, tokens.accent_text),
        (&mut colors.info, tokens.accent),
        (&mut colors.info_foreground, tokens.accent_foreground),
        (&mut colors.info_hover, hover(tokens.accent)),
        (&mut colors.progress_bar, tokens.accent),
        (&mut colors.danger, tokens.danger),
        (&mut colors.danger_hover, hover(tokens.danger)),
        (&mut colors.warning, tokens.warning),
        (&mut colors.warning_hover, hover(tokens.warning)),
        (&mut colors.success, tokens.success),
        (&mut colors.success_hover, hover(tokens.success)),
        (&mut colors.overlay, tokens.scrim),
        (
            &mut colors.description_list_label,
            tokens.content.blend(tokens.hairline.opacity(0.2)),
        ),
        (&mut colors.switch_thumb, tokens.content),
    ] {
        *field = Some(hex(color));
    }
    // gpui-component's light default button plate is `background`, which
    // glass clears; the dark one derives from `input`.
    if tokens.mode == Mode::Light {
        colors.button = Some(hex(tokens.content));
    }
    config
}

/// Makes gpui-component paint `tokens` and redraws `window`.
pub(crate) fn write(tokens: &Tokens, window: Option<&mut Window>, cx: &mut App) {
    let registry = ThemeRegistry::global(cx);
    let base = match tokens.mode {
        Mode::Dark => registry.default_dark_theme().clone(),
        Mode::Light => registry.default_light_theme().clone(),
    };
    let config = Rc::new(component_config(tokens, &base));
    let theme = Theme::global_mut(cx);
    match tokens.mode {
        Mode::Dark => theme.dark_theme = config,
        Mode::Light => theme.light_theme = config,
    }
    Theme::change(component_mode(tokens.mode), window, cx);
}

fn component_mode(mode: Mode) -> ThemeMode {
    match mode {
        Mode::Dark => ThemeMode::Dark,
        Mode::Light => ThemeMode::Light,
    }
}

/// `#rrggbbaa`, which gpui-component parses back into the same color.
fn hex(color: Hsla) -> SharedString {
    format!("#{:08x}", u32::from(Rgba::from(color))).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::material::Material;

    /// gpui-component's resolved colors for `tokens`, over an empty base.
    fn resolved(tokens: &Tokens) -> Theme {
        let base = ThemeConfig {
            mode: component_mode(tokens.mode),
            ..ThemeConfig::default()
        };
        let mut theme = Theme::default();
        theme.apply_config(&Rc::new(component_config(tokens, &base)));
        theme
    }

    #[test]
    fn glass_changes_only_the_background_of_the_config() {
        for mode in [Mode::Dark, Mode::Light] {
            let base = ThemeConfig {
                mode: component_mode(mode),
                ..ThemeConfig::default()
            };
            let glass = component_config(&Tokens::new(mode, Material::Glass, false), &base);
            let opaque = component_config(&Tokens::new(mode, Material::Opaque, false), &base);
            let without_background = |mut config: ThemeConfig| {
                config.colors.background = None;
                format!("{config:?}")
            };
            assert_eq!(
                without_background(glass),
                without_background(opaque),
                "{mode:?}: the material may change only the component background"
            );
        }
    }

    #[test]
    fn root_paints_a_clear_background_under_glass_and_an_opaque_one_otherwise() {
        for mode in [Mode::Dark, Mode::Light] {
            let glass = resolved(&Tokens::new(mode, Material::Glass, false));
            let opaque = resolved(&Tokens::new(mode, Material::Opaque, false));

            assert_eq!(glass.background.a, 0., "{mode:?} glass background");
            assert_eq!(glass.tokens.background.color.a, 0.);
            assert_eq!(opaque.background.a, 1., "{mode:?} opaque background");
            assert_eq!(opaque.tokens.background.color.a, 1.);
        }
    }

    #[test]
    fn component_colors_round_trip_the_tokens() {
        let tokens = Tokens::new(Mode::Light, Material::Opaque, false);
        let theme = resolved(&tokens);
        let close = |a: Hsla, b: Hsla| {
            let (a, b) = (Rgba::from(a), Rgba::from(b));
            [a.r - b.r, a.g - b.g, a.b - b.b, a.a - b.a]
                .iter()
                .all(|delta| delta.abs() < 1. / 255.)
        };
        assert!(close(theme.foreground, tokens.text));
        assert!(close(theme.background, tokens.window));
        assert!(close(theme.popover, tokens.content));
        assert!(close(theme.ring, tokens.accent));
        assert_eq!(theme.mode, ThemeMode::Light);
    }
}
