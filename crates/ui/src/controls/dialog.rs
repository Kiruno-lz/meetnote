//! `ConfirmDialog`: a modal confirm/cancel question. gpui-base's
//! `AlertDialog` host owns modality: it traps focus (the window root cycles
//! Tab inside it), maps Enter to confirm and Esc to cancel,
//! ignores presses on the backdrop and orders the callbacks. MeetNote paints
//! a scrim, a frosted sheet (`BlurLevel::Sheet`) and, on the sheet, the
//! opaque panel that holds the question and its two buttons.

use super::{entrance, Button};
use crate::theme::{self, BlurLevel};
use gpui::{
    div, prelude::*, px, App, FocusHandle, IntoElement, Pixels, Role, SharedString, Window,
};
use gpui_base::{
    AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogDescription, AlertDialogTitle,
};
use std::rc::Rc;

const PANEL_WIDTH: Pixels = px(460.);
/// How far the sheet rises into place as it fades in.
const SHEET_RISE: Pixels = px(8.);

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct ConfirmDialog {
    title: SharedString,
    description: SharedString,
    confirm_label: SharedString,
    cancel_label: SharedString,
    focus: FocusHandle,
    on_confirm: Handler,
    on_cancel: Handler,
}

impl ConfirmDialog {
    /// `title` asks the question and names the dialog for assistive
    /// technology. `focus` must outlive the dialog's frames and hold focus
    /// while it is open, so Enter and Esc reach it.
    pub(crate) fn new(title: impl Into<SharedString>, focus: &FocusHandle) -> Self {
        Self {
            title: title.into(),
            description: SharedString::default(),
            confirm_label: SharedString::default(),
            cancel_label: SharedString::default(),
            focus: focus.clone(),
            on_confirm: Rc::new(|_, _| {}),
            on_cancel: Rc::new(|_, _| {}),
        }
    }

    pub(crate) fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = description.into();
        self
    }

    /// The confirm button and what Enter or a click on it runs.
    pub(crate) fn on_confirm(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.confirm_label = label.into();
        self.on_confirm = Rc::new(handler);
        self
    }

    /// The cancel button and what Esc or a click on it runs.
    pub(crate) fn on_cancel(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.cancel_label = label.into();
        self.on_cancel = Rc::new(handler);
        self
    }
}

impl RenderOnce for ConfirmDialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let viewport = window.viewport_size();
        let panel = div()
            .id("confirm-dialog")
            .debug_selector(|| "confirm-dialog".into())
            .role(Role::Group)
            .aria_label(self.title.clone())
            .w(PANEL_WIDTH)
            .p(tokens.spacing.xl)
            .rounded(tokens.radius.lg)
            .bg(tokens.content)
            .text_color(tokens.text)
            .flex()
            .flex_col()
            .gap(tokens.spacing.md)
            .child(
                AlertDialogTitle::new()
                    .text_size(tokens.font_size.lg)
                    .child(self.title),
            )
            .child(
                AlertDialogDescription::new()
                    .text_sm()
                    .text_color(tokens.text_muted)
                    .child(self.description),
            )
            .child(
                div()
                    .mt(tokens.spacing.sm)
                    .flex()
                    .justify_end()
                    .gap(tokens.spacing.sm)
                    .child(
                        AlertDialogCancel::new().child(
                            Button::new("dialog-cancel", self.cancel_label)
                                .debug_selector(|| "dialog-cancel".into())
                                .secondary(),
                        ),
                    )
                    .child(
                        AlertDialogAction::new().child(
                            Button::new("dialog-confirm", self.confirm_label)
                                .debug_selector(|| "dialog-confirm".into())
                                .primary(),
                        ),
                    ),
            );
        let sheet = theme::frosted(
            tokens.radius.xl,
            BlurLevel::Sheet,
            div().p(tokens.spacing.md).child(theme::layered(panel)),
        );
        let popup = div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(entrance(
                "confirm-dialog-in",
                div().child(sheet),
                tokens.motion.dialog,
                SHEET_RISE,
            ));
        let (on_confirm, on_cancel) = (self.on_confirm, self.on_cancel);
        AlertDialog::new(cx)
            .focus_handle(self.focus)
            // Presses on the scrim neither close the dialog nor reach the
            // page behind it. The host wraps the backdrop in a div of its
            // own that swallows every press it is hit by; out of flow and
            // sized to the window, the scrim leaves that div empty.
            .backdrop(
                div()
                    .absolute()
                    .w(viewport.width)
                    .h(viewport.height)
                    .bg(tokens.scrim)
                    .occlude(),
            )
            .popup(popup)
            .on_ok(move |_, window, cx| {
                on_confirm(window, cx);
                true
            })
            .on_cancel(move |_, window, cx| {
                on_cancel(window, cx);
                true
            })
    }
}
