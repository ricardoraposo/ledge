use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

const NAME: &str = "ledge";
const LEGACY_NAME: &str = "keepnote";

fn base(env: &str, fallback: &str, marker: &str) -> PathBuf {
    if let Some(root) = std::env::var_os("LEDGE_HOME").filter(|value| !value.is_empty()) {
        let name = PathBuf::from(fallback).file_name().map(PathBuf::from).unwrap_or_default();
        return PathBuf::from(root).join(name);
    }
    let parent = std::env::var_os(env).filter(|value| !value.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(fallback));
    let current = parent.join(NAME);
    let legacy = parent.join(LEGACY_NAME);
    if !current.exists() && legacy.join(marker).exists() {
        return legacy;
    }
    current
}

pub fn data_dir() -> PathBuf {
    let path = base("XDG_DATA_HOME", ".local/share", "notes.sqlite");
    let _ = fs::create_dir_all(&path);
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o700));
    path
}

pub fn config_dir() -> PathBuf {
    let path = base("XDG_CONFIG_HOME", ".config", "settings.json");
    let _ = fs::create_dir_all(&path);
    path
}

fn child(name: &str) -> PathBuf {
    let path = data_dir().join(name);
    let _ = fs::create_dir_all(&path);
    path
}

pub fn database_path() -> PathBuf {
    data_dir().join("notes.sqlite")
}

pub fn backups_dir() -> PathBuf {
    child("Backups")
}

pub fn attachments_dir() -> PathBuf {
    child("Attachments")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn runtime_dir() -> PathBuf {
    let root = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let path = root.join(NAME);
    let _ = fs::create_dir_all(&path);
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o700));
    path
}

pub fn isolated() -> bool {
    std::env::var_os("LEDGE_HOME").is_some_and(|value| !value.is_empty())
}

pub fn home_dir() -> PathBuf {
    home()
}
