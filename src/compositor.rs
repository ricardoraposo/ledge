use std::process::{Command, Stdio};

pub const TITLE_SUFFIX: &str = " — Ledge";
const TITLE_PATTERN: &str = " — Ledge$";

fn found(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

fn run(program: &str, args: &[&str]) {
    let _ = Command::new(program).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

pub fn install_window_rules(app_id: &str) {
    if std::env::var_os("SWAYSOCK").is_some() && found("swaymsg") {
        let criteria = format!("[app_id=\"{app_id}\" title=\"{TITLE_PATTERN}\"]");
        for action in ["floating enable", "sticky enable", "border none"] {
            run("swaymsg", &[&format!("for_window {criteria} {action}")]);
        }
        run("swaymsg", &[&format!("for_window [app_id=\"{app_id}\"] floating enable")]);
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && found("hyprctl") {
        for rule in ["float", "pin", "noborder"] {
            run("hyprctl", &["keyword", "windowrulev2", &format!("{rule},class:^({app_id})$,title:({TITLE_PATTERN})")]);
        }
        run("hyprctl", &["keyword", "windowrulev2", &format!("float,class:^({app_id})$")]);
    }
}

pub fn has_program(program: &str) -> bool {
    found(program)
}
