use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{Duration, NaiveDate};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::crypto::{KeyBackend, Sealer, load_key};
use crate::model::{
    DEFAULT_COLOR, Note, Stamp, iso, iso_opt, new_id, now, parse_iso, resolve_color, today, today_date,
};
use crate::paths;
use crate::signals::Listeners;

pub const MAX_PINNED: usize = 5;

pub const GROUP_COLORS: [&str; 12] = [
    "#E5534B", "#F08A3C", "#E8B931", "#9BBF3A", "#3FAE6A", "#2BA59C", "#3A9BD9", "#4F6FE0", "#8A5CD6", "#C653B5",
    "#E05C8A", "#8C8C8C",
];

const NOTE_COLUMNS: &str = "id, title, body, color, state, sort_index, tags, created_at, updated_at, edited_at, \
    deleted_at, daily_day, daily_kept, keep_on_deck, pinned_at, last_opened_day, auto_archived_day, group_id, \
    floating, float_x, float_y, float_w, float_h";

const LOCKED_BODY: &str = "This note was sealed with another computer's key and cannot be read here.";

#[derive(Clone, Debug, PartialEq)]
pub struct NoteGroup {
    pub id: String,
    pub name: String,
    pub color: i64,
    pub sort_index: i64,
    pub created_at: Stamp,
    pub updated_at: Stamp,
    pub deleted_at: Option<Stamp>,
}

impl NoteGroup {
    pub fn new(name: &str, color: i64, sort_index: i64) -> Self {
        let stamp = now();
        NoteGroup {
            id: new_id(),
            name: name.to_string(),
            color,
            sort_index,
            created_at: stamp,
            updated_at: stamp,
            deleted_at: None,
        }
    }

    pub fn color_index(&self) -> usize {
        self.color.rem_euclid(GROUP_COLORS.len() as i64) as usize
    }
}

#[derive(Default)]
pub struct Draft {
    pub title: String,
    pub body: String,
    pub color: Option<i64>,
    pub tags: Vec<String>,
    pub daily_day: Option<String>,
    pub group_id: Option<String>,
}

pub struct Store {
    pub path: PathBuf,
    db: RefCell<Connection>,
    pub sealer: Sealer,
    pub key_backend: KeyBackend,
    notes: RefCell<HashMap<String, Note>>,
    groups: RefCell<HashMap<String, NoteGroup>>,
    pub changed: Listeners,
}

pub fn has_sealed_notes(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(db) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    db.query_row("SELECT 1 FROM notes WHERE substr(body, 1, 3) = X'4B4E31' LIMIT 1", [], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

fn quiet_view(note: &Note) -> Note {
    Note {
        floating: false,
        float_x: None,
        float_y: None,
        float_w: None,
        float_h: None,
        last_opened_day: None,
        ..note.clone()
    }
}

impl Store {
    pub fn open_default() -> Result<Store, String> {
        let path = paths::database_path();
        let (key, backend) = load_key(has_sealed_notes(&path))?;
        Store::open(&path, Sealer::new(&key), backend)
    }

    pub fn open(path: &Path, sealer: Sealer, key_backend: KeyBackend) -> Result<Store, String> {
        let db = Connection::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        db.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
        db.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
        let store = Store {
            path: path.to_path_buf(),
            db: RefCell::new(db),
            sealer,
            key_backend,
            notes: RefCell::new(HashMap::new()),
            groups: RefCell::new(HashMap::new()),
            changed: Listeners::default(),
        };
        store.migrate()?;
        store.reload();
        Ok(store)
    }

    fn migrate(&self) -> Result<(), String> {
        let db = self.db.borrow();
        let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(|e| e.to_string())?;
        if version < 1 {
            db.execute_batch(
                "CREATE TABLE IF NOT EXISTS notes (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    body BLOB,
                    color INTEGER NOT NULL DEFAULT 1,
                    state TEXT NOT NULL DEFAULT 'active',
                    sort_index INTEGER NOT NULL DEFAULT 0,
                    tags TEXT NOT NULL DEFAULT '[]',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    edited_at TEXT NOT NULL,
                    deleted_at TEXT,
                    daily_day TEXT,
                    daily_kept INTEGER NOT NULL DEFAULT 0,
                    keep_on_deck INTEGER NOT NULL DEFAULT 0,
                    pinned_at TEXT,
                    last_opened_day TEXT,
                    auto_archived_day TEXT,
                    group_id TEXT,
                    floating INTEGER NOT NULL DEFAULT 0,
                    float_x INTEGER, float_y INTEGER, float_w INTEGER, float_h INTEGER
                );
                CREATE TABLE IF NOT EXISTS groups (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    color INTEGER NOT NULL DEFAULT 6,
                    sort_index INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    deleted_at TEXT
                );
                CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
                PRAGMA user_version = 1;",
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn reload(&self) {
        let mut notes = HashMap::new();
        let mut groups = HashMap::new();
        {
            let db = self.db.borrow();
            let query = format!("SELECT {NOTE_COLUMNS} FROM notes");
            if let Ok(mut statement) = db.prepare(&query) {
                if let Ok(rows) = statement.query_map([], |row| Ok(self.note_from_row(row))) {
                    for note in rows.flatten() {
                        notes.insert(note.id.clone(), note);
                    }
                }
            }
            if let Ok(mut statement) =
                db.prepare("SELECT id, name, color, sort_index, created_at, updated_at, deleted_at FROM groups")
            {
                let mapped = statement.query_map([], |row| {
                    let created: Option<String> = row.get(4)?;
                    let updated: Option<String> = row.get(5)?;
                    let deleted: Option<String> = row.get(6)?;
                    Ok(NoteGroup {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                        sort_index: row.get(3)?,
                        created_at: parse_iso(created.as_deref()).unwrap_or_else(now),
                        updated_at: parse_iso(updated.as_deref()).unwrap_or_else(now),
                        deleted_at: parse_iso(deleted.as_deref()),
                    })
                });
                if let Ok(rows) = mapped {
                    for group in rows.flatten() {
                        groups.insert(group.id.clone(), group);
                    }
                }
            }
        }
        *self.notes.borrow_mut() = notes;
        *self.groups.borrow_mut() = groups;
        self.backfill_opened();
        self.emit("");
    }

    fn backfill_opened(&self) {
        let missing: Vec<Note> = self
            .notes
            .borrow_mut()
            .values_mut()
            .filter(|note| note.last_opened_day.is_none() && note.deleted_at.is_none())
            .map(|note| {
                note.last_opened_day = Some(today());
                note.clone()
            })
            .collect();
        for note in missing {
            self.write(&note);
        }
    }

    fn note_from_row(&self, row: &Row) -> Note {
        let id: String = row.get(0).unwrap_or_default();
        let blob: Vec<u8> = row.get::<_, Option<Vec<u8>>>(2).ok().flatten().unwrap_or_default();
        let (body, locked) = match self.sealer.open(&blob, &id).map(String::from_utf8) {
            Ok(Ok(text)) => (text, false),
            _ => (LOCKED_BODY.to_string(), true),
        };
        let text = |index: usize| row.get::<_, Option<String>>(index).ok().flatten();
        let number = |index: usize| row.get::<_, Option<i64>>(index).ok().flatten();
        let flag = |index: usize| number(index).unwrap_or(0) != 0;
        let stamp = |index: usize| parse_iso(text(index).as_deref());
        Note {
            id,
            title: text(1).unwrap_or_default(),
            body,
            color: resolve_color(number(3).unwrap_or(DEFAULT_COLOR)),
            state: text(4).unwrap_or_else(|| "active".into()),
            sort_index: number(5).unwrap_or(0),
            tags: serde_json::from_str(&text(6).unwrap_or_else(|| "[]".into())).unwrap_or_default(),
            created_at: stamp(7).unwrap_or_else(now),
            updated_at: stamp(8).unwrap_or_else(now),
            edited_at: stamp(9).unwrap_or_else(now),
            deleted_at: stamp(10),
            daily_day: text(11),
            daily_kept: flag(12),
            keep_on_deck: flag(13),
            pinned_at: stamp(14),
            last_opened_day: text(15),
            auto_archived_day: text(16),
            group_id: text(17),
            floating: flag(18),
            float_x: number(19),
            float_y: number(20),
            float_w: number(21),
            float_h: number(22),
            locked,
        }
    }

    pub fn write(&self, note: &Note) {
        if note.locked {
            return;
        }
        let body = self.sealer.seal(note.body.as_bytes(), &note.id);
        let tags = serde_json::to_string(&note.tags).unwrap_or_else(|_| "[]".into());
        let query = format!(
            "INSERT OR REPLACE INTO notes ({NOTE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, \
             ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)"
        );
        let result = self.db.borrow().execute(
            &query,
            params![
                note.id,
                note.title,
                body,
                note.color,
                note.state,
                note.sort_index,
                tags,
                iso(&note.created_at),
                iso(&note.updated_at),
                iso(&note.edited_at),
                iso_opt(&note.deleted_at),
                note.daily_day,
                note.daily_kept,
                note.keep_on_deck,
                iso_opt(&note.pinned_at),
                note.last_opened_day,
                note.auto_archived_day,
                note.group_id,
                note.floating,
                note.float_x,
                note.float_y,
                note.float_w,
                note.float_h,
            ],
        );
        if let Err(error) = result {
            eprintln!("ledge: cannot save note {}: {error}", note.id);
        }
    }

    pub fn write_group(&self, group: &NoteGroup) {
        let result = self.db.borrow().execute(
            "INSERT OR REPLACE INTO groups (id, name, color, sort_index, created_at, updated_at, deleted_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                group.id,
                group.name,
                group.color,
                group.sort_index,
                iso(&group.created_at),
                iso(&group.updated_at),
                iso_opt(&group.deleted_at),
            ],
        );
        if let Err(error) = result {
            eprintln!("ledge: cannot save group {}: {error}", group.id);
        }
    }

    pub fn emit(&self, note_id: &str) {
        self.changed.emit(note_id);
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.db
            .borrow()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get::<_, Option<String>>(0))
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        let _ = self.db.borrow().execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [key, value]);
    }

    pub fn get(&self, note_id: &str) -> Option<Note> {
        self.notes.borrow().get(note_id).filter(|note| note.deleted_at.is_none()).cloned()
    }

    pub fn raw(&self, note_id: &str) -> Option<Note> {
        self.notes.borrow().get(note_id).cloned()
    }

    pub fn all_notes(&self) -> Vec<Note> {
        self.notes.borrow().values().cloned().collect()
    }

    pub fn live_notes(&self) -> Vec<Note> {
        self.notes.borrow().values().filter(|note| note.deleted_at.is_none()).cloned().collect()
    }

    pub fn active_notes(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = self.live_notes().into_iter().filter(Note::is_active).collect();
        notes.sort_by(|a, b| (a.sort_index, a.created_at).cmp(&(b.sort_index, b.created_at)));
        notes
    }

    pub fn archived_notes(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = self.live_notes().into_iter().filter(Note::is_archived).collect();
        notes.sort_by(|a, b| b.edited_at.cmp(&a.edited_at));
        notes
    }

    pub fn group(&self, group_id: &str) -> Option<NoteGroup> {
        self.groups.borrow().get(group_id).cloned()
    }

    pub fn all_groups(&self) -> Vec<NoteGroup> {
        self.groups.borrow().values().cloned().collect()
    }

    pub fn live_groups(&self) -> Vec<NoteGroup> {
        let mut groups: Vec<NoteGroup> =
            self.groups.borrow().values().filter(|group| group.deleted_at.is_none()).cloned().collect();
        groups.sort_by(|a, b| (a.sort_index, a.created_at).cmp(&(b.sort_index, b.created_at)));
        groups
    }

    pub fn all_tags(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for note in self.live_notes() {
            for tag in note.tags {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut tags: Vec<(String, usize)> = counts.into_iter().collect();
        tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        tags
    }

    fn last_index(&self) -> i64 {
        self.notes
            .borrow()
            .values()
            .filter(|note| note.deleted_at.is_none() && note.is_active())
            .map(|note| note.sort_index + 1)
            .max()
            .unwrap_or(0)
    }

    pub fn create(&self, draft: Draft) -> Note {
        let stamp = now();
        let mut note = Note {
            title: draft.title,
            body: draft.body,
            color: draft.color.unwrap_or(DEFAULT_COLOR),
            tags: draft.tags,
            sort_index: self.last_index(),
            created_at: stamp,
            updated_at: stamp,
            edited_at: stamp,
            daily_day: draft.daily_day,
            last_opened_day: Some(today()),
            group_id: draft.group_id,
            ..Note::default()
        };
        if note.daily_day.is_some() && !note.tags.iter().any(|tag| tag == "daily") {
            note.tags.insert(0, "daily".into());
        }
        self.notes.borrow_mut().insert(note.id.clone(), note.clone());
        self.write(&note);
        self.emit(&note.id);
        note
    }

    fn apply(&self, note_id: &str, emit: bool, change: impl FnOnce(&mut Note)) -> Option<Note> {
        let before = self.notes.borrow().get(note_id).cloned()?;
        if before.locked {
            return Some(before);
        }
        let mut note = before.clone();
        change(&mut note);
        if note == before {
            return Some(before);
        }
        let text_changed = note.title != before.title || note.body != before.body;
        let quiet = quiet_view(&note) == quiet_view(&before);
        if note.tags != before.tags {
            let tagged = note.tags.iter().any(|tag| tag == "daily");
            if tagged && note.daily_day.is_none() {
                note.daily_day = Some(today());
            }
            if !tagged && note.daily_day.is_some() {
                note.daily_day = None;
                note.daily_kept = false;
            }
        }
        if !quiet {
            note.updated_at = now();
        }
        if text_changed {
            note.edited_at = note.updated_at;
        }
        self.notes.borrow_mut().insert(note.id.clone(), note.clone());
        self.write(&note);
        if emit {
            self.emit(note_id);
        }
        Some(note)
    }

    pub fn update(&self, note_id: &str, change: impl FnOnce(&mut Note)) -> Option<Note> {
        self.apply(note_id, true, change)
    }

    pub fn update_silently(&self, note_id: &str, change: impl FnOnce(&mut Note)) -> Option<Note> {
        self.apply(note_id, false, change)
    }

    pub fn mark_opened(&self, note_id: &str) {
        let day = today();
        if let Some(note) = self.get(note_id) {
            if note.last_opened_day.as_deref() != Some(day.as_str()) {
                self.update_silently(note_id, |note| note.last_opened_day = Some(day));
            }
        }
    }

    fn set_deleted(&self, note_id: &str, deleted: Option<Stamp>) {
        let note = {
            let mut notes = self.notes.borrow_mut();
            let Some(note) = notes.get_mut(note_id) else {
                return;
            };
            note.deleted_at = deleted;
            note.updated_at = deleted.unwrap_or_else(now);
            note.clone()
        };
        self.write(&note);
        self.emit(note_id);
    }

    pub fn delete(&self, note_id: &str) {
        self.set_deleted(note_id, Some(now()));
    }

    pub fn undelete(&self, note_id: &str) {
        self.set_deleted(note_id, None);
    }

    pub fn purge(&self, note_id: &str) {
        let _ = self.db.borrow().execute("DELETE FROM notes WHERE id = ?1", [note_id]);
        self.notes.borrow_mut().remove(note_id);
    }

    pub fn purge_deleted(&self) {
        let limit = now() - Duration::days(30);
        let old: Vec<String> = self
            .notes
            .borrow()
            .values()
            .filter(|note| note.deleted_at.is_some_and(|deleted| deleted < limit))
            .map(|note| note.id.clone())
            .collect();
        for note_id in old {
            self.purge(&note_id);
        }
    }

    pub fn archive(&self, note_id: &str, auto: bool) {
        let Some(note) = self.get(note_id) else {
            return;
        };
        if note.is_archived() {
            return;
        }
        self.update(note_id, |note| {
            note.state = "archived".into();
            note.pinned_at = None;
            note.floating = false;
            note.auto_archived_day = auto.then(today);
        });
    }

    pub fn unarchive(&self, note_id: &str) {
        let Some(note) = self.get(note_id) else {
            return;
        };
        if note.is_active() {
            return;
        }
        let index = self.last_index();
        self.update(note_id, |note| {
            note.state = "active".into();
            note.sort_index = index;
            note.last_opened_day = Some(today());
            note.auto_archived_day = None;
            note.daily_kept = note.is_daily();
        });
    }

    pub fn set_pinned(&self, note_id: &str, pinned: bool) -> bool {
        let Some(note) = self.get(note_id) else {
            return false;
        };
        if pinned {
            if note.is_pinned() {
                return true;
            }
            if self.active_notes().iter().filter(|note| note.is_pinned()).count() >= MAX_PINNED {
                return false;
            }
            self.update(note_id, |note| note.pinned_at = Some(now()));
        } else {
            self.update(note_id, |note| note.pinned_at = None);
        }
        true
    }

    pub fn reorder(&self, ordered: &[String]) {
        let mut changed = Vec::new();
        {
            let mut notes = self.notes.borrow_mut();
            for (index, note_id) in ordered.iter().enumerate() {
                if let Some(note) = notes.get_mut(note_id) {
                    if note.sort_index != index as i64 {
                        note.sort_index = index as i64;
                        changed.push(note.clone());
                    }
                }
            }
        }
        for note in changed {
            self.write(&note);
        }
        self.emit("");
    }

    pub fn create_group(&self, name: &str) -> NoteGroup {
        let groups = self.live_groups();
        let color = (groups.len() as i64 * 5) % GROUP_COLORS.len() as i64;
        let order = groups.last().map(|group| group.sort_index + 1).unwrap_or(0);
        let group = NoteGroup::new(name, color, order);
        self.groups.borrow_mut().insert(group.id.clone(), group.clone());
        self.write_group(&group);
        self.emit("");
        group
    }

    pub fn update_group(&self, group_id: &str, change: impl FnOnce(&mut NoteGroup)) {
        let group = {
            let mut groups = self.groups.borrow_mut();
            let Some(group) = groups.get_mut(group_id) else {
                return;
            };
            change(group);
            group.updated_at = now();
            group.clone()
        };
        self.write_group(&group);
        self.emit("");
    }

    pub fn delete_group(&self, group_id: &str) {
        if self.group(group_id).is_none() {
            return;
        }
        for note in self.live_notes() {
            if note.group_id.as_deref() == Some(group_id) {
                self.update_silently(&note.id, |note| note.group_id = None);
            }
        }
        self.update_group(group_id, |group| group.deleted_at = Some(now()));
    }

    pub fn put_group(&self, group: NoteGroup) -> bool {
        let newer = match self.group(&group.id) {
            None => true,
            Some(current) => group.updated_at > current.updated_at,
        };
        if newer {
            self.write_group(&group);
            self.groups.borrow_mut().insert(group.id.clone(), group);
        }
        newer
    }

    pub fn has_group(&self, group_id: &str) -> bool {
        self.groups.borrow().contains_key(group_id)
    }

    pub fn upsert_from_sync(&self, mut incoming: Note) -> bool {
        let Some(current) = self.raw(&incoming.id) else {
            incoming.sort_index = self.last_index();
            self.write(&incoming);
            self.notes.borrow_mut().insert(incoming.id.clone(), incoming);
            return true;
        };
        if incoming.updated_at <= current.updated_at {
            if let Some(opened) = &incoming.last_opened_day {
                if current.last_opened_day.as_deref().unwrap_or("") < opened.as_str() {
                    let mut current = current.clone();
                    current.last_opened_day = Some(opened.clone());
                    self.write(&current);
                    self.notes.borrow_mut().insert(current.id.clone(), current);
                }
            }
            return false;
        }
        incoming.sort_index = current.sort_index;
        incoming.floating = current.floating;
        incoming.float_x = current.float_x;
        incoming.float_y = current.float_y;
        incoming.float_w = current.float_w;
        incoming.float_h = current.float_h;
        if let Some(opened) = &current.last_opened_day {
            if incoming.last_opened_day.as_deref().unwrap_or("") < opened.as_str() {
                incoming.last_opened_day = Some(opened.clone());
            }
        }
        if incoming.title == current.title && incoming.body == current.body {
            incoming.edited_at = current.edited_at;
        }
        self.write(&incoming);
        self.notes.borrow_mut().insert(incoming.id.clone(), incoming);
        true
    }

    pub fn apply_remote_delete(&self, note_id: &str, deleted: Stamp) -> bool {
        let Some(current) = self.raw(note_id) else {
            return false;
        };
        if current.deleted_at.is_some() || current.updated_at >= deleted {
            return false;
        }
        let mut note = current;
        note.deleted_at = Some(deleted);
        note.updated_at = deleted;
        self.write(&note);
        self.notes.borrow_mut().insert(note.id.clone(), note);
        true
    }

    pub fn days_until_archive(&self, note: &Note, after_days: i64) -> Option<i64> {
        if after_days <= 0 || !note.is_active() || note.keep_on_deck || note.is_pinned() || note.is_daily() {
            return None;
        }
        let opened = note
            .last_opened_day
            .as_deref()
            .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
            .unwrap_or_else(today_date);
        Some(after_days - (today_date() - opened).num_days())
    }

    pub fn run_archive_rules(&self, after_days: i64, protected: &HashSet<String>) -> Vec<String> {
        let mut archived = Vec::new();
        if after_days > 0 {
            for note in self.active_notes() {
                if protected.contains(&note.id) {
                    continue;
                }
                if self.days_until_archive(&note, after_days).is_some_and(|left| left <= 0) {
                    self.archive(&note.id, true);
                    archived.push(note.id);
                }
            }
        }
        let days: BTreeSet<String> = self.active_notes().into_iter().filter_map(|note| note.daily_day).collect();
        let keep: Vec<String> = days.into_iter().rev().take(2).collect();
        for note in self.active_notes() {
            if let Some(day) = &note.daily_day {
                if !keep.contains(day) && !note.daily_kept && !protected.contains(&note.id) {
                    self.archive(&note.id, true);
                    archived.push(note.id);
                }
            }
        }
        archived
    }

    pub fn backup_to(&self, target: &Path) -> Result<(), String> {
        self.db.borrow().backup(rusqlite::MAIN_DB, target, None::<fn(rusqlite::backup::Progress)>).map_err(|error| error.to_string())
    }

    pub fn restore_from(&self, source: &Path) -> Result<(), String> {
        self.db
            .borrow_mut()
            .restore(rusqlite::MAIN_DB, source, None::<fn(rusqlite::backup::Progress)>)
            .map_err(|error| error.to_string())?;
        self.reload();
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use aes_gcm::aead::{KeyInit, OsRng};

    pub fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ledge-test-{label}-{}", new_id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    pub fn test_key() -> Vec<u8> {
        aes_gcm::Aes256Gcm::generate_key(&mut OsRng).to_vec()
    }

    pub fn temp_store() -> Store {
        let path = temp_dir("store").join("notes.sqlite");
        Store::open(&path, Sealer::new(&test_key()), KeyBackend::File).unwrap()
    }

    fn days_ago(days: i64) -> String {
        (today_date() - Duration::days(days)).format("%Y-%m-%d").to_string()
    }

    fn titled(title: &str) -> Draft {
        Draft { title: title.into(), ..Draft::default() }
    }

    #[test]
    fn body_is_encrypted_on_disk() {
        let key = test_key();
        let path = temp_dir("seal").join("notes.sqlite");
        let store = Store::open(&path, Sealer::new(&key), KeyBackend::File).unwrap();
        store.create(Draft { title: "t".into(), body: "secret words".into(), ..Draft::default() });
        let raw: Vec<u8> = store.db.borrow().query_row("SELECT body FROM notes", [], |row| row.get(0)).unwrap();
        assert!(!raw.windows(6).any(|window| window == b"secret"));
        let again = Store::open(&path, Sealer::new(&key), KeyBackend::File).unwrap();
        assert_eq!(again.live_notes()[0].body, "secret words");
        let stranger = Store::open(&path, Sealer::new(&test_key()), KeyBackend::File).unwrap();
        assert!(stranger.live_notes()[0].locked);
    }

    #[test]
    fn edited_moves_only_with_text() {
        let store = temp_store();
        let note = store.create(Draft { title: "t".into(), body: "b".into(), ..Draft::default() });
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.update(&note.id, |note| note.color = 4);
        assert_eq!(store.get(&note.id).unwrap().edited_at, note.edited_at);
        store.update(&note.id, |note| note.body = "c".into());
        assert!(store.get(&note.id).unwrap().edited_at > note.edited_at);
    }

    #[test]
    fn pin_limit() {
        let store = temp_store();
        let notes: Vec<Note> = (0..6).map(|index| store.create(titled(&index.to_string()))).collect();
        let results: Vec<bool> = notes.iter().map(|note| store.set_pinned(&note.id, true)).collect();
        assert_eq!(results, vec![true, true, true, true, true, false]);
    }

    #[test]
    fn archive_by_time() {
        let store = temp_store();
        let old = store.create(titled("old"));
        let kept = store.create(titled("kept"));
        store.update(&old.id, |note| note.last_opened_day = Some(days_ago(20)));
        store.update(&kept.id, |note| {
            note.keep_on_deck = true;
            note.last_opened_day = Some(days_ago(20));
        });
        assert_eq!(store.run_archive_rules(14, &HashSet::new()), vec![old.id.clone()]);
        assert_eq!(store.get(&old.id).unwrap().auto_archived_day, Some(today()));
    }

    #[test]
    fn only_two_daily_days_stay() {
        let store = temp_store();
        for days in 0..3 {
            let day = days_ago(days);
            store.create(Draft { title: day.clone(), daily_day: Some(day), ..Draft::default() });
        }
        store.run_archive_rules(0, &HashSet::new());
        assert_eq!(store.active_notes().len(), 2);
    }

    #[test]
    fn quiet_changes_keep_updated() {
        let store = temp_store();
        let note = store.create(titled("q"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.update(&note.id, |note| note.float_w = Some(400));
        assert_eq!(store.get(&note.id).unwrap().updated_at, note.updated_at);
    }

    #[test]
    fn daily_tag_sets_day() {
        let store = temp_store();
        let note = store.create(titled("d"));
        store.update(&note.id, |note| note.tags = vec!["daily".into()]);
        assert_eq!(store.get(&note.id).unwrap().daily_day, Some(today()));
        store.update(&note.id, |note| note.tags.clear());
        assert_eq!(store.get(&note.id).unwrap().daily_day, None);
    }

    #[test]
    fn backup_and_restore() {
        let store = temp_store();
        store.create(titled("before"));
        let target = temp_dir("backup").join("copy.sqlite");
        store.backup_to(&target).unwrap();
        store.create(titled("after"));
        assert_eq!(store.live_notes().len(), 2);
        store.restore_from(&target).unwrap();
        assert_eq!(store.live_notes().len(), 1);
    }
}
