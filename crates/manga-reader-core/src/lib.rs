use std::{cmp::Ordering, collections::HashSet};

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatsRanges {
    pub day: Stats,
    pub week: Stats,
    pub month: Stats,
    pub all: Stats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub active_reading_seconds: u64,
    pub unique_pages: u64,
    pub page_turns: u64,
    pub completed_volumes: u64,
    pub dictionary_lookups: u64,
    pub cards_mined: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_characters: Option<u64>,
    pub ocr_covered_pages: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatEvent {
    pub id: String,
    pub kind: StatEventKind,
    pub local_date: NaiveDate,
    pub occurred_at_ms: i64,
    pub value: u64,
    pub unique_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    #[serde(default)]
    pub sensitive: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatEventKind {
    ActiveReading,
    PageEncounter,
    VolumeComplete,
    DictionaryLookup,
    CardMined,
    OcrCharacters,
}

pub fn calculate_stats(events: &[StatEvent], show_sensitive: bool, today: NaiveDate) -> StatsRanges {
    let week_start = today - Duration::days(6);
    let build = |filter: &dyn Fn(&StatEvent) -> bool| {
        let mut stats = Stats::default();
        let mut unique_pages = HashSet::new();
        for event in events.iter().filter(|event| show_sensitive || !event.sensitive).filter(|event| filter(event)) {
            match event.kind {
                StatEventKind::ActiveReading => stats.active_reading_seconds += event.value,
                StatEventKind::PageEncounter => {
                    stats.page_turns += event.value;
                    if let Some(key) = &event.unique_key { unique_pages.insert(key); }
                }
                StatEventKind::VolumeComplete => stats.completed_volumes += event.value,
                StatEventKind::DictionaryLookup => stats.dictionary_lookups += event.value,
                StatEventKind::CardMined => stats.cards_mined += event.value,
                StatEventKind::OcrCharacters => {
                    stats.ocr_characters = Some(stats.ocr_characters.unwrap_or(0) + event.value);
                    stats.ocr_covered_pages += 1;
                }
            }
        }
        stats.unique_pages = unique_pages.len() as u64;
        stats
    };
    StatsRanges {
        day: build(&|event| event.local_date == today),
        week: build(&|event| event.local_date >= week_start && event.local_date <= today),
        month: build(&|event| event.local_date.year() == today.year() && event.local_date.month() == today.month()),
        all: build(&|_| true),
    }
}

/// Case-insensitive natural ordering for Unicode names with ASCII digit runs.
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left = left.chars().peekable();
    let mut right = right.chars().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a), Some(b)) if a.is_ascii_digit() && b.is_ascii_digit() => {
                let a_num: String = std::iter::from_fn(|| left.next_if(|c| c.is_ascii_digit())).collect();
                let b_num: String = std::iter::from_fn(|| right.next_if(|c| c.is_ascii_digit())).collect();
                let ordering = a_num.trim_start_matches('0').len().cmp(&b_num.trim_start_matches('0').len())
                    .then_with(|| a_num.trim_start_matches('0').cmp(b_num.trim_start_matches('0')))
                    .then_with(|| a_num.len().cmp(&b_num.len()));
                if ordering != Ordering::Equal { return ordering; }
            }
            (Some(_), Some(_)) => {
                let a = left.next().expect("peeked character").to_lowercase().to_string();
                let b = right.next().expect("peeked character").to_lowercase().to_string();
                let ordering = a.cmp(&b);
                if ordering != Ordering::Equal { return ordering; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: StatEventKind, date: &str, key: Option<&str>, sensitive: bool) -> StatEvent {
        StatEvent { id: date.into(), kind, local_date: date.parse().unwrap(), occurred_at_ms: 0, value: 1, unique_key: key.map(str::to_owned), series_id: None, sensitive }
    }

    #[test]
    fn statistics_ranges_and_unique_pages_have_defined_semantics() {
        let today = "2026-09-08".parse().unwrap();
        let events = vec![
            event(StatEventKind::PageEncounter, "2026-09-08", Some("book:1"), false),
            event(StatEventKind::PageEncounter, "2026-09-08", Some("book:1"), false),
            event(StatEventKind::DictionaryLookup, "2026-09-03", None, false),
            event(StatEventKind::CardMined, "2026-08-31", None, false),
        ];
        let stats = calculate_stats(&events, false, today);
        assert_eq!(stats.day.page_turns, 2);
        assert_eq!(stats.day.unique_pages, 1);
        assert_eq!(stats.week.dictionary_lookups, 1);
        assert_eq!(stats.month.cards_mined, 0);
        assert_eq!(stats.all.cards_mined, 1);
        assert_eq!(stats.all.ocr_characters, None);
    }

    #[test]
    fn sensitive_events_are_excluded_until_visibility_is_enabled() {
        let today = "2026-09-08".parse().unwrap();
        let events = vec![event(StatEventKind::PageEncounter, "2026-09-08", Some("sensitive:1"), true)];
        assert_eq!(calculate_stats(&events, false, today).all.page_turns, 0);
        assert_eq!(calculate_stats(&events, true, today).all.page_turns, 1);
    }

    #[test]
    fn natural_order_handles_volume_ten_and_unicode_prefixes() {
        let mut names = vec!["作品 Vol 10", "作品 Vol 2", "作品 Vol 01"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["作品 Vol 01", "作品 Vol 2", "作品 Vol 10"]);
    }
}
