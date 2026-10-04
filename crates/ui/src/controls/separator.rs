//! `Separator`: a 1px horizontal divider in the hairline color. It adds no
//! height: the line is centred on the separator's top edge, straddling it
//! by half a pixel, as the styled separator it replaces drew it.

use crate::theme;
use gpui::{div, prelude::*, px, App, IntoElement, Window};

#[derive(IntoElement)]
pub(crate) struct Separator;

impl Separator {
    pub(crate) fn horizontal() -> Self {
        Self
    }
}

impl RenderOnce for Separator {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .h(px(1.))
                    .w_full()
                    .bg(theme::tokens(cx).hairline),
            )
    }
}
