//! The Providers page: connected providers as cards, the vendors to add
//! as a grouped list, and the detail sheet. Live and sample data share
//! this one render path; the Assistant's model picker reads the same state.

use std::time::{SystemTime, UNIX_EPOCH};

use application::{
    CredentialSummary, ProviderView, llm_protocol, model_choice, model_ids, visible_models,
};
use llm::LlmProtocol;
use llm::vendors::catalog::{SignIn, VendorKind};
use llm::vendors::quota::{Quota, QuotaWindow};
use rune::{Color, El, TextStyle, col, el, icon, row, text};
use rune_kit::{
    Badge, BadgeVariant, Button, ButtonVariant, IconButton, IconButtonVariant, Kit, Select, Switch,
    TextInput,
};
use settings::{ModelChoice, ModelListMode, ModelSelection};
use ui_text::Locale;

use crate::controls::{caption, page_header, scroll};
use crate::workspace_host::providers::{Live, SEARCH, SEARCH_BUTTON, Snapshot, Write};
use crate::{MeetNote, Route};

pub mod logos;
mod sheet;

/// Two card columns once the content plane is at least this wide.
pub const TWO_COLUMNS: f32 = 960.0;
const MAX_WIDTH: f32 = 760.0;

pub fn page(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>, wide: bool) -> El {
    let host = &s.workspace_host.providers;
    let mut body = col().w_full().max_w(MAX_WIDTH).gap(28.0).child(page_header(
        k,
        s.tr("providers"),
        s.tr("providers_caption"),
    ));
    match &host.snapshot {
        Some(snapshot) => {
            let connected = snapshot
                .views
                .iter()
                .filter(|view| view.configured)
                .map(|view| card(s, k, snapshot, view))
                .collect::<Vec<_>>();
            if !connected.is_empty() {
                body = body.child(section(
                    k,
                    s.tr("providers_connected"),
                    grid(connected, wide),
                ));
            }
            body = body.child(
                col()
                    .w_full()
                    .gap(10.0)
                    .child(add_heading(s, k))
                    .child(add_list(s, k, snapshot)),
            );
        }
        None => {
            body = body.child(caption(
                k,
                host.error.as_deref().unwrap_or(s.tr("providers_loading")),
            ))
        }
    }
    if let Some(sheet) = &host.sheet {
        sheet::build(s, k, overlays, sheet);
    }
    col()
        .key("providers-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(scroll(
            k,
            "providers-scroll",
            row().w_full().justify_center().padding(24.0).child(body),
        ))
}

fn section(k: &Kit, heading: &str, content: El) -> El {
    col()
        .w_full()
        .gap(10.0)
        .child(section_heading(k, heading))
        .child(content)
}

fn section_heading(k: &Kit, heading: &str) -> El {
    text(heading.to_owned())
        .font(TextStyle::new(13.0, 650.0))
        .color(k.c.text_muted)
}

/// "添加供应商" with the vendor search at its right: a button, or the
/// open field.
fn add_heading(s: &MeetNote, k: &Kit) -> El {
    let host = &s.workspace_host.providers;
    let search = if host.searching {
        TextInput::new(&host.search)
            .label(s.tr("provider_search"))
            .placeholder(s.tr("provider_search_placeholder"))
            .leading_icon(k.glyph.search)
            .width(Some(240.0))
            .clear_button()
            .on_action(MeetNote::close_provider_search)
            .on_edit(|s: &mut MeetNote| &mut s.workspace_host.providers.search)
            .on_key(MeetNote::provider_search_key)
            .build(k)
            .key(SEARCH)
    } else {
        IconButton::new(k.glyph.search)
            .variant(IconButtonVariant::Ghost)
            .label(s.tr("provider_search"))
            .help(s.tr("provider_search"))
            .on_press_tx(MeetNote::open_provider_search)
            .build(k)
            .key(SEARCH_BUTTON)
    };
    row()
        .w_full()
        .h(28.0)
        .gap(8.0)
        .items_center()
        .child(
            section_heading(k, s.tr("providers_add"))
                .grow(1.0)
                .min_w(0.0),
        )
        .child(search)
}

/// The unconfigured catalog vendors whose name or id holds `query`,
/// ignoring case, in catalog order.
pub(crate) fn search<'a>(views: &'a [ProviderView], query: &str) -> Vec<&'a ProviderView> {
    let query = query.trim().to_lowercase();
    views
        .iter()
        .filter(|view| !view.configured && view.vendor.is_some())
        .filter(|view| {
            view.name.to_lowercase().contains(&query) || view.id.to_lowercase().contains(&query)
        })
        .collect()
}

/// Cards two to a row when `wide`, each column the same width.
fn grid(cards: Vec<El>, wide: bool) -> El {
    let columns = if wide { 2 } else { 1 };
    let mut rows = col().w_full().gap(12.0);
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        let mut line = row().w_full().gap(12.0).items_stretch();
        for _ in 0..columns {
            let cell = cards.next().unwrap_or_else(|| el().a11y_hidden());
            line = line.child(cell.w_frac(1.0 / columns as f32).shrink().min_w(0.0));
        }
        rows = rows.child(line);
    }
    rows
}

/// The vendor's mark on a rounded tile; a custom endpoint gets its initial.
pub(crate) fn logo_tile(k: &Kit, id: &str, name: &str, side: f32) -> El {
    let mark = match logos::logo(id) {
        Some(logo) => icon(logo, side * 0.55).color(k.c.text_base),
        None => text(
            name.chars()
                .next()
                .map_or_else(|| "?".to_owned(), |first| first.to_uppercase().to_string()),
        )
        .font(TextStyle::new(side * 0.42, 650.0))
        .color(k.c.text_muted),
    };
    el().size(side, side)
        .min_w(side)
        .radius(side / 4.0)
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_muted)
        .center()
        .a11y_hidden()
        .child(mark)
}

pub(crate) fn name(k: &Kit, value: &str, size: f32) -> El {
    text(value.to_owned())
        .font(TextStyle::new(size, 650.0))
        .color(k.c.text_base)
        .truncate()
}

pub(crate) fn small(value: impl Into<String>, color: Color) -> El {
    text(value.into())
        .font(TextStyle::new(12.0, 400.0))
        .color(color)
        .truncate()
}

/// What a provider is signed in or keyed with, on one line.
fn status(s: &MeetNote, k: &Kit, view: &ProviderView) -> El {
    let line = row().gap(6.0).items_center().min_w(0.0);
    let line = match &view.credential {
        CredentialSummary::Account { email, plan, .. } => line
            .child(small(email.clone(), k.c.text_muted).shrink())
            .children(
                plan.as_deref()
                    .map(|plan| Badge::new(plan_label(plan)).build(k)),
            ),
        CredentialSummary::ApiKey { last4 } => line.child(small(
            format!("{} ····{last4}", s.tr("provider_api_key")),
            k.c.text_muted,
        )),
        CredentialSummary::None => line.child(small(
            match view.vendor {
                Some(vendor) if vendor.kind == VendorKind::Local => s.tr("provider_local_service"),
                Some(vendor) if vendor.sign_in.is_some() && !view.enabled => {
                    s.tr("provider_signed_out")
                }
                Some(vendor) if !vendor.key_required => s.tr("provider_local_service"),
                _ => s.tr("provider_no_key"),
            },
            k.c.text_muted,
        )),
    };
    line.children((!view.enabled).then(|| {
        Badge::new(s.tr("provider_disabled"))
            .variant(BadgeVariant::Subtle)
            .build(k)
    }))
}

fn card(s: &MeetNote, k: &Kit, snapshot: &Snapshot, view: &ProviderView) -> El {
    let host = &s.workspace_host.providers;
    let live = host.live.get(&view.id);
    let id = view.id.clone();
    let enabled = view.enabled;
    let toggle = id.clone();
    let head = row()
        .w_full()
        .gap(12.0)
        .items_center()
        .child(logo_tile(k, &view.id, &view.name, 40.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(name(k, &view.name, 15.0))
                .child(status(s, k, view)),
        )
        .child(
            Switch::new(enabled)
                .label(format!("{} · {}", view.name, s.tr("provider_enabled")))
                .hide_label()
                .on_toggle_tx(move |s: &mut MeetNote, tx| {
                    s.provider_write(
                        Write::Toggle {
                            id: toggle.clone(),
                            enabled: !enabled,
                        },
                        tx,
                    )
                })
                .build(k)
                .key(format!("provider-{id}-enabled")),
        );
    let in_use = snapshot
        .model
        .as_ref()
        .is_some_and(|choice| choice.provider == view.id);
    let mut card = col()
        .key(format!("provider-{id}"))
        .role("button")
        .label(view.name.clone())
        .focusable(true)
        .focus_ring(2.0, 1.0, k.c.border_focus)
        .w_full()
        .gap(12.0)
        .padding(16.0)
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .hover(k.c.hover)
        .press(k.c.pressed)
        .on_press(move |s: &mut MeetNote| s.open_provider_sheet(&id))
        .child(head);
    if let Some(quota) = live.and_then(|live| live.quota.as_ref()) {
        card = card.children(quota_summary(s, k, quota));
    }
    let footer = row()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(
            small(model_summary(s, view, live), k.c.text_muted)
                .grow(1.0)
                .min_w(0.0),
        )
        .children(in_use.then(|| {
            Badge::new(s.tr("provider_in_use"))
                .variant(BadgeVariant::Accent)
                .build(k)
        }));
    card.child(footer).children(
        (in_use && !view.enabled).then(|| small(s.tr("provider_disabled_in_use"), k.c.danger.fg)),
    )
}

/// "12 个模型 · 每系最新", or only the mode before the list arrives.
fn model_summary(s: &MeetNote, view: &ProviderView, live: Option<&Live>) -> String {
    let selection = &view.settings.models;
    let mode = mode_name(s.locale(), selection.mode);
    match live.and_then(Live::ids) {
        Some(models) => format!(
            "{} · {mode}",
            ui_text::message(
                s.locale(),
                "provider_model_count",
                &visible_models(selection, &models).len().to_string()
            )
        ),
        None if selection.mode == ModelListMode::Picked => format!(
            "{} · {mode}",
            ui_text::message(
                s.locale(),
                "provider_model_count",
                &visible_models(selection, &[]).len().to_string()
            )
        ),
        None => mode.to_owned(),
    }
}

pub(crate) fn mode_name(locale: Locale, mode: ModelListMode) -> &'static str {
    ui_text::text(
        locale,
        match mode {
            ModelListMode::Vendor => "provider_mode_vendor",
            ModelListMode::Picked => "provider_mode_picked",
            ModelListMode::LatestPerFamily => "provider_mode_latest",
        },
    )
}

/// Usage bars, reset cards and balance: what a card shows of a quota.
fn quota_summary(s: &MeetNote, k: &Kit, quota: &Quota) -> Vec<El> {
    let now = unix_now();
    let mut parts = quota
        .windows
        .iter()
        .map(|window| window_row(s, k, window, now))
        .collect::<Vec<_>>();
    let mut chips = row().gap(6.0).items_center();
    let mut any = false;
    if let Some(cards) = quota.reset_cards.filter(|cards| *cards > 0) {
        chips = chips.child(chip(
            k,
            ui_text::message(s.locale(), "provider_reset_cards", &cards.to_string()),
        ));
        any = true;
    }
    if let Some(balance) = &quota.balance {
        chips = chips.child(small(
            ui_text::message(s.locale(), "provider_balance", balance),
            k.c.text_base,
        ));
        any = true;
    }
    if any {
        parts.push(chips);
    }
    parts
}

/// One usage window: its span, a thin bar of what is left, and the reset.
pub(crate) fn window_row(s: &MeetNote, k: &Kit, window: &QuotaWindow, now: u64) -> El {
    const SPAN: f32 = 56.0;
    let locale = s.locale();
    let left = (100.0 - window.used_percent).clamp(0.0, 100.0);
    let mut detail = remaining_text(locale, window.used_percent);
    if let Some(resets_at) = window.resets_at {
        detail = format!("{detail} · {}", reset_text(locale, resets_at, now));
    }
    row()
        .w_full()
        .gap(10.0)
        .items_center()
        .child(
            small(
                window
                    .span_seconds
                    .map_or_else(String::new, |secs| duration(locale, secs)),
                k.c.text_muted,
            )
            .w(SPAN)
            .min_w(SPAN),
        )
        .child(meter(k, left / 100.0).grow(1.0).min_w(40.0))
        .child(small(detail, k.c.text_muted).min_w(0.0))
}

fn meter(k: &Kit, value: f32) -> El {
    row()
        .role("progressbar")
        .range_value(f64::from(value), 0.0, 1.0)
        .h(4.0)
        .radius(2.0)
        .fill(k.c.layer_03)
        .clip_rounded(2.0)
        .child(
            el().h_full()
                .w_frac(value.clamp(0.0, 1.0))
                .radius(2.0)
                .fill(k.c.text_accent),
        )
}

pub(crate) fn chip(k: &Kit, value: String) -> El {
    row()
        .h(20.0)
        .px(8.0)
        .radius(10.0)
        .center()
        .fill(k.c.layer_02)
        .border(1.0, k.c.border_muted)
        .child(small(value, k.c.text_base))
}

/// Unconfigured featured vendors, then a disclosure with the rest grouped
/// by kind, or every vendor a search finds; then a custom endpoint.
fn add_list(s: &MeetNote, k: &Kit, snapshot: &Snapshot) -> El {
    let host = &s.workspace_host.providers;
    let first_featured = llm::vendors::catalog::catalog()
        .iter()
        .find(|vendor| vendor.featured)
        .map(|vendor| vendor.id);
    let query = host.search.text.trim();
    let mut rows = if host.searching && !query.is_empty() {
        let found = search(&snapshot.views, query);
        if found.is_empty() {
            vec![
                row()
                    .key("providers-search-empty")
                    .w_full()
                    .h(44.0)
                    .px(14.0)
                    .items_center()
                    .child(small(s.tr("provider_search_empty"), k.c.text_muted)),
            ]
        } else {
            found
                .into_iter()
                .map(|view| add_row(s, k, view, first_featured == Some(view.id.as_str())))
                .collect()
        }
    } else {
        browse_rows(s, k, snapshot, first_featured)
    };
    rows.push(
        row()
            .key("provider-add-custom")
            .role("button")
            .label(s.tr("provider_add_custom"))
            .focusable(true)
            .focus_ring(2.0, -1.0, k.c.border_focus)
            .w_full()
            .h(44.0)
            .px(14.0)
            .items_center()
            .hover(k.c.hover)
            .on_press(MeetNote::open_custom_sheet)
            .child(
                text(s.tr("provider_add_custom"))
                    .font(TextStyle::new(13.0, 500.0))
                    .color(k.c.text_accent)
                    .a11y_hidden(),
            ),
    );
    let mut list = col()
        .key("providers-add-list")
        .w_full()
        .radius(12.0)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_muted)
        .clip_rounded(12.0);
    for (index, line) in rows.into_iter().enumerate() {
        if index > 0 {
            list = list.child(el().h(1.0).w_full().fill(k.c.border_muted).a11y_hidden());
        }
        list = list.child(line);
    }
    list
}

/// The add list without a search: featured vendors, then the rest behind
/// a disclosure, grouped by kind.
fn browse_rows(
    s: &MeetNote,
    k: &Kit,
    snapshot: &Snapshot,
    first_featured: Option<&str>,
) -> Vec<El> {
    let host = &s.workspace_host.providers;
    let open = snapshot
        .views
        .iter()
        .filter(|view| !view.configured && view.vendor.is_some());
    let featured = open
        .clone()
        .filter(|view| view.vendor.is_some_and(|vendor| vendor.featured));
    let rest = open
        .filter(|view| view.vendor.is_some_and(|vendor| !vendor.featured))
        .collect::<Vec<_>>();
    let mut rows = featured
        .map(|view| add_row(s, k, view, first_featured == Some(view.id.as_str())))
        .collect::<Vec<_>>();
    if !rest.is_empty() {
        let more = host.more;
        rows.push(
            row()
                .key("providers-more")
                .role("button")
                .label(ui_text::message(
                    s.locale(),
                    "provider_more",
                    &rest.len().to_string(),
                ))
                .expanded(more)
                .focusable(true)
                .focus_ring(2.0, -1.0, k.c.border_focus)
                .w_full()
                .h(44.0)
                .px(14.0)
                .gap(8.0)
                .items_center()
                .hover(k.c.hover)
                .on_press(|s: &mut MeetNote| {
                    let host = &mut s.workspace_host.providers;
                    host.more = !host.more;
                })
                .child(
                    text(ui_text::message(
                        s.locale(),
                        "provider_more",
                        &rest.len().to_string(),
                    ))
                    .font(TextStyle::new(13.0, 500.0))
                    .color(k.c.text_base)
                    .grow(1.0)
                    .a11y_hidden(),
                )
                .child(
                    icon(
                        if more {
                            k.glyph.chevron_up
                        } else {
                            k.glyph.chevron_down
                        },
                        14.0,
                    )
                    .color(k.c.icon_muted),
                ),
        );
        if more {
            for (kind, heading) in [
                (VendorKind::Vendor, "provider_kind_vendor"),
                (VendorKind::Relay, "provider_kind_relay"),
                (VendorKind::Local, "provider_kind_local"),
            ] {
                let group = rest
                    .iter()
                    .filter(|view| view.vendor.is_some_and(|vendor| vendor.kind == kind))
                    .collect::<Vec<_>>();
                if group.is_empty() {
                    continue;
                }
                rows.push(
                    text(s.tr(heading))
                        .font(TextStyle::new(11.0, 650.0))
                        .color(k.c.text_muted)
                        .px(14.0)
                        .pt(10.0)
                        .pb(4.0),
                );
                rows.extend(group.into_iter().map(|view| add_row(s, k, view, false)));
            }
        }
    }
    rows
}

fn add_row(s: &MeetNote, k: &Kit, view: &ProviderView, recommended: bool) -> El {
    let subtitle = match view.vendor {
        _ if recommended => s.tr("provider_recommended"),
        Some(vendor) if vendor.kind == VendorKind::Local => s.tr("provider_this_mac"),
        Some(vendor) => match vendor.sign_in {
            Some(SignIn::ChatGpt) => s.tr("provider_sign_in_chatgpt"),
            Some(SignIn::OpenRouter) => s.tr("provider_sign_in_openrouter"),
            None => s.tr("provider_api_key"),
        },
        None => s.tr("provider_custom"),
    };
    let id = view.id.clone();
    let open = id.clone();
    row()
        .key(format!("add-{id}"))
        .w_full()
        .h(56.0)
        .px(14.0)
        .gap(12.0)
        .items_center()
        .hover(k.c.hover)
        .on_press(move |s: &mut MeetNote| s.open_provider_sheet(&open))
        .child(logo_tile(k, &view.id, &view.name, 32.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(name(k, &view.name, 14.0))
                .child(small(subtitle, k.c.text_muted)),
        )
        .child(
            Button::new(s.tr("provider_connect"))
                .variant(ButtonVariant::Outline)
                .on_press(move |s: &mut MeetNote| s.open_provider_sheet(&id))
                .build(k)
                .key(format!("provider-connect-{}", view.id)),
        )
}

/// An API's name on the sheet's endpoint rows.
pub(crate) fn protocol_name(protocol: LlmProtocol) -> &'static str {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => "Chat Completions",
        LlmProtocol::OpenAiResponses => "Responses",
        LlmProtocol::AnthropicMessages => "Anthropic Messages",
    }
}

/// The tag of the API a model of `view` runs on: its short name, shown only
/// when `view` serves more than one API.
pub(crate) fn model_tag(
    view: &ProviderView,
    protocol: Option<settings::LlmProtocol>,
) -> Option<&'static str> {
    let protocol = protocol.filter(|_| view.endpoints.protocols().len() > 1)?;
    Some(match llm_protocol(protocol) {
        LlmProtocol::OpenAiChatCompletions => "Chat",
        LlmProtocol::OpenAiResponses => "Responses",
        LlmProtocol::AnthropicMessages => "Messages",
    })
}

/// "Provider · model", then the API's tag when the provider serves several.
fn model_label(view: &ProviderView, choice: &ModelChoice) -> String {
    let label = format!("{} · {}", view.name, choice.model);
    match model_tag(view, choice.protocol) {
        Some(tag) => format!("{label} · {tag}"),
        None => label,
    }
}

/// The models the Assistant can run on: every enabled provider's visible
/// models, each on the API it resolves to, by label.
pub(crate) fn model_options(s: &MeetNote) -> Vec<(String, ModelChoice)> {
    let host = &s.workspace_host.providers;
    let Some(snapshot) = &host.snapshot else {
        return Vec::new();
    };
    snapshot
        .views
        .iter()
        .filter(|view| view.enabled)
        .flat_map(|view| {
            let live = host
                .live
                .get(&view.id)
                .and_then(|live| live.models.as_deref())
                .unwrap_or_default();
            visible_models(&view.settings.models, &model_ids(live))
                .into_iter()
                .map(|model| {
                    let choice = model_choice(view, live, &model);
                    (model_label(view, &choice), choice)
                })
        })
        .collect()
}

/// The Assistant's model, and whether its provider can run it.
pub(crate) fn current_model(s: &MeetNote) -> Option<(String, bool)> {
    let snapshot = s.workspace_host.providers.snapshot.as_ref()?;
    let choice = snapshot.model.as_ref()?;
    let view = snapshot.view(&choice.provider);
    Some((
        view.map_or_else(
            || format!("{} · {}", choice.provider, choice.model),
            |view| model_label(view, choice),
        ),
        view.is_some_and(|view| view.enabled),
    ))
}

/// The Assistant's model picker; `None` when there is nothing to pick.
pub(crate) fn model_picker(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>) -> Option<El> {
    const KEY: &str = "assistant-model";
    let mut options = model_options(s);
    // A model whose provider is off stays listed, marked, so the picker shows why runs fail.
    let current = current_model(s).map(|(label, enabled)| {
        if enabled {
            label
        } else {
            format!("{label} · {}", s.tr("provider_disabled"))
        }
    });
    if let Some(label) = &current
        && !options.iter().any(|(option, _)| option == label)
    {
        let choice = s
            .workspace_host
            .providers
            .snapshot
            .as_ref()?
            .model
            .clone()?;
        options.insert(0, (label.clone(), choice));
    }
    if options.is_empty() {
        return None;
    }
    let labels = options
        .iter()
        .map(|(label, _)| label.as_str())
        .collect::<Vec<_>>();
    Some(
        Select::new(KEY, &labels)
            .label(s.tr("assistant_model_picker"))
            .placeholder(s.tr("assistant_model_picker"))
            .width(260.0)
            .current(current.as_deref())
            .open(s.select == Some(KEY))
            .on_toggle(|s: &mut MeetNote| s.select = (s.select != Some(KEY)).then_some(KEY))
            .on_close(|s: &mut MeetNote| {
                if s.select == Some(KEY) {
                    s.select = None;
                }
            })
            .on_choose(move |s: &mut MeetNote, picked, tx| {
                s.select = None;
                if let Some((_, choice)) = options.iter().find(|(label, _)| label == picked) {
                    s.set_assistant_model(choice.clone(), tx);
                }
            })
            .build(k, overlays),
    )
}

/// Why the Assistant cannot run on its model, with a way to Providers.
pub(crate) fn model_notice(s: &MeetNote, k: &Kit) -> Option<El> {
    let message = match current_model(s) {
        Some((_, true)) => return None,
        Some((label, false)) => {
            let provider = label.split(" · ").next().unwrap_or_default().to_owned();
            ui_text::message(s.locale(), "assistant_provider_disabled", &provider)
        }
        None if model_options(s).is_empty() => s.tr("assistant_no_models_hint").to_owned(),
        None => return None,
    };
    let empty = current_model(s).is_none();
    Some(
        crate::controls::card(k)
            .key("assistant-model-notice")
            .children(
                empty.then(|| crate::controls::title(k, s.tr("assistant_no_models_title"), 17.0)),
            )
            .child(small(message, if empty { k.c.text_muted } else { k.c.danger.fg }).wrap())
            .child(
                Button::new(s.tr("assistant_open_providers"))
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut MeetNote| s.navigate(Route::Providers))
                    .build(k)
                    .key("assistant-open-providers"),
            ),
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// A span or wait in its largest whole unit, rounded: "5 小时", "7 days".
pub(crate) fn duration(locale: Locale, seconds: u64) -> String {
    let rounded = |unit: u64| ((seconds + unit / 2) / unit).max(1).to_string();
    match seconds {
        s if s >= 86_400 => ui_text::message(locale, "duration_days", &rounded(86_400)),
        s if s >= 3_600 => ui_text::message(locale, "duration_hours", &rounded(3_600)),
        _ => ui_text::message(locale, "duration_minutes", &rounded(60)),
    }
}

/// "剩余 82%" for a window 18% used.
pub(crate) fn remaining_text(locale: Locale, used_percent: f32) -> String {
    let left = (100.0 - used_percent).clamp(0.0, 100.0).round() as u32;
    ui_text::message(locale, "provider_remaining", &left.to_string())
}

/// "2 小时后重置", or "即将重置" once it is due.
pub(crate) fn reset_text(locale: Locale, resets_at: u64, now: u64) -> String {
    match resets_at.saturating_sub(now) {
        0 => ui_text::text(locale, "provider_resets_soon").to_owned(),
        wait => ui_text::message(locale, "provider_resets_in", &duration(locale, wait)),
    }
}

/// "刚刚", "3 分钟前": how long ago a list arrived.
pub(crate) fn ago_text(locale: Locale, seconds: f64) -> String {
    if seconds < 60.0 {
        ui_text::text(locale, "provider_just_now").to_owned()
    } else {
        ui_text::message(locale, "time_ago", &duration(locale, seconds as u64))
    }
}

/// A ChatGPT plan type as its product name: `plus` is "ChatGPT Plus".
pub(crate) fn plan_label(plan: &str) -> String {
    let words = plan
        .split(['_', '-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
        })
        .collect::<Vec<_>>();
    format!("ChatGPT {}", words.join(" "))
}

/// Unix seconds as a UTC calendar date, `2026-12-13`.
pub(crate) fn date_text(seconds: u64) -> String {
    // Howard Hinnant's civil_from_days.
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The host a link shows: `https://platform.openai.com/x` is `platform.openai.com`.
pub(crate) fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chip {
    /// A model `Picked` offers; pressing unpicks it.
    On,
    /// A listed model `Picked` leaves out; pressing picks it.
    Off,
    /// A model the mode offers, read-only.
    Shown,
}

/// The sheet's model chips: in `Picked` every listed, picked and added
/// model is a toggle; the other modes show what they offer.
pub(crate) fn chips(selection: &ModelSelection, live: &[String]) -> Vec<(String, Chip)> {
    if selection.mode != ModelListMode::Picked {
        return visible_models(selection, live)
            .into_iter()
            .map(|model| (model, Chip::Shown))
            .collect();
    }
    let mut models: Vec<String> = Vec::new();
    for model in live.iter().chain(&selection.picked).chain(&selection.added) {
        if !models.contains(model) {
            models.push(model.clone());
        }
    }
    models
        .into_iter()
        .map(|model| {
            let on = selection.picked.contains(&model) || selection.added.contains(&model);
            (model, if on { Chip::On } else { Chip::Off })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn spans_and_waits_read_in_their_largest_unit() {
        for (seconds, zh, en) in [
            (5 * 3_600, "5 小时", "5 hours"),
            (7 * 86_400, "7 天", "7 days"),
            (3_600, "1 小时", "1 hour"),
            (86_400, "1 天", "1 day"),
            (90 * 60, "2 小时", "2 hours"),
            (45 * 60, "45 分钟", "45 minutes"),
            (20, "1 分钟", "1 minute"),
        ] {
            assert_eq!(duration(Locale::ZhCn, seconds), zh);
            assert_eq!(duration(Locale::EnUs, seconds), en);
        }
    }

    #[test]
    fn remaining_and_reset_text_follow_the_locale() {
        assert_eq!(remaining_text(Locale::ZhCn, 18.0), "剩余 82%");
        assert_eq!(remaining_text(Locale::EnUs, 18.4), "82% left");
        assert_eq!(remaining_text(Locale::EnUs, 140.0), "0% left");
        let now = 1_000_000;
        assert_eq!(
            reset_text(Locale::ZhCn, now + 2 * 3_600 - 3, now),
            "2 小时后重置"
        );
        assert_eq!(
            reset_text(Locale::EnUs, now + 3 * 86_400, now),
            "Resets in 3 days"
        );
        assert_eq!(reset_text(Locale::ZhCn, now - 5, now), "即将重置");
        assert_eq!(reset_text(Locale::EnUs, now, now), "Resets soon");
        assert_eq!(ago_text(Locale::ZhCn, 12.0), "刚刚");
        assert_eq!(ago_text(Locale::EnUs, 180.0), "3 minutes ago");
    }

    #[test]
    fn plans_are_title_cased_product_names() {
        assert_eq!(plan_label("plus"), "ChatGPT Plus");
        assert_eq!(plan_label("PRO"), "ChatGPT Pro");
        assert_eq!(
            plan_label("self_serve_business"),
            "ChatGPT Self Serve Business"
        );
    }

    #[test]
    fn dates_and_hosts_are_read_off_their_raw_forms() {
        assert_eq!(date_text(0), "1970-01-01");
        assert_eq!(date_text(1_798_675_200), "2026-12-31");
        assert_eq!(date_text(951_782_400), "2000-02-29");
        assert_eq!(
            host_of("https://platform.openai.com/api-keys"),
            "platform.openai.com"
        );
        assert_eq!(host_of("ollama.com"), "ollama.com");
    }

    fn sample_views() -> Vec<ProviderView> {
        crate::workspace_host::providers::ProviderHost::sample()
            .snapshot
            .unwrap()
            .views
    }

    fn found(views: &[ProviderView], query: &str) -> Vec<String> {
        search(views, query)
            .into_iter()
            .map(|view| view.id.clone())
            .collect()
    }

    #[test]
    fn the_vendor_search_matches_unconfigured_names_and_ids_ignoring_case() {
        let views = sample_views();
        assert_eq!(
            found(&views, " MI "),
            ["google", "moonshot", "mistral", "minimax"]
        );
        assert_eq!(found(&views, "moonshot"), ["moonshot"]);
        // OpenAI, DeepSeek and magpie are connected in the sample
        assert_eq!(found(&views, "open"), ["openrouter"]);
        assert!(found(&views, "deep").is_empty());
        assert!(found(&views, "no such vendor").is_empty());
        let unconfigured = views
            .iter()
            .filter(|view| !view.configured && view.vendor.is_some())
            .count();
        assert_eq!(found(&views, "").len(), unconfigured);
        assert_eq!(unconfigured, llm::vendors::catalog::catalog().len() - 3);
    }

    #[test]
    fn a_model_is_tagged_with_its_api_only_where_several_are_served() {
        use settings::LlmProtocol::{AnthropicMessages, OpenAiResponses};
        let views = sample_views();
        let view = |id: &str| views.iter().find(|view| view.id == id).unwrap();
        assert_eq!(
            model_tag(view("magpie"), Some(AnthropicMessages)),
            Some("Messages")
        );
        assert_eq!(model_tag(view("magpie"), None), None);
        // the ChatGPT sign-in serves Responses alone
        assert_eq!(model_tag(view("openai"), Some(OpenAiResponses)), None);
        let choice = ModelChoice {
            provider: "magpie".into(),
            model: "gpt-5.1".into(),
            protocol: Some(OpenAiResponses),
        };
        assert_eq!(
            model_label(view("magpie"), &choice),
            "magpie · gpt-5.1 · Responses"
        );
    }

    #[test]
    fn chips_toggle_only_in_picked_mode() {
        let live = ids(&["gpt-5", "gpt-5.1", "o3"]);
        let selection = |mode| ModelSelection {
            mode,
            picked: ids(&["o3"]),
            added: ids(&["mine"]),
        };
        assert_eq!(
            chips(&selection(ModelListMode::Picked), &live),
            vec![
                ("gpt-5".to_owned(), Chip::Off),
                ("gpt-5.1".to_owned(), Chip::Off),
                ("o3".to_owned(), Chip::On),
                ("mine".to_owned(), Chip::On),
            ]
        );
        assert_eq!(
            chips(&selection(ModelListMode::Vendor), &live),
            ["gpt-5", "gpt-5.1", "o3", "mine"]
                .map(|model| (model.to_owned(), Chip::Shown))
                .to_vec()
        );
        assert_eq!(
            chips(&selection(ModelListMode::LatestPerFamily), &live),
            ["gpt-5.1", "o3", "mine"]
                .map(|model| (model.to_owned(), Chip::Shown))
                .to_vec()
        );
    }
}
