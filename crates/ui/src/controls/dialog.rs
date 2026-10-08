//! `ConfirmDialog`: a modal confirm/cancel question. gpui-base's
//! `AlertDialog` host owns modality: it traps focus (the window root cycles
//! Tab inside it), maps Enter to confirm and Esc to cancel,
//! ignores presses on the backdrop and orders the callbacks. MeetNote paints
//! a scrim, a frosted sheet (`BlurLevel::Sheet`) and, on the sheet, the
//! opaque panel that holds the question and its two buttons. Under glass the
//! sheet shows only a narrow band of frost around the panel, its corners
//! concentric with the panel's; under the opaque material, with nothing to
//! frost, the sheet's border alone outlines the panel.

use super::{entrance, Button};
use crate::theme::{self, BlurLevel, Material, Tokens};
use gpui::{
    div, prelude::*, px, App, FocusHandle, FontWeight, IntoElement, Pixels, Role, SharedString,
    Window,
};
use gpui_base::{
    AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogDescription, AlertDialogTitle,
};
use std::rc::Rc;

const PANEL_WIDTH: Pixels = px(460.);
/// How far the sheet rises into place as it fades in.
const SHEET_RISE: Pixels = px(8.);
/// The border `theme::frosted` draws around the sheet.
const SHEET_BORDER: Pixels = px(1.);

/// How far the panel sits inside the sheet's outer edge: the sheet's border
/// and, under glass, a narrow band of frost.
fn sheet_gap(tokens: &Tokens) -> Pixels {
    match tokens.material {
        Material::Glass => SHEET_BORDER + tokens.spacing.xs,
        Material::Opaque => SHEET_BORDER,
    }
}

/// The panel's corner radius, concentric with the sheet's.
fn panel_radius(tokens: &Tokens) -> Pixels {
    concentric_radius(tokens.radius.xl, sheet_gap(tokens))
}

/// The corner radius of a surface inset `gap` inside one whose corners are
/// `outer`, so both curves share their centers.
fn concentric_radius(outer: Pixels, gap: Pixels) -> Pixels {
    (outer - gap).max(px(0.))
}

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
        // The panel's fill is opaque under either material: the question is
        // read on it, never on glass.
        let panel = div()
            .id("confirm-dialog")
            .debug_selector(|| "confirm-dialog".into())
            .role(Role::Group)
            .aria_label(self.title.clone())
            .w(PANEL_WIDTH)
            .p(tokens.spacing.xl)
            .rounded(panel_radius(&tokens))
            .bg(tokens.float)
            .text_color(tokens.text)
            .flex()
            .flex_col()
            .gap(tokens.spacing.xl)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(tokens.spacing.sm)
                    .child(
                        AlertDialogTitle::new()
                            .text_size(tokens.font_size.lg)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                div()
                                    .debug_selector(|| "dialog-title".into())
                                    .child(self.title),
                            ),
                    )
                    .child(
                        AlertDialogDescription::new()
                            .text_sm()
                            .text_color(tokens.text_muted)
                            .child(
                                div()
                                    .debug_selector(|| "dialog-description".into())
                                    .child(self.description),
                            ),
                    ),
            )
            .child(
                div()
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
            div()
                .p(sheet_gap(&tokens) - SHEET_BORDER)
                .child(theme::layered(panel)),
        );
        let popup = div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(entrance(
                "confirm-dialog-in",
                div().debug_selector(|| "dialog-sheet".into()).child(sheet),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Mode;
    use gpui::{Bounds, Context, Render, TestAppContext, VisualTestContext};

    const APPEARANCES: [(Mode, Material); 4] = [
        (Mode::Dark, Material::Glass),
        (Mode::Light, Material::Glass),
        (Mode::Dark, Material::Opaque),
        (Mode::Light, Material::Opaque),
    ];

    #[test]
    fn an_inset_radius_stays_concentric_and_never_negative() {
        assert_eq!(concentric_radius(px(16.), px(5.)), px(11.));
        assert_eq!(concentric_radius(px(16.), px(1.)), px(15.));
        assert_eq!(concentric_radius(px(4.), px(6.)), px(0.));
    }

    #[test]
    fn glass_shows_a_narrow_frost_band_and_opaque_only_the_hairline() {
        for (mode, material) in APPEARANCES {
            let tokens = Tokens::new(mode, material, false);
            let gap = sheet_gap(&tokens);
            match material {
                Material::Glass => assert!(
                    gap >= px(4.) && gap <= px(6.),
                    "{mode:?}: a narrow band of frost, not a frame: {gap:?}"
                ),
                Material::Opaque => assert_eq!(
                    gap, SHEET_BORDER,
                    "{mode:?}: without blur the sheet is only a hairline"
                ),
            }
            assert_eq!(
                panel_radius(&tokens) + gap,
                tokens.radius.xl,
                "{mode:?} {material:?}: the panel's corners follow the sheet's"
            );
        }
    }

    struct Harness {
        focus: FocusHandle,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                ConfirmDialog::new("Move the library?", &self.focus)
                    .description("A copy is written to the new place.")
                    .on_confirm("Move", |_, _| {})
                    .on_cancel("Keep", |_, _| {}),
            )
        }
    }

    #[track_caller]
    fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` is not rendered"))
    }

    #[gpui::test]
    fn the_panel_nests_in_the_sheet_and_lays_out_on_the_spacing_scale(cx: &mut TestAppContext) {
        cx.update(crate::controls::window::init);
        for (mode, material) in APPEARANCES {
            // Reduced motion shows the sheet in place at once.
            let tokens = Tokens::new(mode, material, true);
            cx.update(|cx| theme::install_for_test(tokens, cx));
            let (_, cx) = cx.add_window_view(|_, cx| Harness {
                focus: cx.focus_handle(),
            });
            cx.update(|window, cx| window.draw(cx).clear());
            let sheet = bounds(cx, "dialog-sheet");
            let panel = bounds(cx, "confirm-dialog");
            let gap = sheet_gap(&tokens);
            assert_eq!(
                [
                    panel.left() - sheet.left(),
                    panel.top() - sheet.top(),
                    sheet.right() - panel.right(),
                    sheet.bottom() - panel.bottom(),
                ],
                [gap; 4],
                "{mode:?} {material:?}: an even band around the panel"
            );

            let inset = tokens.spacing.xl;
            let title = bounds(cx, "dialog-title");
            let description = bounds(cx, "dialog-description");
            let cancel = bounds(cx, "dialog-cancel");
            let confirm = bounds(cx, "dialog-confirm");
            assert_eq!(title.origin, panel.origin + gpui::point(inset, inset));
            assert_eq!(description.left(), title.left(), "the text shares one edge");
            assert_eq!(
                description.top(),
                title.bottom() + tokens.spacing.sm,
                "the description follows its title closely"
            );
            assert_eq!(
                cancel.top(),
                description.bottom() + inset,
                "the buttons stand apart from the text"
            );
            assert_eq!(cancel.top(), confirm.top());
            assert_eq!(
                confirm.right(),
                panel.right() - inset,
                "the primary button ends the row at the panel's edge"
            );
            assert_eq!(cancel.right() + tokens.spacing.sm, confirm.left());
            assert_eq!(confirm.bottom(), panel.bottom() - inset);
        }
    }
}
