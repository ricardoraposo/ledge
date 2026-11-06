use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use gtk::gio;
use gtk::prelude::*;

use crate::attachments::{self, Attachments, sha256_hex};
use crate::model::{Note, iso, now, parse_day, parse_iso, parse_tags, resolve_color_text};
use crate::settings::Settings;
use crate::signals::Timer;
use crate::store::{NoteGroup, Store};

pub const HEADER: &str = "--- keepnote ---";
const BODY_FENCE: &str = "---";
pub const NOTE_EXT: &str = "hmnote";
pub const GROUP_EXT: &str = "hmgroup";
pub const ATTACHMENTS: &str = "Attachments";

pub fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n")
}

pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut escaping = false;
    for character in value.chars() {
        if escaping {
            out.push(if character == 'n' { '\n' } else { character });
            escaping = false;
        } else if character == '\\' {
            escaping = true;
        } else {
            out.push(character);
        }
    }
    out
}

pub fn serialize_note(note: &Note) -> String {
    let mut lines = vec![HEADER.to_string(), "schema: 1".into(), format!("id: {}", note.id)];
    if let Some(deleted) = &note.deleted_at {
        let stamp = iso(deleted);
        lines.extend([
            "title: ".to_string(),
            "color: 1".into(),
            "state: active".into(),
            "tags: ".into(),
            format!("created: {stamp}"),
            format!("updated: {stamp}"),
            format!("deleted: {stamp}"),
            BODY_FENCE.into(),
            String::new(),
            String::new(),
        ]);
        return lines.join("\n");
    }
    lines.push(format!("title: {}", escape(&note.title)));
    lines.push(format!("color: {}", note.color));
    lines.push(format!("state: {}", note.state));
    lines.push(format!("tags: {}", note.tags.join(",")));
    if let Some(day) = &note.daily_day {
        lines.push(format!("daily: {day}"));
        if note.daily_kept {
            lines.push("daily-kept: yes".into());
        }
    }
    if note.keep_on_deck {
        lines.push("keep: yes".into());
    }
    if let Some(pinned) = &note.pinned_at {
        lines.push(format!("pinned: {}", iso(pinned)));
    }
    if let Some(day) = &note.last_opened_day {
        lines.push(format!("opened: {day}"));
    }
    if let Some(day) = &note.auto_archived_day {
        lines.push(format!("auto-archived: {day}"));
    }
    if let Some(group) = &note.group_id {
        lines.push(format!("group: {group}"));
    }
    lines.push(format!("created: {}", iso(&note.created_at)));
    lines.push(format!("updated: {}", iso(&note.updated_at)));
    lines.push(format!("edited: {}", iso(&note.edited_at)));
    lines.push(BODY_FENCE.into());
    lines.push(String::new());
    lines.push(note.body.clone());
    lines.join("\n")
}

fn fields(contents: &str) -> Option<(HashMap<String, String>, String)> {
    let lines: Vec<&str> = contents.split('\n').collect();
    if lines.first().map(|line| line.trim()) != Some(HEADER) {
        return None;
    }
    let mut map = HashMap::new();
    let mut body_start = None;
    for (index, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == BODY_FENCE {
            body_start = Some(index + 1);
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let mut body = String::new();
    if let Some(start) = body_start {
        if start < lines.len() {
            let mut rest = &lines[start..];
            if rest.first() == Some(&"") {
                rest = &rest[1..];
            }
            body = rest.join("\n");
        }
    }
    Some((map, body))
}

pub fn parse_note(contents: &str) -> Option<Note> {
    let (map, body) = fields(contents)?;
    if map.contains_key("kind") {
        return None;
    }
    let id = map.get("id").filter(|id| !id.is_empty())?.to_uppercase();
    let field = |key: &str| map.get(key).map(String::as_str);
    let updated = parse_iso(field("updated")).unwrap_or_else(now);
    Some(Note {
        id,
        title: unescape(field("title").unwrap_or("")),
        body,
        color: resolve_color_text(field("color")),
        state: if field("state") == Some("archived") { "archived".into() } else { "active".into() },
        tags: parse_tags(field("tags").unwrap_or("")),
        created_at: parse_iso(field("created")).unwrap_or(updated),
        updated_at: updated,
        edited_at: parse_iso(field("edited")).unwrap_or(updated),
        deleted_at: parse_iso(field("deleted")),
        daily_day: parse_day(field("daily")),
        daily_kept: field("daily-kept") == Some("yes"),
        keep_on_deck: field("keep") == Some("yes"),
        pinned_at: parse_iso(field("pinned")),
        last_opened_day: parse_day(field("opened")),
        auto_archived_day: parse_day(field("auto-archived")),
        group_id: field("group").map(str::to_uppercase).filter(|group| !group.is_empty()),
        ..Note::default()
    })
}

pub fn serialize_group(group: &NoteGroup) -> String {
    let mut lines = vec![
        HEADER.to_string(),
        "schema: 1".into(),
        "kind: group".into(),
        format!("id: {}", group.id),
        format!("name: {}", escape(&group.name)),
        format!("color: {}", group.color),
        format!("order: {}", group.sort_index),
        format!("created: {}", iso(&group.created_at)),
        format!("updated: {}", iso(&group.updated_at)),
    ];
    if let Some(deleted) = &group.deleted_at {
        lines.push(format!("deleted: {}", iso(deleted)));
    }
    lines.push(BODY_FENCE.into());
    lines.join("\n")
}

pub fn parse_group(contents: &str) -> Option<NoteGroup> {
    let (map, _) = fields(contents)?;
    if map.get("kind").map(String::as_str) != Some("group") {
        return None;
    }
    let id = map.get("id").filter(|id| !id.is_empty())?.to_uppercase();
    let field = |key: &str| map.get(key).map(String::as_str);
    let updated = parse_iso(field("updated"))?;
    let name: String = unescape(field("name").unwrap_or("Group")).chars().take(60).collect();
    Some(NoteGroup {
        id,
        name: if name.is_empty() { "Group".into() } else { name },
        color: field("color").and_then(|c| c.parse().ok()).unwrap_or(6),
        sort_index: field("order").and_then(|o| o.parse().ok()).unwrap_or(0),
        created_at: parse_iso(field("created")).unwrap_or(updated),
        updated_at: updated,
        deleted_at: parse_iso(field("deleted")),
    })
}

pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)
}

pub fn files_with_extension(folder: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|e| e.to_str()) == Some(extension))
                .filter(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.starts_with('.')))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

pub fn import_attachments(attachments: &Attachments, folder: &Path, body: &str) {
    for digest in attachments::ids_in(body) {
        if attachments.find(&digest).is_some() {
            continue;
        }
        if let Some(candidate) = attachments::find_in(&folder.join(ATTACHMENTS), &digest) {
            if let Ok(data) = fs::read(&candidate) {
                let name = candidate.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
                attachments.add_bytes(&data, &name);
            }
        }
    }
}

pub struct FolderSync {
    store: Rc<Store>,
    settings: Rc<Settings>,
    attachments: Rc<Attachments>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    written: RefCell<HashMap<String, String>>,
    pending: RefCell<HashSet<String>>,
    push_timer: Timer,
    pull_timer: Timer,
    applying: Cell<bool>,
}

impl FolderSync {
    pub fn new(store: Rc<Store>, settings: Rc<Settings>, attachments: Rc<Attachments>) -> Rc<FolderSync> {
        let sync = Rc::new(FolderSync {
            store,
            settings,
            attachments,
            monitor: RefCell::new(None),
            written: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashSet::new()),
            push_timer: Timer::default(),
            pull_timer: Timer::default(),
            applying: Cell::new(false),
        });
        let weak = Rc::downgrade(&sync);
        sync.store.changed.connect(move |note_id| {
            if let Some(sync) = weak.upgrade() {
                sync.on_store_changed(note_id);
            }
        });
        let weak = Rc::downgrade(&sync);
        sync.settings.changed.connect(move |key| {
            if key == "sync_folder" {
                if let Some(sync) = weak.upgrade() {
                    FolderSync::start(&sync);
                }
            }
        });
        FolderSync::start(&sync);
        sync
    }

    fn folder(&self) -> Option<PathBuf> {
        let value = self.settings.string("sync_folder");
        if value.is_empty() {
            return None;
        }
        let path = match value.strip_prefix("~/") {
            Some(rest) => crate::paths::home_dir().join(rest),
            None => PathBuf::from(&value),
        };
        path.is_dir().then_some(path)
    }

    fn start(sync: &Rc<FolderSync>) {
        if let Some(monitor) = sync.monitor.borrow_mut().take() {
            monitor.cancel();
        }
        let Some(folder) = sync.folder() else {
            return;
        };
        let _ = fs::create_dir_all(folder.join(ATTACHMENTS));
        sync.pull();
        sync.push_all();
        let monitor = gio::File::for_path(&folder)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>);
        if let Ok(monitor) = monitor {
            let weak: Weak<FolderSync> = Rc::downgrade(sync);
            monitor.connect_changed(move |_, _, _, _| {
                if let Some(sync) = weak.upgrade() {
                    let again = Rc::downgrade(&sync);
                    sync.pull_timer.start(900, move || {
                        if let Some(sync) = again.upgrade() {
                            sync.pull();
                        }
                    });
                }
            });
            *sync.monitor.borrow_mut() = Some(monitor);
        }
    }

    fn on_store_changed(self: &Rc<Self>, note_id: &str) {
        if self.applying.get() || self.folder().is_none() {
            return;
        }
        self.pending.borrow_mut().insert(note_id.to_string());
        if !self.push_timer.active() {
            let weak = Rc::downgrade(self);
            self.push_timer.start(800, move || {
                if let Some(sync) = weak.upgrade() {
                    sync.flush_push();
                }
            });
        }
    }

    fn flush_push(&self) {
        let pending: HashSet<String> = std::mem::take(&mut *self.pending.borrow_mut());
        if pending.contains("") {
            self.push_all();
            return;
        }
        for note_id in pending {
            if let Some(note) = self.store.raw(&note_id) {
                self.push_note(&note);
            }
        }
    }

    fn write(&self, path: &Path, text: &str) {
        let data = text.as_bytes();
        let digest = sha256_hex(data);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if self.written.borrow().get(&name) == Some(&digest) && path.exists() {
            return;
        }
        if fs::read(path).is_ok_and(|existing| existing == data) {
            self.written.borrow_mut().insert(name, digest);
            return;
        }
        match atomic_write(path, data) {
            Ok(()) => {
                self.written.borrow_mut().insert(name, digest);
            }
            Err(error) => eprintln!("ledge sync: cannot write {}: {error}", path.display()),
        }
    }

    fn push_note(&self, note: &Note) {
        let Some(folder) = self.folder() else {
            return;
        };
        if note.locked {
            return;
        }
        self.write(&folder.join(format!("{}.{NOTE_EXT}", note.id)), &serialize_note(note));
        if note.deleted_at.is_some() {
            return;
        }
        for digest in attachments::ids_in(&note.body) {
            let Some(extension) = self.attachments.extension(&digest) else {
                continue;
            };
            let target = folder.join(ATTACHMENTS).join(format!("{digest}.{extension}"));
            if !target.exists() {
                if let Some(data) = self.attachments.data(&digest) {
                    let _ = atomic_write(&target, &data);
                }
            }
        }
    }

    pub fn push_all(&self) {
        let Some(folder) = self.folder() else {
            return;
        };
        for note in self.store.all_notes() {
            self.push_note(&note);
        }
        for group in self.store.all_groups() {
            self.write(&folder.join(format!("{}.{GROUP_EXT}", group.id)), &serialize_group(&group));
        }
    }

    fn read(&self, path: &Path) -> Option<String> {
        let data = fs::read(path).ok()?;
        let digest = sha256_hex(&data);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if self.written.borrow().get(&name) == Some(&digest) {
            return None;
        }
        self.written.borrow_mut().insert(name, digest);
        Some(String::from_utf8_lossy(&data).into_owned())
    }

    pub fn pull(&self) {
        let Some(folder) = self.folder() else {
            return;
        };
        let mut changed = false;
        self.applying.set(true);
        for path in files_with_extension(&folder, GROUP_EXT) {
            if let Some(group) = self.read(&path).as_deref().and_then(parse_group) {
                changed |= self.store.put_group(group);
            }
        }
        for path in files_with_extension(&folder, NOTE_EXT) {
            let Some(incoming) = self.read(&path).as_deref().and_then(parse_note) else {
                continue;
            };
            if let Some(deleted) = incoming.deleted_at {
                changed |= self.store.apply_remote_delete(&incoming.id, deleted);
                continue;
            }
            let body = incoming.body.clone();
            changed |= self.store.upsert_from_sync(incoming);
            import_attachments(&self.attachments, &folder, &body);
        }
        self.applying.set(false);
        if changed {
            self.store.emit("");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;
    use chrono::Duration;

    #[test]
    fn note_round_trip() {
        let note = Note {
            title: "Line\\one\ntwo".into(),
            body: "---\nbody\n".into(),
            tags: vec!["a".into(), "b".into()],
            color: 5,
            keep_on_deck: true,
            daily_day: Some("2026-11-06".into()),
            group_id: Some("ABC".into()),
            ..Note::default()
        };
        let parsed = parse_note(&serialize_note(&note)).unwrap();
        assert_eq!(parsed.title, note.title);
        assert_eq!(parsed.body, note.body);
        assert_eq!(parsed.tags, vec!["a", "b"]);
        assert_eq!(parsed.color, 5);
        assert!(parsed.keep_on_deck);
        assert_eq!(parsed.daily_day.as_deref(), Some("2026-11-06"));
        assert_eq!(parsed.group_id.as_deref(), Some("ABC"));
        assert_eq!(parsed.updated_at, note.updated_at);
    }

    #[test]
    fn reads_a_mac_file() {
        let text = "--- keepnote ---\nschema: 1\nid: 0F8A6B9E-1D1C-4E1B-9D2F-6C3A1B2C3D4E\ntitle: Lisbon\ncolor: 5\n\
                    state: archived\ntags: trip\ncreated: 2026-10-01T10:00:00.000Z\nupdated: 2026-10-02T10:00:00.000Z\n\
                    ---\n\nFri 11 – Mon 14 Jun";
        let note = parse_note(text).unwrap();
        assert_eq!(note.state, "archived");
        assert_eq!(note.body, "Fri 11 – Mon 14 Jun");
        assert!(parse_note(&text.replace("schema: 1", "schema: 1\nkind: group")).is_none());
    }

    #[test]
    fn group_round_trip() {
        let store = temp_store();
        let group = store.create_group("Work");
        assert_eq!(parse_group(&serialize_group(&group)).unwrap().name, "Work");
    }

    #[test]
    fn upsert_last_writer_wins() {
        let store = temp_store();
        let note = store.create(crate::store::Draft {
            title: "mine".into(),
            body: "local".into(),
            ..Default::default()
        });
        let same_time = Note { body: "remote".into(), ..note.clone() };
        assert!(!store.upsert_from_sync(parse_note(&serialize_note(&same_time)).unwrap()));
        let newer = Note { body: "remote".into(), updated_at: now() + Duration::seconds(5), ..note.clone() };
        assert!(store.upsert_from_sync(parse_note(&serialize_note(&newer)).unwrap()));
        assert_eq!(store.get(&note.id).unwrap().body, "remote");
    }
}
