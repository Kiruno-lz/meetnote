//! `Collapsible`: a disclosure on gpui-base's unstyled `Collapsible`. Its
//! header is a gpui-base button that names the region, reports whether it
//! is expanded and toggles it on a press, Enter or Space; the content
//! renders only while open. The header's styles and children are the
//! caller's; a press toggles it without moving focus.

use super::focus_ring;
use crate::theme;
use gpui::{
    prelude::*, AnyElement, App, ClickEvent, ElementId, Interactivity, IntoElement, MouseButton,
    SharedString, StyleRefinement, Window,
};

#[derive(IntoElement)]
pub(crate) struct Collapsible {
    header: gpui_base::Button,
    label: SharedString,
    open: bool,
    content: Option<AnyElement>,
}

impl Collapsible {
    /// `label` names the header for assistive technology; the header's
    /// children are what it shows.
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            header: gpui_base::Button::new(id),
            label: label.into(),
            open: false,
            content: None,
        }
    }

    /// Whether the content shows; closed by default.
    pub(crate) fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// What a press, Enter or Space on the header runs.
    pub(crate) fn on_toggle(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.header = self.header.on_click(handler);
        self
    }

    /// The region the header discloses.
    pub(crate) fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

/// The header's layout.
impl Styled for Collapsible {
    fn style(&mut self) -> &mut StyleRefinement {
        self.header.style()
    }
}

/// The header's content.
impl ParentElement for Collapsible {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.header.extend(elements);
    }
}

impl InteractiveElement for Collapsible {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.header.interactivity()
    }
}

impl RenderOnce for Collapsible {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let ring = focus_ring(theme::tokens(cx));
        let header = self
            .header
            .accessibility_label(self.label)
            .aria_expanded(self.open)
            .cursor_pointer()
            .focus_visible(move |style| style.shadow(vec![ring]))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
        let collapsible = gpui_base::Collapsible::new()
            .open(self.open)
            .flex()
            .flex_col()
            .child(header);
        match self.content {
            Some(content) => collapsible.content(content),
            None => collapsible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Material, Mode, Tokens};
    use gpui::{div, px, Context, Modifiers, Render, TestAppContext};
    use std::{cell::Cell, rc::Rc};

    struct Harness {
        open: bool,
        toggles: Rc<Cell<usize>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let toggles = self.toggles.clone();
            Collapsible::new("family", "Whisper")
                .debug_selector(|| "family-header".into())
                .open(self.open)
                .p_4()
                .on_toggle(cx.listener(move |harness, _, _, cx| {
                    toggles.set(toggles.get() + 1);
                    harness.open = !harness.open;
                    cx.notify();
                }))
                .child(div().child("Whisper"))
                .content(div().debug_selector(|| "family-content".into()).h(px(40.)))
        }
    }

    #[test]
    fn the_label_names_the_header() {
        assert_eq!(
            Collapsible::new("family", "Whisper").accessibility_label(),
            "Whisper"
        );
    }

    #[gpui::test]
    fn the_header_toggles_the_content_under_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            theme::install_for_test(Tokens::new(Mode::Dark, Material::Glass, false), cx)
        });
        let toggles = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view({
            let toggles = toggles.clone();
            move |_, _| Harness {
                open: false,
                toggles,
            }
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("family-content").is_none(), "closed");

        let header = cx.debug_bounds("family-header").unwrap();
        cx.simulate_click(header.center(), Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        let content = cx
            .debug_bounds("family-content")
            .expect("a press opens the content");
        assert_eq!(
            content.top(),
            header.bottom(),
            "the content follows the header"
        );
        assert!(
            cx.update(|window, cx| window.focused(cx)).is_none(),
            "a press must not move focus"
        );

        cx.update(|window, cx| window.focus_next(cx));
        assert!(
            cx.update(|window, cx| window.focused(cx)).is_some(),
            "the header is a tab stop"
        );
        cx.simulate_click(header.center(), Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("family-content").is_none(), "closed again");
        assert_eq!(toggles.get(), 2);
    }
}
