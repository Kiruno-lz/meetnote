// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! `edge_faded`: content fades out at the edges of its region instead of
//! being cut off, for scroll lists, transcripts and long single-line labels
//! (a tail fade instead of an ellipsis).
//!
//! Under glass the fade is zui's per-pixel `EdgeFade` mask: over a
//! see-through backdrop no painted overlay can fade content out, because what
//! lies behind the window is not a paintable color. Under the opaque material
//! the region sits on a known opaque fill, so the fade is a gradient of that
//! fill painted over the content's edge and no zui-only API is called.
//!
//! Scroll fades are gated on the scroll handle at paint time, after the
//! scroll container's prepaint clamped this frame's offset: an edge fades
//! only while content is hidden past it.

use super::{tokens, Material};
use gpui::{
    div, fill, linear_color_stop, linear_gradient, prelude::*, px, AnyElement, App, Bounds,
    Corners, EdgeFade, Element, ElementId, Global, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, ScrollHandle, SharedString, Window, WindowId,
};
use std::cell::RefCell;

/// The ramp of a scroll list's fade.
pub(crate) const SCROLL_FADE: Pixels = px(24.);
/// The ramp of a label's tail fade.
pub(crate) const LABEL_FADE: Pixels = px(20.);

/// How a fade paints under a material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FadePaint {
    /// zui's per-pixel `EdgeFade` alpha mask.
    Mask,
    /// A gradient of the region's opaque backdrop fill over the faded edge.
    Overlay,
}

/// Pure: glass masks, opaque overlays its known backdrop.
pub(crate) fn fade_paint(material: Material) -> FadePaint {
    match material {
        Material::Glass => FadePaint::Mask,
        Material::Opaque => FadePaint::Overlay,
    }
}

// GPUI replaces rather than composes nested EdgeFade scopes. The active
// wrapper scopes are kept during painting so a label keeps its scroll
// container's vertical fade; window ids keep another window's draw from
// inheriting it.
#[derive(Default)]
struct PaintFades(RefCell<Vec<(WindowId, EdgeFade)>>);

impl Global for PaintFades {}

fn active_fade(window: &Window, cx: &App) -> Option<EdgeFade> {
    cx.try_global::<PaintFades>()?
        .0
        .borrow()
        .iter()
        .rev()
        .find(|(id, _)| *id == window.window_handle().window_id())
        .map(|(_, fade)| *fade)
}

fn inherit_vertical_fade(mut label: EdgeFade, parent: EdgeFade) -> EdgeFade {
    if !label.top && !label.bottom && (parent.top || parent.bottom) {
        label.bounds.origin.y = parent.bounds.origin.y;
        label.bounds.size.height = parent.bounds.size.height;
        label.top = parent.top;
        label.bottom = parent.bottom;
        label.band_top = Some(parent.band_top.unwrap_or(parent.band));
        label.band_bottom = Some(parent.band_bottom.unwrap_or(parent.band));
    }
    label
}

/// Fades `child` at its own edges over `backdrop`, the opaque fill the
/// region sits on under the opaque material. Select edges with
/// `fade_top`/`fade_bottom`/`fade_right`, gate them with `fade_overflow_y` or
/// `fade_label_overflow`.
pub(crate) fn edge_faded(band: Pixels, backdrop: Hsla, child: impl IntoElement) -> EdgeFaded {
    EdgeFaded {
        band,
        backdrop,
        radius: px(0.),
        top: false,
        bottom: false,
        right: false,
        scroll_y: None,
        scroll_x: None,
        child: child.into_any_element(),
    }
}

/// A vertical scroll list: `child` must track `scroll`. Both edges fade
/// while content is hidden past them.
pub(crate) fn scroll_faded(
    scroll: &ScrollHandle,
    backdrop: Hsla,
    child: impl IntoElement,
) -> EdgeFaded {
    edge_faded(SCROLL_FADE, backdrop, child)
        .fade_top(true)
        .fade_bottom(true)
        .fade_overflow_y(scroll)
}

/// A single-line label that fades out at its tail instead of ending in an
/// ellipsis when `text` is wider than the room it gets.
pub(crate) fn tail_faded_label(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    backdrop: Hsla,
) -> EdgeFaded {
    let scroll = ScrollHandle::new();
    edge_faded(
        LABEL_FADE,
        backdrop,
        div()
            .id(id)
            .w_full()
            .min_w_0()
            .flex()
            .overflow_hidden()
            .track_scroll(&scroll)
            .child(div().flex_none().whitespace_nowrap().child(text.into())),
    )
    .fade_right(true)
    .fade_label_overflow(&scroll)
}

pub(crate) struct EdgeFaded {
    band: Pixels,
    backdrop: Hsla,
    /// The corner radius of the region's backdrop, which the overlays follow.
    radius: Pixels,
    top: bool,
    bottom: bool,
    right: bool,
    scroll_y: Option<ScrollHandle>,
    scroll_x: Option<ScrollHandle>,
    child: AnyElement,
}

impl EdgeFaded {
    /// The region's backdrop has corners of `radius`; the opaque fallback's
    /// overlays round the same corners instead of painting past them.
    pub(crate) fn rounded(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }

    pub(crate) fn fade_top(mut self, on: bool) -> Self {
        self.top = on;
        self
    }

    pub(crate) fn fade_bottom(mut self, on: bool) -> Self {
        self.bottom = on;
        self
    }

    pub(crate) fn fade_right(mut self, on: bool) -> Self {
        self.right = on;
        self
    }

    /// Gates the vertical fades on `handle`'s overflow, read at paint time.
    /// Render-time gating would ride the last frame's offset, which goes
    /// stale on the frame content shrinks.
    pub(crate) fn fade_overflow_y(mut self, handle: &ScrollHandle) -> Self {
        self.scroll_y = Some(handle.clone());
        self
    }

    /// Gates the right fade on a label's horizontal overflow and eases it in
    /// as the overflow grows to one band, so a label a fraction too wide does
    /// not suddenly dim its last characters.
    pub(crate) fn fade_label_overflow(mut self, handle: &ScrollHandle) -> Self {
        self.scroll_x = Some(handle.clone());
        self
    }

    /// The active edges this frame and the extra width the right ramp starts
    /// beyond the region while it eases in.
    fn active_edges(&self) -> (bool, bool, bool, Pixels) {
        let (mut top, mut bottom, mut right) = (self.top, self.bottom, self.right);
        if let Some(scroll) = &self.scroll_y {
            let scrolled = -f32::from(scroll.offset().y);
            let max_scroll = f32::from(scroll.max_offset().y);
            top &= scrolled > 1.;
            bottom &= scrolled < max_scroll - 1.;
        }
        let mut outset_right = px(0.);
        if let Some(scroll) = &self.scroll_x {
            let overflow =
                (f32::from(scroll.max_offset().x) + f32::from(scroll.offset().x)).max(0.);
            right &= overflow > 0.;
            outset_right = px(label_fade_outset(overflow, f32::from(self.band)));
        }
        (top, bottom, right, outset_right)
    }
}

impl IntoElement for EdgeFaded {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EdgeFaded {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let (top, bottom, right, outset_right) = self.active_edges();
        if !(top || bottom || right) {
            self.child.paint(window, cx);
            return;
        }
        match fade_paint(tokens(cx).material) {
            FadePaint::Mask => {
                let mut region = bounds;
                region.size.width += outset_right;
                let mut fade = EdgeFade {
                    bounds: region,
                    band: self.band,
                    band_top: None,
                    band_bottom: None,
                    top,
                    bottom,
                    left: false,
                    right,
                };
                if self.scroll_x.is_some() {
                    if let Some(parent) = active_fade(window, cx) {
                        fade = inherit_vertical_fade(fade, parent);
                    }
                }
                if !cx.has_global::<PaintFades>() {
                    cx.set_global(PaintFades::default());
                }
                let window_id = window.window_handle().window_id();
                cx.global::<PaintFades>()
                    .0
                    .borrow_mut()
                    .push((window_id, fade));
                window.with_edge_fade(Some(fade), |window| self.child.paint(window, cx));
                cx.global::<PaintFades>().0.borrow_mut().pop();
            }
            FadePaint::Overlay => {
                self.child.paint(window, cx);
                let opaque = linear_color_stop(self.backdrop, 0.);
                let clear = linear_color_stop(self.backdrop.alpha(0.), 1.);
                let band = self.band.min(bounds.size.height);
                let radius = self.radius.min(band);
                if top {
                    let edge = Bounds::new(bounds.origin, gpui::size(bounds.size.width, band));
                    window.paint_quad(
                        fill(edge, linear_gradient(180., opaque, clear)).corner_radii(Corners {
                            top_left: radius,
                            top_right: radius,
                            ..Corners::default()
                        }),
                    );
                }
                if bottom {
                    let edge = Bounds::new(
                        gpui::point(bounds.left(), bounds.bottom() - band),
                        gpui::size(bounds.size.width, band),
                    );
                    window.paint_quad(fill(edge, linear_gradient(0., opaque, clear)).corner_radii(
                        Corners {
                            bottom_left: radius,
                            bottom_right: radius,
                            ..Corners::default()
                        },
                    ));
                }
                if right {
                    let width = self.band.min(bounds.size.width);
                    let edge = Bounds::new(
                        gpui::point(bounds.right() + outset_right - width, bounds.top()),
                        gpui::size(width, bounds.size.height),
                    );
                    // Clipped to the label, so an easing ramp starts beyond it.
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        window.paint_quad(fill(edge, linear_gradient(270., opaque, clear)))
                    });
                }
            }
        }
    }
}

fn label_fade_outset(overflow: f32, band: f32) -> f32 {
    let band = band.max(1.);
    let progress = (overflow / band).clamp(0., 1.);
    let eased = progress * progress * (3. - 2. * progress);
    band * (1. - eased)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render};
    use std::rc::Rc;

    #[test]
    fn glass_masks_and_opaque_overlays_the_backdrop() {
        assert_eq!(fade_paint(Material::Glass), FadePaint::Mask);
        assert_eq!(fade_paint(Material::Opaque), FadePaint::Overlay);
    }

    #[test]
    fn label_fade_enters_continuously_and_settles_at_one_band() {
        let band = 20.;
        assert_eq!(label_fade_outset(0., band), band);
        assert_eq!(label_fade_outset(band, band), 0.);
        assert_eq!(label_fade_outset(100., band), 0.);
        let mut previous = 1.;
        for step in 0..=400 {
            let overflow = step as f32 / 10.;
            let edge_alpha = (label_fade_outset(overflow, band) / band).powi(2);
            assert!(edge_alpha <= previous);
            assert!(previous - edge_alpha < 0.02);
            previous = edge_alpha;
        }
    }

    /// A glass window: a scroll list with a label that overflows when
    /// `width` is narrower than its text.
    struct Fixture {
        width: f32,
        top: bool,
        bottom: bool,
        observed: Rc<RefCell<Vec<Option<EdgeFade>>>>,
    }

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let probe = |width: f32| {
                let observed = self.observed.clone();
                gpui::canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| observed.borrow_mut().push(active_fade(window, cx)),
                )
                .w(px(width))
                .h(px(20.))
                .flex_none()
            };
            let scroll = ScrollHandle::new();
            let label = edge_faded(
                px(20.),
                gpui::black(),
                div()
                    .id("label")
                    .w(px(self.width))
                    .overflow_hidden()
                    .track_scroll(&scroll)
                    .flex()
                    .child(probe(200.)),
            )
            .fade_right(true)
            .fade_label_overflow(&scroll);
            div()
                .child(
                    edge_faded(
                        px(24.),
                        gpui::black(),
                        div().w(px(300.)).h(px(100.)).child(label).child(probe(20.)),
                    )
                    .fade_top(self.top)
                    .fade_bottom(self.bottom),
                )
                .child(probe(20.))
        }
    }

    #[gpui::test]
    fn a_label_keeps_its_scroll_fade_and_fades_its_tail_only_when_it_overflows(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            crate::theme::install_for_test(
                crate::theme::Tokens::new(crate::theme::Mode::Dark, Material::Glass, false),
                cx,
            );
        });
        let observed = Rc::new(RefCell::new(Vec::new()));
        let handle = cx.add_window(|_, _| Fixture {
            width: 80.,
            top: true,
            bottom: true,
            observed: observed.clone(),
        });
        for (top, bottom) in [(true, true), (true, false), (false, true), (false, false)] {
            for width in [80., 240., 80.] {
                handle
                    .update(cx, |view, _, cx| {
                        view.width = width;
                        view.top = top;
                        view.bottom = bottom;
                        cx.notify();
                    })
                    .unwrap();
                observed.borrow_mut().clear();
                cx.update_window(handle.into(), |_, window, cx| {
                    window.draw(cx).clear();
                    assert!(active_fade(window, cx).is_none(), "paint scope leaked");
                })
                .unwrap();
                let seen = observed.borrow();
                let [label, sibling, outside] = &seen[seen.len() - 3..] else {
                    unreachable!()
                };
                assert!(outside.is_none(), "fade leaked outside the list");
                let overflowing = width < 200.;
                if top || bottom {
                    let label = label.unwrap();
                    let sibling = sibling.unwrap();
                    assert_eq!((label.top, label.bottom), (top, bottom));
                    assert_eq!(label.bounds.origin.y, sibling.bounds.origin.y);
                    assert_eq!(label.bounds.size.height, sibling.bounds.size.height);
                    assert_eq!(label.right, overflowing);
                    assert!(!sibling.right, "the label fade leaked to a sibling");
                } else {
                    assert!(sibling.is_none());
                    assert_eq!(label.is_some(), overflowing);
                }
            }
        }
    }
}
