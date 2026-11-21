//! The provider detail sheet: account, key, models, endpoints and limits,
//! edited as a draft until it is saved.

use application::{CredentialSummary, ProviderView, model_choice};
use llm::LlmProtocol;
use llm::vendors::catalog::{self, SignIn, Vendor, VendorKind};
use rune::{El, Icon, ScrollSpec, TextEdit, TextStyle, col, el, icon, row, text};
use rune_kit::{
    Badge, Button, ButtonVariant, Collapsible, ControlSize, Dialog, IconButton, IconButtonVariant,
    Kit, ScrollBar, ScrollBarStyle, SegmentedControl, Spinner, Switch, TextField, TextInput,
};
use settings::{AssistantAuthMode, ModelListMode};

use super::{
    Chip, ago_text, chip, chips, date_text, host_of, logo_tile, mode_name, model_tag, name,
    plan_label, protocol_name, small, window_row,
};
use crate::controls::select;
use crate::workspace_host::providers::{Live, Sheet, Work, Write, slot};
use crate::{Confirm, MeetNote};

pub(crate) static REFRESH: Icon = Icon {
    name: "meetnote-refresh",
    view: 16.0,
    body: r#"<path d="M13 8a5 5 0 1 1-1.5-3.55M13 2.5V5h-2.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

const MODES: [ModelListMode; 3] = [
    ModelListMode::Vendor,
    ModelListMode::Picked,
    ModelListMode::LatestPerFamily,
];
const LABEL_WIDTH: f32 = 76.0;
/// The API names' column on the endpoint rows.
const API_WIDTH: f32 = 132.0;

fn draft(s: &mut MeetNote) -> Option<&mut Sheet> {
    s.workspace_host.providers.sheet.as_mut()
}

pub(super) fn build(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>, sheet: &Sheet) {
    let host = &s.workspace_host.providers;
    let view = host
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.view(&sheet.id));
    let vendor = catalog::vendor(&sheet.id);
    let configured = view.is_some_and(|view| view.configured);
    // what the draft lacks shows at its field; a bad number at the bottom
    let error = sheet.error.as_deref().or(sheet
        .invalid()
        .filter(|key| *key == "provider_invalid_number")
        .map(|key| s.tr(key)));
    let title = match view {
        Some(view) if vendor.is_some() => view.name.clone(),
        _ if sheet.name.text.trim().is_empty() => s.tr("provider_custom").to_owned(),
        _ => sheet.name.text.trim().to_owned(),
    };
    let mut body = col()
        .w_full()
        .gap(18.0)
        .px(16.0)
        .pb(12.0)
        .child(identity(s, k, sheet, vendor, &title));
    // a custom endpoint is its name and addresses: they come first
    if vendor.is_none() {
        body = body.child(form_row(
            k,
            s.tr("provider_endpoints"),
            custom(s, k, overlays, sheet),
        ));
    }
    if let Some(sign_in) = vendor.and_then(|vendor| vendor.sign_in) {
        body = body.child(form_row(
            k,
            s.tr("provider_account"),
            account(s, k, sheet, view, sign_in),
        ));
    }
    if vendor.is_none_or(|vendor| vendor.kind != VendorKind::Local) {
        body = body.child(form_row(
            k,
            s.tr(if vendor.is_some_and(|vendor| vendor.sign_in.is_some()) {
                "provider_api_key_or"
            } else {
                "provider_api_key"
            }),
            api_key(s, k, sheet, view, vendor),
        ));
    }
    body = body
        .child(form_row(
            k,
            s.tr("provider_models"),
            models(s, k, sheet, view, configured),
        ))
        .children(vendor.map(|vendor| {
            form_row(
                k,
                s.tr("provider_endpoints"),
                endpoints(s, k, sheet, view, vendor),
            )
        }))
        .child(advanced(s, k, sheet, vendor))
        .children(error.map(|error| {
            text(error.to_owned())
                .key("provider-sheet-error")
                .font(TextStyle::new(12.0, 400.0))
                .color(k.c.danger.fg)
                .wrap()
                .w_full()
        }));
    let scrolled = col()
        .key("provider-sheet-scroll")
        .w_full()
        .max_h(420.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .clip_fade(0.0, 0.0, 0.0, 16.0)
        .child(body)
        .scroll_bar_styled(ScrollBarStyle::from_kit(k));
    let id = sheet.id.clone();
    let footer = row()
        .w_full()
        .gap(8.0)
        .px(16.0)
        .pt(12.0)
        .pb(16.0)
        .border_sides([0.0, 1.0, 0.0, 0.0], k.c.border_muted)
        .children(configured.then(|| {
            Button::new(s.tr("provider_remove"))
                .variant(ButtonVariant::Danger)
                .on_press(move |s: &mut MeetNote| {
                    s.confirm = Some(Confirm::RemoveProvider(id.clone()))
                })
                .build(k)
                .key("provider-remove")
        }))
        .child(el().grow(1.0))
        .child(
            Button::new(s.tr("cancel"))
                .variant(ButtonVariant::Neutral)
                .on_press(MeetNote::close_provider_sheet)
                .build(k)
                .key("provider-cancel"),
        )
        .child(
            Button::new(s.tr("provider_save"))
                .variant(ButtonVariant::Submit)
                .disabled(host.writing())
                .on_press_tx(MeetNote::save_provider_sheet)
                .build(k)
                .key("provider-save")
                .default_button(),
        );
    Dialog::new("provider-sheet", title)
        .open(true)
        .fit()
        .width(620.0)
        .on_close(MeetNote::close_provider_sheet)
        .child(scrolled)
        .child(footer)
        .build(k, overlays);
}

/// A settings-form row: a fixed label column and its controls.
fn form_row(k: &Kit, label: &str, content: El) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(
            text(label.to_owned())
                .font(TextStyle::new(13.0, 500.0))
                .color(k.c.text_muted)
                .wrap()
                .w(LABEL_WIDTH)
                .min_w(LABEL_WIDTH)
                .pt(5.0),
        )
        .child(content.grow(1.0).min_w(0.0))
}

fn identity(s: &MeetNote, k: &Kit, sheet: &Sheet, vendor: Option<&Vendor>, title: &str) -> El {
    let kind = match vendor.map(|vendor| vendor.kind) {
        Some(VendorKind::Vendor) => s.tr("provider_kind_vendor"),
        Some(VendorKind::Relay) => s.tr("provider_kind_relay"),
        Some(VendorKind::Local) => s.tr("provider_kind_local"),
        None => s.tr("provider_custom"),
    };
    let mut about = col().grow(1.0).min_w(0.0).gap(3.0);
    if let Some(vendor) = vendor {
        let website = vendor.website;
        about = about.child(
            text(format!("{} ↗", host_of(website)))
                .key("provider-website")
                .role("link")
                .label(host_of(website).to_owned())
                .focusable(true)
                .focus_ring(2.0, 1.0, k.c.border_focus)
                .font(TextStyle::new(13.0, 500.0))
                .color(k.c.text_accent)
                .on_press_tx(move |_: &mut MeetNote, tx| {
                    let _ = tx.open_url(website);
                }),
        );
    }
    row()
        .w_full()
        .gap(12.0)
        .items_center()
        .child(logo_tile(k, &sheet.id, title, 40.0))
        .child(about.child(small(kind, k.c.text_muted)))
        .child(
            Switch::new(sheet.draft.enabled)
                .label(s.tr("provider_enabled"))
                .on_toggle(|s: &mut MeetNote| {
                    if let Some(sheet) = draft(s) {
                        sheet.draft.enabled = !sheet.draft.enabled;
                    }
                })
                .build(k)
                .key("provider-sheet-enabled"),
        )
}

fn account(
    s: &MeetNote,
    k: &Kit,
    sheet: &Sheet,
    view: Option<&ProviderView>,
    sign_in: SignIn,
) -> El {
    let host = &s.workspace_host.providers;
    let id = sheet.id.clone();
    if let Some(CredentialSummary::Account {
        email,
        plan,
        subscription_until,
    }) = view.map(|view| &view.credential)
    {
        let live = host.live.get(&id);
        let quota = live.and_then(|live| live.quota.as_ref());
        let refresh = id.clone();
        let mut signed_in = col()
            .key("provider-account")
            .w_full()
            .gap(10.0)
            .padding(12.0)
            .radius(10.0)
            .border(1.0, k.c.border_muted)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(icon(k.glyph.check, 14.0).color(k.c.success.fg))
                    .child(name(k, email, 13.0).shrink().min_w(0.0))
                    .children(
                        plan.as_deref()
                            .map(|plan| Badge::new(plan_label(plan)).build(k)),
                    )
                    .child(el().grow(1.0))
                    .child(refresh_button(
                        s,
                        k,
                        "provider-refresh-quota",
                        s.tr("provider_refresh_quota"),
                        host.working(&id, Work::Quota),
                        move |s, tx| s.provider_work(&refresh, Work::Quota, tx),
                    )),
            );
        let now = super::unix_now();
        if let Some(quota) = quota {
            signed_in = signed_in.children(
                quota
                    .windows
                    .iter()
                    .map(|window| window_row(s, k, window, now)),
            );
        }
        let cards = quota
            .and_then(|quota| quota.reset_cards)
            .filter(|cards| *cards > 0);
        let until = quota
            .and_then(|quota| quota.subscription_until)
            .or(*subscription_until);
        if cards.is_some() || until.is_some() {
            signed_in = signed_in.child(
                row()
                    .gap(8.0)
                    .items_center()
                    .children(cards.map(|cards| {
                        chip(
                            k,
                            ui_text::message(
                                s.locale(),
                                "provider_reset_cards",
                                &cards.to_string(),
                            ),
                        )
                    }))
                    .children(until.map(|until| {
                        small(
                            ui_text::message(
                                s.locale(),
                                "provider_subscription_until",
                                &date_text(until),
                            ),
                            k.c.text_muted,
                        )
                    })),
            );
        }
        if let Some(error) = live.and_then(|live| live.quota_error.as_deref()) {
            signed_in = signed_in.child(small(error.to_owned(), k.c.danger.fg).wrap());
        }
        let sign_out = id.clone();
        return signed_in.child(
            row().child(
                Button::new(s.tr("provider_sign_out"))
                    .variant(ButtonVariant::Outline)
                    .on_press_tx(move |s: &mut MeetNote, tx| {
                        s.provider_write(Write::SignOut(sign_out.clone()), tx)
                    })
                    .build(k)
                    .key("provider-sign-out"),
            ),
        );
    }
    if host
        .sign_in
        .as_ref()
        .is_some_and(|sign_in| sign_in.id == id)
    {
        let ready = host
            .sign_in
            .as_ref()
            .is_some_and(|sign_in| sign_in.url.is_some());
        return row()
            .key("provider-signing-in")
            .w_full()
            .gap(8.0)
            .items_center()
            .flex_wrap()
            .child(Spinner::new().build(k))
            .child(small(s.tr("provider_signing_in"), k.c.text_base).grow(1.0))
            .child(
                Button::new(s.tr("provider_copy_link"))
                    .variant(ButtonVariant::Outline)
                    .disabled(!ready)
                    .on_press_tx(MeetNote::copy_sign_in_link)
                    .build(k)
                    .key("provider-copy-link"),
            )
            .child(
                Button::new(s.tr("cancel"))
                    .variant(ButtonVariant::Neutral)
                    .on_press(MeetNote::cancel_sign_in)
                    .build(k)
                    .key("provider-cancel-sign-in"),
            );
    }
    let start = id.clone();
    row()
        .w_full()
        .gap(8.0)
        .flex_wrap()
        .child(
            Button::new(s.tr(match sign_in {
                SignIn::ChatGpt => "provider_sign_in_chatgpt",
                SignIn::OpenRouter => "provider_sign_in_openrouter",
            }))
            .variant(ButtonVariant::Submit)
            .on_press_tx(move |s: &mut MeetNote, tx| s.start_sign_in(&start, tx))
            .build(k)
            .key("provider-sign-in"),
        )
        .children((sign_in == SignIn::ChatGpt).then(|| {
            Button::new(s.tr("provider_import_codex"))
                .variant(ButtonVariant::Outline)
                .on_press_tx(|s: &mut MeetNote, tx| s.provider_write(Write::ImportCodex, tx))
                .build(k)
                .key("provider-import-codex")
        }))
}

fn refresh_button(
    s: &MeetNote,
    k: &Kit,
    key: &str,
    label: &str,
    busy: bool,
    press: impl Fn(&mut MeetNote, &mut rune::Tx) + 'static,
) -> El {
    if busy {
        return Spinner::new().size(14.0).build(k).key(key.to_owned());
    }
    let _ = s;
    IconButton::new(&REFRESH)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Regular)
        .label(label)
        .help(label)
        .on_press_tx(press)
        .build(k)
        .key(key.to_owned())
}

fn api_key(
    s: &MeetNote,
    k: &Kit,
    sheet: &Sheet,
    view: Option<&ProviderView>,
    vendor: Option<&Vendor>,
) -> El {
    let host = &s.workspace_host.providers;
    let saved = match view.map(|view| &view.credential) {
        Some(CredentialSummary::ApiKey { last4 }) => Some(last4.as_str()),
        _ => None,
    };
    let placeholder = saved.map_or_else(
        || s.tr("provider_key_placeholder").to_owned(),
        |last4| ui_text::message(s.locale(), "provider_key_saved", last4),
    );
    let mut details = row().w_full().gap(8.0).items_center();
    if let Some(url) = vendor.and_then(|vendor| vendor.keys_url) {
        details = details.child(
            text(s.tr("provider_get_key"))
                .key("provider-get-key")
                .role("link")
                .focusable(true)
                .focus_ring(2.0, 1.0, k.c.border_focus)
                .font(TextStyle::new(12.0, 500.0))
                .color(k.c.text_accent)
                .on_press_tx(move |_: &mut MeetNote, tx| {
                    let _ = tx.open_url(url);
                }),
        );
    }
    details = details.child(el().grow(1.0));
    if vendor.is_some_and(|vendor| vendor.balance.is_some()) && saved.is_some() {
        let id = sheet.id.clone();
        let live = host.live.get(&id);
        let balance = live
            .and_then(|live| live.quota.as_ref())
            .and_then(|quota| quota.balance.as_deref());
        if let Some(balance) = balance {
            details = details.child(small(
                ui_text::message(s.locale(), "provider_balance", balance),
                k.c.text_base,
            ));
        } else if let Some(error) = live.and_then(|live| live.quota_error.as_deref()) {
            details = details.child(small(error.to_owned(), k.c.danger.fg).shrink().min_w(0.0));
        }
        details = details.child(refresh_button(
            s,
            k,
            "provider-refresh-balance",
            s.tr("provider_refresh_balance"),
            host.working(&id, Work::Quota),
            move |s, tx| s.provider_work(&id, Work::Quota, tx),
        ));
    }
    col()
        .w_full()
        .gap(6.0)
        .child(
            TextInput::new(&sheet.key)
                .secure()
                .width(None)
                .label(s.tr("provider_api_key"))
                .placeholder(placeholder)
                .on_edit(|s: &mut MeetNote| &mut draft(s).expect("the sheet is open").key)
                .on_submit(MeetNote::save_provider_sheet)
                .build(k)
                .key("provider-key"),
        )
        .child(details)
}

fn models(
    s: &MeetNote,
    k: &Kit,
    sheet: &Sheet,
    view: Option<&ProviderView>,
    configured: bool,
) -> El {
    let host = &s.workspace_host.providers;
    let locale = s.locale();
    let id = sheet.id.clone();
    let live = host.live.get(&id);
    let listed = live.and_then(Live::ids).unwrap_or_default();
    let served = live
        .and_then(|live| live.models.as_deref())
        .unwrap_or_default();
    let selection = &sheet.draft.models;
    let labels = MODES.map(|mode| mode_name(locale, mode));
    let mut wrap = row().key("provider-models").w_full().gap(6.0).flex_wrap();
    let chips = chips(selection, &listed);
    if chips.is_empty() {
        wrap = wrap.child(small(s.tr("provider_no_models"), k.c.text_muted));
    }
    for (model, state) in chips {
        let tag =
            view.and_then(|view| model_tag(view, model_choice(view, served, &model).protocol));
        wrap = wrap.child(model_chip(k, model, state, tag));
    }
    let fetching = host.working(&id, Work::Models);
    let testing = host.working(&id, Work::Test);
    let (status, failed) = if !configured {
        (s.tr("provider_save_first").to_owned(), false)
    } else if fetching {
        (s.tr("provider_fetching").to_owned(), false)
    } else if testing {
        (
            ui_text::message(
                locale,
                "provider_testing",
                &host
                    .test_model(&id)
                    .map(|choice| choice.model)
                    .unwrap_or_default(),
            ),
            false,
        )
    } else if let Some((model, result)) = live.and_then(|live| live.test.as_ref()) {
        match result {
            Ok(()) => (
                ui_text::message(locale, "provider_test_passed", model),
                false,
            ),
            Err(error) => (
                ui_text::message(locale, "provider_test_failed", error),
                true,
            ),
        }
    } else if let Some(error) = live.and_then(|live| live.models_error.as_deref()) {
        (error.to_owned(), true)
    } else if let Some(live) = live.filter(|live| live.models.is_some()) {
        (
            format!(
                "{} · {}",
                mode_name(locale, selection.mode),
                ago_text(locale, (s.now - live.fetched_at).max(0.0))
            ),
            false,
        )
    } else {
        (mode_name(locale, selection.mode).to_owned(), false)
    };
    let (refetch, test) = (id.clone(), id);
    col()
        .w_full()
        .gap(10.0)
        .child(
            SegmentedControl::new(
                &labels,
                MODES.iter().position(|mode| *mode == selection.mode),
            )
            .label(s.tr("provider_models"))
            .fit()
            .on_pick(|s: &mut MeetNote, index: usize, _: &mut rune::Tx| {
                if let Some(sheet) = draft(s) {
                    sheet.set_mode(MODES[index]);
                }
            })
            .build(k)
            .key("provider-mode"),
        )
        .child(wrap)
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .flex_wrap()
                .child(
                    TextInput::new(&sheet.new_model)
                        .width(Some(180.0))
                        .label(s.tr("provider_add_model"))
                        .placeholder(s.tr("provider_add_model"))
                        .on_edit(|s: &mut MeetNote| {
                            &mut draft(s).expect("the sheet is open").new_model
                        })
                        .on_submit(|s: &mut MeetNote, _| {
                            if let Some(sheet) = draft(s) {
                                sheet.add_model();
                            }
                        })
                        .build(k)
                        .key("provider-new-model"),
                )
                .child(
                    Button::new(s.tr("provider_add"))
                        .variant(ButtonVariant::Outline)
                        .disabled(sheet.new_model.text.trim().is_empty())
                        .on_press(|s: &mut MeetNote| {
                            if let Some(sheet) = draft(s) {
                                sheet.add_model();
                            }
                        })
                        .build(k)
                        .key("provider-add-model"),
                )
                .child(el().grow(1.0))
                .child(
                    Button::new(s.tr("provider_refetch"))
                        .variant(ButtonVariant::Ghost)
                        .disabled(!configured || fetching)
                        .on_press_tx(move |s: &mut MeetNote, tx| {
                            s.provider_work(&refetch, Work::Models, tx)
                        })
                        .build(k)
                        .key("provider-refetch"),
                )
                .child(
                    Button::new(s.tr("provider_test"))
                        .variant(ButtonVariant::Ghost)
                        .disabled(!configured || testing)
                        .on_press_tx(move |s: &mut MeetNote, tx| {
                            s.provider_work(&test, Work::Test, tx)
                        })
                        .build(k)
                        .key("provider-test"),
                ),
        )
        .child(
            small(
                status,
                if failed {
                    k.c.danger.fg
                } else {
                    k.c.text_muted
                },
            )
            .wrap()
            .key("provider-model-status"),
        )
}

/// A model as a chip, with the tag of the API it runs on when its
/// provider serves several.
fn model_chip(k: &Kit, model: String, state: Chip, tag: Option<&str>) -> El {
    // A key path splits at '/', which aggregator ids contain.
    let key = format!("provider-model-{}", model.replace('/', ":"));
    let chip = row()
        .key(key)
        .h(24.0)
        .px(10.0)
        .radius(12.0)
        .center()
        .label(match tag {
            Some(tag) => format!("{model}, {tag}"),
            None => model.clone(),
        });
    let ink = |color: rune::Color| {
        row()
            .gap(6.0)
            .items_center()
            .a11y_hidden()
            .child(
                text(model.clone())
                    .font(TextStyle::new(12.0, 500.0))
                    .color(color),
            )
            .children(tag.map(|tag| {
                text(tag.to_owned())
                    .key("tag")
                    .font(TextStyle::new(10.5, 500.0))
                    .color(color.alpha(0.62))
            }))
    };
    match state {
        Chip::Shown => chip
            .fill(k.c.layer_02)
            .border(1.0, k.c.border_muted)
            .child(ink(k.c.text_base)),
        Chip::On | Chip::Off => {
            let on = state == Chip::On;
            let chip = if on {
                chip.fill(k.c.text_accent).child(ink(k.c.page))
            } else {
                chip.border(1.0, k.c.border_base)
                    .border_dash(3.0, 2.0)
                    .child(ink(k.c.text_muted))
            };
            chip.role("checkbox")
                .checked(on)
                .focusable(true)
                .focus_ring(2.0, 1.0, k.c.border_focus)
                .hover(k.c.hover)
                .on_press(move |s: &mut MeetNote| {
                    if let Some(sheet) = draft(s) {
                        sheet.toggle_model(&model);
                    }
                })
        }
    }
}

/// A vendor's region choice above one row per API it is asked at: the
/// region's address, or the typed override; a ChatGPT sign-in's backend.
fn endpoints(
    s: &MeetNote,
    k: &Kit,
    sheet: &Sheet,
    view: Option<&ProviderView>,
    vendor: &'static Vendor,
) -> El {
    let signed_in =
        view.filter(|view| matches!(view.credential, CredentialSummary::Account { .. }));
    let endpoints = signed_in.map_or_else(|| sheet.endpoints(), |view| view.endpoints.clone());
    let mut content = col().key("provider-endpoints").w_full().gap(6.0);
    if vendor.regions.len() > 1 {
        let labels = vendor
            .regions
            .iter()
            .map(|region| match region.label {
                "Global" => s.tr("provider_region_global"),
                "China" => s.tr("provider_region_china"),
                other => other,
            })
            .collect::<Vec<_>>();
        let current = sheet.region().and_then(|region| {
            vendor
                .regions
                .iter()
                .position(|candidate| candidate.label == region.label)
        });
        let regions = vendor.regions;
        content = content.child(
            SegmentedControl::new(&labels, current)
                .label(s.tr("provider_region"))
                .fit()
                .on_pick(move |s: &mut MeetNote, index: usize, _: &mut rune::Tx| {
                    if let Some(sheet) = draft(s) {
                        sheet.draft.region = (index != 0).then(|| regions[index].label.to_owned());
                    }
                })
                .build(k)
                .key("provider-region")
                .mb(4.0),
        );
    }
    content.children(endpoints.iter().map(|(protocol, url)| {
        row()
            .w_full()
            .h(22.0)
            .gap(12.0)
            .items_center()
            .child(
                small(protocol_name(protocol), k.c.text_base)
                    .w(API_WIDTH)
                    .min_w(API_WIDTH),
            )
            .child(
                text(url.to_owned())
                    .key(format!("provider-endpoint-{}", api_slug(protocol)))
                    .font(TextStyle {
                        size: 12.0,
                        ..k.ty.mono
                    })
                    .color(k.c.text_muted)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
    }))
}

/// A custom endpoint's name, key header and the address of each API it
/// serves, at least one.
fn custom(s: &MeetNote, k: &Kit, overlays: &mut Vec<El>, sheet: &Sheet) -> El {
    let auth = sheet
        .draft
        .custom
        .as_ref()
        .map_or(AssistantAuthMode::Bearer, |custom| custom.auth_mode);
    let mut name = TextField::new(s.tr("provider_name"), &sheet.name)
        .on_edit(|s: &mut MeetNote| &mut draft(s).expect("the sheet is open").name);
    if sheet.invalid() == Some("provider_name_required") {
        name = name.error(s.tr("provider_name_required"));
    }
    col()
        .w_full()
        .gap(10.0)
        .child(name.build(k).key("provider-name"))
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_center()
                .child(small(s.tr("provider_auth_header"), k.c.text_muted).grow(1.0))
                .child(select(
                    s,
                    k,
                    overlays,
                    ("provider-auth", s.tr("provider_auth_header")),
                    &["Authorization: Bearer", "x-api-key"],
                    usize::from(auth == AssistantAuthMode::XApiKey),
                    |s, index| {
                        if let Some(custom) = draft(s).and_then(|sheet| sheet.draft.custom.as_mut())
                        {
                            custom.auth_mode = if index == 0 {
                                AssistantAuthMode::Bearer
                            } else {
                                AssistantAuthMode::XApiKey
                            };
                        }
                    },
                )),
        )
        .children(
            sheet
                .protocols()
                .into_iter()
                .map(|protocol| url_field(k, sheet, protocol, "https://")),
        )
        .children((sheet.invalid() == Some("provider_url_required")).then(|| {
            small(s.tr("provider_url_required"), k.c.danger.fg)
                .key("provider-urls-error")
                .live_region(rune::LiveRegion::Assertive)
        }))
}

/// The field for `protocol`'s address: a vendor's override, empty for the
/// region's own shown as its placeholder.
fn url_field(k: &Kit, sheet: &Sheet, protocol: LlmProtocol, placeholder: &str) -> El {
    TextField::new(protocol_name(protocol), &sheet.urls[slot(protocol)])
        .placeholder(placeholder)
        .on_edit(move |s: &mut MeetNote| draft(s).expect("the sheet is open").url_mut(protocol))
        .build(k)
        .key(format!("provider-url-{}", api_slug(protocol)))
}

fn advanced(s: &MeetNote, k: &Kit, sheet: &Sheet, vendor: Option<&Vendor>) -> El {
    let mut fields = col().w_full().gap(10.0).pt(4.0);
    if let (Some(_), Some(region)) = (vendor, sheet.region()) {
        fields = fields.children(
            region
                .endpoints
                .iter()
                .map(|(protocol, url)| url_field(k, sheet, protocol, url)),
        );
    }
    let fields = fields.child(
        row()
            .w_full()
            .gap(10.0)
            .child(
                text_field(
                    k,
                    ("provider-context", s.tr("provider_context_window")),
                    &sheet.context,
                    s.tr("provider_default"),
                    |sheet| &mut sheet.context,
                )
                .w_frac(1.0 / 3.0),
            )
            .child(
                text_field(
                    k,
                    ("provider-timeout", s.tr("provider_timeout")),
                    &sheet.timeout,
                    s.tr("provider_default"),
                    |sheet| &mut sheet.timeout,
                )
                .w_frac(1.0 / 3.0),
            )
            .child(
                text_field(
                    k,
                    ("provider-max-output", s.tr("provider_max_output")),
                    &sheet.max_output,
                    s.tr("provider_default"),
                    |sheet| &mut sheet.max_output,
                )
                .w_frac(1.0 / 3.0),
            ),
    );
    Collapsible::new(s.tr("provider_advanced"), sheet.advanced)
        .on_toggle(|s: &mut MeetNote| {
            if let Some(sheet) = draft(s) {
                sheet.advanced = !sheet.advanced;
            }
        })
        .child(fields)
        .build(k)
        .key("provider-advanced")
}

/// A labelled single-line field on the sheet's draft.
fn text_field(
    k: &Kit,
    (key, label): (&'static str, &str),
    value: &TextEdit,
    placeholder: &str,
    edit: fn(&mut Sheet) -> &mut TextEdit,
) -> El {
    TextField::new(label, value)
        .placeholder(placeholder)
        .on_edit(move |s: &mut MeetNote| edit(draft(s).expect("the sheet is open")))
        .build(k)
        .key(key)
}

/// An API's name in element keys.
fn api_slug(protocol: LlmProtocol) -> &'static str {
    match protocol {
        LlmProtocol::OpenAiChatCompletions => "chat",
        LlmProtocol::OpenAiResponses => "responses",
        LlmProtocol::AnthropicMessages => "messages",
    }
}
