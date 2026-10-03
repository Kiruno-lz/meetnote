//! `NavList` and `NavEntry`: the sidebar's route list (`Role::TabList`) and
//! an entry of it that opens a route, on gpui-base's unstyled `Button`. For
//! assistive technology an entry is a tab of the list (`Role::Tab`),
//! selected on its route. A press, Enter or Space opens the
//! route and leaves focus on the entry, so Tab goes on into the route's
//! page; Up and Down move focus to the entry before or after it without
//! opening it. The caller keeps each entry's focus handle, which the entries
//! beside it move focus to.

use super::{keyboard_focused, with_focus_ring};
use crate::theme;
use gpui::{
    div, prelude::*, AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, FontWeight,
    Interactivity, IntoElement, KeyDownEvent, Role, SharedString, Stateful, StyleRefinement,
    Window,
};

/// The labelled list the entries are in.
#[derive(IntoElement)]
pub(crate) struct NavList {
    base: Stateful<Div>,
    label: SharedString,
}

impl NavList {
    /// `label` names the list for assistive technology.
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: div().id(id),
            label: label.into(),
        }
    }
}

impl Styled for NavList {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for NavList {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

/// The entries.
impl ParentElement for NavList {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for NavList {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base.role(Role::TabList).aria_label(self.label)
    }
}

#[derive(IntoElement)]
pub(crate) struct NavEntry {
    base: gpui_base::Button,
    label: SharedString,
    focus: FocusHandle,
    selected: bool,
    previous: Option<FocusHandle>,
    next: Option<FocusHandle>,
}

impl NavEntry {
    /// `label` names the entry for assistive technology; the entry holds
    /// focus through `focus`.
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        focus: &FocusHandle,
    ) -> Self {
        Self {
            base: gpui_base::Button::new(id),
            label: label.into(),
            focus: focus.clone(),
            selected: false,
            previous: None,
            next: None,
        }
    }

    /// Whether the entry's route is the current one: filled with the
    /// selection wash and set in a heavier weight.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The entries Up and Down move focus to.
    pub(crate) fn beside(
        mut self,
        previous: Option<&FocusHandle>,
        next: Option<&FocusHandle>,
    ) -> Self {
        self.previous = previous.cloned();
        self.next = next.cloned();
        self
    }

    /// What a press, Enter or Space runs: opening the route.
    pub(crate) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.base = self.base.on_click(handler);
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

/// The entry's content, such as its label.
impl ParentElement for NavEntry {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl InteractiveElement for NavEntry {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for NavEntry {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let ring = keyboard_focused(&self.focus, window);
        let (previous, next) = (self.previous, self.next);
        let entry = self
            .base
            .track_focus(&self.focus)
            .role(Role::Tab)
            .accessibility_label(self.label)
            .aria_selected(self.selected)
            .relative()
            .h(tokens.control_height.md)
            .px(tokens.spacing.md)
            .flex()
            .items_center()
            .rounded(tokens.radius.sm)
            .text_sm()
            .cursor_pointer()
            .map(|entry| {
                if self.selected {
                    entry.bg(tokens.selection).font_weight(FontWeight::SEMIBOLD)
                } else {
                    entry.hover(move |style| style.bg(tokens.wash))
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if modifiers.modified() {
                    return;
                }
                let target = match event.keystroke.key.as_str() {
                    "up" => previous.as_ref(),
                    "down" => next.as_ref(),
                    _ => return,
                };
                if let Some(target) = target {
                    target.focus(window, cx);
                }
                cx.stop_propagation();
            });
        with_focus_ring(entry, ring, &self.focus, &tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_label_names_the_entry(cx: &mut gpui::TestAppContext) {
        let focus = cx.update(|cx| cx.focus_handle());
        assert_eq!(
            NavEntry::new("nav-Models", "Models", &focus).accessibility_label(),
            "Models"
        );
    }
}
