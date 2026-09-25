//! `ProgressBar`: a labelled linear progress bar on gpui-base's unstyled
//! `Progress` root, which reports the value (or its absence while
//! indeterminate) to assistive technology.

use crate::theme;
use gpui::{
    div, prelude::*, px, relative, Animation, AnimationExt as _, App, ElementId, IntoElement,
    Pixels, SharedString, Window,
};
use gpui_base::{Progress, ProgressIndicator, ProgressTrack};
use std::time::Duration;

const HEIGHT: Pixels = px(6.);
/// One sweep of the indeterminate segment.
const SWEEP: Duration = Duration::from_millis(1_200);
/// The indeterminate segment's share of the track.
const SEGMENT: f32 = 0.3;

#[derive(IntoElement)]
pub(crate) struct ProgressBar {
    id: ElementId,
    label: SharedString,
    value: f32,
    indeterminate: bool,
}

impl ProgressBar {
    /// `label` names the bar for assistive technology.
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value: 0.,
            indeterminate: false,
        }
    }

    /// The completed share in percent, clamped to 0…100.
    pub(crate) fn value(mut self, percent: f32) -> Self {
        self.value = percent.clamp(0., 100.);
        self
    }

    /// Progress of unknown extent: a sweeping segment, or a dimmed full
    /// track under reduced motion.
    pub(crate) fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    #[cfg(test)]
    pub(crate) fn accessibility_label(&self) -> &SharedString {
        &self.label
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let radius = HEIGHT / 2.;
        let indicator = ProgressIndicator::new()
            .absolute()
            .top_0()
            .h_full()
            .rounded(radius)
            .bg(tokens.accent);
        let indicator = match (self.indeterminate, cx.reduce_motion()) {
            (false, _) => indicator
                .left_0()
                .w(relative(self.value / 100.))
                .into_any_element(),
            (true, true) => indicator.left_0().w_full().opacity(0.5).into_any_element(),
            (true, false) => div()
                .absolute()
                .size_full()
                .child(indicator.w(relative(SEGMENT)))
                .with_animation(
                    "progress-sweep",
                    Animation::new(SWEEP).repeat(),
                    |sweep, delta| sweep.left(relative(delta * (1. + SEGMENT) - SEGMENT)),
                )
                .into_any_element(),
        };
        Progress::new(self.id)
            .value(self.value)
            .indeterminate(self.indeterminate)
            .aria_label(self.label)
            .relative()
            .w_full()
            .h(HEIGHT)
            .rounded(radius)
            .overflow_hidden()
            .child(
                ProgressTrack::new()
                    .absolute()
                    .size_full()
                    .bg(tokens.accent.opacity(0.2)),
            )
            .child(indicator)
    }
}
