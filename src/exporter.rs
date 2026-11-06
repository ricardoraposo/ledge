use std::fs;
use std::path::{Path, PathBuf};

use crate::attachments::{self, Attachments};
use crate::model::Note;
use crate::store::{Draft, Store};
use crate::sync::{self, GROUP_EXT, NOTE_EXT, parse_group, parse_note};

pub fn safe_name(text: &str, fallback: &str) -> String {
    let mut cleaned = String::new();
    let mut in_gap = false;
    for character in text.chars() {
        let unsafe_char = matches!(character, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (character as u32) < 0x20;
        if unsafe_char {
            if !in_gap {
                cleaned.push(' ');
            }
            in_gap = true;
        } else {
            cleaned.push(character);
            in_gap = false;
        }
    }
    let trimmed = cleaned.trim().trim_matches('.');
    let name: String = trimmed.chars().take(80).collect();
    if name.is_empty() { fallback.to_string() } else { name }
}

pub fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Note").to_string();
    let suffix = path.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    let mut index = 2;
    loop {
        let candidate = path.with_file_name(format!("{stem} {index}{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
        index += 1;
    }
}

pub fn note_markdown(note: &Note) -> String {
    let head = if note.title.trim().is_empty() { String::new() } else { format!("# {}\n\n", note.title) };
    let tags = if note.tags.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", note.tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" "))
    };
    format!("{head}{}{tags}\n", note.body)
}

pub fn export_notes(attachments: &Attachments, notes: &[Note], folder: &Path) -> Vec<PathBuf> {
    let mut written = Vec::new();
    for note in notes {
        let directory = unique(folder.join(safe_name(&note.display_title(), "Note")));
        if fs::create_dir_all(&directory).is_err() {
            continue;
        }
        let mut body = note_markdown(note);
        let digests = attachments::ids_in(&body);
        if !digests.is_empty() {
            let _ = fs::create_dir_all(directory.join("attachments"));
        }
        for digest in digests {
            let (Some(data), Some(extension)) = (attachments.data(&digest), attachments.extension(&digest)) else {
                continue;
            };
            let file = format!("{digest}.{extension}");
            if fs::write(directory.join("attachments").join(&file), data).is_ok() {
                body = body.replace(&format!("attachment:{digest}"), &format!("attachments/{file}"));
            }
        }
        let name = directory.file_name().and_then(|n| n.to_str()).unwrap_or("Note").to_string();
        let target = directory.join(format!("{name}.md"));
        if fs::write(&target, body).is_ok() {
            written.push(target);
        }
    }
    written
}

pub fn export_plain(notes: &[Note], target: &Path) -> std::io::Result<()> {
    let parts: Vec<String> = notes
        .iter()
        .map(|note| {
            let title = note.display_title();
            let rule = "=".repeat(title.chars().count().min(60));
            format!("{title}\n{rule}\n\n{}", note.body.trim())
        })
        .collect();
    fs::write(target, parts.join("\n\n\n") + "\n")
}

pub fn import_folder(store: &Store, attachments: &Attachments, folder: &Path) -> (usize, usize) {
    let mut notes = 0;
    let mut groups = 0;
    for path in sync::files_with_extension(folder, GROUP_EXT) {
        let text = String::from_utf8_lossy(&fs::read(&path).unwrap_or_default()).into_owned();
        if let Some(group) = parse_group(&text) {
            if group.deleted_at.is_none() && !store.has_group(&group.id) {
                store.put_group(group);
                groups += 1;
            }
        }
    }
    for path in sync::files_with_extension(folder, NOTE_EXT) {
        let text = String::from_utf8_lossy(&fs::read(&path).unwrap_or_default()).into_owned();
        let Some(note) = parse_note(&text) else {
            continue;
        };
        if note.deleted_at.is_some() {
            continue;
        }
        let body = note.body.clone();
        if store.upsert_from_sync(note) {
            notes += 1;
        }
        sync::import_attachments(attachments, folder, &body);
    }
    for path in sync::files_with_extension(folder, "md") {
        let mut text = String::from_utf8_lossy(&fs::read(&path).unwrap_or_default()).into_owned();
        let mut title = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Note").to_string();
        if let Some(rest) = text.strip_prefix("# ") {
            let (first, remainder) = rest.split_once('\n').unwrap_or((rest, ""));
            title = first.trim().to_string();
            text = remainder.to_string();
        }
        store.create(Draft { title, body: text.trim_matches('\n').to_string(), ..Draft::default() });
        notes += 1;
    }
    store.emit("");
    (notes, groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe() {
        assert_eq!(safe_name("a/b: c?", "Note"), "a b  c");
        assert_eq!(safe_name("...", "Note"), "Note");
    }
}
