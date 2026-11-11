use rune::color::hex;
use rune::{Color, El, ScrollSpec, TextStyle, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, ScrollBarStyle, Select};

use crate::MeetNote;

pub fn theme(dark: bool) -> Kit {
    let mut k = Kit::neutral(dark);
    let (page, chrome, card, accent) = if dark {
        ("#16191f", "#1b2029", "#1d232d", "#abc3ff")
    } else {
        ("#f9fafc", "#eef1f6", "#ffffff", "#2a55a5")
    };
    k.c.page = hex(page).unwrap();
    k.c.bg_deep = hex(chrome).unwrap();
    k.c.layer_01 = hex(card).unwrap();
    k.c.text_accent = hex(accent).unwrap();
    k.c.icon_accent = k.c.text_accent;
    k.c.border_focus = k.c.text_accent;
    k
}

pub fn title(k: &Kit, value: impl Into<String>, size: f32) -> El {
    text(value.into())
        .font(TextStyle::new(size, 650.0))
        .color(k.c.text_base)
        .wrap()
}

pub fn label(k: &Kit, value: impl Into<String>) -> El {
    text(value.into())
        .font(TextStyle::new(14.0, 400.0))
        .color(k.c.text_base)
        .wrap()
}

pub fn caption(k: &Kit, value: impl Into<String>) -> El {
    text(value.into())
        .font(TextStyle::new(12.0, 400.0))
        .color(k.c.text_muted)
        .wrap()
}

pub fn eyebrow(k: &Kit, value: impl Into<String>) -> El {
    text(value.into())
        .font(TextStyle::new(11.0, 650.0))
        .color(k.c.text_accent)
}

pub fn card(k: &Kit) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(12.0)
        .padding(18.0)
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
}

pub fn page_header(k: &Kit, section: &str, heading: &str, description: &str) -> El {
    col()
        .w_full()
        .gap(7.0)
        .child(eyebrow(k, section))
        .child(title(k, heading, 30.0))
        .child(caption(k, description))
}

pub fn action(
    k: &Kit,
    key: &str,
    name: &str,
    variant: ButtonVariant,
    press: impl Fn(&mut MeetNote) + 'static,
) -> El {
    Button::new(name)
        .control_size(ControlSize::Large)
        .variant(variant)
        .on_press(press)
        .build(k)
        .key(key)
}

pub fn icon_action(
    k: &Kit,
    key: &str,
    icon: &'static rune::Icon,
    name: &str,
    press: impl Fn(&mut MeetNote, &mut rune::Tx) + 'static,
) -> El {
    rune_kit::IconButton::new(icon)
        .label(name)
        .help(name)
        .control_size(ControlSize::Large)
        .on_press_tx(press)
        .build(k)
        .key(key)
}

pub fn scroll(k: &Kit, key: &str, body: El) -> El {
    use rune_kit::ScrollBar;
    col()
        .key(key)
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .clip_fade(0.0, 7.0, 0.0, 10.0)
        .child(body)
        .scroll_bar_styled(ScrollBarStyle::from_kit(k))
}

pub fn select(
    s: &MeetNote,
    k: &Kit,
    overlays: &mut Vec<El>,
    (key, name): (&'static str, &str),
    options: &[&str],
    choice: usize,
    set: impl Fn(&mut MeetNote, usize) + Clone + 'static,
) -> El {
    let values: Vec<_> = options.iter().map(ToString::to_string).collect();
    Select::new(key, options)
        .label(name)
        .width(190.0)
        .current(options.get(choice).copied())
        .open(s.select == Some(key))
        .on_toggle(move |s: &mut MeetNote| s.select = (s.select != Some(key)).then_some(key))
        .on_close(move |s: &mut MeetNote| {
            if s.select == Some(key) {
                s.select = None;
            }
        })
        .on_choose(move |s: &mut MeetNote, picked, _| {
            if let Some(index) = values.iter().position(|value| value == picked) {
                set(s, index);
            }
            s.select = None;
        })
        .build(k, overlays)
}

pub fn field(k: &Kit, name: &str, control: El) -> El {
    row()
        .w_full()
        .items_center()
        .gap(16.0)
        .child(label(k, name).grow(1.0).min_w(0.0))
        .child(control)
}

pub fn dot(color: Color) -> El {
    el().size(7.0, 7.0).radius(4.0).fill(color)
}
