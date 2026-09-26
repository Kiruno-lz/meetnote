//! `Tree` and `TreeItem`: a labelled tree (`Role::Tree`) that is one tab
//! stop, and its items (`Role::TreeItem` with their level, selection and
//! expansion). The tree keeps keyboard focus itself; the selected item is
//! its active descendant for assistive technology and shows the focus ring
//! while the tree holds keyboard focus. Up and Down step through the visible
//! items, Left collapses an item or moves to its parent and Right expands it
//! or moves to its first child (`TreeStep`, through gpui-base's tree key
//! bindings); the owner decides what each step selects, and the tree then
//! scrolls the new selection into view. A press on an item focuses the
//! tree, without the ring, and runs the item's click handler.
//!
//! `disclosure` is an item's expand arrow, a pointer affordance: the
//! keyboard expands and collapses with Right and Left.

use super::{keyboard_focused, window, with_focus_ring};
use crate::theme;
use gpui::{
    div, prelude::*, px, AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, Interactivity,
    IntoElement, Role, SharedString, Stateful, StyleRefinement, Window,
};
use gpui_base::actions::{SelectDown, SelectLeft, SelectRight, SelectUp};
use std::rc::Rc;

/// A key's step through a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeStep {
    /// Up: the visible item before the selected one.
    Previous,
    /// Down: the visible item after it.
    Next,
    /// Left: collapse the item, or move to its parent.
    Collapse,
    /// Right: expand the item, or move to its first child.
    Expand,
}

type StepHandler = Rc<dyn Fn(&TreeStep, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct Tree {
    base: Stateful<Div>,
    label: SharedString,
    focus: FocusHandle,
    has_items: bool,
    has_selection: bool,
    on_step: Option<StepHandler>,
}

impl Tree {
    /// `label` names the tree for assistive technology; the tree holds
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
            has_items: true,
            has_selection: false,
            on_step: None,
        }
    }

    /// Whether the tree lists any item; an empty tree is no tab stop.
    pub(crate) fn has_items(mut self, has_items: bool) -> Self {
        self.has_items = has_items;
        self
    }

    /// Whether one of its items is selected, which then shows the ring in
    /// place of the tree.
    pub(crate) fn has_selection(mut self, has_selection: bool) -> Self {
        self.has_selection = has_selection;
        self
    }

    /// What a step key runs while the tree holds focus.
    pub(crate) fn on_step(
        mut self,
        handler: impl Fn(&TreeStep, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_step = Some(Rc::new(handler));
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

impl Styled for Tree {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Tree {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Tree {}

/// The items.
impl ParentElement for Tree {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Tree {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let ring = !self.has_selection && keyboard_focused(&self.focus, window);
        let step = |step: TreeStep, handler: &Option<StepHandler>| {
            let handler = handler.clone();
            move |window: &mut Window, cx: &mut App| {
                if let Some(handler) = &handler {
                    handler(&step, window, cx);
                    window::reveal_focus(window, cx);
                }
            }
        };
        let (previous, next, collapse, expand) = (
            step(TreeStep::Previous, &self.on_step),
            step(TreeStep::Next, &self.on_step),
            step(TreeStep::Collapse, &self.on_step),
            step(TreeStep::Expand, &self.on_step),
        );
        let tree = self
            .base
            .role(Role::Tree)
            .aria_label(self.label)
            .track_focus(&self.focus.clone().tab_stop(self.has_items))
            .key_context(gpui_base::tree_key_context())
            .relative()
            .on_action(move |_: &SelectUp, window, cx| previous(window, cx))
            .on_action(move |_: &SelectDown, window, cx| next(window, cx))
            .on_action(move |_: &SelectLeft, window, cx| collapse(window, cx))
            .on_action(move |_: &SelectRight, window, cx| expand(window, cx));
        with_focus_ring(tree, ring, &self.focus, &tokens)
    }
}

#[derive(IntoElement)]
pub(crate) struct TreeItem {
    base: Stateful<Div>,
    label: SharedString,
    tree: FocusHandle,
    level: usize,
    selected: bool,
    expanded: Option<bool>,
}

impl TreeItem {
    /// `label` names the item for assistive technology; `tree` is the focus
    /// handle of the tree it is in.
    pub(crate) fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        tree: &FocusHandle,
    ) -> Self {
        Self {
            base: div().id(id),
            label: label.into(),
            tree: tree.clone(),
            level: 0,
            selected: false,
            expanded: None,
        }
    }

    /// How deep the item is: 0 for a root.
    pub(crate) fn depth(mut self, depth: usize) -> Self {
        self.level = depth;
        self
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether an item with children shows them; `None` for a leaf.
    pub(crate) fn expanded(mut self, expanded: Option<bool>) -> Self {
        self.expanded = expanded;
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

impl Styled for TreeItem {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for TreeItem {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for TreeItem {}

impl ParentElement for TreeItem {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for TreeItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let ring = self.selected && keyboard_focused(&self.tree, window);
        let item = self
            .base
            .role(Role::TreeItem)
            .aria_label(self.label)
            .aria_level(self.level + 1)
            .aria_selected(self.selected)
            .when_some(self.expanded, |item, expanded| item.aria_expanded(expanded))
            .when(self.selected, |item| item.aria_active_descendant())
            .relative();
        with_focus_ring(item, ring, &self.tree, &tokens)
    }
}

/// The expand arrow of an item with children: a press toggles it.
pub(crate) fn disclosure(
    id: impl Into<ElementId>,
    expanded: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .mr_1()
        .child(if expanded { "▾" } else { "▸" })
        .on_click(on_toggle)
}

/// The room a leaf leaves where an item with children has its arrow.
pub(crate) fn disclosure_space() -> Div {
    div().w(px(10.))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_labels_name_the_tree_and_its_items(cx: &mut gpui::TestAppContext) {
        let focus = cx.update(|cx| cx.focus_handle());
        assert_eq!(
            Tree::new("tree", "Audio projects", &focus).accessibility_label(),
            "Audio projects"
        );
        assert_eq!(
            TreeItem::new("doc", "Weekly sync", &focus).accessibility_label(),
            "Weekly sync"
        );
    }
}
