use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use gtk::gio;
use gtk::glib;

use crate::allnotes::AllNotes;
use crate::attachments::Attachments;
use crate::backups;
use crate::card::{Card, Toast};
use crate::cli::{COMMANDS, is_help, usage};
use crate::compositor::install_window_rules;
use crate::daily;
use crate::deck::Deck;
use crate::exporter;
use crate::settings::Settings;
use crate::signals::idle;
use crate::store::{Draft, Store};
use crate::sync::FolderSync;

pub const APP_ID: &str = "io.github.ricardoraposo.Ledge";
const STYLE: &str = include_str!("style.css");

pub struct Kn {
    pub gtk: adw::Application,
    pub store: Rc<Store>,
    pub settings: Rc<Settings>,
    pub attachments: Rc<Attachments>,
    sync: RefCell<Option<Rc<FolderSync>>>,
    cards: RefCell<HashMap<String, Rc<Card>>>,
    toasts: RefCell<Vec<Rc<Toast>>>,
    deck: RefCell<Option<Rc<Deck>>>,
    all_notes: RefCell<Option<Rc<AllNotes>>>,
    pub dialog_open: Cell<bool>,
    textures: RefCell<HashMap<String, gdk::Texture>>,
    size_css: gtk::CssProvider,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

thread_local! {
    static INSTANCE: OnceCell<Rc<Kn>> = const { OnceCell::new() };
}

pub fn app() -> Rc<Kn> {
    INSTANCE.with(|cell| cell.get().cloned()).expect("Ledge has not started")
}

fn started() -> bool {
    INSTANCE.with(|cell| cell.get().is_some())
}

pub fn run() -> glib::ExitCode {
    let application = adw::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    application.connect_startup(startup);
    application.connect_command_line(|_, line| command_line(line));
    application.connect_activate(|_| {});
    glib::set_application_name("Ledge");
    application.run()
}

fn startup(application: &adw::Application) {
    if !gtk4_layer_shell::is_supported() {
        eprintln!(
            "ledge: this compositor has no wlr-layer-shell (GNOME does not). Ledge needs sway, Hyprland, \
             KDE Plasma, niri, river, labwc or Wayfire."
        );
    }
    let store = match Store::open_default() {
        Ok(store) => Rc::new(store),
        Err(error) => {
            eprintln!("ledge: {error}");
            std::process::exit(1);
        }
    };
    let settings = Rc::new(Settings::load());
    let attachments = Rc::new(Attachments::new(store.clone()));
    let kn = Rc::new(Kn {
        gtk: application.clone(),
        store: store.clone(),
        settings: settings.clone(),
        attachments: attachments.clone(),
        sync: RefCell::new(None),
        cards: RefCell::new(HashMap::new()),
        toasts: RefCell::new(Vec::new()),
        deck: RefCell::new(None),
        all_notes: RefCell::new(None),
        dialog_open: Cell::new(false),
        textures: RefCell::new(HashMap::new()),
        size_css: gtk::CssProvider::new(),
        hold: RefCell::new(None),
    });
    INSTANCE.with(|cell| {
        let _ = cell.set(kn.clone());
    });
    *kn.hold.borrow_mut() = Some(application.hold());
    kn.load_css();
    settings.changed.connect(|_| app().apply_size_css());
    install_window_rules(APP_ID);
    let deck = Deck::new();
    deck.window.present();
    *kn.deck.borrow_mut() = Some(deck);
    *kn.sync.borrow_mut() = Some(FolderSync::new(store, settings, attachments));
    kn.housekeeping();
    idle(|| app().restore_floating());
    glib::timeout_add_seconds_local(3600, || {
        app().housekeeping();
        glib::ControlFlow::Continue
    });
}

fn command_line(line: &gio::ApplicationCommandLine) -> glib::ExitCode {
    let args: Vec<String> = line.arguments().iter().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let Some(first) = args.first() else {
        return glib::ExitCode::SUCCESS;
    };
    if is_help(first) {
        line.print_literal(&usage());
        return glib::ExitCode::SUCCESS;
    }
    let command = first.trim_start_matches('-').to_string();
    if !COMMANDS.iter().any(|(name, _)| *name == command) {
        line.printerr_literal(&format!("ledge: unknown command '{command}'\n{}", usage()));
        return glib::ExitCode::from(2);
    }
    if !started() {
        return glib::ExitCode::FAILURE;
    }
    let rest = args[1..].join(" ");
    idle(move || app().run_command(&command, &rest));
    glib::ExitCode::SUCCESS
}

impl Kn {
    pub fn reduce_motion(&self) -> bool {
        gtk::Settings::default().is_some_and(|settings| !settings.is_gtk_enable_animations())
    }

    pub fn deck(&self) -> Option<Rc<Deck>> {
        self.deck.borrow().clone()
    }

    pub fn deck_window(&self) -> Option<gtk::Window> {
        self.deck.borrow().as_ref().map(|deck| deck.window.clone())
    }

    fn load_css(&self) {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let provider = gtk::CssProvider::new();
        provider.load_from_string(STYLE);
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        gtk::style_context_add_provider_for_display(
            &display,
            &self.size_css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        self.apply_size_css();
    }

    fn apply_size_css(&self) {
        let points = self.settings.text_points();
        let lift = self.settings.lift();
        self.size_css.load_from_string(&format!(
            ".note-text, .note-title {{ font-size: {points}pt; }}\n\
             .note-title {{ font-size: {:.1}pt; }}\n\
             .peek-body {{ font-size: {:.1}pt; }}\n\
             .tabs.open .tab:hover, .tabs.open .tab:focus-visible {{ min-width: {}px; }}\n\
             .tabs.open .tab.in-group:hover, .tabs.open .tab.in-group:focus-visible {{ min-width: {}px; }}\n",
            points * 1.2,
            points * 0.92,
            40 + lift,
            50 + lift,
        ));
    }

    fn housekeeping(&self) {
        let protected: HashSet<String> = self.cards.borrow().keys().cloned().collect();
        let archived = self.store.run_archive_rules(self.settings.int("archive_after_days"), &protected);
        if !archived.is_empty() {
            let count = archived.len();
            self.announce(&format!("{count} note{} moved to the Archive", if count > 1 { "s" } else { "" }), true);
        }
        self.store.purge_deleted();
        if let Err(error) = backups::daily_backup(&self.store) {
            eprintln!("ledge: backup failed: {error}");
        }
    }

    fn run_command(&self, command: &str, rest: &str) {
        match command {
            "new" => {
                self.new_note(rest);
            }
            "daily" => self.open_daily(),
            "all" => self.show_all_notes("all", None),
            "archive" => self.show_all_notes("archive", None),
            "focus" => {
                if let Some(deck) = self.deck() {
                    deck.enter_focus_mode();
                }
            }
            "toggle" => {
                if let Some(deck) = self.deck() {
                    deck.toggle();
                }
            }
            "settings" => self.show_settings(None),
            "backup" => match backups::backup_now(&self.store, "backup") {
                Ok(path) => {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                    self.announce(&format!("Backed up to {name}"), false);
                }
                Err(error) => self.announce(&format!("Backup failed: {error}"), false),
            },
            "quit" => self.quit_all(),
            _ => {}
        }
    }

    pub fn quit_all(&self) {
        let cards: Vec<Rc<Card>> = self.cards.borrow().values().cloned().collect();
        for card in cards {
            card.editor.flush();
            card.save_size();
        }
        self.hold.borrow_mut().take();
        self.gtk.quit();
    }

    pub fn open_note(&self, note_id: &str, anchor_y: Option<i32>, focus: bool) -> Option<Rc<Card>> {
        let note = self.store.get(note_id)?;
        if note.is_archived() {
            self.show_all_notes("archive", Some(note_id.to_string()));
            return None;
        }
        let existing = self.cards.borrow().get(note_id).cloned();
        if let Some(card) = &existing {
            if card.floating == note.floating {
                card.present_card(focus);
                return Some(card.clone());
            }
            card.close_card(false);
        }
        let docked: Vec<Rc<Card>> = self.cards.borrow().values().filter(|card| !card.floating).cloned().collect();
        for other in docked {
            other.close_card(false);
        }
        let card = if note.floating { Card::floating(note_id) } else { Card::docked(note_id, anchor_y) };
        self.cards.borrow_mut().insert(note_id.to_string(), card.clone());
        card.present_card(focus);
        Some(card)
    }

    pub fn card_closed(&self, card: &Rc<Card>) {
        let current = self.cards.borrow().get(&card.note_id).cloned();
        if current.is_some_and(|current| Rc::ptr_eq(&current, card)) {
            self.cards.borrow_mut().remove(&card.note_id);
        }
        if let Some(note) = self.store.get(&card.note_id) {
            if note.title.trim().is_empty() && note.body.trim().is_empty() && note.tags.is_empty() {
                self.store.delete(&card.note_id);
                self.store.purge(&card.note_id);
                self.store.emit("");
            }
        }
    }

    pub fn has_card(&self, note_id: &str) -> bool {
        self.cards.borrow().contains_key(note_id)
    }

    pub fn new_note(&self, text: &str) -> crate::model::Note {
        let note = self.store.create(Draft { body: text.trim().to_string(), ..Draft::default() });
        self.open_note(&note.id, None, true);
        note
    }

    pub fn open_daily(&self) {
        let (note, _) = daily::open_or_create_today(&self.store, &self.settings);
        self.open_note(&note.id, None, true);
    }

    pub fn toggle_float(&self, note_id: &str) {
        let Some(note) = self.store.get(note_id) else {
            return;
        };
        let floating = !note.floating;
        let card = self.cards.borrow().get(note_id).cloned();
        if let Some(card) = &card {
            card.editor.flush();
        }
        self.store.update(note_id, |note| note.floating = floating);
        if let Some(card) = card {
            let still_open = self.cards.borrow().get(note_id).is_some_and(|current| Rc::ptr_eq(current, &card));
            if still_open {
                card.close_card(false);
            }
        }
        if floating {
            self.open_note(note_id, None, true);
        }
        self.announce(if floating { "Floating" } else { "Back on the deck" }, true);
    }

    fn restore_floating(&self) {
        for note in self.store.active_notes() {
            if note.floating && !self.has_card(&note.id) {
                let card = Card::floating(&note.id);
                self.cards.borrow_mut().insert(note.id.clone(), card.clone());
                card.present_card(false);
            }
        }
    }

    pub fn archive_note(&self, note_id: &str) {
        if let Some(note) = self.store.get(note_id) {
            self.store.archive(note_id, false);
            self.announce(&format!("Archived “{}”", note.display_title()), false);
        }
    }

    pub fn delete_with_undo(&self, note_id: &str) {
        let Some(note) = self.store.get(note_id) else {
            return;
        };
        let card = self.cards.borrow().get(note_id).cloned();
        if let Some(card) = card {
            card.close_card(false);
        }
        self.store.delete(note_id);
        let seconds = match self.settings.int("undo_seconds") {
            0 => 10,
            value => value,
        };
        let id = note_id.to_string();
        self.show_toast(
            &format!("Deleted “{}”", note.display_title()),
            Some("Undo"),
            Some(Box::new(move || app().store.undelete(&id))),
            seconds as f64,
        );
    }

    pub fn copy_note(&self, note_id: &str) {
        if let Some(note) = self.store.get(note_id) {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&note.copy_text());
                self.announce("Note copied", false);
            }
        }
    }

    pub fn show_toast(&self, text: &str, action_label: Option<&str>, action: Option<Box<dyn FnOnce()>>, seconds: f64) {
        let previous: Vec<Rc<Toast>> = self.toasts.borrow().clone();
        for toast in previous {
            toast.dismiss();
        }
        let toast = Toast::show(text, action_label, action, seconds);
        self.toasts.borrow_mut().push(toast);
    }

    pub fn toast_closed(&self, toast: &Rc<Toast>) {
        self.toasts.borrow_mut().retain(|item| !Rc::ptr_eq(item, toast));
    }

    pub fn announce(&self, text: &str, quiet: bool) {
        if quiet {
            if let Some(window) = self.deck_window() {
                window.update_property(&[gtk::accessible::Property::Description(text)]);
            }
            return;
        }
        self.show_toast(text, None, None, 2.5);
    }

    pub fn image_texture(&self, digest: &str) -> Option<gdk::Texture> {
        if let Some(texture) = self.textures.borrow().get(digest) {
            return Some(texture.clone());
        }
        let data = self.attachments.data(digest)?;
        let texture = gdk::Texture::from_bytes(&glib::Bytes::from_owned(data)).ok()?;
        self.textures.borrow_mut().insert(digest.to_string(), texture.clone());
        Some(texture)
    }

    pub fn open_link(&self, url: &str, parent: Option<&gtk::Window>) {
        let target = if let Some(digest) = url.strip_prefix("attachment:") {
            let Some(path) = self.attachments.open_path(digest) else {
                self.announce("This attachment is not on this computer yet", false);
                return;
            };
            gio::File::for_path(path).uri().to_string()
        } else if !url.contains("://") && !url.starts_with("mailto:") {
            format!("https://{url}")
        } else {
            url.to_string()
        };
        gtk::UriLauncher::new(&target).launch(parent, None::<&gio::Cancellable>, |_| {});
    }

    pub fn show_all_notes(&self, section: &str, select: Option<String>) {
        let existing = self.all_notes.borrow().clone();
        let window = match existing {
            Some(window) => window,
            None => {
                let window = AllNotes::new();
                window.window.connect_destroy(|_| {
                    app().all_notes.borrow_mut().take();
                });
                *self.all_notes.borrow_mut() = Some(window.clone());
                window
            }
        };
        window.show_section(section, select);
        window.window.present();
    }

    pub fn all_notes_window(&self) -> Option<gtk::Window> {
        self.all_notes.borrow().as_ref().map(|all| all.window.clone().upcast())
    }

    pub fn show_settings(&self, parent: Option<gtk::Window>) {
        let parent = parent.or_else(|| self.all_notes_window());
        crate::prefs::show_preferences(parent.as_ref());
    }

    pub fn move_to_group(&self, note_id: &str, group_id: Option<String>) {
        if self.store.get(note_id).is_some() {
            self.store.update(note_id, |note| note.group_id = group_id);
        }
    }

    pub fn new_group_with(&self, note_id: Option<&str>, name: &str) {
        let name: String = name.trim().chars().take(60).collect();
        if name.is_empty() {
            return;
        }
        let group = self.store.create_group(&name);
        if let Some(note_id) = note_id {
            self.store.update(note_id, |note| note.group_id = Some(group.id));
        }
    }

    pub fn fill_group_menu(&self, menu: &gio::Menu, note_id: &str, prefix: &str) {
        let note = self.store.get(note_id);
        let current = note.as_ref().and_then(|n| n.group_id.clone());
        for group in self.store.live_groups() {
            let mark = if current.as_deref() == Some(group.id.as_str()) { "✓ " } else { "" };
            menu.append(Some(&format!("{mark}{}", group.name)), Some(&format!("{prefix}.move-to::{note_id}|{}", group.id)));
        }
        if current.is_some() {
            menu.append(Some("No Group"), Some(&format!("{prefix}.move-to::{note_id}|")));
        }
        menu.append(Some("New Group…"), Some(&format!("{prefix}.new-group::{note_id}")));
    }

    pub fn export_note(&self, note_id: &str, parent: Option<gtk::Window>) {
        let Some(note) = self.store.get(note_id) else {
            return;
        };
        let dialog = gtk::FileDialog::builder().title("Export as Markdown").accept_label("Export").build();
        self.dialog_open.set(true);
        dialog.select_folder(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
            let kn = app();
            kn.dialog_open.set(false);
            let Some(folder) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            let written = exporter::export_notes(&kn.attachments, &[note], &folder);
            match written.first() {
                Some(path) => {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                    kn.announce(&format!("Exported to {name}"), false);
                }
                None => kn.announce("Nothing exported", false),
            }
        });
    }

    pub fn database_path(&self) -> PathBuf {
        self.store.path.clone()
    }
}
