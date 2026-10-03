// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! WCAG 2.1 contrast arithmetic and the glass tint alpha it implies. Pure
//! functions over sRGB colors; nothing here reads the platform.

use gpui::Rgba;

/// Body text on glass: WCAG AA for normal text.
pub(crate) const BODY_TEXT_CONTRAST: f32 = 4.5;
/// Muted text on glass: WCAG AA for large text and UI components.
pub(crate) const MUTED_TEXT_CONTRAST: f32 = 3.0;
/// `min_glass_alpha` searches the alpha in steps of `1 / ALPHA_STEPS`.
pub(crate) const ALPHA_STEPS: u32 = 1_000;

/// The darkest and the brightest desktop a glass window can sit on. The
/// blurred desktop behind the window is unknown, so glass must hold up over
/// both extremes.
pub(crate) const WORST_CASE_BACKDROPS: [Rgba; 2] = [
    Rgba {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 1.,
    },
    Rgba {
        r: 1.,
        g: 1.,
        b: 1.,
        a: 1.,
    },
];

/// WCAG 2.1 relative luminance of an opaque sRGB color, 0 (black) … 1 (white).
pub(crate) fn relative_luminance(color: Rgba) -> f32 {
    let linear = |channel: f32| {
        if channel <= 0.040_45 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// WCAG 2.1 contrast ratio of two opaque colors, 1 … 21, symmetric.
pub(crate) fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (a, b) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// `top` painted source-over onto the opaque `bottom`: the opaque color the
/// eye receives.
pub(crate) fn composite(top: Rgba, bottom: Rgba) -> Rgba {
    let alpha = top.a.clamp(0., 1.);
    let mix = |top: f32, bottom: f32| top * alpha + bottom * (1. - alpha);
    Rgba {
        r: mix(top.r, bottom.r),
        g: mix(top.g, bottom.g),
        b: mix(top.b, bottom.b),
        a: 1.,
    }
}

/// Whether `glass` (with its alpha) over every worst-case backdrop keeps
/// `text` at `BODY_TEXT_CONTRAST` and `muted` at `MUTED_TEXT_CONTRAST`.
pub(crate) fn glass_is_legible(glass: Rgba, text: Rgba, muted: Rgba) -> bool {
    WORST_CASE_BACKDROPS.iter().all(|backdrop| {
        let surface = composite(glass, *backdrop);
        contrast_ratio(composite(text, surface), surface) >= BODY_TEXT_CONTRAST
            && contrast_ratio(composite(muted, surface), surface) >= MUTED_TEXT_CONTRAST
    })
}

/// The least alpha, in steps of `1 / ALPHA_STEPS`, at which `tint` keeps
/// chrome text legible over every worst-case desktop. 1 (opaque) when no
/// translucent tint does.
pub(crate) fn min_glass_alpha(tint: Rgba, text: Rgba, muted: Rgba) -> f32 {
    (0..=ALPHA_STEPS)
        .map(|step| step as f32 / ALPHA_STEPS as f32)
        .find(|alpha| glass_is_legible(tint.alpha(*alpha), text, muted))
        .unwrap_or(1.)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(value: f32) -> Rgba {
        Rgba {
            r: value,
            g: value,
            b: value,
            a: 1.,
        }
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn contrast_ratio_matches_known_wcag_values() {
        let [black, white] = WORST_CASE_BACKDROPS;
        assert_close(relative_luminance(black), 0.);
        assert_close(relative_luminance(white), 1.);
        assert_close(contrast_ratio(black, white), 21.);
        assert_close(contrast_ratio(white, black), 21.);
        assert_close(contrast_ratio(grey(0.5), grey(0.5)), 1.);
        // #777777 on white is the textbook just-below-AA grey: 4.48:1.
        let ratio = contrast_ratio(gpui::rgb(0x777777), white);
        assert!((ratio - 4.48).abs() < 0.01, "#777 on white: {ratio}");
    }

    #[test]
    fn composite_paints_source_over_an_opaque_backdrop() {
        let [black, white] = WORST_CASE_BACKDROPS;
        let half_white = white.alpha(0.5);
        let mixed = composite(half_white, black);
        assert_close(mixed.r, 0.5);
        assert_close(mixed.g, 0.5);
        assert_close(mixed.b, 0.5);
        assert_close(mixed.a, 1.);
        assert_eq!(composite(white.alpha(0.), black), black);
        assert_eq!(composite(white, black), white);
    }

    #[test]
    fn min_glass_alpha_is_the_least_alpha_that_is_legible_over_both_extremes() {
        let tint = gpui::rgb(0x111318);
        let text = gpui::rgb(0xe9edf5);
        let muted = gpui::rgb(0x8992a3);

        let alpha = min_glass_alpha(tint, text, muted);

        assert!(alpha > 0. && alpha < 1., "a dark tint needs partial glass");
        assert!(glass_is_legible(tint.alpha(alpha), text, muted));
        let below = alpha - 1. / ALPHA_STEPS as f32;
        assert!(!glass_is_legible(tint.alpha(below), text, muted));
        for backdrop in WORST_CASE_BACKDROPS {
            let surface = composite(tint.alpha(alpha), backdrop);
            assert!(contrast_ratio(text, surface) >= BODY_TEXT_CONTRAST);
            assert!(contrast_ratio(muted, surface) >= MUTED_TEXT_CONTRAST);
        }
    }

    #[test]
    fn min_glass_alpha_falls_back_to_opaque_when_no_tint_is_legible() {
        let tint = gpui::rgb(0x808080);
        assert_eq!(min_glass_alpha(tint, tint, tint), 1.);
    }
}
