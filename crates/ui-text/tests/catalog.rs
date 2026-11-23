//! Checks the translation tables in `src/lib.rs` as a catalog: every key
//! must exist in both locales, resolve to a value, and be written in the
//! language of its locale.

use std::collections::BTreeSet;

use ui_text::{message, message_pair, text, Locale};

const SOURCE: &str = include_str!("../src/lib.rs");

/// Untranslated terms that stay the same in the Chinese interface.
const SHARED_TERMS: [&str; 3] = ["detail_id", "detail_asr", "provider_api_key"];

#[derive(Clone, Copy, Debug)]
enum Table {
    Text,
    Message,
    Pair,
}

fn section(table: Table) -> &'static str {
    let message = SOURCE.find("pub fn message(").expect("message table");
    let pair = SOURCE
        .find("pub fn message_pair(")
        .expect("message pair table");
    match table {
        Table::Text => &SOURCE[..message],
        Table::Message => &SOURCE[message..pair],
        Table::Pair => &SOURCE[pair..],
    }
}

fn keys(table: Table, locale: Locale) -> BTreeSet<&'static str> {
    let needle = match locale {
        Locale::ZhCn => "(Locale::ZhCn, \"",
        Locale::EnUs => "(Locale::EnUs, \"",
    };
    let section = section(table);
    section
        .match_indices(needle)
        .map(|(start, _)| {
            let rest = &section[start + needle.len()..];
            &rest[..rest.find('"').expect("closed key")]
        })
        .collect()
}

fn render(table: Table, locale: Locale, key: &str) -> String {
    match table {
        Table::Text => text(locale, key).to_owned(),
        Table::Message => message(locale, key, "2"),
        Table::Pair => message_pair(locale, key, "A", "B"),
    }
}

fn is_chinese(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{303f}' | '\u{4e00}'..='\u{9fff}' | '，' | '：' | '（' | '）' | '？' | '！' | '；')
}

const TABLES: [Table; 3] = [Table::Text, Table::Message, Table::Pair];

#[test]
fn every_key_exists_in_both_locales() {
    for table in TABLES {
        let zh = keys(table, Locale::ZhCn);
        let en = keys(table, Locale::EnUs);
        assert!(!zh.is_empty(), "{table:?} has no keys");
        let zh_only: Vec<_> = zh.difference(&en).collect();
        let en_only: Vec<_> = en.difference(&zh).collect();
        assert!(
            zh_only.is_empty() && en_only.is_empty(),
            "{table:?}: missing en-US {zh_only:?}, missing zh-CN {en_only:?}"
        );
    }
}

#[test]
fn every_text_key_resolves_in_both_locales() {
    for locale in [Locale::ZhCn, Locale::EnUs] {
        let empty: Vec<_> = keys(Table::Text, locale)
            .into_iter()
            .filter(|key| text(locale, key).trim().is_empty())
            .collect();
        assert!(
            empty.is_empty(),
            "{locale:?} renders empty text for {empty:?}"
        );
    }
    assert_eq!(text(Locale::EnUs, "no_such_key"), "");
}

#[test]
fn english_entries_contain_no_chinese() {
    for table in TABLES {
        let mixed: Vec<_> = keys(table, Locale::EnUs)
            .into_iter()
            .filter(|key| render(table, Locale::EnUs, key).chars().any(is_chinese))
            .collect();
        assert!(
            mixed.is_empty(),
            "{table:?} en-US entries with Chinese: {mixed:?}"
        );
    }
}

#[test]
fn chinese_entries_are_written_in_chinese() {
    for table in TABLES {
        let untranslated: Vec<_> = keys(table, Locale::ZhCn)
            .into_iter()
            .filter(|key| !SHARED_TERMS.contains(key))
            .filter(|key| !render(table, Locale::ZhCn, key).chars().any(is_chinese))
            .collect();
        assert!(
            untranslated.is_empty(),
            "{table:?} zh-CN entries without Chinese: {untranslated:?}"
        );
    }
}
