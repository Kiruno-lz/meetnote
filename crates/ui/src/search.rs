//! On-demand search of persisted workspace facts, with cancellable reads.
use crate::{
    components::timestamp,
    controller::RootView,
    controls::{Button, Scrollbar, TextField, TextFieldEvent},
    i18n::{self, text as tr, Locale},
    route::Route,
    state::ContentMode,
    theme,
};
use application::{DocumentSearchHit, DocumentSearchResults, SearchSource};
use gpui::{div, prelude::*, Context, Entity, Focusable, IntoElement, ScrollHandle, Window};
use runtime::CancellationToken;

pub(crate) struct WorkspaceSearch {
    pub(crate) input: Entity<TextField>,
    pub(crate) pending: bool,
    pub(crate) result: Option<Result<DocumentSearchResults, String>>,
    submitted: bool,
    query: String,
    generation: u64,
    cancellation: CancellationToken,
    scroll: ScrollHandle,
}

impl WorkspaceSearch {
    pub(crate) fn new(locale: Locale, window: &mut Window, cx: &mut Context<RootView>) -> Self {
        let input = cx.new(|cx| {
            TextField::new(
                tr(locale, "workspace_search"),
                tr(locale, "search_placeholder"),
                window,
                cx,
            )
        });
        cx.subscribe(&input, |root, _, event, cx| match event {
            TextFieldEvent::Change => {
                let query = root.search.input.read(cx).value(cx);
                if query != root.search.query {
                    root.search.invalidate();
                    root.search.query = query;
                    root.search.submitted = false;
                    cx.notify();
                }
            }
            TextFieldEvent::Submit => root.submit_workspace_search(cx),
        })
        .detach();
        Self {
            input,
            pending: false,
            result: None,
            submitted: false,
            query: String::new(),
            generation: 0,
            cancellation: CancellationToken::new(),
            scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.cancellation.cancel();
        self.generation += 1;
        self.pending = false;
        self.result = None;
    }

    pub(crate) fn follow_locale(&self, locale: Locale, cx: &mut Context<RootView>) {
        self.input.update(cx, |input, cx| {
            input.set_label(tr(locale, "workspace_search"), cx);
            input.set_placeholder(tr(locale, "search_placeholder"), cx);
        });
    }
}

impl Drop for WorkspaceSearch {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl RootView {
    pub(crate) fn submit_workspace_search(&mut self, cx: &mut Context<Self>) {
        self.search.invalidate();
        let query = self.search.input.read(cx).value(cx);
        self.search.query = query.clone();
        if query.trim().is_empty() || self.documents_root_switch.switching {
            cx.notify();
            return;
        }
        self.search.submitted = true;
        self.search.pending = true;
        self.search.scroll.set_offset(Default::default());
        self.search.cancellation = CancellationToken::new();
        let cancellation = self.search.cancellation.clone();
        let generation = self.search.generation;
        let documents = self.services.workspace.documents().frozen();
        cx.spawn(async move |this, cx| {
            let result =
                runtime::spawn_blocking(move || documents.search(&query, 50, &cancellation)).await;
            this.update(cx, |this, cx| {
                if generation == this.search.generation {
                    this.search.pending = false;
                    this.search.result = Some(result);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn refresh_workspace_search(&mut self, cx: &mut Context<Self>) {
        if self.search.submitted {
            self.submit_workspace_search(cx);
        }
    }

    pub(crate) fn clear_workspace_search(&mut self, cx: &mut Context<Self>) {
        self.search.invalidate();
        self.search.submitted = false;
        self.search.query.clear();
        self.search
            .input
            .update(cx, |input, cx| input.set_value("", cx));
        cx.notify();
    }

    fn open_search_hit(&mut self, hit: &DocumentSearchHit, cx: &mut Context<Self>) {
        // Expand ancestors so clearing the search reveals the same selection.
        let mut id = Some(hit.document_id.clone());
        for _ in 0..3 {
            id = id
                .and_then(|id| self.projects.iter().find(|document| document.id() == &id))
                .and_then(|document| document.hierarchy.parent_id.clone());
            if let Some(parent) = &id {
                self.ui.expanded_projects.insert(parent.clone());
            }
        }
        self.ui.content_mode = match hit.source {
            SearchSource::Transcript => ContentMode::Original,
            SearchSource::Markdown => ContentMode::Markdown,
            _ => ContentMode::Summary,
        };
        self.navigate(Route::Meetings, cx);
        self.select_project(hit.document_id.clone(), cx);
        if hit.source == SearchSource::Markdown {
            self.load_saved_document(hit.document_id.clone(), cx);
        }
        self.reveal_tree_selection = true;
    }

    pub(crate) fn focus_workspace_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Meetings, cx);
        self.ui.sidebar_open = true;
        let now = cx.background_executor().now();
        self.sidebar_sizing.update(cx, |sizing, _| {
            sizing.set_open(true, std::time::Duration::ZERO, now);
        });
        self.search
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    }
}

pub(crate) fn input(root: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let locale = root.ui.locale;
    div()
        .px(tokens.spacing.sm)
        .pb(tokens.spacing.sm)
        .flex()
        .flex_col()
        .gap(tokens.spacing.xs)
        .child(
            div()
                .debug_selector(|| "workspace-search-input".into())
                .child(root.search.input.clone()),
        )
        .child(
            div()
                .flex()
                .gap(tokens.spacing.xs)
                .child(
                    Button::new("workspace-search-submit", tr(locale, "search_submit"))
                        .debug_selector(|| "workspace-search-submit".into())
                        .compact()
                        .disabled(root.documents_root_switch.switching)
                        .on_click(cx.listener(|root, _, _, cx| root.submit_workspace_search(cx))),
                )
                .when(!root.search.input.read(cx).value(cx).is_empty(), |row| {
                    row.child(
                        Button::new("workspace-search-clear", tr(locale, "search_clear"))
                            .debug_selector(|| "workspace-search-clear".into())
                            .compact()
                            .ghost()
                            .on_click(
                                cx.listener(|root, _, _, cx| root.clear_workspace_search(cx)),
                            ),
                    )
                }),
        )
}

pub(crate) fn results(root: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let scroll = root.search.scroll.clone();
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .child(theme::scroll_faded(
            &scroll,
            tokens.sidebar,
            result_list(root, cx),
        ))
        .child(Scrollbar::vertical("workspace-search-scrollbar", &scroll))
}

fn result_list(root: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let tokens = *theme::tokens(cx);
    let locale = root.ui.locale;
    let mut list = div()
        .id("workspace-search-results")
        .debug_selector(|| "workspace-search-results".into())
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&root.search.scroll)
        .p(tokens.spacing.sm)
        .flex()
        .flex_col()
        .gap(tokens.spacing.sm)
        .text_sm();
    if root.search.pending {
        return list.child(
            div()
                .debug_selector(|| "workspace-search-pending".into())
                .child(tr(locale, "search_pending")),
        );
    }
    let Some(result) = &root.search.result else {
        return list.child(tr(locale, "search_hint"));
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            return list.child(
                div()
                    .debug_selector(|| "workspace-search-error".into())
                    .child(i18n::message(locale, "search_failed", error)),
            )
        }
    };
    let count = format!(
        "{}{}",
        result.hits.len(),
        if result.truncated { "+" } else { "" }
    );
    list = list.child(
        div()
            .debug_selector(|| "workspace-search-count".into())
            .text_color(tokens.text_muted)
            .child(i18n::message(locale, "search_count", &count)),
    );
    if result.hits.is_empty() {
        list = list.child(
            div()
                .debug_selector(|| "workspace-search-empty".into())
                .child(tr(locale, "search_empty")),
        );
    }
    if result.truncated {
        list = list.child(tr(locale, "search_truncated"));
    }
    if result.unreadable_count > 0 {
        list = list.child(i18n::message(
            locale,
            "search_unreadable",
            &result.unreadable_count.to_string(),
        ));
        for (id, error) in &result.unreadable {
            list = list.child(
                div()
                    .text_color(tokens.text_muted)
                    .child(format!("{id}: {error}")),
            );
        }
    }
    for hit in &result.hits {
        let source = match hit.source {
            SearchSource::Title => "search_title",
            SearchSource::Tag => "search_tag",
            SearchSource::Summary => "search_summary",
            SearchSource::Transcript => "search_transcript",
            SearchSource::Markdown => "search_markdown",
        };
        let label = match hit.start_ms {
            Some(start) => format!("{} · {}", tr(locale, source), timestamp(start)),
            None => tr(locale, source).into(),
        };
        let selected = root.ui.selected_project.as_ref() == Some(&hit.document_id);
        let target = hit.clone();
        let id = hit.document_id.as_str().to_string();
        list = list.child(
            div()
                .flex_shrink_0()
                .rounded(tokens.radius.sm)
                .p(tokens.spacing.xs)
                .when(selected, |row| row.bg(tokens.selection))
                .child(
                    Button::new(format!("search-open-{id}"), hit.title.clone())
                        .debug_selector(move || format!("search-open-{id}"))
                        .ghost()
                        .w_full()
                        .on_click(
                            cx.listener(move |root, _, _, cx| root.open_search_hit(&target, cx)),
                        ),
                )
                .child(div().text_color(tokens.text_muted).child(label))
                .child(div().child(hit.snippet.clone())),
        );
    }
    list
}
