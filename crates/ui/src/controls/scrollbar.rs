//! Scrollbars. `Scrollbar` is the vertical scrollbar of a scroll area, on
//! gpui-base's `Scrollbar`; it overlays the area it is a child of, which
//! must be `relative`. The area scrolls by wheel, trackpad and keyboard
//! without it, so a scrollbar is a pointer affordance only: no tab stop and
//! no label.
//!
//! gpui-base draws every scrollbar, including those inside text inputs,
//! from its global scrollbar theme; `follow_tokens` keeps that theme on the
//! token thumb and on the system scroller preference: shown while scrolling
//! for overlay scrollers, under the pointer otherwise.

use crate::theme::Tokens;
use gpui::{div, prelude::*, App, ElementId, IntoElement, ScrollHandle, Window};
use gpui_base::{ScrollbarMode, ScrollbarStyles, ScrollbarTheme};

#[derive(IntoElement)]
pub(crate) struct Scrollbar {
    id: ElementId,
    scroll: ScrollHandle,
}

impl Scrollbar {
    /// The vertical scrollbar of the area `scroll` tracks.
    pub(crate) fn vertical(id: impl Into<ElementId>, scroll: &ScrollHandle) -> Self {
        Self {
            id: id.into(),
            scroll: scroll.clone(),
        }
    }
}

impl RenderOnce for Scrollbar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().absolute().inset_0().child(
            gpui_base::Scrollbar::vertical(&self.scroll)
                .id(self.id)
                .viewport_from_layout(),
        )
    }
}

/// Points gpui-base's scrollbar theme at `tokens`.
pub(crate) fn follow_tokens(tokens: &Tokens, cx: &mut App) {
    let mode = if cx.should_auto_hide_scrollbars() {
        ScrollbarMode::Scrolling
    } else {
        ScrollbarMode::Hover
    };
    gpui_base::Theme::global_mut(cx).scrollbar = ScrollbarTheme {
        mode,
        styles: styles(tokens),
    };
}

fn styles(tokens: &Tokens) -> ScrollbarStyles {
    let thumb = |color| {
        move |thumb: gpui_base::ScrollbarThumbStyle| thumb.bg(color).radius(tokens.radius.sm)
    };
    ScrollbarStyles::default()
        .track_active(|track| track.border_color(tokens.hairline))
        .thumb(thumb(tokens.scrollbar_thumb))
        .thumb_hover(thumb(tokens.scrollbar_thumb_hover))
        .thumb_active(thumb(tokens.scrollbar_thumb_hover))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_app_points_every_gpui_base_scrollbar_at_the_system_preference(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            crate::controller::init_app(crate::theme::Accessibility::default(), cx);
            // gpui-base shows a scrollbar only while scrolling by default;
            // gpui's test platform reports classic scrollbars, which show
            // under the pointer.
            assert_eq!(
                gpui_base::Theme::global(cx).scrollbar.mode,
                ScrollbarMode::Hover
            );
        });
    }
}
