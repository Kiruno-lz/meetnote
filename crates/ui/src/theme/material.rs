// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! Resolving the saved appearance against the system: light or dark, glass
//! or opaque. Pure functions; the caller supplies the system state.

use gpui::{WindowAppearance, WindowBackgroundAppearance};
use settings::{MaterialPreference, ThemePreference};

/// The palette in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Light,
    Dark,
}

/// The window chrome material in effect. The content plane is opaque under
/// either material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Material {
    /// Translucent chrome over the blurred desktop.
    Glass,
    /// Opaque chrome.
    Opaque,
}

impl Material {
    /// How the platform composites the window behind MeetNote's paint. It is
    /// re-applied after every theme or material change: gpui's macOS window
    /// removes its `NSVisualEffectView` whenever the value is not `Blurred`.
    pub(crate) fn window_background_appearance(self) -> WindowBackgroundAppearance {
        match self {
            Self::Glass => WindowBackgroundAppearance::Blurred,
            Self::Opaque => WindowBackgroundAppearance::Opaque,
        }
    }
}

/// Glass only when the user chose it and the system does not ask to reduce
/// transparency.
pub(crate) fn effective_material(
    preference: MaterialPreference,
    reduce_transparency: bool,
) -> Material {
    match preference {
        MaterialPreference::Glass if !reduce_transparency => Material::Glass,
        MaterialPreference::Glass | MaterialPreference::Opaque => Material::Opaque,
    }
}

/// The palette for `preference` when the window's system appearance is
/// `system`.
pub(crate) fn resolve_mode(preference: ThemePreference, system: WindowAppearance) -> Mode {
    match preference {
        ThemePreference::Light => Mode::Light,
        ThemePreference::Dark => Mode::Dark,
        ThemePreference::System => match system {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Mode::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_material_is_glass_only_for_glass_without_reduced_transparency() {
        for (preference, reduce_transparency, expected) in [
            (MaterialPreference::Glass, false, Material::Glass),
            (MaterialPreference::Glass, true, Material::Opaque),
            (MaterialPreference::Opaque, false, Material::Opaque),
            (MaterialPreference::Opaque, true, Material::Opaque),
        ] {
            assert_eq!(
                effective_material(preference, reduce_transparency),
                expected,
                "{preference:?}, reduce transparency {reduce_transparency}"
            );
        }
    }

    #[test]
    fn glass_blurs_the_window_and_opaque_does_not() {
        assert_eq!(
            Material::Glass.window_background_appearance(),
            WindowBackgroundAppearance::Blurred
        );
        assert_eq!(
            Material::Opaque.window_background_appearance(),
            WindowBackgroundAppearance::Opaque
        );
    }

    #[test]
    fn system_preference_follows_the_window_and_explicit_ones_do_not() {
        let systems = [
            (WindowAppearance::Light, Mode::Light),
            (WindowAppearance::VibrantLight, Mode::Light),
            (WindowAppearance::Dark, Mode::Dark),
            (WindowAppearance::VibrantDark, Mode::Dark),
        ];
        for (system, followed) in systems {
            assert_eq!(resolve_mode(ThemePreference::System, system), followed);
            assert_eq!(resolve_mode(ThemePreference::Light, system), Mode::Light);
            assert_eq!(resolve_mode(ThemePreference::Dark, system), Mode::Dark);
        }
    }
}
