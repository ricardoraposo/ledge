use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::model::today;
use crate::paths;
use crate::store::Store;

const KEEP: usize = 7;

pub fn list_backups() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(paths::backups_dir())
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| name.starts_with("notes-") && name.ends_with(".sqlite"))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found.reverse();
    found
}

pub fn backup_now(store: &Store, label: &str) -> Result<PathBuf, String> {
    let stamp = Local::now().format("%Y-%m-%d-%H%M%S");
    let target = paths::backups_dir().join(format!("notes-{stamp}-{label}.sqlite"));
    store.backup_to(&target)?;
    let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o600));
    prune();
    Ok(target)
}

fn prune() {
    for old in list_backups().into_iter().skip(KEEP) {
        let _ = fs::remove_file(old);
    }
}

pub fn daily_backup(store: &Store) -> Result<Option<PathBuf>, String> {
    let day = today();
    if store.meta("last_backup_day").as_deref() == Some(day.as_str()) {
        return Ok(None);
    }
    let path = backup_now(store, "daily")?;
    store.set_meta("last_backup_day", &day);
    Ok(Some(path))
}

pub fn restore(store: &Store, path: &Path) -> Result<(), String> {
    backup_now(store, "before-restore")?;
    store.restore_from(path)
}
