use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::paths;
use crate::signals::Listeners;

fn defaults() -> Map<String, Value> {
    let value = json!({
        "edge": "right",
        "archive_after_days": 14,
        "undo_seconds": 10,
        "text_size": "medium",
        "magnification": "small",
        "daily_template": "## Today\n- [ ] ",
        "carry_over": "bring",
        "sync_folder": "",
        "show_deck_buttons": true,
        "peek": true,
        "all_notes_sort": "edited"
    });
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

pub struct Settings {
    path: Option<PathBuf>,
    values: RefCell<Map<String, Value>>,
    pub changed: Listeners,
}

impl Settings {
    pub fn load() -> Self {
        let path = paths::settings_path();
        let mut values = defaults();
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(Value::Object(stored)) = serde_json::from_str::<Value>(&text) {
                for (key, value) in stored {
                    values.insert(key, value);
                }
            }
        }
        Settings { path: Some(path), values: RefCell::new(values), changed: Listeners::default() }
    }

    #[cfg(test)]
    pub fn in_memory(pairs: &[(&str, Value)]) -> Self {
        let mut values = defaults();
        for (key, value) in pairs {
            values.insert(key.to_string(), value.clone());
        }
        Settings { path: None, values: RefCell::new(values), changed: Listeners::default() }
    }

    pub fn get(&self, key: &str) -> Value {
        self.values.borrow().get(key).cloned().or_else(|| defaults().get(key).cloned()).unwrap_or(Value::Null)
    }

    pub fn string(&self, key: &str) -> String {
        match self.get(key) {
            Value::String(text) => text,
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }

    pub fn int(&self, key: &str) -> i64 {
        match self.get(key) {
            Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|f| f as i64)).unwrap_or(0),
            Value::String(text) => text.parse().unwrap_or(0),
            Value::Bool(flag) => i64::from(flag),
            _ => 0,
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        match self.get(key) {
            Value::Bool(flag) => flag,
            Value::Number(number) => number.as_i64().unwrap_or(0) != 0,
            Value::String(text) => !text.is_empty(),
            _ => false,
        }
    }

    pub fn set(&self, key: &str, value: Value) {
        if self.values.borrow().get(key) == Some(&value) {
            return;
        }
        self.values.borrow_mut().insert(key.to_string(), value);
        if let Some(path) = &self.path {
            let text = serde_json::to_string_pretty(&Value::Object(self.values.borrow().clone())).unwrap_or_default();
            let tmp = path.with_extension("tmp");
            if fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, path)).is_err() {
                eprintln!("ledge: cannot save settings to {}", path.display());
            }
        }
        self.changed.emit(key);
    }

    pub fn right_edge(&self) -> bool {
        self.string("edge") != "left"
    }

    pub fn text_points(&self) -> f64 {
        match self.string("text_size").as_str() {
            "small" => 12.0,
            "large" => 15.0,
            "xlarge" => 17.0,
            _ => 13.5,
        }
    }

    pub fn lift(&self) -> i32 {
        match self.string("magnification").as_str() {
            "off" => 0,
            "medium" => 22,
            "large" => 34,
            _ => 12,
        }
    }
}
