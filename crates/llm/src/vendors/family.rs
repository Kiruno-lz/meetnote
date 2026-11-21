//! "Latest per family": a model's family is its id with the version numbers
//! taken out, so `claude-opus-4-8` and `claude-opus-5-5` are `claude-opus`,
//! `o3` and `o1` are `o`, and `o4-mini` is `o-mini`. Nothing about a vendor
//! is compiled in; what is read off the id alone is
//!
//! - a version: a number (`4`, `4.5`, the `4` and `5` of `claude-opus-4-5`),
//!   the digits a word ends in (`qwen3`, `o4`, `k2`), a `v` and its digits
//!   (`v3`, `v1:0`), or the digits a word starts with that are not a size
//!   (`gpt-4o` is `gpt-o`, `glm-4.5v` is `glm-v`);
//! - a size, which stays in the name: `8b`, `70b`, `1.5b`, `a22b`, `8x7b`,
//!   `128e`, `32k`, so `qwen3-32b` and `qwen3-8b` are two families;
//! - a snapshot: a date or revision after the version (`20251101`, `0528`,
//!   `2507`, `2024-08-06`, `05-20`, `001`), read as the alias it was taken of;
//! - a preview (`preview`, `exp`, `beta`): the same model before release;
//! - `latest`, an alias of the newest, which is the alias too.
//!
//! Every other word stays in the name (`-mini`, `-pro`, `-flash-lite`,
//! `-codex`, `:free`, a provider prefix such as `anthropic/`).
//!
//! Ported from magpie internal/catalog/family.go.

use regex::Regex;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::LazyLock;

/// What an id's name says about it: its family and where it stands in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    /// The id without its version, snapshot or preview.
    pub name: String,
    /// Every version number, in the order the id has them.
    pub version: Vec<u32>,
    /// Its dates or revision, empty for an alias.
    pub snapshot: String,
    pub preview: bool,
}

static SEPARATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-_/:@\[\]\s]+").unwrap());
/// A number: 4, 4.5, 2.0.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(?:\.\d+)*$").unwrap());
/// A word that ends in its version: qwen3, qwen2.5, o4, k2, v1, llama3.1.
static TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([a-z][a-z.]*?[a-z])(\d+(?:\.\d+)*)$|^([a-z])(\d+(?:\.\d+)*)$").unwrap()
});
/// A word that starts with its version: 4o, 4.5v, 3n.
static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+(?:\.\d+)*)([a-z]+)$").unwrap());
/// A size, kept in the name: 8b, 1.5b, 70b, 128e, 32k, 8x7b, a22b, e4b.
static SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[a-z]?\d+(?:\.\d+)?|\d+x\d+(?:\.\d+)?)[bmkte]$").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:19|20)\d\d$").unwrap());

const PREVIEW_WORDS: [&str; 5] = ["preview", "exp", "experimental", "beta", "alpha"];

/// Reads `id`'s family off its name.
pub fn family_of(id: &str) -> Family {
    let id = id.trim().to_lowercase();
    let words = SEPARATOR.split(&id).collect::<Vec<_>>();
    let separators = SEPARATOR
        .find_iter(&id)
        .map(|found| found.as_str())
        .collect::<Vec<_>>();

    // which purely numeric words are a snapshot rather than a version
    let mut snapshot = vec![false; words.len()];
    for (index, word) in words.iter().enumerate() {
        if !NUMBER.is_match(word) || word.contains('.') {
            continue;
        }
        // a date or revision: 0528, 2507, 20251101, 2024, 001, 05
        if word.len() >= 4 || word.len() >= 2 && word.starts_with('0') {
            snapshot[index] = true;
        }
    }
    for index in 0..words.len() {
        let word = words[index];
        if snapshot[index] || !NUMBER.is_match(word) || word.len() != 2 {
            continue;
        }
        // the day after a month (05-20), the month before a year (12-2024)
        if index > 0 && snapshot[index - 1]
            || words.get(index + 1).is_some_and(|next| YEAR.is_match(next))
        {
            snapshot[index] = true;
        }
    }

    let mut family = Family {
        name: String::new(),
        version: Vec::new(),
        snapshot: String::new(),
        preview: false,
    };
    let keep = |name: &mut String, index: usize, word: &str| {
        if !name.is_empty() && index > 0 {
            name.push_str(separators[index - 1]);
        }
        name.push_str(word);
    };
    let add_version = |version: &mut Vec<u32>, value: &str| {
        version.extend(value.split('.').map(|part| part.parse().unwrap_or(0)));
    };
    for (index, word) in words.iter().copied().enumerate() {
        if word.is_empty() {
            continue;
        }
        if snapshot[index] {
            if !family.snapshot.is_empty() {
                family.snapshot.push('-');
            }
            family.snapshot.push_str(word);
        } else if PREVIEW_WORDS.contains(&word) {
            family.preview = true;
        } else if word == "latest" {
        } else if NUMBER.is_match(word) {
            add_version(&mut family.version, word);
        } else if SIZE.is_match(word) {
            keep(&mut family.name, index, word);
        } else if let Some(captures) = TAIL.captures(word) {
            let (stem, version) = match (captures.get(1), captures.get(2)) {
                (Some(stem), Some(version)) => (stem.as_str(), version.as_str()),
                _ => (&captures[3], &captures[4]),
            };
            // v3, v1: the version alone
            if stem != "v" {
                keep(&mut family.name, index, stem);
            }
            add_version(&mut family.version, version);
        } else if let Some(captures) = HEAD.captures(word) {
            add_version(&mut family.version, &captures[1]);
            keep(&mut family.name, index, &captures[2]);
        } else {
            keep(&mut family.name, index, word);
        }
    }
    if family.name.is_empty() {
        // all numbers: a family of its own
        family.name = id;
    }
    family
}

/// Whether `a`, of the same family as `b`, is the one to serve over it: the
/// higher version; at the same version a release over a preview, then the
/// alias over a dated snapshot (the alias names the vendor's current one),
/// then the later snapshot.
fn newer_in_family(a: &Family, b: &Family) -> bool {
    match a.version.cmp(&b.version) {
        Ordering::Equal => {}
        order => return order == Ordering::Greater,
    }
    if a.preview != b.preview {
        return b.preview;
    }
    if a.snapshot.is_empty() != b.snapshot.is_empty() {
        return a.snapshot.is_empty();
    }
    a.snapshot > b.snapshot
}

/// `ids` with only the newest of each family left, in their order; a
/// family's first listed wins a tie.
pub fn best_in_family(ids: &[String]) -> Vec<String> {
    let families = ids.iter().map(|id| family_of(id)).collect::<Vec<_>>();
    let mut best = HashMap::<&str, usize>::new();
    for (index, family) in families.iter().enumerate() {
        let current = best.entry(family.name.as_str()).or_insert(index);
        if newer_in_family(family, &families[*current]) {
            *current = index;
        }
    }
    ids.iter()
        .zip(&families)
        .enumerate()
        .filter(|(index, (_, family))| best[family.name.as_str()] == *index)
        .map(|(_, (id, _))| id.clone())
        .collect()
}

/// The newest id of `current`'s family in `ids`, when it is newer than
/// `current`: the model a chosen one is upgraded to.
pub fn newest_in_family<'a>(ids: &'a [String], current: &str) -> Option<&'a str> {
    let current = family_of(current);
    ids.iter()
        .map(|id| (id.as_str(), family_of(id)))
        .filter(|(_, family)| family.name == current.name)
        .fold(None::<(&str, Family)>, |best, candidate| match best {
            Some(best) if !newer_in_family(&candidate.1, &best.1) => Some(best),
            _ => Some(candidate),
        })
        .filter(|(_, family)| newer_in_family(family, &current))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn family_of_reads_name_version_snapshot_and_preview() {
        let cases: &[(&str, &str, &[u32], &str, bool)] = &[
            ("claude-opus-4-8", "claude-opus", &[4, 8], "", false),
            ("claude-opus-5-5", "claude-opus", &[5, 5], "", false),
            ("claude-opus-5", "claude-opus", &[5], "", false),
            // a dated snapshot is its alias
            (
                "claude-opus-4-5-20251101",
                "claude-opus",
                &[4, 5],
                "20251101",
                false,
            ),
            (
                "claude-haiku-4-5-20251001",
                "claude-haiku",
                &[4, 5],
                "20251001",
                false,
            ),
            ("gpt-4o-2024-08-06", "gpt-o", &[4], "2024-08-06", false),
            ("deepseek-r1-0528", "deepseek-r", &[1], "0528", false),
            (
                "qwen3-235b-a22b-instruct-2507",
                "qwen-235b-a22b-instruct",
                &[3],
                "2507",
                false,
            ),
            (
                "gemini-2.5-flash-preview-05-20",
                "gemini-flash",
                &[2, 5],
                "05-20",
                true,
            ),
            (
                "gemini-embedding-001",
                "gemini-embedding",
                &[],
                "001",
                false,
            ),
            ("command-r7b-12-2024", "command-r7b", &[], "12-2024", false),
            // the version mid-name
            (
                "claude-3-5-sonnet-20241022",
                "claude-sonnet",
                &[3, 5],
                "20241022",
                false,
            ),
            (
                "meta-llama/llama-3.3-70b-instruct",
                "meta-llama/llama-70b-instruct",
                &[3, 3],
                "",
                false,
            ),
            // sizes and suffixes stay in the name
            ("gpt-5.4-mini", "gpt-mini", &[5, 4], "", false),
            ("gpt-5.5-pro", "gpt-pro", &[5, 5], "", false),
            ("gpt-4.1-nano", "gpt-nano", &[4, 1], "", false),
            ("gpt-5.3-codex-spark", "gpt-codex-spark", &[5, 3], "", false),
            ("qwen3.6-27b", "qwen-27b", &[3, 6], "", false),
            ("qwen3.6-35b-a3b", "qwen-35b-a3b", &[3, 6], "", false),
            (
                "mistralai/mixtral-8x7b-instruct",
                "mistralai/mixtral-8x7b-instruct",
                &[],
                "",
                false,
            ),
            ("gemma-3n-e4b-it", "gemma-n-e4b-it", &[3], "", false),
            // gemini: flash, flash-lite and pro are three
            ("gemini-3.5-flash", "gemini-flash", &[3, 5], "", false),
            (
                "gemini-3.5-flash-lite",
                "gemini-flash-lite",
                &[3, 5],
                "",
                false,
            ),
            ("gemini-3.1-pro-preview", "gemini-pro", &[3, 1], "", true),
            ("gemini-pro-latest", "gemini-pro", &[], "", false),
            // the o-series
            ("o3", "o", &[3], "", false),
            ("o4-mini", "o-mini", &[4], "", false),
            ("o1-pro", "o-pro", &[1], "", false),
            // a word that ends in its version
            ("kimi-k2.6", "kimi-k", &[2, 6], "", false),
            ("glm-4.6v", "glm-v", &[4, 6], "", false),
            ("deepseek-v4-flash", "deepseek-flash", &[4], "", false),
            // a provider prefix stays
            (
                "anthropic/claude-opus-4.5",
                "anthropic/claude-opus",
                &[4, 5],
                "",
                false,
            ),
            (
                "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
                "us.anthropic.claude-sonnet",
                &[4, 5, 1, 0],
                "20250929",
                false,
            ),
            ("Qwen/Qwen3.8-27B", "qwen/qwen-27b", &[3, 8], "", false),
            (
                "claude-sonnet-4-5[1m]",
                "claude-sonnet[1m",
                &[4, 5],
                "",
                false,
            ),
        ];
        for (id, name, version, snapshot, preview) in cases {
            assert_eq!(
                family_of(id),
                Family {
                    name: (*name).into(),
                    version: version.to_vec(),
                    snapshot: (*snapshot).into(),
                    preview: *preview,
                },
                "{id}"
            );
        }
    }

    #[test]
    fn different_models_never_share_a_family() {
        for (a, b) in [
            ("gpt-5.4", "gpt-5.4-mini"),
            ("gpt-5.4-mini", "gpt-5.4-nano"),
            ("gpt-5.5", "gpt-5.5-pro"),
            ("gpt-4", "gpt-4o"),
            ("o3", "o3-mini"),
            ("o3", "o3-pro"),
            ("gemini-3.5-flash", "gemini-3.1-pro-preview"),
            ("gemini-3.5-flash", "gemini-3.5-flash-lite"),
            ("qwen3-8b", "qwen3-32b"),
            ("llama-3.1-8b-instruct", "llama-3.1-70b-instruct"),
            ("gpt-6.1-sol", "gpt-6.1-terra"),
            ("claude-opus-5-5", "claude-sonnet-5-5"),
            ("kimi-k2-thinking", "kimi-k2"),
            ("openai/gpt-5", "azure/gpt-5"),
            ("deepseek/deepseek-r1:free", "deepseek/deepseek-r1"),
        ] {
            assert_ne!(family_of(a).name, family_of(b).name, "{a} and {b}");
        }
    }

    #[test]
    fn best_in_family_keeps_the_newest_of_each_in_list_order() {
        let cases: &[(&str, &[&str], &[&str])] = &[
            (
                "codex",
                &[
                    "gpt-6.1-sol",
                    "gpt-6-astra",
                    "gpt-6-sol",
                    "gpt-6-luna",
                    "gpt-5.6-sol",
                    "gpt-5.6-terra",
                    "gpt-5.6-luna",
                ],
                &["gpt-6.1-sol", "gpt-6-astra", "gpt-6-luna", "gpt-5.6-terra"],
            ),
            // a newer one listed takes the old one's place by itself
            (
                "opus 5.6",
                &["claude-opus-5-6", "claude-opus-5-5", "claude-sonnet-5-5"],
                &["claude-opus-5-6", "claude-sonnet-5-5"],
            ),
            (
                "openai sizes",
                &[
                    "gpt-5.6",
                    "gpt-5.5",
                    "gpt-5.4-mini",
                    "gpt-5-mini",
                    "gpt-4.1-nano",
                    "gpt-5.4-nano",
                    "gpt-5.5-pro",
                    "gpt-5-pro",
                ],
                &["gpt-5.6", "gpt-5.4-mini", "gpt-5.4-nano", "gpt-5.5-pro"],
            ),
            (
                "o-series",
                &["o1", "o3", "o3-mini", "o4-mini", "o1-pro", "o3-pro"],
                &["o3", "o4-mini", "o3-pro"],
            ),
            (
                "gemini",
                &[
                    "gemini-2.5-flash",
                    "gemini-3.5-flash",
                    "gemini-2.5-pro",
                    "gemini-3.1-pro-preview",
                    "gemini-3.1-flash-lite-preview",
                    "gemini-3.1-flash-lite",
                    "gemini-pro-latest",
                ],
                &[
                    "gemini-3.5-flash",
                    "gemini-3.1-pro-preview",
                    "gemini-3.1-flash-lite",
                ],
            ),
            // at one version, a release over its preview
            (
                "preview",
                &["gemini-3-pro-image-preview", "gemini-3-pro-image"],
                &["gemini-3-pro-image"],
            ),
            // the alias over its dated snapshots, whatever their dates say
            (
                "snapshots",
                &["gpt-4o", "gpt-4o-2024-08-06", "gpt-4o-2024-11-20"],
                &["gpt-4o"],
            ),
            (
                "snapshots alone",
                &["mistral-medium-2505", "mistral-medium-2508"],
                &["mistral-medium-2508"],
            ),
            (
                "qwen sizes",
                &[
                    "Qwen/Qwen3.5-27B",
                    "Qwen/Qwen3.6-27B",
                    "Qwen/Qwen3.8-27B",
                    "Qwen/Qwen3.6-35B-A3B",
                    "Qwen/Qwen3.5-35B-A3B",
                ],
                &["Qwen/Qwen3.8-27B", "Qwen/Qwen3.6-35B-A3B"],
            ),
            (
                "mid-name version",
                &["claude-3-5-sonnet-20241022", "claude-sonnet-4-5"],
                &["claude-sonnet-4-5"],
            ),
            (
                "bedrock",
                &[
                    "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
                    "us.anthropic.claude-sonnet-5-5",
                    "us.meta.llama3-1-70b-instruct-v1:0",
                    "us.meta.llama3-3-70b-instruct-v1:0",
                ],
                &[
                    "us.anthropic.claude-sonnet-5-5",
                    "us.meta.llama3-3-70b-instruct-v1:0",
                ],
            ),
            (
                "provider prefix",
                &["openai/gpt-5.5", "openai/gpt-5.6", "azure/gpt-5.5"],
                &["openai/gpt-5.6", "azure/gpt-5.5"],
            ),
            ("none", &[], &[]),
        ];
        for (name, input, expected) in cases {
            assert_eq!(best_in_family(&ids(input)), ids(expected), "{name}");
        }
    }

    #[test]
    fn newest_in_family_upgrades_only_to_a_newer_member() {
        let listed = ids(&[
            "claude-sonnet-5-5",
            "claude-opus-5-6",
            "claude-opus-5-5",
            "gpt-4o-2024-11-20",
        ]);
        assert_eq!(
            newest_in_family(&listed, "claude-opus-4-8"),
            Some("claude-opus-5-6")
        );
        assert_eq!(newest_in_family(&listed, "claude-opus-5-6"), None);
        assert_eq!(newest_in_family(&listed, "claude-haiku-5-5"), None);
        // a dated snapshot is not newer than the alias it was taken of
        assert_eq!(newest_in_family(&listed, "gpt-4o"), None);
    }
}
