use std::path::PathBuf;

use adw::prelude::*;
use gtk::gio;
use serde_json::{Value, json};

use crate::app::app;
use crate::backups;
use crate::crypto::KeyBackend;
use crate::shortcuts::{SHORTCUTS, install_sway_bindings, sway_snippet};

fn choices(key: &str) -> Vec<(Value, &'static str)> {
    match key {
        "edge" => vec![(json!("right"), "Right"), (json!("left"), "Left")],
        "magnification" => vec![(json!("off"), "Off"), (json!("small"), "Small"), (json!("medium"), "Medium"), (json!("large"), "Large")],
        "text_size" => vec![
            (json!("small"), "Small"),
            (json!("medium"), "Medium"),
            (json!("large"), "Large"),
            (json!("xlarge"), "Extra Large"),
        ],
        "archive_after_days" => vec![(json!(0), "Never"), (json!(7), "7 days"), (json!(14), "14 days"), (json!(30), "30 days")],
        "undo_seconds" => vec![(json!(5), "5 seconds"), (json!(10), "10 seconds"), (json!(20), "20 seconds"), (json!(30), "30 seconds")],
        "carry_over" => vec![(json!("bring"), "Bring them in"), (json!("leave"), "Leave them")],
        _ => Vec::new(),
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn choice_row(key: &'static str, title: &str) -> adw::ComboRow {
    let options = choices(key);
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    let row = adw::ComboRow::builder().title(title).model(&gtk::StringList::new(&labels)).build();
    let current = app().settings.get(key);
    let index = options.iter().position(|(value, _)| same(value, &current)).unwrap_or(0);
    row.set_selected(index as u32);
    row.connect_selected_notify(move |row| {
        let options = choices(key);
        if let Some((value, _)) = options.get(row.selected() as usize) {
            app().settings.set(key, value.clone());
        }
    });
    row
}

fn switch_row(key: &'static str, title: &str, subtitle: &str) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).active(app().settings.flag(key)).build();
    row.connect_active_notify(move |row| app().settings.set(key, Value::Bool(row.is_active())));
    row
}

fn group(title: &str, description: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    if !description.is_empty() {
        group.set_description(Some(description));
    }
    group
}

pub fn show_preferences(parent: Option<&gtk::Window>) {
    let kn = app();
    let dialog = adw::PreferencesDialog::builder().title("Settings").search_enabled(false).build();

    let general = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();
    let look = group("Notes", "");
    look.add(&choice_row("text_size", "Text size"));
    look.add(&choice_row("undo_seconds", "Time to undo a delete"));
    general.add(&look);
    let archive = group(
        "Archive by time",
        "Notes you have not opened for this long move to the Archive. Pinned notes, notes kept on the deck and \
         dailies are left alone.",
    );
    archive.add(&choice_row("archive_after_days", "Archive after"));
    general.add(&archive);
    let backup_group = group(
        "Backups",
        "Once a day Ledge copies its database next to it and keeps the seven most recent copies.",
    );
    let backup_row = adw::ActionRow::builder().title("Back up now").build();
    let backup_button = gtk::Button::builder().label("Back Up").valign(gtk::Align::Center).build();
    let toast_target = dialog.downgrade();
    backup_button.connect_clicked(move |_| {
        let message = match backups::backup_now(&app().store, "manual") {
            Ok(_) => "Backed up".to_string(),
            Err(error) => format!("Backup failed: {error}"),
        };
        if let Some(dialog) = toast_target.upgrade() {
            dialog.add_toast(adw::Toast::new(&message));
        }
    });
    backup_row.add_suffix(&backup_button);
    backup_group.add(&backup_row);
    let restore_row = adw::ExpanderRow::builder().title("Restore").subtitle("Your current notes are backed up first").build();
    for path in backups::list_backups() {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().replace("notes-", "");
        let item = adw::ActionRow::builder().title(&name).build();
        let button = gtk::Button::builder()
            .label("Restore")
            .valign(gtk::Align::Center)
            .css_classes(["destructive-action"])
            .build();
        let target = dialog.downgrade();
        button.connect_clicked(move |_| {
            if let Some(dialog) = target.upgrade() {
                confirm_restore(&dialog, path.clone());
            }
        });
        item.add_suffix(&button);
        restore_row.add_row(&item);
    }
    backup_group.add(&restore_row);
    general.add(&backup_group);
    let place = group("Where it lives", "");
    place.add(
        &adw::ActionRow::builder()
            .title("Database")
            .subtitle(kn.database_path().to_string_lossy().as_ref())
            .subtitle_selectable(true)
            .build(),
    );
    let key_text = if kn.store.key_backend == KeyBackend::Keyring {
        "Keyring (Secret Service)"
    } else {
        "Key file next to the database"
    };
    place.add(&adw::ActionRow::builder().title("Encryption key").subtitle(key_text).build());
    general.add(&place);
    dialog.add(&general);

    let deck = adw::PreferencesPage::builder().title("Deck").icon_name("view-dual-symbolic").build();
    let deck_group = adw::PreferencesGroup::new();
    deck_group.add(&choice_row("edge", "Screen edge"));
    deck_group.add(&choice_row("magnification", "Hover magnification"));
    deck_group.add(&switch_row("peek", "Quick look on hover", "Touching a tab shows the note"));
    deck_group.add(&switch_row("show_deck_buttons", "New Note and Daily buttons", ""));
    deck.add(&deck_group);
    dialog.add(&deck);

    let daily_page = adw::PreferencesPage::builder().title("Daily").icon_name("x-office-calendar-symbolic").build();
    let daily_group = group("New dailies", "");
    daily_group.add(&choice_row("carry_over", "Unfinished tasks from the last day"));
    daily_page.add(&daily_group);
    let template_group = group("Template", "What a new daily starts with (Markdown)");
    let template_view = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .css_classes(["template-editor"])
        .build();
    template_view.buffer().set_text(&kn.settings.string("daily_template"));
    template_view.buffer().connect_changed(|buffer| {
        let (start, end) = buffer.bounds();
        app().settings.set("daily_template", Value::String(buffer.text(&start, &end, true).to_string()));
    });
    let frame = gtk::Frame::builder().child(&template_view).build();
    frame.set_size_request(-1, 140);
    template_group.add(&frame);
    daily_page.add(&template_group);
    dialog.add(&daily_page);

    let sync_page = adw::PreferencesPage::builder().title("Sync").icon_name("emblem-synchronizing-symbolic").build();
    let sync_group = group(
        "Sync through a folder",
        "Ledge writes one plain-text .hmnote file per note into the folder. Point Syncthing, Nextcloud or \
         Dropbox at it to move notes between computers, including KeepNote on macOS. The files are not encrypted.",
    );
    let folder_value = kn.settings.string("sync_folder");
    let folder_row = adw::ActionRow::builder()
        .title("Folder")
        .subtitle(if folder_value.is_empty() { "Off" } else { folder_value.as_str() })
        .build();
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    let stop = gtk::Button::builder().label("Turn Off").valign(gtk::Align::Center).visible(!folder_value.is_empty()).build();
    let row_target = folder_row.downgrade();
    let stop_target = stop.downgrade();
    let parent_target = dialog.downgrade();
    choose.connect_clicked(move |_| {
        let picker = gtk::FileDialog::builder().title("Sync Folder").build();
        let window = parent_target.upgrade().and_then(|dialog| dialog.root()).and_downcast::<gtk::Window>();
        let row_target = row_target.clone();
        let stop_target = stop_target.clone();
        picker.select_folder(window.as_ref(), None::<&gio::Cancellable>, move |result| {
            let Some(path) = result.ok().and_then(|f| f.path()) else {
                return;
            };
            let text = path.to_string_lossy().into_owned();
            app().settings.set("sync_folder", Value::String(text.clone()));
            if let Some(row) = row_target.upgrade() {
                row.set_subtitle(&text);
            }
            if let Some(stop) = stop_target.upgrade() {
                stop.set_visible(true);
            }
        });
    });
    let row_target = folder_row.downgrade();
    stop.connect_clicked(move |button| {
        app().settings.set("sync_folder", Value::String(String::new()));
        if let Some(row) = row_target.upgrade() {
            row.set_subtitle("Off");
        }
        button.set_visible(false);
    });
    folder_row.add_suffix(&stop);
    folder_row.add_suffix(&choose);
    sync_group.add(&folder_row);
    sync_page.add(&sync_group);
    dialog.add(&sync_page);

    let keys_page = adw::PreferencesPage::builder()
        .title("Shortcuts")
        .icon_name("preferences-desktop-keyboard-shortcuts-symbolic")
        .build();
    let keys_group = group(
        "Global shortcuts",
        "Wayland does not let an app take keys for itself. The compositor runs `ledge COMMAND` for each \
         shortcut instead.",
    );
    for (keys, command, label) in SHORTCUTS {
        keys_group.add(&adw::ActionRow::builder().title(label).subtitle(format!("{keys}  →  ledge {command}")).build());
    }
    keys_page.add(&keys_group);
    let sway_group = group("sway", "");
    let install_row = adw::ActionRow::builder()
        .title("Install for sway")
        .subtitle("Writes ~/.config/sway/config.d/ledge and reloads sway")
        .build();
    let install_button = gtk::Button::builder()
        .label("Install")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    let toast_target = dialog.downgrade();
    install_button.connect_clicked(move |_| {
        let message = install_sway_bindings();
        if let Some(dialog) = toast_target.upgrade() {
            dialog.add_toast(adw::Toast::new(&message));
        }
    });
    install_row.add_suffix(&install_button);
    sway_group.add(&install_row);
    let snippet = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    snippet.buffer().set_text(&sway_snippet());
    sway_group.add(&gtk::Frame::builder().child(&snippet).build());
    keys_page.add(&sway_group);
    dialog.add(&keys_page);

    kn.dialog_open.set(true);
    dialog.connect_closed(|_| app().dialog_open.set(false));
    dialog.present(parent);
}

fn confirm_restore(dialog: &adw::PreferencesDialog, path: PathBuf) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let alert = adw::AlertDialog::new(
        Some("Restore this backup?"),
        Some(&format!("Ledge backs up your current notes first, then switches to {name}.")),
    );
    alert.add_response("cancel", "Cancel");
    alert.add_response("restore", "Restore");
    alert.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
    let target = dialog.downgrade();
    alert.choose(Some(dialog), None::<&gio::Cancellable>, move |response| {
        if response != "restore" {
            return;
        }
        let message = match backups::restore(&app().store, &path) {
            Ok(()) => "Restored".to_string(),
            Err(error) => format!("Restore failed: {error}"),
        };
        if let Some(dialog) = target.upgrade() {
            dialog.add_toast(adw::Toast::new(&message));
        }
    });
}
