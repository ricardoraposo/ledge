use chrono::NaiveDate;

use crate::markdown;
use crate::model::{Note, today_date};
use crate::settings::Settings;
use crate::store::{Draft, Store};

pub fn daily_title(day: NaiveDate) -> String {
    format!("Daily {}", day.format("%d/%m"))
}

pub fn find_daily(store: &Store, day: &str) -> Option<Note> {
    let mut candidates: Vec<Note> =
        store.live_notes().into_iter().filter(|note| note.daily_day.as_deref() == Some(day)).collect();
    candidates.sort_by(|a, b| (!a.is_active(), a.created_at).cmp(&(!b.is_active(), b.created_at)));
    candidates.into_iter().next()
}

pub fn previous_daily(store: &Store, before: &str) -> Option<Note> {
    let mut earlier: Vec<Note> = store
        .live_notes()
        .into_iter()
        .filter(|note| note.daily_day.as_deref().is_some_and(|day| day < before))
        .collect();
    earlier.sort_by(|a, b| (a.daily_day.clone(), a.created_at).cmp(&(b.daily_day.clone(), b.created_at)));
    earlier.pop()
}

pub fn carry_over_section(previous: &Note) -> String {
    let tasks = markdown::unfinished_tasks(&previous.body);
    if tasks.is_empty() {
        return String::new();
    }
    let Some(day) = previous.daily_day.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else {
        return String::new();
    };
    let mut lines = vec![format!("### From {}, {}", day.format("%A"), day.format("%d/%m"))];
    lines.extend(tasks.into_iter().map(|task| format!("- [ ] {task}")));
    lines.join("\n")
}

pub fn open_or_create_today(store: &Store, settings: &Settings) -> (Note, bool) {
    let day = today_date();
    let key = day.format("%Y-%m-%d").to_string();
    if let Some(existing) = find_daily(store, &key) {
        if existing.is_archived() {
            store.unarchive(&existing.id);
        }
        return (store.get(&existing.id).unwrap_or(existing), false);
    }
    let mut body = settings.string("daily_template");
    if settings.string("carry_over") == "bring" {
        if let Some(previous) = previous_daily(store, &key) {
            let section = carry_over_section(&previous);
            if !section.is_empty() {
                let separator = if body.trim().is_empty() { "" } else { "\n\n" };
                body = format!("{}{separator}{section}", body.trim_end_matches('\n'));
            }
        }
    }
    let note = store.create(Draft {
        title: daily_title(day),
        body,
        daily_day: Some(key),
        color: Some(6),
        ..Draft::default()
    });
    (note, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;
    use chrono::Duration;
    use serde_json::json;

    #[test]
    fn carry_over() {
        let store = temp_store();
        let yesterday = (today_date() - Duration::days(1)).format("%Y-%m-%d").to_string();
        store.create(Draft {
            title: "y".into(),
            body: "- [ ] call the plumber\n- [x] done".into(),
            daily_day: Some(yesterday),
            ..Draft::default()
        });
        let settings = Settings::in_memory(&[("daily_template", json!("## Today\n- [ ] ")), ("carry_over", json!("bring"))]);
        let (note, created) = open_or_create_today(&store, &settings);
        assert!(created);
        assert!(note.body.contains("- [ ] call the plumber"));
        assert!(!note.body.contains("done"));
        let (again, created_again) = open_or_create_today(&store, &settings);
        assert_eq!(again.id, note.id);
        assert!(!created_again);
    }
}
