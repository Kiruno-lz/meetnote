//! Notifications as rune-kit's `ToastStack` runs them: a repeated message pulses in place, the
//! stack expands under the pointer, and timers hold while it is expanded or the window is hidden.

use std::collections::HashMap;
use std::time::Duration;

use rune::{El, Tx, el};
use rune_kit::{Kit, Toast, ToastEffect, ToastEvent, ToastPage, ToastStack};

use crate::MeetNote;

/// Seconds a notification stays when nothing holds it.
const SECONDS: f64 = 4.0;
/// Toast width set by rune-kit's `Toast`.
const WIDTH: f32 = 320.0;
/// Height assumed until a toast reports its own layout.
const FALLBACK_HEIGHT: f32 = 44.0;

#[derive(Default)]
pub struct Toasts {
    stack: ToastStack<String>,
    heights: HashMap<u64, f32>,
}

impl Toasts {
    /// Shows `message`; the returned delay is when the next timer comes due.
    pub fn show(&mut self, message: String, now: f64) -> Option<f64> {
        let effects = self.stack.step(ToastEvent::Shown {
            key: message.clone(),
            options: message,
            secs: Some(SECONDS),
            now,
        });
        self.wake(effects, now)
    }

    /// Closes the toasts whose time is up; true when any closed.
    pub fn tick(&mut self, now: f64) -> bool {
        if self.stack.next_due().is_none_or(|due| due > now + 1e-3) {
            return false;
        }
        let before = self.stack.entries().len();
        self.stack.step(ToastEvent::Tick { now });
        self.forget_closed();
        self.stack.entries().len() != before
    }

    pub fn dismiss(&mut self, id: u64, now: f64) -> Option<f64> {
        let effects = self.stack.step(ToastEvent::Dismissed { id: Some(id), now });
        self.forget_closed();
        self.wake(effects, now)
    }

    pub fn hover(&mut self, inside: bool, now: f64) -> Option<f64> {
        let effects = self.stack.step(if inside {
            ToastEvent::PointerOver { now }
        } else {
            ToastEvent::PointerLeft { now }
        });
        self.wake(effects, now)
    }

    pub fn page(&mut self, shown: bool, now: f64) -> Option<f64> {
        let page = if shown {
            ToastPage::Shown
        } else {
            ToastPage::Hidden
        };
        let effects = self.stack.step(ToastEvent::Page { page, now });
        self.wake(effects, now)
    }

    /// Records a toast's measured height; true when it changed.
    fn measure(&mut self, id: u64, height: f32) -> bool {
        self.heights.insert(id, height) != Some(height)
    }

    #[cfg(test)]
    pub fn messages(&self) -> impl Iterator<Item = &str> {
        self.stack
            .entries()
            .iter()
            .map(|entry| entry.options.as_str())
    }

    fn height(&self, id: u64) -> f32 {
        self.heights.get(&id).copied().unwrap_or(FALLBACK_HEIGHT)
    }

    fn forget_closed(&mut self) {
        let entries = self.stack.entries();
        self.heights
            .retain(|id, _| entries.iter().any(|entry| entry.id == *id));
    }

    fn wake(&self, effects: impl IntoIterator<Item = ToastEffect>, now: f64) -> Option<f64> {
        effects.into_iter().find_map(|effect| match effect {
            ToastEffect::WakeAt(due) => Some((due - now).max(0.0)),
            _ => None,
        })
    }
}

impl MeetNote {
    pub fn notify(&mut self, message: impl Into<String>, tx: &mut Tx) {
        let wake = self.toasts.show(message.into(), self.now);
        schedule(wake, tx);
    }

    pub fn window_visibility(&mut self, shown: bool, tx: &mut Tx) {
        let wake = self.toasts.page(shown, self.now);
        schedule(wake, tx);
    }
}

fn schedule(wake: Option<f64>, tx: &mut Tx) {
    if let Some(secs) = wake {
        tx.wake_after(Duration::from_secs_f64(secs));
    }
}

/// The stack at the window's bottom-right corner, laid out by `ToastStack::place`.
pub fn region(s: &MeetNote, k: &Kit) -> Option<El> {
    let toasts = &s.toasts;
    let entries = toasts.stack.entries();
    if entries.is_empty() {
        return None;
    }
    let height = |id| toasts.height(id);
    let mut tallest = 0.0_f32;
    let mut region = el()
        .key("toast-region")
        .role("region")
        .label("Notifications")
        .on_hover_change(|s: &mut MeetNote, inside, tx| {
            let wake = s.toasts.hover(inside, s.now);
            schedule(wake, tx);
        });
    for entry in entries {
        let Some(place) = toasts.stack.place(entry.id, height) else {
            continue;
        };
        if !place.visible {
            continue;
        }
        tallest = tallest.max(place.lift + place.height);
        let id = entry.id;
        // Behind the front toast in a collapsed stack only the card's edge peeks out, as tall
        // as the front one; its words return when the stack expands.
        let toast = if place.content {
            Toast::new(format!("toast-{id}"), entry.options.as_str())
                .on_close(move |s: &mut MeetNote| {
                    s.toasts.dismiss(id, s.now);
                })
                .build(k)
                .on_layout(move |s: &mut MeetNote, rect: [f32; 4]| {
                    s.toasts.measure(id, rect[3]);
                })
        } else {
            Toast::new(format!("toast-{id}"), "")
                .build(k)
                .h(place.height)
                .a11y_hidden()
        };
        region = region.child(
            el().key(format!("toast-slot-{id}"))
                .pin(Some(0.0), None, None, Some(0.0))
                .offset(0.0, -place.lift)
                .scale(place.scale)
                .child(toast),
        );
    }
    Some(
        region
            .pin(None, None, Some(32.0), Some(48.0))
            .size(WIDTH, tallest.max(FALLBACK_HEIGHT)),
    )
}

#[cfg(test)]
mod tests {
    use super::Toasts;

    #[test]
    fn a_toast_closes_when_its_time_is_up() {
        let mut toasts = Toasts::default();
        assert_eq!(toasts.show("Saved".into(), 0.0), Some(4.0));
        assert!(!toasts.tick(3.9));
        assert!(toasts.tick(4.0));
        assert_eq!(toasts.messages().count(), 0);
    }

    #[test]
    fn a_repeated_message_does_not_stack_a_copy() {
        let mut toasts = Toasts::default();
        toasts.show("Saved".into(), 0.0);
        toasts.show("Saved".into(), 1.0);
        toasts.show("Imported".into(), 1.0);
        assert_eq!(toasts.messages().collect::<Vec<_>>(), ["Saved", "Imported"]);
    }

    #[test]
    fn hover_and_a_hidden_window_hold_the_timer() {
        let mut toasts = Toasts::default();
        toasts.show("Saved".into(), 0.0);
        toasts.hover(true, 1.0);
        assert!(!toasts.tick(10.0));
        assert_eq!(toasts.hover(false, 10.0), Some(3.0));
        toasts.page(false, 11.0);
        assert!(!toasts.tick(20.0));
        assert_eq!(toasts.page(true, 20.0), Some(2.0));
        assert!(toasts.tick(22.0));
    }

    #[test]
    fn dismissing_forgets_the_measured_height() {
        let mut toasts = Toasts::default();
        toasts.show("Saved".into(), 0.0);
        let id = toasts.stack.entries()[0].id;
        assert!(toasts.measure(id, 52.0));
        assert!(!toasts.measure(id, 52.0));
        toasts.dismiss(id, 1.0);
        assert!(toasts.heights.is_empty());
    }
}
