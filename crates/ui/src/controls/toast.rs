//! `Toast`: a frosted card (`BlurLevel::Menu`) that shows one notice, led
//! by its tone's icon. A warning or an error carries a close button, is an
//! alert for assistive technology and takes the presses on it. Information
//! and success fade on their own, are a status, and let presses through to
//! what lies under them, so a passing confirmation never blocks the next
//! click. Long messages scroll within the card; its close button stays
//! outside that scroll area. The card drops into place as it fades in and rises as it fades
//! out, changing only its opacity and position; under reduced motion it
//! shows and goes at once.

use super::{entrance, Button, Scrollbar};
use crate::state::NoticeTone;
use crate::theme::{self, BlurLevel};
use gpui::{
    div, prelude::*, px, Animation, AnimationExt as _, App, ClickEvent, ElementId, Pixels, Role,
    ScrollHandle, SharedString, Window,
};
use std::sync::Arc;

/// How far the card drops into place, and rises as it leaves.
const DROP: Pixels = px(8.);
/// The widest card; a longer notice wraps.
const MAX_WIDTH: Pixels = px(480.);
/// Feedback leaves the page readable even when a service returns many lines.
const MAX_MESSAGE_HEIGHT: Pixels = px(160.);
const ICON_SIZE: Pixels = px(16.);

type CloseHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub(crate) struct Toast {
    id: ElementId,
    label: SharedString,
    tone: NoticeTone,
    leaving: bool,
    close: Option<(SharedString, CloseHandler)>,
}

impl Toast {
    /// `label` is the notice the card shows, which names it for assistive
    /// technology. A toast with another `id` enters anew.
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        tone: NoticeTone,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tone,
            leaving: false,
            close: None,
        }
    }

    /// Plays the exit: the card rises as it fades out.
    pub(crate) fn leaving(mut self, leaving: bool) -> Self {
        self.leaving = leaving;
        self
    }

    /// The close button, named `label`, and what a press on it runs.
    pub(crate) fn on_close(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.close = Some((label.into(), Box::new(handler)));
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility(&self) -> (&SharedString, Role) {
        (&self.label, self.role())
    }

    fn role(&self) -> Role {
        if self.tone.fades() {
            Role::Status
        } else {
            Role::Alert
        }
    }
}

impl RenderOnce for Toast {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let scroll = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        let (icon, color) = match self.tone {
            NoticeTone::Info => ("icons/info.svg", tokens.text_muted),
            NoticeTone::Success => ("icons/circle-check.svg", tokens.success),
            NoticeTone::Warning => ("icons/triangle-alert.svg", tokens.warning),
            NoticeTone::Error => ("icons/circle-x.svg", tokens.danger),
        };
        let role = self.role();
        let closable = self.close.is_some();
        let row = div()
            .id(self.id.clone())
            .debug_selector(|| "notice-toast".into())
            .role(role)
            .aria_label(self.label.clone())
            .when(!self.tone.fades(), |row| row.occlude())
            .max_w(MAX_WIDTH)
            .min_h(tokens.control_height.lg)
            .pl(tokens.spacing.md)
            .pr(if closable {
                tokens.spacing.xs
            } else {
                tokens.spacing.md
            })
            .py(tokens.spacing.xs)
            .flex()
            .items_center()
            .gap(tokens.spacing.sm)
            .text_sm()
            .text_color(tokens.text)
            .child(
                gpui::svg()
                    .path(icon)
                    .flex_none()
                    .size(ICON_SIZE)
                    .text_color(color),
            )
            .child(
                div()
                    .id(ElementId::NamedChild(
                        Arc::new(self.id.clone()),
                        "message".into(),
                    ))
                    .debug_selector(|| "notice-toast-message".into())
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .max_h(MAX_MESSAGE_HEIGHT)
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .child(
                        div()
                            .debug_selector(|| "notice-toast-text".into())
                            .child(self.label),
                    )
                    .child(Scrollbar::vertical("notice-toast-scrollbar", &scroll)),
            )
            .children(self.close.map(|(label, handler)| {
                Button::new("notice-toast-close", label)
                    .debug_selector(|| "notice-toast-close".into())
                    .ghost()
                    .compact()
                    .icon("icons/close.svg")
                    .on_click(handler)
            }));
        let card = div().child(theme::frosted(tokens.radius.lg, BlurLevel::Menu, row));
        let duration = tokens.motion.menu;
        let id = Arc::new(self.id);
        if !self.leaving {
            return entrance(
                ElementId::NamedChild(id, "in".into()),
                card,
                duration,
                -DROP,
            );
        }
        if duration.is_zero() {
            return card.opacity(0.).into_any_element();
        }
        card.relative()
            .with_animation(
                ElementId::NamedChild(id, "out".into()),
                Animation::new(duration).with_easing(gpui::ease_in_out),
                |card, delta| card.opacity(1. - delta).top(-DROP * delta),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Material, Mode, Tokens};
    use gpui::{Context, Modifiers, Render, TestAppContext};
    use std::{cell::Cell, rc::Rc};

    /// A button filling the window under a toast of `tone`.
    struct Harness {
        tone: NoticeTone,
        presses: Rc<Cell<usize>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let presses = self.presses.clone();
            div()
                .size_full()
                .relative()
                .child(
                    Button::new("under", "Under")
                        .size_full()
                        .on_click(move |_, _, _| presses.set(presses.get() + 1)),
                )
                .child(div().absolute().top_0().left_0().child(Toast::new(
                    "toast",
                    "已导入 周会",
                    self.tone,
                )))
        }
    }

    fn presses_through(tone: NoticeTone, cx: &mut TestAppContext) -> usize {
        cx.update(|cx| {
            theme::install_for_test(Tokens::new(Mode::Light, Material::Opaque, true), cx)
        });
        let presses = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let presses = presses.clone();
            move |_, _| Harness { tone, presses }
        });
        cx.update(|window, cx| window.draw(cx).clear());
        let toast = cx.debug_bounds("notice-toast").expect("the toast shows");
        cx.simulate_click(toast.center(), Modifiers::none());
        presses.get()
    }

    #[gpui::test]
    fn a_fading_toast_lets_presses_through(cx: &mut TestAppContext) {
        assert_eq!(presses_through(NoticeTone::Success, cx), 1);
    }

    #[gpui::test]
    fn a_toast_that_stays_takes_the_presses_on_it(cx: &mut TestAppContext) {
        assert_eq!(presses_through(NoticeTone::Error, cx), 0);
    }

    #[test]
    fn the_notice_names_the_toast_and_only_a_fading_one_is_a_status() {
        for (tone, role) in [
            (NoticeTone::Info, Role::Status),
            (NoticeTone::Success, Role::Status),
            (NoticeTone::Warning, Role::Alert),
            (NoticeTone::Error, Role::Alert),
        ] {
            let toast = Toast::new("toast", "已导入 周会", tone);
            assert_eq!(toast.accessibility(), (&"已导入 周会".into(), role));
        }
    }
}
