pub const COMMANDS: [(&str, &str); 9] = [
    ("new", "Make a note and open it (text after it becomes the note)"),
    ("daily", "Open today's daily note"),
    ("all", "Open All Notes"),
    ("archive", "Open the Archive"),
    ("focus", "Give the deck the keyboard (arrows, Return, Space, Esc)"),
    ("toggle", "Fan the deck out or fold it"),
    ("settings", "Open Settings"),
    ("backup", "Back up the notes now"),
    ("quit", "Quit Ledge"),
];

pub fn usage() -> String {
    let mut lines = vec![
        "Usage: ledge [command]".to_string(),
        String::new(),
        "With no command, Ledge starts and waits on the screen edge.".into(),
        String::new(),
    ];
    lines.extend(COMMANDS.iter().map(|(name, text)| format!("  {name:<10} {text}")));
    lines.push(String::new());
    lines.push("  install-sway  Write the global shortcuts to ~/.config/sway/config.d/ledge".into());
    lines.join("\n") + "\n"
}

pub fn is_help(argument: &str) -> bool {
    matches!(argument.trim_start_matches('-'), "h" | "help")
}
