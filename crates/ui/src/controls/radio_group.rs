//! A labelled gpui-base radio group. Options become radio buttons only
//! when inserted here, so every exclusive choice has a named container.
//! A segmented group draws its options as one control: a track whose
//! selected segment is raised.

use super::Button;
use crate::theme;
use gpui::{
    prelude::*, px, App, Axis, ElementId, Interactivity, IntoElement, Pixels, SharedString,
    StyleRefinement, Window,
};

/// The track's inset around its segments.
const TRACK_INSET: Pixels = px(2.);

#[derive(IntoElement)]
pub(crate) struct RadioGroup {
    base: gpui_base::RadioGroup,
    label: SharedString,
    options: Vec<(bool, Button)>,
    segmented: bool,
}

impl RadioGroup {
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: gpui_base::RadioGroup::new(id).axis(Axis::Horizontal),
            label: label.into(),
            options: Vec::new(),
            segmented: false,
        }
    }

    /// Draws the options as the segments of one control.
    pub(crate) fn segmented(mut self) -> Self {
        self.segmented = true;
        self
    }

    pub(crate) fn option(mut self, selected: bool, button: Button) -> Self {
        self.options.push((selected, button));
        self
    }

    pub(crate) fn options(mut self, options: impl IntoIterator<Item = (bool, Button)>) -> Self {
        for (selected, button) in options {
            self = self.option(selected, button);
        }
        self
    }
}

impl Styled for RadioGroup {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}
impl InteractiveElement for RadioGroup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}
impl StatefulInteractiveElement for RadioGroup {}

impl RenderOnce for RadioGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let mut base = self.base;
        if self.segmented {
            base = base
                .flex()
                .items_center()
                .p(TRACK_INSET)
                .gap(TRACK_INSET)
                .rounded(tokens.radius.md)
                .bg(tokens.wash);
        }
        for (selected, button) in self.options {
            base = base.child(if self.segmented {
                button
                    .segment(selected)
                    .h(tokens.control_height.md - TRACK_INSET * 2.)
            } else {
                button.selected(selected)
            });
        }
        base.aria_label(self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_group_keeps_its_required_accessible_label() {
        let group = RadioGroup::new("scene", "Assistant scene")
            .option(true, Button::new("ask", "Ask"))
            .option(false, Button::new("organize", "Organize"));
        assert_eq!(group.label, "Assistant scene");
        assert!(!group.segmented);
        let segmented = RadioGroup::new("mode", "Content view")
            .segmented()
            .option(true, Button::new("summary", "Summary"));
        assert!(segmented.segmented);
        assert_eq!(segmented.options.len(), 1);
    }
}
