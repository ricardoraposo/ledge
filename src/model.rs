use std::sync::LazyLock;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, SubsecRound, Utc};
use fancy_regex::Regex as FancyRegex;
use regex::Regex;

pub type Stamp = DateTime<Utc>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub paper: &'static str,
    pub spine: &'static str,
    pub accent: &'static str,
    pub secondary: &'static str,
}

pub struct PaperColor {
    pub id: i64,
    pub slug: &'static str,
    pub name: &'static str,
    pub swatch: Swatch,
}

pub const PALETTE: [PaperColor; 5] = [
    PaperColor {
        id: 1,
        slug: "butter",
        name: "Butter",
        swatch: Swatch { paper: "#F7E3A1", spine: "#EBC550", accent: "#7A5A00", secondary: "#57503D" },
    },
    PaperColor {
        id: 2,
        slug: "coral",
        name: "Coral",
        swatch: Swatch { paper: "#F6C1B1", spine: "#EE9279", accent: "#8F2F15", secondary: "#574841" },
    },
    PaperColor {
        id: 6,
        slug: "mint",
        name: "Mint",
        swatch: Swatch { paper: "#BFE5CC", spine: "#86CDA1", accent: "#1F6B45", secondary: "#495148" },
    },
    PaperColor {
        id: 5,
        slug: "sky",
        name: "Sky",
        swatch: Swatch { paper: "#BCD9F2", spine: "#86BCEB", accent: "#1F5C99", secondary: "#484E52" },
    },
    PaperColor {
        id: 4,
        slug: "lilac",
        name: "Lilac",
        swatch: Swatch { paper: "#D8CDF2", spine: "#B09EEA", accent: "#5B3FA8", secondary: "#4F4B52" },
    },
];

pub const COLOR_ORDER: [i64; 5] = [1, 2, 6, 5, 4];
pub const DEFAULT_COLOR: i64 = 1;

pub fn resolve_color(value: i64) -> i64 {
    if PALETTE.iter().any(|color| color.id == value) {
        return value;
    }
    match value {
        0 => 1,
        3 => 2,
        7 => 6,
        _ => DEFAULT_COLOR,
    }
}

pub fn resolve_color_text(raw: Option<&str>) -> i64 {
    raw.and_then(|text| text.trim().parse::<i64>().ok()).map(resolve_color).unwrap_or(DEFAULT_COLOR)
}

fn paper(color: i64) -> &'static PaperColor {
    let id = resolve_color(color);
    PALETTE.iter().find(|entry| entry.id == id).unwrap_or(&PALETTE[0])
}

pub fn color_slug(color: i64) -> &'static str {
    paper(color).slug
}

pub fn color_name(color: i64) -> &'static str {
    paper(color).name
}

pub fn swatch(color: i64) -> Swatch {
    paper(color).swatch
}

pub fn next_color(color: i64, step: i64) -> i64 {
    let current = resolve_color(color);
    let index = COLOR_ORDER.iter().position(|id| *id == current).unwrap_or(0) as i64;
    let count = COLOR_ORDER.len() as i64;
    COLOR_ORDER[(index + step).rem_euclid(count) as usize]
}

pub fn now() -> Stamp {
    Utc::now().trunc_subsecs(3)
}

pub fn today() -> String {
    Local::now().date_naive().format("%Y-%m-%d").to_string()
}

pub fn today_date() -> NaiveDate {
    Local::now().date_naive()
}

pub fn iso(value: &Stamp) -> String {
    value.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn iso_opt(value: &Option<Stamp>) -> Option<String> {
    value.as_ref().map(iso)
}

pub fn parse_iso(value: Option<&str>) -> Option<Stamp> {
    let text = value?.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&Utc));
    }
    if let Ok(parsed) = DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f%:z") {
        return Some(parsed.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(text, format) {
            return Some(parsed.and_utc());
        }
    }
    None
}

pub fn parse_day(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().map(|day| day.format("%Y-%m-%d").to_string())
}

pub fn parse_tags(text: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || c == ',') {
        let tag = raw.trim().trim_start_matches('#').to_lowercase();
        if !tag.is_empty() && !seen.contains(&tag) {
            seen.push(tag);
        }
    }
    seen
}

static INLINE_MARKUP: LazyLock<Vec<(FancyRegex, &'static str)>> = LazyLock::new(|| {
    [
        (r"!\[([^\]]*)\]\([^)]*\)", "$1"),
        (r"\[([^\]]+)\]\([^)]*\)", "$1"),
        (r"(\*\*|__)(.+?)\1", "$2"),
        (r"(?<![\w*])\*(?!\s)(.+?)(?<!\s)\*(?!\w)", "$1"),
        (r"(?<![\w_])_(?!\s)(.+?)(?<!\s)_(?!\w)", "$1"),
        (r"~~(.+?)~~", "$1"),
        (r"==(.+?)==", "$1"),
        (r"`([^`]+)`", "$1"),
    ]
    .into_iter()
    .map(|(pattern, replacement)| (FancyRegex::new(pattern).expect("inline markup pattern"), replacement))
    .collect()
});

static BLOCK_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(#{1,6}\s+|>\s?|[-*+]\s+\[[ xX]\]\s+|[-*+]\s+|\d+[.)]\s+)").unwrap());

pub fn plain_line(line: &str) -> String {
    let mut text = BLOCK_PREFIX.replace(line, "").into_owned();
    for (pattern, replacement) in INLINE_MARKUP.iter() {
        text = pattern.replace_all(&text, *replacement).into_owned();
    }
    text.trim().to_string()
}

pub fn is_divider(line: &str) -> bool {
    let compact: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3 && matches!(compact[0], '-' | '*' | '_') && compact.iter().all(|c| *c == compact[0])
}

pub fn plain_text(body: &str) -> String {
    let mut lines = Vec::new();
    let mut in_code = false;
    for line in body.lines() {
        if line.trim().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if !in_code && is_divider(line) {
            continue;
        }
        lines.push(if in_code { line.to_string() } else { plain_line(line) });
    }
    lines.join("\n")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    pub color: i64,
    pub state: String,
    pub sort_index: i64,
    pub tags: Vec<String>,
    pub created_at: Stamp,
    pub updated_at: Stamp,
    pub edited_at: Stamp,
    pub deleted_at: Option<Stamp>,
    pub daily_day: Option<String>,
    pub daily_kept: bool,
    pub keep_on_deck: bool,
    pub pinned_at: Option<Stamp>,
    pub last_opened_day: Option<String>,
    pub auto_archived_day: Option<String>,
    pub group_id: Option<String>,
    pub floating: bool,
    pub float_x: Option<i64>,
    pub float_y: Option<i64>,
    pub float_w: Option<i64>,
    pub float_h: Option<i64>,
    pub locked: bool,
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

impl Default for Note {
    fn default() -> Self {
        let stamp = now();
        Note {
            id: new_id(),
            title: String::new(),
            body: String::new(),
            color: DEFAULT_COLOR,
            state: "active".into(),
            sort_index: 0,
            tags: Vec::new(),
            created_at: stamp,
            updated_at: stamp,
            edited_at: stamp,
            deleted_at: None,
            daily_day: None,
            daily_kept: false,
            keep_on_deck: false,
            pinned_at: None,
            last_opened_day: None,
            auto_archived_day: None,
            group_id: None,
            floating: false,
            float_x: None,
            float_y: None,
            float_w: None,
            float_h: None,
            locked: false,
        }
    }
}

impl Note {
    pub fn is_daily(&self) -> bool {
        self.daily_day.is_some()
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned_at.is_some()
    }

    pub fn is_archived(&self) -> bool {
        self.state == "archived"
    }

    pub fn is_active(&self) -> bool {
        self.state == "active"
    }

    pub fn display_title(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }
        for line in self.body.lines() {
            let text = plain_line(line);
            if !text.is_empty() {
                return text;
            }
        }
        "New Note".into()
    }

    pub fn preview(&self) -> String {
        let title = self.display_title();
        let mut parts: Vec<String> = Vec::new();
        for line in plain_text(&self.body).lines() {
            let text = line.trim();
            if text.is_empty() || (parts.is_empty() && text == title) {
                continue;
            }
            parts.push(text.to_string());
            if parts.iter().map(|part| part.chars().count()).sum::<usize>() > 160 {
                break;
            }
        }
        parts.join(" · ")
    }

    pub fn spine_label(&self) -> String {
        let label = self.display_title();
        if label.chars().count() > 18 {
            let short: String = label.chars().take(16).collect();
            return format!("{}…", short.trim_end());
        }
        label
    }

    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let needle = query.to_lowercase();
        if let Some(tag) = needle.strip_prefix('#') {
            return self.tags.iter().any(|t| t == tag);
        }
        let haystack = format!("{} {} {}", self.title, self.tags.join(" "), self.body).to_lowercase();
        needle.split_whitespace().all(|word| haystack.contains(word))
    }

    pub fn copy_text(&self) -> String {
        let head = if self.title.trim().is_empty() { String::new() } else { format!("# {}\n\n", self.title) };
        head + &self.body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(parse_tags("#Trip, work  #trip"), vec!["trip", "work"]);
    }

    #[test]
    fn colors() {
        assert_eq!(resolve_color(0), 1);
        assert_eq!(resolve_color(99), 1);
        assert_eq!(next_color(1, 1), 2);
        assert_eq!(next_color(4, 1), 1);
        assert_eq!(next_color(1, -1), 4);
    }

    #[test]
    fn titles() {
        let note = Note { body: "## Groceries\n- [ ] eggs".into(), ..Note::default() };
        assert_eq!(note.display_title(), "Groceries");
        assert_eq!(note.preview(), "eggs");
        assert_eq!(plain_text("**a** [b](c)"), "a b");
        assert_eq!(plain_line("_x_ and *y*"), "x and y");
    }

    #[test]
    fn iso_round_trip() {
        let stamp = now();
        assert_eq!(parse_iso(Some(&iso(&stamp))), Some(stamp));
        assert!(parse_iso(Some("2026-10-01T10:00:00.000Z")).is_some());
        assert!(parse_iso(Some("2026-10-01T10:00:00+00:00")).is_some());
    }
}
