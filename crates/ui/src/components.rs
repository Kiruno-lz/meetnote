use crate::{
    controller::RootView,
    controls::{Button, Separator, TreeStep},
    i18n,
    state::ContentMode,
    theme::{self, Tokens},
};
use domain::{DocumentState, ProjectId, ProjectStatus};
use gpui::{div, prelude::*, px, Context, Pixels, Render, SharedString, Window};

/// A tree row being dragged, with the revision the row showed when the drag
/// started; the move is checked against it.
#[derive(Clone)]
pub(crate) struct ProjectDrag {
    pub(crate) project_id: ProjectId,
    pub(crate) revision: u64,
}

pub(crate) struct DragGhost {
    pub(crate) title: String,
}

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme::tokens(cx);
        div()
            .p_3()
            .rounded(tokens.radius.sm)
            .bg(tokens.drag_ghost)
            .text_sm()
            .child(self.title.clone())
    }
}

pub(crate) fn empty_card(message: impl Into<SharedString>, tokens: &Tokens) -> impl IntoElement {
    div()
        .flex_1()
        .rounded(tokens.radius.md)
        .bg(tokens.surface)
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .text_color(tokens.text_muted)
        .child(message.into())
}

pub(crate) fn settings_section(
    title: impl Into<SharedString>,
    rows: Vec<gpui::AnyElement>,
) -> impl IntoElement {
    div()
        .pb_4()
        .flex_shrink_0()
        .child(div().mb_1().text_lg().child(title.into()))
        .children(rows)
        .child(div().mt_4().child(Separator::horizontal()))
}

/// The settings label column; wide enough for the longest label in either
/// locale, so every control starts at the same x.
pub(crate) const SETTING_LABEL_WIDTH: Pixels = px(200.);
/// The width of selects and text fields in a settings row.
pub(crate) const SETTING_CONTROL_WIDTH: Pixels = px(360.);

/// A settings row: the label in the fixed label column, the control after
/// it. Rows are at least one large control tall, so rows of selects, fields
/// and buttons share one height.
pub(crate) fn setting_control_row(
    label: impl Into<SharedString>,
    control: gpui::AnyElement,
    tokens: &Tokens,
) -> impl IntoElement {
    div()
        .mt(tokens.spacing.sm)
        .min_h(tokens.control_height.lg)
        .flex()
        .items_center()
        .gap(tokens.spacing.lg)
        .text_sm()
        .child(
            div()
                .w(SETTING_LABEL_WIDTH)
                .flex_shrink_0()
                .text_color(tokens.text_muted)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .child(control),
        )
}

pub(crate) fn mode_button(
    label: impl Into<SharedString>,
    mode: ContentMode,
    active: ContentMode,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let label = label.into();
    let name = match mode {
        ContentMode::Summary => "summary",
        ContentMode::Original => "original",
        ContentMode::Edit => "edit",
    };
    Button::new(format!("content-mode-{name}"), label)
        .debug_selector(move || format!("content-mode-{name}"))
        .compact()
        .selected(mode == active)
        .on_click(cx.listener(move |this, _, _, cx| this.set_content_mode(mode, cx)))
}

pub(crate) fn ordered_projects(
    projects: &[DocumentState],
    expanded: &std::collections::BTreeSet<ProjectId>,
) -> Vec<(DocumentState, u8)> {
    fn append(
        projects: &[DocumentState],
        parent: Option<&ProjectId>,
        depth: u8,
        expanded: &std::collections::BTreeSet<ProjectId>,
        output: &mut Vec<(DocumentState, u8)>,
    ) {
        let mut children: Vec<_> = projects
            .iter()
            .filter(|project| project.hierarchy.parent_id.as_ref() == parent)
            .cloned()
            .collect();
        children.sort_by_key(|project| project.hierarchy.sibling_order);
        for child in children {
            let id = child.id().clone();
            output.push((child, depth));
            if expanded.contains(&id) {
                append(
                    projects,
                    Some(&id),
                    depth.saturating_add(1),
                    expanded,
                    output,
                );
            }
        }
    }
    let mut output = Vec::new();
    append(projects, None, 0, expanded, &mut output);
    output
}

/// What a key step in the project tree does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TreeMove {
    Select(ProjectId),
    Expand(ProjectId),
    Collapse(ProjectId),
}

/// What `step` does in the tree of `projects` with `expanded` open, from
/// the `selected` row: Up and Down select the visible row before or after
/// it (the first row without a visible selection); Right expands a
/// collapsed row with children or selects its first child; Left collapses
/// an expanded row or selects its parent. `None` at either end.
pub(crate) fn tree_move(
    projects: &[DocumentState],
    expanded: &std::collections::BTreeSet<ProjectId>,
    selected: Option<&ProjectId>,
    step: TreeStep,
) -> Option<TreeMove> {
    let rows = ordered_projects(projects, expanded);
    let id = |index: usize| rows.get(index).map(|(row, _)| row.id().clone());
    let Some(index) = selected.and_then(|id| rows.iter().position(|(row, _)| row.id() == id))
    else {
        return matches!(step, TreeStep::Previous | TreeStep::Next)
            .then(|| id(0))
            .flatten()
            .map(TreeMove::Select);
    };
    let (row, _) = &rows[index];
    let has_children = projects
        .iter()
        .any(|child| child.hierarchy.parent_id.as_ref() == Some(row.id()));
    let open = expanded.contains(row.id());
    match step {
        TreeStep::Previous => index.checked_sub(1).and_then(id).map(TreeMove::Select),
        TreeStep::Next => id(index + 1).map(TreeMove::Select),
        TreeStep::Expand if has_children && !open => Some(TreeMove::Expand(row.id().clone())),
        TreeStep::Expand if has_children => id(index + 1).map(TreeMove::Select),
        TreeStep::Expand => None,
        TreeStep::Collapse if has_children && open => Some(TreeMove::Collapse(row.id().clone())),
        TreeStep::Collapse => row.hierarchy.parent_id.clone().map(TreeMove::Select),
    }
}

pub(crate) fn status_label(locale: i18n::Locale, status: ProjectStatus) -> &'static str {
    i18n::status(locale, status)
}

pub(crate) fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let total_minutes = seconds / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let seconds = seconds % 60;
    format!(
        "{hours:02}:{minutes:02}:{seconds:02}.{:03}",
        milliseconds % 1_000
    )
}

/// A Unix-millisecond instant as its minute in UTC, `2026-08-29 10:40 UTC`:
/// UTC, so the text does not depend on the machine's time zone.
pub(crate) fn utc_time(unix_ms: u64) -> String {
    let seconds = unix_ms / 1_000;
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    let minute = seconds % 86_400 / 60;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        minute / 60,
        minute % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_from_march + 2) / 5 + 1) as u32;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    } as u32;
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

pub(crate) fn backend_ids(descriptors: &[application::BackendDescriptor]) -> String {
    descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::{tree_move, utc_time, TreeMove};
    use crate::controls::TreeStep;
    use domain::{DocumentState, ProjectId};
    use std::collections::BTreeSet;

    /// Two roots, the first with two children; the second child has a
    /// child of its own.
    fn projects() -> Vec<DocumentState> {
        let document = |id: &str, parent: Option<&str>, order: u32| {
            let mut document = DocumentState::new(ProjectId::new(id), id, "a.wav");
            document.hierarchy.parent_id = parent.map(ProjectId::new);
            document.hierarchy.sibling_order = order;
            document
        };
        vec![
            document("a", None, 0),
            document("a1", Some("a"), 0),
            document("a2", Some("a"), 1),
            document("a2x", Some("a2"), 0),
            document("b", None, 1),
        ]
    }

    fn open(ids: &[&str]) -> BTreeSet<ProjectId> {
        ids.iter().copied().map(ProjectId::new).collect()
    }

    #[test]
    fn up_and_down_step_through_the_visible_rows() {
        let (projects, expanded) = (projects(), open(&["a"]));
        let step =
            |from: &str, step| tree_move(&projects, &expanded, Some(&ProjectId::new(from)), step);
        let select = |id: &str| Some(TreeMove::Select(ProjectId::new(id)));
        assert_eq!(step("a", TreeStep::Next), select("a1"));
        // `a2x` is hidden under the collapsed `a2`.
        assert_eq!(step("a2", TreeStep::Next), select("b"));
        assert_eq!(
            step("b", TreeStep::Next),
            None,
            "Down stops at the last row"
        );
        assert_eq!(step("a1", TreeStep::Previous), select("a"));
        assert_eq!(
            step("a", TreeStep::Previous),
            None,
            "Up stops at the first row"
        );
        // Without a visible selection, Up and Down start at the first row.
        for from in [None, Some(ProjectId::new("a2x"))] {
            for key in [TreeStep::Previous, TreeStep::Next] {
                assert_eq!(
                    tree_move(&projects, &expanded, from.as_ref(), key),
                    select("a")
                );
            }
            assert_eq!(
                tree_move(&projects, &expanded, from.as_ref(), TreeStep::Expand),
                None
            );
        }
    }

    #[test]
    fn right_expands_then_enters_and_left_collapses_then_leaves() {
        let projects = projects();
        let step = |expanded: &[&str], from: &str, step| {
            tree_move(
                &projects,
                &open(expanded),
                Some(&ProjectId::new(from)),
                step,
            )
        };
        let id = ProjectId::new;
        assert_eq!(
            step(&[], "a", TreeStep::Expand),
            Some(TreeMove::Expand(id("a")))
        );
        assert_eq!(
            step(&["a"], "a", TreeStep::Expand),
            Some(TreeMove::Select(id("a1")))
        );
        assert_eq!(
            step(&["a"], "a1", TreeStep::Expand),
            None,
            "a leaf has nothing to open"
        );
        assert_eq!(
            step(&["a"], "a", TreeStep::Collapse),
            Some(TreeMove::Collapse(id("a")))
        );
        assert_eq!(
            step(&["a"], "a1", TreeStep::Collapse),
            Some(TreeMove::Select(id("a")))
        );
        assert_eq!(
            step(&["a", "a2"], "a2", TreeStep::Collapse),
            Some(TreeMove::Collapse(id("a2")))
        );
        assert_eq!(
            step(&["a"], "a2", TreeStep::Collapse),
            Some(TreeMove::Select(id("a")))
        );
        assert_eq!(
            step(&[], "b", TreeStep::Collapse),
            None,
            "a root leaf has no parent"
        );
    }

    #[test]
    fn utc_time_reads_the_gregorian_minute_in_utc() {
        assert_eq!(utc_time(0), "1970-01-01 00:00 UTC");
        // A leap day, and its last minute's seconds are dropped.
        assert_eq!(utc_time(951_782_400_000), "2000-02-29 00:00 UTC");
        assert_eq!(utc_time(951_868_799_999), "2000-02-29 23:59 UTC");
        assert_eq!(utc_time(1_788_000_000_000), "2026-08-29 10:40 UTC");
        assert_eq!(utc_time(4_107_542_400_000), "2100-03-01 00:00 UTC");
    }
}
