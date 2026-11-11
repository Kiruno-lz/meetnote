use rune::{Color, Cx, El, Icon, KeyEvent, Spring, SystemMaterial, Tx, col, el, row};
use rune_kit::{
    Badge, BadgeVariant, ControlSize, IconButton, IconButtonVariant, Kit, PaneState, glyphs,
    shell::{SIDEBAR, WindowTitlebar, is_sidebar_toggle},
};

use crate::controls::title;
use rune_kit::shell::docked_pane::{self, PaneDivider};

use crate::{MeetNote, Route, documents, settings};

const SIDEBAR_WIDTH: f32 = 226.0;
const RAIL_WIDTH: f32 = 64.0;
const FRAME_GUTTER: f32 = 5.0;
const FRAME_RADIUS: f32 = 10.0;
const CHROME_TINT: f32 = 0.24;
const SHELL_TINT: f32 = 0.45;
const WORKSPACE_TINT: f32 = 0.29;
static BACK: Icon = Icon {
    name: "meetnote-back",
    view: 16.0,
    body: r#"<path d="M10 3.5 5.5 8 10 12.5" fill="none" stroke="currentColor" stroke-width="1.4"/>"#,
};

#[derive(PartialEq, Eq)]
struct Location {
    route: Route,
    document: Option<String>,
}

pub struct Shell {
    pane: PaneState,
    resize: docked_pane::ResizeInteraction,
    back: Vec<Location>,
    forward: Vec<Location>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            pane: PaneState::new(SIDEBAR_WIDTH).range(192.0, 360.0),
            resize: docked_pane::ResizeInteraction::default(),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }
}

impl MeetNote {
    fn location(&self, route: Route) -> Location {
        Location {
            route,
            document: (route == Route::Documents && self.documents.has_selection())
                .then(|| self.documents.selection().0.to_owned()),
        }
    }

    fn show(&mut self, location: Location) {
        if self.route != location.route {
            self.shell.resize = docked_pane::ResizeInteraction::default();
        }
        self.route = location.route;
        if let Some(id) = location.document {
            self.documents.pick(&id);
        }
        self.select = None;
    }

    fn visit(&mut self, location: Location) {
        let previous = self.location(self.route);
        if previous != location {
            self.shell.back.push(previous);
            self.shell.forward.clear();
            self.show(location);
        }
    }

    pub fn navigate(&mut self, route: Route) {
        self.visit(self.location(route));
    }

    pub fn select_document(&mut self, id: &str) {
        self.visit(Location {
            route: Route::Documents,
            document: Some(id.into()),
        });
    }

    pub fn reset_workspace_history(&mut self) {
        self.shell.back.clear();
        self.shell.forward.clear();
    }

    fn travel(&mut self, forward: bool) {
        let previous = self.location(self.route);
        let (from, to) = if forward {
            (&mut self.shell.forward, &mut self.shell.back)
        } else {
            (&mut self.shell.back, &mut self.shell.forward)
        };
        if let Some(location) = from.pop() {
            to.push(previous);
            self.show(location);
        }
    }

    fn toggle_sidebar(&mut self, tx: &mut Tx) {
        if self.route.has_sidebar() {
            self.shell.pane.toggle();
            if !self.shell.pane.is_open() {
                self.shell.resize = docked_pane::ResizeInteraction::default();
                tx.focus("toggle-sidebar", tx.focus_visible());
            }
        }
    }
}

pub fn shortcuts(s: &mut MeetNote, event: &KeyEvent, tx: &mut Tx) -> bool {
    if s.confirm.is_some() || s.select.is_some() {
        return false;
    }
    if s.document_tree_shortcut(event, tx) {
        return true;
    }
    if is_sidebar_toggle(event) && s.route.has_sidebar() {
        s.toggle_sidebar(tx);
    } else if event.is_command('f') && event.mods.shift {
        s.navigate(Route::Documents);
        s.shell.pane.set_open(true);
        tx.focus("workspace-search", true);
    } else if let Some(route) = [
        ('1', Route::Assistant),
        ('2', Route::Documents),
        ('3', Route::Models),
        ('4', Route::Settings),
        (',', Route::Settings),
    ]
    .into_iter()
    .find_map(|(key, route)| event.is_command(key).then_some(route))
    {
        s.navigate(route);
    } else if event.is_command('[') || event.is_command(']') {
        s.travel(event.is_command(']'));
    } else if event.is_command('s')
        && s.route == Route::Documents
        && s.documents.mode == documents::Mode::Edit
    {
        s.save_document(tx);
    } else if event.is_command('o') {
        s.import_audio(tx);
    } else {
        return false;
    }
    true
}

impl Route {
    fn has_sidebar(self) -> bool {
        matches!(self, Route::Documents | Route::Settings)
    }
}

fn rail(s: &MeetNote, k: &Kit) -> El {
    let nav = col()
        .w_full()
        .items_center()
        .gap(5.0)
        .role("tablist")
        .label(s.tr("navigation"))
        .focus_group()
        .children(Route::ALL.into_iter().map(|route| {
            let icon = match route {
                Route::Assistant => &glyphs::SPARKLE,
                Route::Documents => &glyphs::FOLDER,
                Route::Models => &glyphs::FILE_CODE,
                Route::Settings => &glyphs::KEYBOARD,
            };
            let active = route == s.route;
            IconButton::new(icon)
                .label(route.name(s.locale()))
                .help(route.name(s.locale()))
                .control_size(ControlSize::ExtraLarge)
                .variant(if active {
                    IconButtonVariant::Neutral
                } else {
                    IconButtonVariant::GhostMuted
                })
                .on_press(move |s: &mut MeetNote| s.navigate(route))
                .build(k)
                .key(route.key())
                .role("tab")
                .focus_selected(active)
        }));
    col()
        .key("navigation-rail")
        .role("navigation")
        .label(s.tr("navigation"))
        .w(RAIL_WIDTH)
        .min_w(RAIL_WIDTH)
        .h_full()
        .py(12.0)
        .items_center()
        .child(nav)
        .child(el().grow(1.0))
        .child(
            IconButton::new(&glyphs::MIC)
                .label(s.tr("import_audio"))
                .help(s.tr("import_audio"))
                .control_size(ControlSize::ExtraLarge)
                .disabled(s.workspace_host.busy())
                .on_press_tx(MeetNote::import_audio)
                .build(k)
                .key("import-audio"),
        )
}

fn sidebar(s: &MeetNote, k: &Kit, motion: Spring) -> El {
    let (key, content) = if s.route == Route::Documents {
        ("project-sidebar", documents::sidebar(s, k))
    } else {
        ("settings-sidebar", settings::sidebar(s, k))
    };
    docked_pane::leading(
        &s.shell.pane,
        &s.shell.resize,
        col().key(key).padding(14.0).child(content),
        motion,
    )
    .key("sidebar-slot")
}

pub fn layout(s: &MeetNote, k: &Kit, cx: &Cx<MeetNote>, page: El) -> El {
    let glass = s.settings.glass && !s.reduce_transparency;
    let open = s.shell.pane.is_open();
    let sidebar_label = s.tr("toggle_sidebar");
    let toolbar = WindowTitlebar::from_insets(cx.chrome_insets(), 1.0)
        .height(46.0)
        .fill(Color::TRANSPARENT)
        .leading_padding(10.0)
        .trailing_padding(16.0)
        .leading(
            IconButton::new(&BACK)
                .label(s.tr("navigate_back"))
                .help(s.tr("navigate_back"))
                .disabled(s.shell.back.is_empty())
                .on_press(|s: &mut MeetNote| s.travel(false))
                .build(k)
                .key("navigate-back"),
        )
        .leading(
            IconButton::new(&glyphs::ARROW_RIGHT)
                .label(s.tr("navigate_forward"))
                .help(s.tr("navigate_forward"))
                .disabled(s.shell.forward.is_empty())
                .on_press(|s: &mut MeetNote| s.travel(true))
                .build(k)
                .key("navigate-forward"),
        )
        .leading(
            IconButton::new(&SIDEBAR)
                .label(sidebar_label)
                .help(sidebar_label)
                .disabled(!s.route.has_sidebar())
                .expanded(open && s.route.has_sidebar())
                .on_press_tx(MeetNote::toggle_sidebar)
                .build(k)
                .key("toggle-sidebar"),
        )
        .leading(title(k, "MeetNote", 15.0));
    let toolbar = if s.sample_mode {
        toolbar.trailing(
            Badge::new("RUNE PREVIEW")
                .variant(BadgeVariant::Accent)
                .build(k),
        )
    } else {
        toolbar
    }
    .build(k);

    let motion = if cx.is_reduced_motion() {
        Spring::NONE
    } else {
        Spring::new(0.28, 0.0)
    };
    let frame = row()
        .key("inner-frame")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .mr(FRAME_GUTTER)
        .mb(FRAME_GUTTER)
        .continuous_corners()
        .clip_rounded(FRAME_RADIUS)
        .fill(if glass { Color::TRANSPARENT } else { k.c.page })
        .children(glass.then(|| {
            // Native views ignore ancestor clips. This one matches the entire frame,
            // so the sidebar can resize without exposing an extra material edge.
            el().key("workspace-glass")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .continuous_corners()
                .radius(FRAME_RADIUS)
                .system_material(SystemMaterial::Popover)
                .fill(k.c.page)
                .inert()
                .a11y_hidden()
                .child(el().w_full().h_full().fill(k.c.page.alpha(WORKSPACE_TINT)))
        }))
        .children(s.route.has_sidebar().then(|| sidebar(s, k, motion)))
        .children((s.route.has_sidebar() && open).then(|| {
            let peak = if k.dark {
                Color::WHITE.alpha(0.38)
            } else {
                Color::BLACK.alpha(0.30)
            };
            PaneDivider::new(s.tr("resize_sidebar"), k)
                .glow(18.0, peak, 110.0)
                .build(
                    &s.shell.pane,
                    &s.shell.resize,
                    |s: &mut MeetNote| (&mut s.shell.pane, &mut s.shell.resize),
                    motion,
                )
        }))
        .child(
            col()
                .key("content-plane")
                .grow(1.0)
                .min_w(0.0)
                .min_h(0.0)
                .h_full()
                .motion(motion)
                .child(page),
        )
        .child(
            el().key("frame-outline")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .continuous_corners()
                .radius(FRAME_RADIUS)
                .border(1.0, k.c.border_muted)
                .inert()
                .a11y_hidden(),
        );
    let chrome = col().w_full().h_full().child(toolbar).child(
        row()
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .child(rail(s, k))
            .child(frame),
    );
    let shell = el().key("window-shell").w_full().h_full();
    if glass {
        shell
            .child(
                el().key("shell-glass")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .system_material(SystemMaterial::Popover)
                    .fill(k.c.bg_deep)
                    .inert()
                    .a11y_hidden()
                    .child(el().w_full().h_full().fill(k.c.bg_deep.alpha(CHROME_TINT))),
            )
            .child(chrome.fill(k.c.layer_01.alpha(SHELL_TINT)))
    } else {
        shell.fill(k.c.bg_deep).child(chrome)
    }
}
