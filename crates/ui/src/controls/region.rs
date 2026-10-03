//! `Region`: a labelled group (`Role::Group`) that holds focus, such as a
//! card focus moves to when it comes up or a panel whose controls return
//! focus to what opened it once it closes. It is no tab stop, and its ring
//! shows while it holds keyboard focus itself.

use super::{keyboard_focused, with_focus_ring};
use crate::theme;
use gpui::{
    div, prelude::*, AnyElement, App, Div, ElementId, FocusHandle, Interactivity, IntoElement,
    Role, SharedString, Stateful, StyleRefinement, Window,
};

#[derive(IntoElement)]
pub(crate) struct Region {
    base: Stateful<Div>,
    label: SharedString,
    focus: FocusHandle,
}

impl Region {
    /// `label` names the group for assistive technology; the group holds
    /// focus through `focus`.
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        focus: &FocusHandle,
    ) -> Self {
        Self {
            base: div().id(id),
            label: label.into(),
            focus: focus.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

impl Styled for Region {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Region {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Region {}

impl ParentElement for Region {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Region {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let ring = keyboard_focused(&self.focus, window);
        let region = self
            .base
            .role(Role::Group)
            .aria_label(self.label)
            .track_focus(&self.focus)
            .relative();
        with_focus_ring(region, ring, &self.focus, &tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_label_names_the_group(cx: &mut gpui::TestAppContext) {
        let focus = cx.update(|cx| cx.focus_handle());
        assert_eq!(
            Region::new("review", "Pending change", &focus).accessibility_label(),
            "Pending change"
        );
    }
}
