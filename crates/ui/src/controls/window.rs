//! The window's first layer and its chrome. `root` paints the window's base
//! fill in the UI font and text color, and holds keyboard focus whenever no
//! control does, as a Mac window does: at launch, and once the control that
//! had it is no longer drawn. Keys then always reach the root, which moves
//! focus on Tab and Shift-Tab (Ctrl-Tab and Ctrl-Shift-Tab too, which leave
//! a text area that takes Tab for indenting) and runs the window's
//! shortcuts (`Shortcut`). Focus inside a gpui-base focus trap, such as an
//! open dialog's, cycles through the trap's tab stops and never reaches the
//! page behind it, and no shortcut runs. A control that keyboard focus
//! moves to is scrolled into view in the scroll area that holds it.
//! `DragStrip` is a titlebar strip that drags the window.

use crate::theme;
use gpui::{
    actions, div, prelude::*, px, Action, App, Bounds, Div, ElementId, FocusHandle, Global,
    Interactivity, IntoElement, KeyBinding, MouseButton, MouseMoveEvent, Pixels, ScrollHandle,
    Stateful, StyleRefinement, Window, WindowControlArea,
};
use gpui_base::actions::Confirm;
use std::rc::Rc;

actions!(
    meetnote_window,
    [
        FocusNext,
        FocusPrevious,
        OpenEntry1,
        OpenEntry2,
        OpenEntry3,
        OpenEntry4,
        OpenSettings,
        NavigateBack,
        NavigateForward,
        ToggleSidebar,
        ImportAudio,
        SaveDocument,
        SearchWorkspace,
    ]
);

/// A window shortcut, bound in the root's key context. A focused control
/// that binds the same keys comes first: a multi-line field outdents with
/// ⌘[, and a field with no use for it passes it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shortcut {
    /// Opens the rail's entry at this index, of the first four.
    Entry(usize),
    Settings,
    Back,
    Forward,
    ToggleSidebar,
    ImportAudio,
    /// Saves the visible document editor, when its owner allows it.
    SaveDocument,
    Search,
}

impl Shortcut {
    /// The keystroke, in gpui's key binding syntax.
    pub(crate) fn keys(self) -> &'static str {
        match self {
            Self::Entry(index) => ["cmd-1", "cmd-2", "cmd-3", "cmd-4"][index],
            Self::Settings => "cmd-,",
            Self::Back => "cmd-[",
            Self::Forward => "cmd-]",
            Self::ToggleSidebar => "ctrl-cmd-s",
            Self::ImportAudio => "cmd-o",
            Self::SaveDocument => "cmd-s",
            Self::Search => "cmd-shift-f",
        }
    }
}

/// What a shortcut runs.
type ShortcutHandler = Rc<dyn Fn(&Shortcut, &mut Window, &mut App)>;

const KEY_CONTEXT: &str = "MeetNoteWindow";
/// The key context of gpui-base's select root, which `Dropdown` renders.
const SELECT_CONTEXT: &str = "Select";
/// The least room a revealed control keeps from its scroll area's edge, so
/// the area's edge fade leaves it legible.
const REVEAL_MARGIN: Pixels = px(24.);

/// Installs gpui-base's key bindings (inputs, selects, dialogs), the tree's
/// moves and the window's own: Tab moves focus (inputs bind Tab themselves,
/// closer to the focus), the shortcuts, and Space opens a select or chooses
/// in its menu like Enter.
pub(crate) fn init(cx: &mut App) {
    gpui_base::init(cx);
    super::tree::init(cx);
    super::dialog::init(cx);
    let window = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, window),
        KeyBinding::new("shift-tab", FocusPrevious, window),
        KeyBinding::new("ctrl-tab", FocusNext, window),
        KeyBinding::new("ctrl-shift-tab", FocusPrevious, window),
        KeyBinding::new(Shortcut::Entry(0).keys(), OpenEntry1, window),
        KeyBinding::new(Shortcut::Entry(1).keys(), OpenEntry2, window),
        KeyBinding::new(Shortcut::Entry(2).keys(), OpenEntry3, window),
        KeyBinding::new(Shortcut::Entry(3).keys(), OpenEntry4, window),
        KeyBinding::new(Shortcut::Settings.keys(), OpenSettings, window),
        KeyBinding::new(Shortcut::Back.keys(), NavigateBack, window),
        KeyBinding::new(Shortcut::Forward.keys(), NavigateForward, window),
        KeyBinding::new(Shortcut::ToggleSidebar.keys(), ToggleSidebar, window),
        KeyBinding::new(Shortcut::ImportAudio.keys(), ImportAudio, window),
        KeyBinding::new(Shortcut::SaveDocument.keys(), SaveDocument, window),
        KeyBinding::new(Shortcut::Search.keys(), SearchWorkspace, window),
        KeyBinding::new("space", Confirm { secondary: false }, Some(SELECT_CONTEXT)),
    ]);
}

/// Runs `shortcut` for its action `A`, unless a focus trap (an open
/// dialog) holds the keyboard.
fn shortcut<A: Action>(
    run: &ShortcutHandler,
    shortcut: Shortcut,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let run = run.clone();
    move |_, window, cx| {
        if gpui_base::active_focus_trap(window, cx).is_none() {
            run(&shortcut, window, cx);
        }
    }
}

/// Prepares a new window for `root`: rem-based sizes follow the body font
/// size, and on macOS AppKit's accessibility hit test reaches GPUI's
/// elements (it asks the window, which forwards to its content view).
pub(crate) fn attach(window: &mut Window, cx: &App) {
    window.set_rem_size(theme::tokens(cx).font_size.md);
    #[cfg(target_os = "macos")]
    gpui_base::install_window_hit_test_forwarder(window);
}

/// `content` as the window's root element, which holds focus through
/// `focus` whenever no control does and runs `on_shortcut` for the window's
/// shortcuts. The frame's scroll areas register themselves again as they
/// render.
pub(crate) fn root(
    content: impl IntoElement,
    focus: &FocusHandle,
    on_shortcut: impl Fn(&Shortcut, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Div {
    cx.default_global::<FocusReveal>().areas.clear();
    // Once this frame is drawn: focus that left every drawn control, or
    // never reached one, returns to the root. Focus moved since this frame
    // started drawing, such as to a menu opening, waits for the next frame.
    let held = focus.clone();
    let drawn_with = window.focused(cx);
    window.defer(cx, move |window, cx| {
        if window.focused(cx) == drawn_with && !held.contains_focused(window, cx) {
            held.focus(window, cx);
        }
    });
    let tokens = theme::tokens(cx);
    let run: ShortcutHandler = Rc::new(on_shortcut);
    div()
        .key_context(KEY_CONTEXT)
        .track_focus(focus)
        // A press on no control leaves focus where it is; the root only
        // takes focus that no control holds.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_action(|_: &FocusNext, window, cx| move_focus(true, window, cx))
        .on_action(|_: &FocusPrevious, window, cx| move_focus(false, window, cx))
        .on_action(shortcut::<OpenEntry1>(&run, Shortcut::Entry(0)))
        .on_action(shortcut::<OpenEntry2>(&run, Shortcut::Entry(1)))
        .on_action(shortcut::<OpenEntry3>(&run, Shortcut::Entry(2)))
        .on_action(shortcut::<OpenEntry4>(&run, Shortcut::Entry(3)))
        .on_action(shortcut::<OpenSettings>(&run, Shortcut::Settings))
        .on_action(shortcut::<NavigateBack>(&run, Shortcut::Back))
        .on_action(shortcut::<NavigateForward>(&run, Shortcut::Forward))
        .on_action(shortcut::<ToggleSidebar>(&run, Shortcut::ToggleSidebar))
        .on_action(shortcut::<ImportAudio>(&run, Shortcut::ImportAudio))
        .on_action(shortcut::<SaveDocument>(&run, Shortcut::SaveDocument))
        .on_action(shortcut::<SearchWorkspace>(&run, Shortcut::Search))
        .relative()
        .size_full()
        .font_family(tokens.font_family)
        .bg(tokens.root_background())
        .text_color(tokens.text)
        .child(content)
}

/// Moves focus to the next (or previous) tab stop. Tab stops wrap around the
/// window; inside a focus trap, focus steps on past the stops outside it,
/// and gives up after one lap if the trap has none. The new stop is
/// revealed once it paints.
fn move_focus(forward: bool, window: &mut Window, cx: &mut App) {
    let step = |window: &mut Window, cx: &mut App| {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
    };
    let trap = gpui_base::active_focus_trap(window, cx);
    step(window, cx);
    if let Some(trap) = trap {
        let first = window.focused(cx);
        while !trap.contains_focused(window, cx) {
            step(window, cx);
            if window.focused(cx) == first {
                break;
            }
        }
    }
    reveal_focus(window, cx);
}

/// The scroll areas of the frame being drawn, and the control keyboard
/// focus has just moved to, which is revealed once it paints its ring.
#[derive(Default)]
struct FocusReveal {
    areas: Vec<ScrollHandle>,
    pending: Option<FocusHandle>,
}

impl Global for FocusReveal {}

/// Registers `scroll` as a scroll area of the frame being drawn; its
/// scrollbar does, so every scrolled list of a page is one.
pub(super) fn register_scroll_area(scroll: &ScrollHandle, cx: &mut App) {
    cx.default_global::<FocusReveal>()
        .areas
        .push(scroll.clone());
}

/// Scrolls the focused control into view once it next paints its ring,
/// after keyboard focus moved to it or, in a composite control such as the
/// project tree, within it.
pub(super) fn reveal_focus(window: &Window, cx: &mut App) {
    let focused = window.focused(cx);
    cx.default_global::<FocusReveal>().pending = focused;
}

/// Where a control lies relative to a scroll area.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Placement {
    /// Not in the area's content.
    Outside,
    /// In the area and in view.
    Shown,
    /// In the area; this offset brings it into view.
    ScrollTo(Pixels),
}

/// Where `target` lies relative to the scroll area whose viewport is
/// `viewport`, scrolled to `offset` of at most `max` (offsets are negative
/// as the content moves up). `clip` is the content mask `target` painted
/// in: inside the area it starts at or below the viewport's top, and it is
/// empty where an inner clip lies wholly outside the viewport.
fn placement(
    viewport: Bounds<Pixels>,
    offset: Pixels,
    max: Pixels,
    clip: Bounds<Pixels>,
    target: Bounds<Pixels>,
) -> Placement {
    let slack = px(1.);
    let clipped_by_area = clip.top() >= viewport.top() - slack
        && (clip.bottom() <= viewport.bottom() + slack || clip.size.height <= px(0.));
    let across =
        target.left() >= viewport.left() - slack && target.right() <= viewport.right() + slack;
    if !(clipped_by_area && across) {
        return Placement::Outside;
    }
    let margin = REVEAL_MARGIN.min(viewport.size.height / 4.);
    let (top, bottom) = (viewport.top() + margin, viewport.bottom() - margin);
    let wanted = if target.top() < top || target.size.height > bottom - top {
        offset + (top - target.top())
    } else if target.bottom() > bottom {
        offset - (target.bottom() - bottom)
    } else {
        return Placement::Shown;
    };
    let wanted = wanted.clamp(-max, px(0.));
    if wanted == offset {
        Placement::Shown
    } else {
        Placement::ScrollTo(wanted)
    }
}

/// Called as a control with keyboard focus through `focus` paints its ring
/// at `bounds`: when focus has just moved to it, scrolls the innermost
/// registered area holding it until it is in view, and draws again.
pub(super) fn reveal_painted(
    focus: &FocusHandle,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let clip = window.content_mask().bounds;
    let reveal = cx.default_global::<FocusReveal>();
    if reveal.pending.as_ref() != Some(focus) {
        return;
    }
    reveal.pending = None;
    let innermost = reveal
        .areas
        .iter()
        .filter_map(|area| {
            let viewport = area.bounds();
            match placement(viewport, area.offset().y, area.max_offset().y, clip, bounds) {
                Placement::Outside => None,
                placement => Some((area, viewport.size.height, placement)),
            }
        })
        .min_by(|(_, a, _), (_, b, _)| f32::from(*a).total_cmp(&f32::from(*b)));
    if let Some((area, _, Placement::ScrollTo(offset))) = innermost {
        area.set_offset(gpui::point(area.offset().x, offset));
        let handle = window.window_handle();
        cx.defer(move |cx| {
            handle.update(cx, |_, window, _| window.refresh()).ok();
        });
    }
}

/// A titlebar strip that drags the window: a `WindowControlArea::Drag`
/// region that hands the drag to the platform once the pointer moves with
/// the button held, and zooms on a double click. The window owns titlebar
/// dragging (`WindowOptions::app_owns_titlebar_drag`), so only these strips
/// move it. It is window chrome for the pointer, with no tab stop or name.
#[derive(IntoElement)]
pub(crate) struct DragStrip {
    id: ElementId,
    base: Stateful<Div>,
}

impl DragStrip {
    pub(crate) fn titlebar(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
        }
    }
}

impl Styled for DragStrip {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for DragStrip {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for DragStrip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // A left press in the strip that has not yet moved the window.
        let armed = window.use_keyed_state(self.id, cx, |_, _| false);
        let set = |armed: &gpui::Entity<bool>, value: bool, cx: &mut App| {
            armed.update(cx, |armed, _| *armed = value)
        };
        let (down, up, out, moved) = (armed.clone(), armed.clone(), armed.clone(), armed);
        self.base
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| set(&down, true, cx))
            .on_mouse_up(MouseButton::Left, move |_, _, cx| set(&up, false, cx))
            .on_mouse_down_out(move |_, _, cx| set(&out, false, cx))
            // Only while the button is still held: AppKit resolves a second
            // click inside its drag session as a titlebar double click, so a
            // stale arm must not start one from a plain hover.
            .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                if *moved.read(cx) && event.pressed_button == Some(MouseButton::Left) {
                    set(&moved, false, cx);
                    window.start_window_move();
                }
            })
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else {
                        window.zoom_window();
                    }
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    fn bounds(top: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(100.), px(top)), size(px(200.), px(height)))
    }

    #[test]
    fn a_control_below_or_above_the_viewport_scrolls_into_view() {
        let viewport = bounds(100., 400.);
        let max = px(1_000.);
        // Its bottom edge lands the margin above the viewport's.
        assert_eq!(
            placement(viewport, px(0.), max, viewport, bounds(600., 30.)),
            Placement::ScrollTo(px(-(630. - 476.)))
        );
        // Scrolled down 300: a control at the top edge scrolls back.
        assert_eq!(
            placement(viewport, px(-300.), max, viewport, bounds(80., 30.)),
            Placement::ScrollTo(px(-300. + 44.))
        );
        assert_eq!(
            placement(viewport, px(0.), max, viewport, bounds(200., 30.)),
            Placement::Shown
        );
    }

    #[test]
    fn a_reveal_stays_within_the_scroll_range() {
        let viewport = bounds(100., 400.);
        assert_eq!(
            placement(viewport, px(-20.), px(40.), viewport, bounds(600., 30.)),
            Placement::ScrollTo(px(-40.)),
            "never past the end"
        );
        assert_eq!(
            placement(viewport, px(-10.), px(40.), viewport, bounds(110., 30.)),
            Placement::ScrollTo(px(0.)),
            "never before the start"
        );
        assert_eq!(
            placement(viewport, px(0.), px(0.), viewport, bounds(110., 30.)),
            Placement::Shown,
            "an area that cannot scroll leaves it"
        );
    }

    #[test]
    fn a_control_taller_than_the_viewport_shows_its_top() {
        let viewport = bounds(100., 400.);
        assert_eq!(
            placement(viewport, px(0.), px(1_000.), viewport, bounds(300., 900.)),
            Placement::ScrollTo(px(-176.))
        );
    }

    #[test]
    fn only_a_control_the_area_clips_belongs_to_it() {
        let viewport = bounds(100., 400.);
        let window = bounds(0., 800.);
        assert_eq!(
            placement(viewport, px(0.), px(500.), window, bounds(40., 30.)),
            Placement::Outside,
            "a control above the area, clipped by the window only"
        );
        let beside = Bounds::new(point(px(400.), px(600.)), size(px(80.), px(30.)));
        assert_eq!(
            placement(viewport, px(0.), px(500.), viewport, beside),
            Placement::Outside,
            "a control beside the area"
        );
        // An inner clip wholly below the viewport empties the mask there.
        let empty = Bounds::new(point(px(100.), px(640.)), size(px(200.), px(0.)));
        assert!(matches!(
            placement(viewport, px(0.), px(500.), empty, bounds(640., 30.)),
            Placement::ScrollTo(_)
        ));
    }
}
