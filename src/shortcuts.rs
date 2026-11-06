use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::compositor::has_program;
use crate::paths::home_dir;

pub const SHORTCUTS: [(&str, &str, &str); 5] = [
    ("Super+Alt+N", "new", "New note"),
    ("Super+Alt+A", "all", "All Notes"),
    ("Super+Alt+E", "archive", "Archive"),
    ("Super+Alt+Y", "daily", "Today's Daily"),
    ("Super+Ctrl+Alt+D", "focus", "Focus Deck"),
];

fn launcher() -> String {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("ledge");
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    home_dir().join(".local/bin/ledge").to_string_lossy().into_owned()
}

fn bindings(binary: &str) -> Vec<String> {
    SHORTCUTS
        .iter()
        .map(|(keys, command, _)| {
            let mut combo = keys.replace("Super", "Mod4").replace("Alt", "Mod1").replace("Ctrl", "Control");
            if let Some(last) = combo.pop() {
                combo.extend(last.to_lowercase());
            }
            format!("bindsym {combo} exec {binary} {command}")
        })
        .collect()
}

pub fn sway_snippet() -> String {
    let binary = launcher();
    let mut lines = vec!["# Ledge".to_string(), format!("exec {binary}")];
    lines.extend(bindings(&binary));
    lines.push("for_window [app_id=\"io.github.ricardoraposo.Ledge\"] floating enable".into());
    lines.join("\n") + "\n"
}

fn sway_config_path() -> PathBuf {
    home_dir().join(".config/sway/config")
}

pub fn install_sway_bindings() -> String {
    let config = sway_config_path();
    if !config.exists() {
        return "No sway config found at ~/.config/sway/config".into();
    }
    let include_dir = config.parent().map(|p| p.join("config.d")).unwrap_or_default();
    let _ = fs::create_dir_all(&include_dir);
    let binary = launcher();
    let mut lines = vec!["# Ledge".to_string()];
    lines.extend(bindings(&binary));
    lines.push("for_window [app_id=\"io.github.ricardoraposo.Ledge\"] floating enable".into());
    if fs::write(include_dir.join("ledge"), lines.join("\n") + "\n").is_err() {
        return "Could not write ~/.config/sway/config.d/ledge".into();
    }
    let text = fs::read_to_string(&config).unwrap_or_default();
    let include_line = "include ~/.config/sway/config.d/ledge";
    if !text.contains(include_line) && !text.contains("include ~/.config/sway/config.d/*") {
        if let Ok(mut file) = OpenOptions::new().append(true).open(&config) {
            let _ = writeln!(file, "\n{include_line}");
        }
    }
    if has_program("swaymsg") {
        let _ = Command::new("swaymsg").arg("reload").stdout(Stdio::null()).stderr(Stdio::null()).status();
        return "Shortcuts installed and sway reloaded".into();
    }
    "Shortcuts installed; reload sway to use them".into()
}
