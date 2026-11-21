//! A provider's model list: `GET {base}/models` at each distinct base its
//! APIs are served at, merged in the vendor's order.

use crate::codex;
use crate::http::{auth_headers, endpoint_url, Http, VENDOR_TIMEOUT};
use crate::vendors::endpoints::{default_auth, Endpoints, PROTOCOLS};
use crate::vendors::oauth::chatgpt::{ChatGptEndpoints, ChatGptTokens};
use crate::{AuthMode, Credential, LlmCancellation, LlmError, LlmErrorKind, LlmProtocol};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Anthropic pages its list; a runaway `has_more` stops here.
const MAX_PAGES: usize = 20;

/// One model of a provider's merged list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedModel {
    pub id: String,
    /// The APIs that serve it, in [`PROTOCOLS`] order: the ones whose list
    /// named it and the ones whose list could not be read, or what magpie's
    /// `native_endpoints` says.
    pub protocols: Vec<LlmProtocol>,
}

/// The models every distinct base of `endpoints` lists for `credential`,
/// merged: the first readable list's order, then the others' new ids. The
/// key goes in `auth`, or each API's own header when `None`. A list that
/// cannot be read is skipped unless none can.
pub fn list_models(
    endpoints: &Endpoints,
    auth: Option<AuthMode>,
    credential: Option<&Credential>,
    cancellation: &LlmCancellation,
) -> Result<Vec<ListedModel>, LlmError> {
    let mut lists = Vec::new();
    for (base, protocols) in list_bases(endpoints) {
        let protocol = protocols[0];
        let entries = Http::for_url(base, VENDOR_TIMEOUT).and_then(|http| {
            let headers =
                auth_headers(protocol, auth.unwrap_or(default_auth(protocol)), credential)?;
            list_entries(&http, base, protocol, headers, cancellation)
        });
        if matches!(&entries, Err(error) if error.kind == LlmErrorKind::Cancelled) {
            return entries.map(|_| Vec::new());
        }
        lists.push((protocols, entries));
    }
    merge(lists)
}

/// Each distinct base URL with the APIs served at it, in [`PROTOCOLS`]
/// order; a list is asked with its first API's headers.
fn list_bases(endpoints: &Endpoints) -> Vec<(&str, Vec<LlmProtocol>)> {
    let mut bases: Vec<(&str, Vec<LlmProtocol>)> = Vec::new();
    for (protocol, base) in endpoints.iter() {
        let trimmed = base.trim_end_matches('/');
        match bases
            .iter_mut()
            .find(|(known, _)| known.trim_end_matches('/') == trimmed)
        {
            Some((_, protocols)) => protocols.push(protocol),
            None => bases.push((base, vec![protocol])),
        }
    }
    bases
}

/// A listed id, with the APIs magpie's gateway says serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    id: String,
    native: Option<Vec<LlmProtocol>>,
}

/// The lists, each with the APIs served at its base, as one list.
fn merge(
    lists: Vec<(Vec<LlmProtocol>, Result<Vec<Entry>, LlmError>)>,
) -> Result<Vec<ListedModel>, LlmError> {
    let unread = lists
        .iter()
        .filter(|(_, entries)| entries.is_err())
        .flat_map(|(protocols, _)| protocols.iter().copied())
        .collect::<Vec<_>>();
    let mut merged: Vec<(String, Vec<LlmProtocol>, Option<Vec<LlmProtocol>>)> = Vec::new();
    let mut index = HashMap::new();
    let mut first_error = None;
    let mut read = false;
    for (protocols, entries) in lists {
        let entries = match entries {
            Ok(entries) => entries,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            }
        };
        read = true;
        for entry in entries {
            let at = *index.entry(entry.id.clone()).or_insert_with(|| {
                merged.push((entry.id, Vec::new(), None));
                merged.len() - 1
            });
            let (_, served, native) = &mut merged[at];
            served.extend_from_slice(&protocols);
            if entry.native.is_some() {
                *native = entry.native;
            }
        }
    }
    if !read {
        return Err(first_error.unwrap_or_else(|| {
            LlmError::new(
                LlmErrorKind::ConnectionNotConfigured,
                "the provider serves no API to list models at",
            )
        }));
    }
    Ok(merged
        .into_iter()
        .map(|(id, mut served, native)| {
            let protocols = match native {
                Some(native) => native,
                None => {
                    served.extend_from_slice(&unread);
                    PROTOCOLS
                        .into_iter()
                        .filter(|protocol| served.contains(protocol))
                        .collect()
                }
            };
            ListedModel { id, protocols }
        })
        .collect())
}

/// The Codex models a ChatGPT sign-in's plan offers, in the backend's order.
pub fn list_chatgpt_models(
    endpoints: &ChatGptEndpoints,
    tokens: &ChatGptTokens,
    cancellation: &LlmCancellation,
) -> Result<Vec<String>, LlmError> {
    let base_url = endpoints.codex_base_url();
    let http = Http::for_url(&base_url, VENDOR_TIMEOUT)?;
    Ok(codex::models(
        &http,
        &base_url,
        &tokens.access_token,
        &tokens.account_id,
        cancellation,
    )?
    .into_iter()
    .map(|model| model.slug)
    .collect())
}

/// The ids `base_url` lists, in its order.
pub(crate) fn list_with(
    http: &Http,
    base_url: &str,
    protocol: LlmProtocol,
    headers: Vec<(&'static str, String)>,
    cancellation: &LlmCancellation,
) -> Result<Vec<String>, LlmError> {
    Ok(
        list_entries(http, base_url, protocol, headers, cancellation)?
            .into_iter()
            .map(|entry| entry.id)
            .collect(),
    )
}

/// The entries `base_url` lists, deduplicated, in its order; Anthropic's
/// pages are followed.
fn list_entries(
    http: &Http,
    base_url: &str,
    protocol: LlmProtocol,
    headers: Vec<(&'static str, String)>,
    cancellation: &LlmCancellation,
) -> Result<Vec<Entry>, LlmError> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    let mut after = None::<String>;
    for _ in 0..MAX_PAGES {
        let mut url = endpoint_url(base_url, "/models")?;
        if protocol == LlmProtocol::AnthropicMessages {
            let mut query = url.query_pairs_mut();
            if let Some(after) = &after {
                query.append_pair("after_id", after);
            }
            query.append_pair("limit", "1000");
        }
        let page = http
            .get(url)
            .headers(headers.clone())
            .header("Accept", "application/json")
            .send(cancellation)?
            .json()?;
        for entry in parse_page(&page)? {
            if seen.insert(entry.id.clone()) {
                ids.push(entry);
            }
        }
        after = page
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| {
                page.get("last_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .flatten();
        if after.is_none() {
            break;
        }
    }
    Ok(ids)
}

/// The entries of one page: `data[]`, `models[]` or a bare array of
/// objects (by `id`, else `name`) or strings; Gemini's `models/` prefix is
/// dropped, and magpie's `native_endpoints` read.
fn parse_page(page: &Value) -> Result<Vec<Entry>, LlmError> {
    let entries = page
        .get("data")
        .or_else(|| page.get("models"))
        .unwrap_or(page)
        .as_array()
        .ok_or_else(|| {
            LlmError::new(
                LlmErrorKind::ResponseInvalid,
                "model list response has no data or models array",
            )
        })?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let id = entry
                .as_str()
                .or_else(|| entry.get("id").and_then(Value::as_str))
                .or_else(|| entry.get("name").and_then(Value::as_str))?;
            let id = id.strip_prefix("models/").unwrap_or(id).trim();
            (!id.is_empty()).then(|| Entry {
                id: id.to_owned(),
                native: entry
                    .get("native_endpoints")
                    .and_then(Value::as_array)
                    .map(|paths| native_protocols(paths)),
            })
        })
        .collect())
}

/// The APIs magpie's gateway paths name, in [`PROTOCOLS`] order.
fn native_protocols(paths: &[Value]) -> Vec<LlmProtocol> {
    let named = |suffix: &str| {
        paths
            .iter()
            .filter_map(Value::as_str)
            .any(|path| path.trim_end_matches('/').ends_with(suffix))
    };
    PROTOCOLS
        .into_iter()
        .filter(|protocol| {
            named(match protocol {
                LlmProtocol::OpenAiChatCompletions => "/chat/completions",
                LlmProtocol::OpenAiResponses => "/responses",
                LlmProtocol::AnthropicMessages => "/messages",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pages_are_read_in_every_listed_shape() {
        for page in [
            json!({"object": "list", "data": [{"id": "a"}, {"id": "models/b"}]}),
            json!({"models": [{"name": "models/a"}, {"name": "b"}]}),
            json!([{"id": "a"}, "b"]),
        ] {
            let ids = parse_page(&page)
                .unwrap()
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>();
            assert_eq!(ids, ["a", "b"], "{page}");
        }
        assert_eq!(
            parse_page(&json!({"error": "nope"})).unwrap_err().kind,
            LlmErrorKind::ResponseInvalid
        );
    }

    use LlmProtocol::{
        AnthropicMessages as Messages, OpenAiChatCompletions as Chat, OpenAiResponses as Responses,
    };

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            native: None,
        }
    }

    fn listed(id: &str, protocols: &[LlmProtocol]) -> ListedModel {
        ListedModel {
            id: id.into(),
            protocols: protocols.to_vec(),
        }
    }

    fn missing() -> LlmError {
        LlmError::new(LlmErrorKind::CapabilityUnsupported, "no list here")
    }

    #[test]
    fn magpies_native_endpoints_are_read_per_model() {
        let page = json!({"data": [
            {"id": "claude-x", "native_endpoints": ["/v1/messages", "/v1/chat/completions"]},
            {"id": "gpt-x", "native_endpoints": ["/v1/responses", "/v1/systemone"]},
            {"id": "plain"}
        ]});
        assert_eq!(
            parse_page(&page).unwrap(),
            [
                Entry {
                    id: "claude-x".into(),
                    native: Some(vec![Chat, Messages])
                },
                Entry {
                    id: "gpt-x".into(),
                    native: Some(vec![Responses])
                },
                entry("plain"),
            ]
        );
    }

    #[test]
    fn lists_merge_in_the_first_lists_order_with_the_apis_that_named_each() {
        let merged = merge(vec![
            (vec![Chat, Responses], Ok(vec![entry("b"), entry("a")])),
            (vec![Messages], Ok(vec![entry("a"), entry("c")])),
        ])
        .unwrap();
        assert_eq!(
            merged,
            [
                listed("b", &[Chat, Responses]),
                listed("a", &[Chat, Responses, Messages]),
                listed("c", &[Messages]),
            ]
        );
    }

    #[test]
    fn native_endpoints_win_over_the_lists_that_named_a_model() {
        let merged = merge(vec![
            (
                vec![Chat, Responses, Messages],
                Ok(vec![
                    Entry {
                        id: "claude-x".into(),
                        native: Some(vec![Messages]),
                    },
                    entry("other"),
                ]),
            ),
            (vec![Messages], Err(missing())),
        ])
        .unwrap();
        assert_eq!(
            merged,
            [
                listed("claude-x", &[Messages]),
                listed("other", &[Chat, Responses, Messages]),
            ]
        );
    }

    #[test]
    fn an_unreadable_list_counts_as_serving_every_model_until_all_fail() {
        let merged = merge(vec![
            (vec![Chat], Err(missing())),
            (vec![Messages], Ok(vec![entry("a")])),
        ])
        .unwrap();
        assert_eq!(merged, [listed("a", &[Chat, Messages])]);

        let error = merge(vec![
            (
                vec![Chat],
                Err(LlmError::new(LlmErrorKind::Authentication, "bad key")),
            ),
            (vec![Messages], Err(missing())),
        ])
        .unwrap_err();
        assert_eq!(error.kind, LlmErrorKind::Authentication);
        assert_eq!(
            merge(Vec::new()).unwrap_err().kind,
            LlmErrorKind::ConnectionNotConfigured
        );
    }

    #[test]
    fn identical_bases_are_listed_once() {
        let endpoints = Endpoints {
            chat: Some("https://a/v1".into()),
            responses: Some("https://a/v1/".into()),
            messages: Some("https://a/anthropic/v1".into()),
        };
        assert_eq!(
            list_bases(&endpoints),
            [
                ("https://a/v1", vec![Chat, Responses]),
                ("https://a/anthropic/v1", vec![Messages]),
            ]
        );
    }
}
