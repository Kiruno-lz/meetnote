//! L1: the pixel comparison the L3 screenshots (`visual.rs`) use.

#[path = "support/pixels.rs"]
mod pixels;

use pixels::{compare, diff, Comparison, Rgba};

const SIDE: u32 = 100;

fn gray(value: u8) -> Rgba {
    Rgba {
        width: SIDE,
        height: SIDE,
        pixels: [value, value, value, 255].repeat((SIDE * SIDE) as usize),
    }
}

/// `base` with the first `count` pixels' red channel moved by `delta`.
fn with_changed_pixels(base: &Rgba, count: usize, delta: u8) -> Rgba {
    let mut pixels = base.pixels.clone();
    for pixel in pixels.chunks_exact_mut(4).take(count) {
        pixel[0] = pixel[0].wrapping_add(delta);
    }
    Rgba { pixels, ..*base }
}

#[test]
fn identical_images_have_no_differing_pixels() {
    let base = gray(100);
    let comparison = compare(&base, &gray(100));
    assert_eq!(
        comparison,
        Comparison::Compared {
            differing: 0,
            total: 10_000
        }
    );
    assert!(!comparison.changed());
}

#[test]
fn one_percent_differing_pixels_is_a_change() {
    let base = gray(100);
    let comparison = compare(&base, &with_changed_pixels(&base, 100, 3));
    assert_eq!(
        comparison,
        Comparison::Compared {
            differing: 100,
            total: 10_000
        }
    );
    assert!(comparison.changed());
}

#[test]
fn under_half_a_percent_differing_pixels_is_no_change() {
    let base = gray(100);
    let comparison = compare(&base, &with_changed_pixels(&base, 40, 3));
    assert_eq!(
        comparison,
        Comparison::Compared {
            differing: 40,
            total: 10_000
        }
    );
    assert!(!comparison.changed());
}

#[test]
fn exactly_half_a_percent_is_the_last_unchanged_ratio() {
    let base = gray(100);
    assert!(!compare(&base, &with_changed_pixels(&base, 50, 3)).changed());
    assert!(compare(&base, &with_changed_pixels(&base, 51, 3)).changed());
}

#[test]
fn a_channel_differs_only_beyond_the_tolerance() {
    let base = gray(100);
    let within = compare(&base, &with_changed_pixels(&base, 10_000, 2));
    assert_eq!(
        within,
        Comparison::Compared {
            differing: 0,
            total: 10_000
        }
    );
    let lower = compare(&with_changed_pixels(&base, 10_000, 3), &base);
    assert_eq!(
        lower,
        Comparison::Compared {
            differing: 10_000,
            total: 10_000
        }
    );
}

#[test]
fn alpha_counts_as_a_channel() {
    let base = gray(100);
    let mut translucent = gray(100);
    translucent.pixels[3] = 200;
    assert_eq!(
        compare(&base, &translucent),
        Comparison::Compared {
            differing: 1,
            total: 10_000
        }
    );
}

#[test]
fn a_size_change_is_a_change() {
    let base = gray(100);
    let wider = Rgba {
        width: SIDE * 2,
        height: SIDE / 2,
        pixels: base.pixels.clone(),
    };
    let comparison = compare(&base, &wider);
    assert_eq!(
        comparison,
        Comparison::SizeChanged {
            expected: (SIDE, SIDE),
            actual: (SIDE * 2, SIDE / 2)
        }
    );
    assert!(comparison.changed());
    assert!(diff(&base, &wider).is_none());
}

#[test]
fn the_diff_marks_differing_pixels_red_over_the_dimmed_baseline() {
    let base = gray(100);
    let diff = diff(&base, &with_changed_pixels(&base, 1, 3)).unwrap();
    assert_eq!((diff.width, diff.height), (SIDE, SIDE));
    assert_eq!(&diff.pixels[..8], &[255, 0, 0, 255, 25, 25, 25, 255]);
}
