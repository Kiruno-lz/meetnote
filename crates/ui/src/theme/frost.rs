// Portions adapted from Zeron (https://github.com/zeronsh/zeron), Copyright (c) 2026 Wing, MIT License.

//! `frosted`: a floating surface (menu, dropdown, dialog sheet). Under glass
//! its whole subtree paints inside one scene layer, the backdrop blur first,
//! then the tint, the border and the content: in a single layer the blur
//! cannot be reordered above the card's own washes and dividers. Under the
//! opaque material the same surface is an opaque token fill with a shadow,
//! and no zui-only API is called.
//!
//! `layered` gives a subtree a fresh draw order inside a frosted card, whose
//! single layer otherwise groups equal orders by primitive kind.

use super::{tokens, BlurLevel, Material, Tokens};
use gpui::{
    div, point, prelude::*, px, AnyElement, App, Bounds, BoxShadow, Corners, Element, ElementId,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Window,
};

/// How a floating surface paints under the material in effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrostPaint {
    /// The backdrop blur painted before everything else; `None` paints none.
    pub(crate) blur: Option<Pixels>,
    /// The card's fill: a translucent tint over the blur, or opaque.
    pub(crate) fill: Hsla,
    pub(crate) border: Hsla,
    /// The drop shadow; floats on glass cast none.
    pub(crate) shadow: Option<Hsla>,
}

/// The paint of a floating surface at `level`. Pure: glass blurs and tints,
/// opaque fills with a shadow. A menu's text sits on its tint, so the menu
/// tint is the legible `frost`; a sheet's text sits on an opaque panel.
pub(crate) fn frost_paint(tokens: &Tokens, level: BlurLevel) -> FrostPaint {
    match tokens.material {
        Material::Glass => FrostPaint {
            blur: Some(px(level.radius())),
            fill: match level {
                BlurLevel::Menu => tokens.frost,
                BlurLevel::Sheet => tokens.sheet_frost,
            },
            border: tokens.hairline,
            shadow: None,
        },
        Material::Opaque => FrostPaint {
            blur: None,
            fill: tokens.float,
            border: tokens.border,
            shadow: Some(tokens.shadow),
        },
    }
}

/// Frosts `child`: a card of corner `radius` whose backdrop blurs at
/// `level` under glass. The card takes the child's size.
pub(crate) fn frosted(radius: Pixels, level: BlurLevel, child: impl IntoElement) -> Frosted {
    Frosted {
        radius,
        level,
        child: Some(child.into_any_element()),
        card: None,
        paint: None,
    }
}

pub(crate) struct Frosted {
    radius: Pixels,
    level: BlurLevel,
    child: Option<AnyElement>,
    card: Option<AnyElement>,
    paint: Option<FrostPaint>,
}

impl IntoElement for Frosted {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Frosted {
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
        let paint = frost_paint(tokens(cx), self.level);
        let mut card = div()
            .rounded(self.radius)
            .overflow_hidden()
            .border_1()
            .border_color(paint.border)
            .bg(paint.fill);
        if let Some(color) = paint.shadow {
            card = card.shadow(vec![BoxShadow {
                color,
                offset: point(px(0.), px(8.)),
                blur_radius: px(24.),
                spread_radius: px(0.),
                inset: false,
            }]);
        }
        let mut card = card.children(self.child.take()).into_any_element();
        let layout = card.request_layout(window, cx);
        self.card = Some(card);
        self.paint = Some(paint);
        (layout, ())
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
        if let Some(card) = &mut self.card {
            card.prepaint(window, cx);
        }
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
        let (Some(card), Some(paint)) = (&mut self.card, self.paint) else {
            return;
        };
        match paint.blur {
            Some(blur) => window.paint_layer(bounds, |window| {
                window.paint_backdrop_blur(bounds, Corners::all(self.radius), blur);
                card.paint(window, cx);
            }),
            None => card.paint(window, cx),
        }
    }
}

/// Paints `child` in its own scene layer under glass, above everything the
/// enclosing frosted card painted so far. Without a layer to escape (opaque)
/// it paints `child` in place.
pub(crate) fn layered(child: impl IntoElement) -> Layered {
    Layered {
        child: child.into_any_element(),
    }
}

pub(crate) struct Layered {
    child: AnyElement,
}

impl IntoElement for Layered {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Layered {
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
        match tokens(cx).material {
            Material::Glass => window.paint_layer(bounds, |window| self.child.paint(window, cx)),
            Material::Opaque => self.child.paint(window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;

    #[test]
    fn opaque_floats_fill_opaquely_with_a_shadow_and_glass_blurs_a_tint() {
        for mode in [Mode::Dark, Mode::Light] {
            for level in [BlurLevel::Menu, BlurLevel::Sheet] {
                let opaque = frost_paint(&Tokens::new(mode, Material::Opaque, false), level);
                assert_eq!(opaque.blur, None, "{mode:?} {level:?}: opaque never blurs");
                assert_eq!(opaque.fill.a, 1., "{mode:?} {level:?}: opaque fill");
                assert!(opaque.shadow.is_some(), "{mode:?} {level:?}: opaque shadow");

                let glass = frost_paint(&Tokens::new(mode, Material::Glass, false), level);
                assert_eq!(glass.blur, Some(px(level.radius())));
                assert!(glass.fill.a < 1., "{mode:?} {level:?}: glass tint");
                assert_eq!(
                    glass.shadow, None,
                    "{mode:?} {level:?}: glass casts no shadow"
                );
                assert_eq!(
                    glass.fill.alpha(1.),
                    opaque.fill,
                    "the tint is the float fill"
                );
            }
        }
    }
}
