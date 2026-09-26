//! The theme: design tokens, the appearance in effect and how it reaches the
//! window.
//!
//! The saved `AppearanceSettings` resolve against the window's system
//! appearance (`ThemePreference::System`) and the system's reduce-transparency
//! flag into a light or dark palette and a glass or opaque material. The
//! resulting `Tokens` live in a gpui global that pages and controls read
//! through `tokens(cx)`; the same tokens set the window's background
//! appearance and gpui-base's scrollbar theme. Only this module holds color
//! literals.

mod contrast;
mod edge_fade;
mod frost;
mod material;
mod platform;
mod tokens;

pub(crate) use edge_fade::{scroll_faded, tail_faded_label};
pub(crate) use frost::{frosted, layered};
pub(crate) use material::{Material, Mode};
pub(crate) use platform::{system_accessibility, Accessibility};
pub(crate) use tokens::Tokens;

use crate::controls;
use gpui::{App, Global, Window, WindowAppearance, WindowBackgroundAppearance};
use settings::AppearanceSettings;

/// Backdrop blur strength of floating surfaces. Two levels bound the blur
/// kernels on screen; open/close animations never change the radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlurLevel {
    /// Menus, dropdowns and the Assistant input bar.
    Menu,
    /// Sheets and dialogs.
    Sheet,
}

impl BlurLevel {
    /// Blur radius in pixels.
    pub(crate) const fn radius(self) -> f32 {
        match self {
            Self::Menu => 16.,
            Self::Sheet => 32.,
        }
    }
}

struct ThemeState {
    preference: AppearanceSettings,
    accessibility: Accessibility,
    tokens: Tokens,
    /// What the last `apply` set on the window.
    background: WindowBackgroundAppearance,
}

impl Global for ThemeState {}

/// Installs the theme. Until `attach` applies the saved appearance to a
/// window, the tokens are the dark opaque palette.
pub(crate) fn init(accessibility: Accessibility, cx: &mut App) {
    cx.set_reduce_motion(accessibility.reduce_motion);
    let tokens = Tokens::new(Mode::Dark, Material::Opaque, cx.reduce_motion());
    cx.set_global(ThemeState {
        preference: AppearanceSettings::default(),
        accessibility,
        tokens,
        background: WindowBackgroundAppearance::Opaque,
    });
    controls::follow_tokens(&tokens, cx);
}

/// The tokens in effect.
pub(crate) fn tokens(cx: &App) -> &Tokens {
    &cx.global::<ThemeState>().tokens
}

/// Applies `preference` to `window` and follows the window's system
/// appearance from then on.
pub(crate) fn attach(preference: AppearanceSettings, window: &mut Window, cx: &mut App) {
    cx.global_mut::<ThemeState>().preference = preference;
    apply(window, cx);
    window
        .observe_window_appearance(|window, cx| {
            system_appearance_changed(window.appearance(), window, cx)
        })
        .detach();
}

/// Makes `preference` the appearance in effect; every window re-applies it
/// once the current update finishes.
pub(crate) fn set_preference(preference: AppearanceSettings, cx: &mut App) {
    let state = cx.global_mut::<ThemeState>();
    if state.preference == preference {
        return;
    }
    state.preference = preference;
    cx.defer(|cx| {
        for window in cx.windows() {
            window.update(cx, |_, window, cx| apply(window, cx)).ok();
        }
    });
}

/// Resolves the preference against `window`'s system appearance and applies
/// the result.
pub(crate) fn apply(window: &mut Window, cx: &mut App) {
    apply_for(window.appearance(), window, cx);
}

/// What the window's appearance observer runs. gpui's test window cannot
/// change its appearance, so tests call this with a simulated one.
pub(crate) fn system_appearance_changed(
    system: WindowAppearance,
    window: &mut Window,
    cx: &mut App,
) {
    apply_for(system, window, cx);
}

fn apply_for(system: WindowAppearance, window: &mut Window, cx: &mut App) {
    let reduce_motion = cx.reduce_motion();
    let state = cx.global_mut::<ThemeState>();
    let mode = material::resolve_mode(state.preference.theme, system);
    let material = material::effective_material(
        state.preference.material,
        state.accessibility.reduce_transparency,
    );
    let tokens = Tokens::new(mode, material, reduce_motion);
    let background = material.window_background_appearance();
    state.tokens = tokens;
    state.background = background;
    controls::follow_tokens(&tokens, cx);
    // Re-applied on every change: gpui's macOS window removes its
    // `NSVisualEffectView` whenever the value is not `Blurred`.
    window.set_background_appearance(background);
    window.refresh();
}

/// Installs `tokens` alone, without a window to follow, for primitive
/// tests.
#[cfg(test)]
pub(crate) fn install_for_test(tokens: Tokens, cx: &mut App) {
    cx.set_global(ThemeState {
        preference: AppearanceSettings::default(),
        accessibility: Accessibility::default(),
        tokens,
        background: tokens.material.window_background_appearance(),
    });
}

/// The background appearance the last `apply` set on the window. gpui's
/// test window does not record it.
#[cfg(test)]
pub(crate) fn window_background(cx: &App) -> WindowBackgroundAppearance {
    cx.global::<ThemeState>().background
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_files::SOURCES;

    #[test]
    fn blur_levels_are_the_menu_level_and_a_larger_sheet_level() {
        // `BlurLevel::radius` matches exhaustively, so a third level cannot
        // be added without a radius of its own.
        assert_eq!(BlurLevel::Menu.radius(), 16.);
        assert_eq!(BlurLevel::Sheet.radius(), 32.);
    }

    /// Color literal constructors; only `theme/` may call them.
    const COLOR_LITERALS: [&str; 3] = ["rgb(", "rgba(", "hsla("];

    #[test]
    fn sources_outside_the_theme_have_no_color_literals() {
        let found = SOURCES
            .iter()
            .filter(|(path, _)| !path.starts_with("theme/"))
            .flat_map(|(path, source)| {
                source.lines().enumerate().flat_map(move |(index, line)| {
                    COLOR_LITERALS.iter().flat_map(move |pattern| {
                        line.matches(*pattern)
                            .map(move |_| format!("{path}:{}: {pattern}", index + 1))
                    })
                })
            })
            .collect::<Vec<_>>();
        assert!(
            found.is_empty(),
            "{} color literals outside theme/: {found:#?}",
            found.len()
        );
    }
}
