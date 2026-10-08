//! A sidebar divider with pointer capture, a hover glow and resisted limits.
//! Width is window-local UI state; it never enters Workspace persistence.

use crate::theme;
use gpui::{
    div, prelude::*, px, Animation, AnimationExt, AnyElement, App, Context, Div, Entity,
    IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, RenderOnce, SharedString,
    Window,
};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_WIDTH: f32 = 256.;
const MIN_WIDTH: f32 = 208.;
const MAX_WIDTH: f32 = 360.;
const MIN_CONTENT_WIDTH: f32 = 520.;

#[derive(Debug)]
pub(crate) struct SidebarSizing {
    width: f32,
    maximum: f32,
    drag: Option<(f32, f32)>,
    hovered: bool,
    overshoot: f32,
    bounce: f32,
    generation: usize,
    fold: Option<Fold>,
}

#[derive(Debug)]
struct Fold {
    started: Instant,
    duration: Duration,
    from: f32,
    to: f32,
}

impl Fold {
    fn value(&self) -> f32 {
        let progress = (self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.);
        self.from + (self.to - self.from) * (1. - (1. - progress).powi(3))
    }
}

impl Default for SidebarSizing {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            maximum: MAX_WIDTH,
            drag: None,
            hovered: false,
            overshoot: 0.,
            bounce: 0.,
            generation: 0,
            fold: None,
        }
    }
}

impl SidebarSizing {
    pub(crate) fn folding(&self) -> bool {
        self.fold
            .as_ref()
            .is_some_and(|fold| fold.started.elapsed() < fold.duration)
    }

    fn visible_fraction(&self, open: bool) -> f32 {
        self.fold
            .as_ref()
            .map_or(if open { 1. } else { 0. }, Fold::value)
    }

    pub(crate) fn set_open(&mut self, open: bool, duration: Duration) {
        let from = self.visible_fraction(!open);
        self.reset_interaction();
        self.fold = (!duration.is_zero()).then(|| Fold {
            started: Instant::now(),
            duration,
            from,
            to: if open { 1. } else { 0. },
        });
    }
    pub(crate) fn reset_interaction(&mut self) {
        self.drag = None;
        self.hovered = false;
        self.overshoot = 0.;
        self.bounce = 0.;
    }

    fn resize(&mut self, pointer: f32) {
        let Some((start, width)) = self.drag else {
            return;
        };
        let proposed = width + pointer - start;
        self.width = proposed.clamp(MIN_WIDTH, self.maximum);
        let excess = proposed - self.width;
        self.overshoot = excess.signum() * 16. * (1. - (-excess.abs() / 100.).exp());
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.bounce = self.overshoot;
            self.overshoot = 0.;
            self.generation += 1;
            cx.notify();
        }
    }
}

fn install_pointer_capture(state: Entity<SidebarSizing>, window: &mut Window, _: &mut App) {
    // Register at the window so a drag continues past the narrow divider
    // and releases even when the pointer is outside the sidebar/window.
    window.on_mouse_event({
        let state = state.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase.bubble() && state.read(cx).drag.is_some() {
                state.update(cx, |state, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        state.resize(event.position.x.into());
                        cx.notify();
                    } else {
                        state.release(cx);
                    }
                });
                window.refresh();
            }
        }
    });
    window.on_mouse_event({
        let state = state.clone();
        move |event: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() && event.button == MouseButton::Left && state.read(cx).drag.is_some()
            {
                state.update(cx, |state, cx| state.release(cx));
                window.refresh();
            }
        }
    });
}

#[derive(IntoElement)]
pub(crate) struct SidebarPanel {
    id: &'static str,
    label: gpui::SharedString,
    sizing: Entity<SidebarSizing>,
    available: f32,
    child: AnyElement,
    open: bool,
}

impl SidebarPanel {
    pub(crate) fn new(
        id: &'static str,
        label: impl Into<SharedString>,
        sizing: &Entity<SidebarSizing>,
        available: Pixels,
        open: bool,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            sizing: sizing.clone(),
            available: available.into(),
            child: child.into_any_element(),
            open,
        }
    }
}

impl RenderOnce for SidebarPanel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *theme::tokens(cx);
        let state = self.sizing.clone();
        state.update(cx, |state, _| {
            state.maximum = MAX_WIDTH.min((self.available - MIN_CONTENT_WIDTH).max(MIN_WIDTH));
            state.width = state.width.clamp(MIN_WIDTH, state.maximum);
        });
        let sizing = state.read(cx);
        let dragging = sizing.drag.is_some();
        let fraction = sizing.visible_fraction(self.open);
        let glow = (sizing.hovered || dragging) && !sizing.folding();
        let (width, bounce, generation) = (
            sizing.width + sizing.overshoot,
            sizing.bounce,
            sizing.generation,
        );
        let hover = state.clone();
        let down = state.clone();
        let handle = div()
            .id("sidebar-resize-handle")
            .role(gpui::Role::Splitter)
            .aria_label(self.label)
            .debug_selector(|| "sidebar-resize-handle".into())
            .absolute()
            .right_0()
            .top_0()
            .h_full()
            .w(px(8.))
            .cursor_col_resize()
            .occlude()
            .on_hover(move |hovered, window, cx| {
                hover.update(cx, |state, cx| {
                    if state.hovered != *hovered {
                        state.hovered = *hovered;
                        cx.notify();
                    }
                });
                window.refresh();
            })
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                window.prevent_default();
                down.update(cx, |state, cx| {
                    state.drag = Some((event.position.x.into(), state.width));
                    state.bounce = 0.;
                    cx.notify();
                });
                window.refresh();
                cx.stop_propagation();
            })
            .when(glow, |handle| {
                handle.child(
                    div()
                        .debug_selector(|| "sidebar-resize-glow".into())
                        .absolute()
                        .top_0()
                        .right(px(-1.))
                        .h_full()
                        .w(px(3.))
                        .child(theme::sidebar_glow()),
                )
            });
        let capture = gpui::canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                install_pointer_capture(state.clone(), window, cx);
            },
        )
        .absolute()
        .size_full();
        let panel: Div = div()
            .debug_selector(move || self.id.into())
            .relative()
            .w(px(width * fraction))
            .h_full()
            .flex_shrink_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(tokens.sidebar)
            .rounded_l(tokens.radius.xl)
            .border_r_1()
            .border_color(tokens.hairline)
            .child(
                div()
                    .w(px(width))
                    .h_full()
                    .flex_shrink_0()
                    .opacity((fraction * 2.).min(1.))
                    .child(self.child),
            )
            .child(handle)
            .child(capture);
        if !dragging && bounce != 0. && !tokens.motion.layout.is_zero() && fraction == 1. {
            panel
                .with_animation(
                    ("sidebar-rebound", generation),
                    Animation::new(tokens.motion.layout),
                    move |panel, delta| {
                        // A damped return settles exactly at the resting
                        // width; reduced motion skips the offset.
                        let offset = bounce
                            * (1. - delta).powi(3)
                            * (delta * std::f32::consts::PI * 2.).cos();
                        panel.w(px(width + offset))
                    },
                )
                .into_any_element()
        } else {
            panel.into_any_element()
        }
    }
}
