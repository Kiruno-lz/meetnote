//! A labelled gpui-base radio group. Options become radio buttons only
//! when inserted here, so every exclusive choice has a named container.

use super::Button;
use gpui::{
    prelude::*, App, Axis, ElementId, Interactivity, IntoElement, SharedString, StyleRefinement,
    Window,
};

#[derive(IntoElement)]
pub(crate) struct RadioGroup {
    base: gpui_base::RadioGroup,
    label: SharedString,
}

impl RadioGroup {
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            base: gpui_base::RadioGroup::new(id).axis(Axis::Horizontal),
            label: label.into(),
        }
    }

    pub(crate) fn option(mut self, selected: bool, button: Button) -> Self {
        self.base = self.base.child(button.selected(selected));
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
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base.aria_label(self.label)
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
    }
}
