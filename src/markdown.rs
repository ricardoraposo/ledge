use std::sync::LazyLock;

use fancy_regex::Regex as FancyRegex;
use regex::Regex;

pub const BULLET: &str = "•";
pub const OPEN_BOX: &str = "☐";
pub const DONE_BOX: &str = "☑";
pub const INDENT: &str = "    ";

macro_rules! pattern {
    ($name:ident, $source:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($source).unwrap());
    };
}

pattern!(FENCE, r"^\s*```");
pattern!(MD_CHECK, r"^(\s*)[-*+] \[([ xX])\](?: |$)");
pattern!(MD_BULLET, r"^(\s*)[-*+] ");
pattern!(DISPLAY_ITEM, r"^(\s*)([•☐☑])(?: |$)");
pattern!(LIST_LINE, r"^(\s*)(?:([•☐☑])|(\d+)([.)]))( |$)");
pattern!(QUOTE_LINE, r"^(\s*)> ?");
pattern!(HEADING, r"^(#{1,6})( +)");
pattern!(TABLE_ROW, r"^\s*\|.*\|\s*$");
pattern!(IMAGE_LINE, r"^[ \t]*(!\[)([^\]\n]*)(\])(\(attachment:([0-9a-f]{64})\))[ \t]*$");
pattern!(LINK, r"(\[)([^\]\n]+)(\]\()([^)\s]+)(\))");

static BARE_URL: LazyLock<FancyRegex> =
    LazyLock::new(|| FancyRegex::new(r#"(?<![(\w])(https?://[^\s<>()]+[^\s<>().,;:!?'"])"#).unwrap());

static INLINE_RULES: LazyLock<Vec<(&'static str, FancyRegex)>> = LazyLock::new(|| {
    [
        ("code", r"(`)([^`\n]+)(`)"),
        ("bold", r"(\*\*|__)(?=\S)(.+?)(?<=\S)(\1)"),
        ("italic", r"(?<![*\w])(\*)(?=[^\s*])(.+?)(?<=[^\s*])(\*)(?![*\w])"),
        ("italic", r"(?<![_\w])(_)(?=[^\s_])(.+?)(?<=[^\s_])(_)(?![_\w])"),
        ("strike", r"(~~)(?=\S)(.+?)(?<=\S)(~~)"),
        ("highlight", r"(==)(?=\S)(.+?)(?<=\S)(==)"),
    ]
    .into_iter()
    .map(|(tag, source)| (tag, FancyRegex::new(source).unwrap()))
    .collect()
});

pub fn image_digest(line: &str) -> Option<String> {
    IMAGE_LINE.captures(line).map(|caps| caps[5].to_string())
}

pub fn to_display(markdown: &str) -> String {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in markdown.split('\n') {
        if FENCE.is_match(line) {
            in_code = !in_code;
            out.push(line.to_string());
            continue;
        }
        let mut converted = line.to_string();
        if !in_code {
            if let Some(caps) = MD_CHECK.captures(line) {
                let mark = if &caps[2] == " " { OPEN_BOX } else { DONE_BOX };
                let end = caps.get(0).unwrap().end();
                converted = format!("{}{} {}", &caps[1], mark, &line[end..]);
            } else if let Some(caps) = MD_BULLET.captures(line) {
                let end = caps.get(0).unwrap().end();
                converted = format!("{}{} {}", &caps[1], BULLET, &line[end..]);
            }
        }
        out.push(converted);
    }
    out.join("\n")
}

pub fn to_markdown(display: &str) -> String {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in display.split('\n') {
        if FENCE.is_match(line) {
            in_code = !in_code;
            out.push(line.to_string());
            continue;
        }
        let mut converted = line.to_string();
        if !in_code {
            if let Some(caps) = DISPLAY_ITEM.captures(line) {
                let marker = match &caps[2] {
                    BULLET => "-",
                    OPEN_BOX => "- [ ]",
                    _ => "- [x]",
                };
                let end = caps.get(0).unwrap().end();
                converted = format!("{}{} {}", &caps[1], marker, &line[end..]);
            }
        }
        out.push(converted);
    }
    out.join("\n")
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub indent: String,
    pub marker: String,
    pub number: Option<i64>,
    pub delimiter: String,
    pub prefix_len: usize,
    pub content: String,
}

impl ListItem {
    pub fn is_check(&self) -> bool {
        self.marker == OPEN_BOX || self.marker == DONE_BOX
    }
}

pub fn parse_list_line(line: &str) -> Option<ListItem> {
    let caps = LIST_LINE.captures(line)?;
    let end = caps.get(0).unwrap().end();
    let indent = caps[1].to_string();
    let prefix_len = line[..end].chars().count();
    let content = line[end..].to_string();
    if let Some(glyph) = caps.get(2) {
        return Some(ListItem {
            indent,
            marker: glyph.as_str().to_string(),
            number: None,
            delimiter: String::new(),
            prefix_len,
            content,
        });
    }
    let number = caps[3].parse::<i64>().ok()?;
    let delimiter = caps[4].to_string();
    Some(ListItem { indent, marker: format!("{}{}", &caps[3], delimiter), number: Some(number), delimiter, prefix_len, content })
}

pub fn continuation(line: &str) -> Option<String> {
    if let Some(item) = parse_list_line(line) {
        if let Some(number) = item.number {
            return Some(format!("{}{}{} ", item.indent, number + 1, item.delimiter));
        }
        let marker = if item.is_check() { OPEN_BOX.to_string() } else { item.marker.clone() };
        return Some(format!("{}{} ", item.indent, marker));
    }
    if let Some(caps) = QUOTE_LINE.captures(line) {
        if line.trim() != ">" {
            return Some(format!("{}> ", &caps[1]));
        }
    }
    None
}

pub fn is_empty_item(line: &str) -> bool {
    if let Some(item) = parse_list_line(line) {
        return item.content.trim().is_empty();
    }
    match QUOTE_LINE.find(line) {
        Some(found) => line[found.end()..].trim().is_empty(),
        None => false,
    }
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

pub fn toggled_check(line: &str) -> String {
    if let Some(item) = parse_list_line(line) {
        if item.is_check() {
            let marker = if item.marker == OPEN_BOX { DONE_BOX } else { OPEN_BOX };
            return format!("{}{} {}", item.indent, marker, item.content);
        }
        return format!("{}{} {}", item.indent, OPEN_BOX, item.content);
    }
    format!("{}{} {}", leading_whitespace(line), OPEN_BOX, line.trim_start())
}

pub fn set_block(line: &str, kind: &str) -> String {
    let item = parse_list_line(line);
    let mut body: String = match &item {
        Some(item) => item.content.clone(),
        None => line.trim_start().to_string(),
    };
    let indent: String = match &item {
        Some(item) => item.indent.clone(),
        None => leading_whitespace(line).to_string(),
    };
    if let Some(found) = HEADING.find(&body) {
        body = body[found.end()..].to_string();
    }
    if item.is_none() {
        if let Some(found) = QUOTE_LINE.find(&body) {
            body = body[found.end()..].to_string();
        }
    }
    if let Some(level) = kind.strip_prefix('h').and_then(|digit| digit.parse::<usize>().ok()) {
        if let Some(caps) = HEADING.captures(line.trim_start()) {
            if caps[1].len() == level {
                return indent + &body;
            }
        }
        return format!("{}{} {}", indent, "#".repeat(level), body);
    }
    match kind {
        "bullet" => match &item {
            Some(item) if item.marker == BULLET => indent + &body,
            _ => format!("{indent}{BULLET} {body}"),
        },
        "check" => match &item {
            Some(item) if item.is_check() => indent + &body,
            _ => format!("{indent}{OPEN_BOX} {body}"),
        },
        "number" => match &item {
            Some(item) if item.number.is_some() => indent + &body,
            _ => format!("{indent}1. {body}"),
        },
        "quote" => {
            if QUOTE_LINE.is_match(line) {
                indent + &body
            } else {
                format!("{indent}> {body}")
            }
        }
        _ => line.to_string(),
    }
}

pub fn renumber(lines: &[String], start: usize) -> Vec<String> {
    let mut out = lines.to_vec();
    let Some(first) = out.get(start).and_then(|line| parse_list_line(line)) else {
        return out;
    };
    if first.number.is_none() {
        return out;
    }
    let depth = first.indent.chars().count();
    let mut top = start;
    while top > 0 {
        match parse_list_line(&out[top - 1]) {
            Some(above) if above.indent.chars().count() >= depth => top -= 1,
            _ => break,
        }
    }
    let mut counter: Option<i64> = None;
    let mut index = top;
    while index < out.len() {
        let Some(item) = parse_list_line(&out[index]) else {
            break;
        };
        let item_depth = item.indent.chars().count();
        if item_depth == depth {
            let Some(number) = item.number else {
                break;
            };
            match counter {
                None => counter = Some(number),
                Some(previous) => {
                    let next = previous + 1;
                    counter = Some(next);
                    out[index] = format!("{}{}{} {}", item.indent, next, item.delimiter, item.content);
                }
            }
        } else if item_depth < depth {
            break;
        }
        index += 1;
    }
    out
}

pub fn unfinished_tasks(markdown: &str) -> Vec<String> {
    markdown
        .split('\n')
        .filter_map(|line| {
            let caps = MD_CHECK.captures(line)?;
            if &caps[2] != " " {
                return None;
            }
            let rest = line[caps.get(0).unwrap().end()..].trim();
            (!rest.is_empty()).then(|| rest.to_string())
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub tag: &'static str,
    pub data: Option<String>,
}

fn span(start: usize, end: usize, tag: &'static str) -> Span {
    Span { start, end, tag, data: None }
}

fn char_map(line: &str) -> Vec<usize> {
    let mut map = vec![0; line.len() + 1];
    let mut count = 0;
    for (byte, _) in line.char_indices() {
        map[byte] = count;
        count += 1;
    }
    map[line.len()] = count;
    map
}

pub fn style_spans(text: &str, cursor_line: Option<usize>) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut in_code = false;
    let lines: Vec<&str> = text.split('\n').collect();
    for (number, line) in lines.iter().enumerate() {
        let is_cursor = cursor_line == Some(number);
        let length = line.chars().count();
        let end = offset + length;
        let image = if in_code { None } else { IMAGE_LINE.captures(line) };
        if let Some(caps) = image {
            let map = char_map(line);
            spans.push(Span { start: offset, end, tag: "image", data: Some(caps[5].to_string()) });
            if is_cursor || number == lines.len() - 1 {
                let open = caps.get(1).unwrap();
                let close = caps.get(3).unwrap();
                let target = caps.get(4).unwrap();
                spans.push(span(offset + map[open.start()], offset + map[close.end()], "syntax"));
                spans.push(span(offset + map[target.start()], offset + map[target.end()], "hidden"));
            } else {
                spans.push(span(offset, end, "hidden"));
            }
        } else if FENCE.is_match(line) {
            in_code = !in_code;
            spans.push(span(offset, end, "codeblock"));
            spans.push(span(offset, end, "syntax"));
        } else if in_code {
            spans.push(span(offset, end, "codeblock"));
        } else {
            spans.extend(line_spans(line, offset, is_cursor));
        }
        offset = end + 1;
    }
    spans
}

fn line_spans(line: &str, offset: usize, is_cursor: bool) -> Vec<Span> {
    let mut spans = Vec::new();
    let map = char_map(line);
    let length = map[line.len()];
    let hide: &'static str = if is_cursor { "syntax" } else { "hidden" };
    if crate::model::is_divider(line) {
        return vec![span(offset, offset + length, "hr")];
    }
    if TABLE_ROW.is_match(line) {
        spans.push(span(offset, offset + length, "table"));
        for (byte, _) in line.match_indices('|') {
            spans.push(span(offset + map[byte], offset + map[byte] + 1, "syntax"));
        }
        return spans;
    }
    let mut content_start = 0;
    if let Some(caps) = HEADING.captures(line) {
        let level = caps[1].len().min(3);
        let tag = match level {
            1 => "h1",
            2 => "h2",
            _ => "h3",
        };
        let heading_end = caps.get(0).unwrap().end();
        spans.push(span(offset, offset + length, tag));
        spans.push(span(offset, offset + map[heading_end], hide));
        content_start = heading_end;
    } else {
        if let Some(found) = QUOTE_LINE.find(line) {
            spans.push(span(offset, offset + length, "quote"));
            spans.push(span(offset + map[found.start()], offset + map[found.end()], "syntax"));
            content_start = found.end();
        }
        if let Some(item) = parse_list_line(line) {
            let indent_bytes = item.indent.len();
            let marker_bytes = item.marker.len();
            let marker_start = offset + map[indent_bytes];
            let marker_end = offset + map[indent_bytes + marker_bytes];
            if item.is_check() {
                let gap = usize::from(line[indent_bytes + marker_bytes..].starts_with(' '));
                spans.push(span(marker_start, marker_end + gap, "checkbox"));
                if item.marker == DONE_BOX {
                    spans.push(span(offset + item.prefix_len, offset + length, "done"));
                }
            } else {
                spans.push(span(marker_start, marker_end, "marker"));
            }
            content_start = line.char_indices().nth(item.prefix_len).map(|(byte, _)| byte).unwrap_or(line.len());
        }
    }
    let body = &line[content_start..];
    let at = |byte: usize| offset + map[content_start + byte];
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for caps in LINK.captures_iter(body) {
        let whole = caps.get(0).unwrap();
        let label = caps.get(2).unwrap();
        taken.push((whole.start(), whole.end()));
        spans.push(span(at(whole.start()), at(label.start()), hide));
        spans.push(Span { start: at(label.start()), end: at(label.end()), tag: "link", data: Some(caps[4].to_string()) });
        spans.push(span(at(label.end()), at(whole.end()), hide));
    }
    for found in BARE_URL.find_iter(body).flatten() {
        if taken.iter().any(|(a, b)| *a <= found.start() && found.start() < *b) {
            continue;
        }
        spans.push(Span {
            start: at(found.start()),
            end: at(found.end()),
            tag: "link",
            data: Some(found.as_str().to_string()),
        });
        taken.push((found.start(), found.end()));
    }
    let mut code_ranges: Vec<(usize, usize)> = Vec::new();
    for (tag, rule) in INLINE_RULES.iter() {
        for caps in rule.captures_iter(body).flatten() {
            let whole = caps.get(0).unwrap();
            if taken.iter().chain(code_ranges.iter()).any(|(a, b)| *a <= whole.start() && whole.start() < *b) {
                continue;
            }
            let open = caps.get(1).unwrap();
            let close = caps.get(3).unwrap();
            spans.push(span(at(open.start()), at(open.end()), hide));
            spans.push(span(at(open.end()), at(close.start()), tag));
            spans.push(span(at(close.start()), at(close.end()), hide));
            if *tag == "code" {
                code_ranges.push((whole.start(), whole.end()));
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lists_become_glyphs_and_back() {
        let source = "- one\n- [ ] todo\n- [x] done\n    - nested\n1. first";
        let shown = to_display(source);
        assert_eq!(shown, "• one\n☐ todo\n☑ done\n    • nested\n1. first");
        assert_eq!(to_markdown(&shown), source);
    }

    #[test]
    fn code_blocks_are_left_alone() {
        assert_eq!(to_display("```\n- not a list\n```\n- list"), "```\n- not a list\n```\n• list");
    }

    #[test]
    fn empty_checkbox_at_end_of_line() {
        assert_eq!(to_display("- [ ]"), "☐ ");
    }

    #[test]
    fn continuation_rules() {
        assert_eq!(continuation("• milk").as_deref(), Some("• "));
        assert_eq!(continuation("☑ done").as_deref(), Some("☐ "));
        assert_eq!(continuation("  3. three").as_deref(), Some("  4. "));
        assert_eq!(continuation("> quote").as_deref(), Some("> "));
        assert_eq!(continuation("plain"), None);
    }

    #[test]
    fn empty_item_ends_list() {
        assert!(is_empty_item("• "));
        assert!(is_empty_item("4. "));
        assert!(!is_empty_item("• x"));
    }

    #[test]
    fn toggle() {
        assert_eq!(toggled_check("☐ task"), "☑ task");
        assert_eq!(toggled_check("☑ task"), "☐ task");
        assert_eq!(toggled_check("• task"), "☐ task");
        assert_eq!(toggled_check("task"), "☐ task");
    }

    #[test]
    fn set_block_toggles() {
        assert_eq!(set_block("hello", "h2"), "## hello");
        assert_eq!(set_block("## hello", "h2"), "hello");
        assert_eq!(set_block("• hello", "check"), "☐ hello");
        assert_eq!(set_block("hello", "quote"), "> hello");
    }

    #[test]
    fn renumber_list() {
        let lines = strings(&["1. a", "2. b", "2. new", "3. c", "after"]);
        assert_eq!(renumber(&lines, 2), strings(&["1. a", "2. b", "3. new", "4. c", "after"]));
    }

    #[test]
    fn unfinished() {
        assert_eq!(unfinished_tasks("- [ ] a\n- [x] b\n- [ ] \n  - [ ] c"), strings(&["a", "c"]));
    }

    fn tags(text: &str, cursor: Option<usize>) -> Vec<(String, &'static str)> {
        let chars: Vec<char> = text.chars().collect();
        style_spans(text, cursor).into_iter().map(|s| (chars[s.start..s.end].iter().collect(), s.tag)).collect()
    }

    #[test]
    fn inline_hides_markers_off_the_cursor_line() {
        let spans = tags("a **bold** b", None);
        assert!(spans.contains(&("bold".into(), "bold")));
        assert!(spans.contains(&("**".into(), "hidden")));
    }

    #[test]
    fn cursor_line_shows_markers() {
        assert!(tags("a **bold** b", Some(0)).contains(&("**".into(), "syntax")));
    }

    #[test]
    fn link_keeps_url() {
        let spans = style_spans("see [map](https://x.y/z)", None);
        let link = spans.iter().find(|s| s.tag == "link").unwrap();
        assert_eq!(link.data.as_deref(), Some("https://x.y/z"));
    }

    #[test]
    fn checkbox_and_done() {
        let spans = tags("☑ done", None);
        assert!(spans.contains(&("☑ ".into(), "checkbox")));
        assert!(spans.contains(&("done".into(), "done")));
    }

    #[test]
    fn offsets_count_characters() {
        let spans = tags("ção **negrito**", None);
        assert!(spans.contains(&("negrito".into(), "bold")));
    }

    #[test]
    fn image_lines() {
        let digest = "a".repeat(64);
        let text = format!("![shot](attachment:{digest})\nafter");
        let spans = style_spans(&text, None);
        assert!(spans.iter().any(|s| s.tag == "image" && s.data.as_deref() == Some(digest.as_str())));
        assert_eq!(image_digest(&format!("![x](attachment:{digest})")), Some(digest));
    }
}
