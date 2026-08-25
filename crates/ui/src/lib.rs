use application::BackendInventory;
use gpui::{div, prelude::*, rgb, size, App, Bounds, Context, Window, WindowBounds, WindowOptions};
use gpui_platform::application;

struct RootView {
    inventory: BackendInventory,
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x171717))
            .flex()
            .flex_col()
            .p_6()
            .gap_3()
            .text_color(rgb(0xf5f5f5))
            .child(div().text_2xl().child("meetnote"))
            .child(div().text_lg().child("Rust-native meeting intelligence platform"))
            .child(div().child(format!("ASR: {}", backend_ids(&self.inventory.asr))))
            .child(div().child(format!(
                "Diarization: {}",
                backend_ids(&self.inventory.diarization)
            )))
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .bg(rgb(0x242424))
                    .border_1()
                    .border_color(rgb(0x3b3b3b))
                    .rounded_md()
                    .child("v0.1 foundation: GPUI shell, contracts, backend managers, and model catalog"),
            )
    }
}

pub fn run(inventory: BackendInventory) {
    application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(gpui::px(900.0), gpui::px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| RootView {
                    inventory: inventory.clone(),
                })
            },
        )
        .expect("failed to open meetnote window");
        cx.activate(true);
    });
}

fn backend_ids(descriptors: &[application::BackendDescriptor]) -> String {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
