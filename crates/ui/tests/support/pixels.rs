//! Pixel comparison for the L3 screenshots in `visual.rs`. Its unit tests
//! are the `pixels` target, so they run in the default `cargo test -p ui`.
//!
//! A pixel differs when any RGBA channel differs by more than
//! `CHANNEL_TOLERANCE`. A screenshot changed when its size changed or more
//! than `MAX_DIFFERING_PER_MILLE` / 1000 (0.5%) of its pixels differ.

pub const CHANNEL_TOLERANCE: u8 = 2;
pub const MAX_DIFFERING_PER_MILLE: usize = 5;

/// A row-major RGBA8 image.
#[derive(Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Comparison {
    /// The sizes differ; no pixel is compared.
    SizeChanged {
        expected: (u32, u32),
        actual: (u32, u32),
    },
    Compared {
        differing: usize,
        total: usize,
    },
}

impl Comparison {
    pub fn changed(&self) -> bool {
        match *self {
            Comparison::SizeChanged { .. } => true,
            Comparison::Compared { differing, total } => {
                differing * 1000 > total * MAX_DIFFERING_PER_MILLE
            }
        }
    }
}

fn pixel_differs(expected: &[u8], actual: &[u8]) -> bool {
    expected
        .iter()
        .zip(actual)
        .any(|(expected, actual)| expected.abs_diff(*actual) > CHANNEL_TOLERANCE)
}

pub fn compare(expected: &Rgba, actual: &Rgba) -> Comparison {
    if (expected.width, expected.height) != (actual.width, actual.height) {
        return Comparison::SizeChanged {
            expected: (expected.width, expected.height),
            actual: (actual.width, actual.height),
        };
    }
    let pairs = expected
        .pixels
        .chunks_exact(4)
        .zip(actual.pixels.chunks_exact(4));
    Comparison::Compared {
        differing: pairs
            .filter(|(expected, actual)| pixel_differs(expected, actual))
            .count(),
        total: (expected.width * expected.height) as usize,
    }
}

/// The differing pixels in opaque red over the expected image at a quarter
/// of its brightness; `None` when the sizes differ.
pub fn diff(expected: &Rgba, actual: &Rgba) -> Option<Rgba> {
    if (expected.width, expected.height) != (actual.width, actual.height) {
        return None;
    }
    let pixels = expected
        .pixels
        .chunks_exact(4)
        .zip(actual.pixels.chunks_exact(4))
        .flat_map(|(expected, actual)| {
            if pixel_differs(expected, actual) {
                [255, 0, 0, 255]
            } else {
                [expected[0] / 4, expected[1] / 4, expected[2] / 4, 255]
            }
        })
        .collect();
    Some(Rgba {
        width: expected.width,
        height: expected.height,
        pixels,
    })
}
