//! The tooltip of a control shown without its label: the label on a small
//! frosted tag (`BlurLevel::Menu`), a tooltip for assistive technology.
//! GPUI shows it once the pointer has rested on the control for 500 ms,
//! beside the pointer, and hides it as the pointer leaves or presses.

use crate::theme::{self, BlurLevel};
use gpui::{div, prelude::*, px, AnyView, App, Context, Pixels, SharedString, Window};

/// Where the tag sits from the pointer, which GPUI places it at: below the
/// cursor arrow, so the pointer never covers the label.
const POINTER_GAP: (Pixels, Pixels) = (px(6.), px(18.));

pub(super) struct Tooltip {
    label: SharedString,
}

impl Tooltip {
    /// Builds the tooltip of `label` whenever GPUI shows it.
    pub(super) fn builder(
        label: SharedString,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        move |_, cx| {
            let label = label.clone();
            cx.new(|_| Self { label }).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let tag = gpui_base::Tooltip::new("tooltip")
            .px(tokens.spacing.sm)
            .py(tokens.spacing.xs)
            .flex()
            .items_center()
            .text_xs()
            .text_color(tokens.text)
            .whitespace_nowrap()
            .child(self.label.clone());
        div().pl(POINTER_GAP.0).pt(POINTER_GAP.1).child(
            div()
                .debug_selector(|| "tooltip".into())
                .child(theme::frosted(tokens.radius.sm, BlurLevel::Menu, tag)),
        )
    }
}
