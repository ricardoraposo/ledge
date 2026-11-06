use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::pango;
use serde_json::Value;

use crate::app::app;
use crate::editor::{NoteEditor, relative_edited};
use crate::exporter::{export_notes, export_plain, import_folder};
use crate::model::{Note, color_slug};
use crate::widgets::{clear, remove_classes_with_prefix, set_label};

const SORTS: [(&str, &str); 3] = [("edited", "Edited"), ("created", "Created"), ("title", "Title")];

struct NoteRow {
    row: gtk::ListBoxRow,
    bar: gtk::Box,
    title: gtk::Label,
    date: gtk::Label,
    preview: gtk::Label,
    tags: gtk::Label,
}

impl NoteRow {
    fn new(note: &Note) -> NoteRow {
        let row = gtk::ListBoxRow::builder().css_classes(["note-row"]).build();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let bar = gtk::Box::builder().css_classes(["row-bar"]).build();
        bar.set_size_request(5, -1);
        content.append(&bar);
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(["row-title"])
            .build();
        let date = gtk::Label::builder().css_classes(["row-date", "dim-label"]).build();
        top.append(&title);
        top.append(&date);
        let preview = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .lines(2)
            .wrap(true)
            .css_classes(["row-preview", "dim-label"])
            .build();
        let tags = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::End).css_classes(["row-tags"]).build();
        text.append(&top);
        text.append(&preview);
        text.append(&tags);
        content.append(&text);
        row.set_child(Some(&content));
        let note_row = NoteRow { row, bar, title, date, preview, tags };
        note_row.update(note);
        note_row
    }

    fn update(&self, note: &Note) {
        remove_classes_with_prefix(&self.bar, "spine-");
        self.bar.add_css_class(&format!("spine-{}", color_slug(note.color)));
        let mark = if note.is_pinned() { "📌 " } else { "" };
        self.title.set_text(&format!("{mark}{}", note.display_title()));
        self.date.set_text(&relative_edited(&note.edited_at).replace("Edited ", ""));
        let preview = note.preview();
        self.preview.set_text(&preview);
        self.preview.set_visible(!preview.is_empty());
        let tags = note.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join("  ");
        self.tags.set_text(&tags);
        self.tags.set_visible(!note.tags.is_empty());
    }
}

pub struct AllNotes {
    pub window: adw::ApplicationWindow,
    title_widget: adw::WindowTitle,
    sidebar: gtk::ListBox,
    search: gtk::SearchEntry,
    sort: gtk::DropDown,
    list: gtk::ListBox,
    detail: gtk::Stack,
    note_holder: gtk::Box,
    note_actions: gtk::Box,
    left: gtk::Box,
    section: RefCell<String>,
    selected: RefCell<Option<String>>,
    editor: RefCell<Option<Rc<NoteEditor>>>,
    rows: RefCell<HashMap<String, NoteRow>>,
    row_order: RefCell<Vec<String>>,
    section_rows: RefCell<Vec<(gtk::ListBoxRow, Option<String>)>>,
    rebuilding: Cell<bool>,
    handler: Cell<u64>,
}

impl AllNotes {
    pub fn new() -> Rc<AllNotes> {
        let window = adw::ApplicationWindow::builder()
            .application(&app().gtk)
            .title("All Notes")
            .default_width(1120)
            .default_height(720)
            .build();
        window.add_css_class("all-notes");
        let header = adw::HeaderBar::new();
        let focus_button = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .active(true)
            .tooltip_text("Hide Sidebar and List")
            .build();
        header.pack_start(&focus_button);
        let new_button = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New Note (Ctrl+N)").build();
        header.pack_start(&new_button);
        let menu = gio::Menu::new();
        menu.append(Some("Settings"), Some("win.settings"));
        let io = gio::Menu::new();
        io.append(Some("Import Notes…"), Some("win.import"));
        io.append(Some("Export All as Markdown…"), Some("win.export-md"));
        io.append(Some("Export All as Plain Text…"), Some("win.export-txt"));
        menu.append_section(None, &io);
        header.pack_end(
            &gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&menu).tooltip_text("Menu").build(),
        );
        let title_widget = adw::WindowTitle::new("All Notes", "");
        header.set_title_widget(Some(&title_widget));

        let sidebar = gtk::ListBox::builder()
            .css_classes(["navigation-sidebar"])
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(210)
            .css_classes(["sidebar-pane"])
            .build();
        let search = gtk::SearchEntry::builder().placeholder_text("Search title, tags and text").hexpand(true).build();
        let sort = gtk::DropDown::from_strings(&SORTS.map(|(_, label)| label));
        sort.set_tooltip_text(Some("Sort"));
        let stored = app().settings.string("all_notes_sort");
        sort.set_selected(SORTS.iter().position(|(key, _)| *key == stored).unwrap_or(0) as u32);
        let search_row = gtk::Box::builder().spacing(6).css_classes(["list-toolbar"]).build();
        search_row.append(&search);
        search_row.append(&sort);
        let list = gtk::ListBox::builder().css_classes(["notes-list"]).selection_mode(gtk::SelectionMode::Single).build();
        list.set_placeholder(Some(
            &gtk::Label::builder().label("No notes here").css_classes(["dim-label"]).margin_top(40).build(),
        ));
        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let list_pane = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(330)
            .css_classes(["list-pane"])
            .build();
        list_pane.append(&search_row);
        list_pane.append(&list_scroll);

        let detail = gtk::Stack::new();
        let empty = adw::StatusPage::builder()
            .icon_name("accessories-text-editor-symbolic")
            .title("No Note Selected")
            .description("Pick a note, or make one with Ctrl+N.")
            .build();
        detail.add_named(&empty, Some("empty"));
        let note_holder = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["detail-pane"]).build();
        let note_actions = gtk::Box::builder().spacing(6).css_classes(["detail-actions"]).build();
        detail.add_named(&note_holder, Some("note"));

        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left.append(&sidebar_scroll);
        left.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        left.append(&list_pane);
        let paned = gtk::Paned::builder()
            .start_child(&left)
            .end_child(&detail)
            .shrink_start_child(false)
            .resize_start_child(false)
            .build();
        let view = adw::ToolbarView::new();
        view.set_content(Some(&paned));
        view.add_top_bar(&header);
        window.set_content(Some(&view));

        let all = Rc::new(AllNotes {
            window,
            title_widget,
            sidebar,
            search,
            sort,
            list,
            detail,
            note_holder,
            note_actions,
            left,
            section: RefCell::new("all".into()),
            selected: RefCell::new(None),
            editor: RefCell::new(None),
            rows: RefCell::new(HashMap::new()),
            row_order: RefCell::new(Vec::new()),
            section_rows: RefCell::new(Vec::new()),
            rebuilding: Cell::new(false),
            handler: Cell::new(0),
        });

        let weak = Rc::downgrade(&all);
        focus_button.connect_toggled(move |button| {
            if let Some(all) = weak.upgrade() {
                all.left.set_visible(button.is_active());
                button.set_tooltip_text(Some(if button.is_active() {
                    "Hide Sidebar and List"
                } else {
                    "Show Sidebar and List"
                }));
            }
        });
        let weak = Rc::downgrade(&all);
        new_button.connect_clicked(move |_| {
            if let Some(all) = weak.upgrade() {
                all.new_note();
            }
        });
        let weak = Rc::downgrade(&all);
        all.sidebar.connect_row_selected(move |_, row| {
            if let (Some(all), Some(row)) = (weak.upgrade(), row) {
                all.on_section_selected(row);
            }
        });
        let weak = Rc::downgrade(&all);
        all.search.connect_search_changed(move |_| {
            if let Some(all) = weak.upgrade() {
                all.refresh_list();
            }
        });
        let weak = Rc::downgrade(&all);
        all.sort.connect_selected_notify(move |sort| {
            if let Some(all) = weak.upgrade() {
                let key = SORTS[(sort.selected() as usize).min(SORTS.len() - 1)].0;
                app().settings.set("all_notes_sort", Value::String(key.into()));
                all.refresh_list();
            }
        });
        let weak = Rc::downgrade(&all);
        all.list.connect_row_selected(move |_, row| {
            if let Some(all) = weak.upgrade() {
                if all.rebuilding.get() {
                    return;
                }
                let note_id = row.and_then(|row| all.note_id_for(row));
                all.show_note(note_id);
            }
        });
        let weak = Rc::downgrade(&all);
        all.list.connect_row_activated(move |_, _| {
            if let Some(all) = weak.upgrade() {
                if let Some(editor) = all.editor.borrow().as_ref() {
                    editor.focus_text(false);
                }
            }
        });
        all.install_actions();
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&all);
        keys.connect_key_pressed(move |_, keyval, _, _| {
            if let Some(all) = weak.upgrade() {
                if keyval == gdk::Key::Escape && all.search.has_focus() {
                    all.search.set_text("");
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        all.window.add_controller(keys);
        let weak = Rc::downgrade(&all);
        all.handler.set(app().store.changed.connect(move |note_id| {
            if let Some(all) = weak.upgrade() {
                all.on_store_changed(note_id);
            }
        }));
        let weak = Rc::downgrade(&all);
        all.window.connect_destroy(move |_| {
            if let Some(all) = weak.upgrade() {
                if let Some(editor) = all.editor.borrow_mut().take() {
                    editor.flush();
                }
                app().store.changed.disconnect(all.handler.get());
            }
        });
        all.refresh_sidebar();
        all
    }

    fn install_actions(self: &Rc<Self>) {
        for name in ["settings", "import", "export-md", "export-txt", "new"] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                let Some(all) = weak.upgrade() else {
                    return;
                };
                match name {
                    "settings" => app().show_settings(Some(all.window.clone().upcast())),
                    "import" => all.import(),
                    "export-md" => all.export_markdown(),
                    "export-txt" => all.export_text(),
                    _ => all.new_note(),
                }
            });
            self.window.add_action(&action);
        }
        app().gtk.set_accels_for_action("win.new", &["<Control>n"]);
    }

    fn note_id_for(&self, row: &gtk::ListBoxRow) -> Option<String> {
        let index = row.index();
        if index < 0 {
            return None;
        }
        self.row_order.borrow().get(index as usize).cloned()
    }

    pub fn show_section(self: &Rc<Self>, section: &str, select: Option<String>) {
        *self.section.borrow_mut() = section.to_string();
        if let Some(note_id) = select {
            if app().store.get(&note_id).is_some_and(|note| note.is_archived()) {
                *self.section.borrow_mut() = "archive".into();
            }
            *self.selected.borrow_mut() = Some(note_id);
        }
        self.refresh_sidebar();
    }

    fn sidebar_row(&self, key: Option<&str>, label: &str, icon: &str, count: Option<usize>) -> gtk::ListBoxRow {
        let Some(key) = key else {
            let row = gtk::ListBoxRow::builder().selectable(false).activatable(false).build();
            row.set_child(Some(
                &gtk::Label::builder().label(label).xalign(0.0).css_classes(["sidebar-heading", "dim-label"]).build(),
            ));
            self.section_rows.borrow_mut().push((row.clone(), None));
            return row;
        };
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        if let Some(class) = icon.strip_prefix('#') {
            let dot = gtk::Box::builder().css_classes(["group-dot", class]).valign(gtk::Align::Center).build();
            dot.set_size_request(10, 10);
            content.append(&dot);
        } else {
            content.append(&gtk::Image::from_icon_name(icon));
        }
        content.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).ellipsize(pango::EllipsizeMode::End).build());
        if let Some(count) = count {
            content.append(&gtk::Label::builder().label(count.to_string()).css_classes(["dim-label", "caption"]).build());
        }
        row.set_child(Some(&content));
        self.section_rows.borrow_mut().push((row.clone(), Some(key.to_string())));
        row
    }

    fn refresh_sidebar(self: &Rc<Self>) {
        self.rebuilding.set(true);
        clear(&self.sidebar);
        self.section_rows.borrow_mut().clear();
        let store = &app().store;
        let active = store.active_notes();
        let mut rows = vec![
            self.sidebar_row(Some("all"), "All Notes", "view-list-symbolic", Some(active.len())),
            self.sidebar_row(Some("pinned"), "Pinned", "view-pin-symbolic", Some(active.iter().filter(|n| n.is_pinned()).count())),
            self.sidebar_row(
                Some("daily"),
                "Daily",
                "x-office-calendar-symbolic",
                Some(store.live_notes().iter().filter(|n| n.is_daily()).count()),
            ),
            self.sidebar_row(Some("archive"), "Archive", "folder-symbolic", Some(store.archived_notes().len())),
        ];
        let groups = store.live_groups();
        if !groups.is_empty() {
            rows.push(self.sidebar_row(None, "Groups", "", None));
            for group in groups {
                let count = active.iter().filter(|n| n.group_id.as_deref() == Some(group.id.as_str())).count();
                rows.push(self.sidebar_row(
                    Some(&format!("group:{}", group.id)),
                    &group.name,
                    &format!("#group-color-{}", group.color_index()),
                    Some(count),
                ));
            }
        }
        let tags = store.all_tags();
        if !tags.is_empty() {
            rows.push(self.sidebar_row(None, "Tags", "", None));
            for (tag, count) in tags {
                rows.push(self.sidebar_row(Some(&format!("tag:{tag}")), &format!("#{tag}"), "mail-attachment-symbolic", Some(count)));
            }
        }
        let section = self.section.borrow().clone();
        let mut selected = None;
        for (row, key) in self.section_rows.borrow().iter() {
            self.sidebar.append(row);
            if key.as_deref() == Some(section.as_str()) {
                selected = Some(row.clone());
            }
        }
        let selected = selected.unwrap_or_else(|| {
            *self.section.borrow_mut() = "all".into();
            rows[0].clone()
        });
        self.sidebar.select_row(Some(&selected));
        self.rebuilding.set(false);
        self.refresh_list();
    }

    fn on_section_selected(self: &Rc<Self>, row: &gtk::ListBoxRow) {
        if self.rebuilding.get() {
            return;
        }
        let key = self.section_rows.borrow().iter().find(|(r, _)| r == row).and_then(|(_, key)| key.clone());
        if let Some(key) = key {
            *self.section.borrow_mut() = key;
            self.refresh_list();
        }
    }

    fn notes_in_section(&self) -> Vec<Note> {
        let store = &app().store;
        let section = self.section.borrow().clone();
        let mut notes = match section.as_str() {
            "archive" => store.archived_notes(),
            "daily" => store.live_notes().into_iter().filter(|n| n.is_daily()).collect(),
            _ => {
                let active = store.active_notes();
                if section == "pinned" {
                    active.into_iter().filter(|n| n.is_pinned()).collect()
                } else if let Some(group) = section.strip_prefix("group:") {
                    active.into_iter().filter(|n| n.group_id.as_deref() == Some(group)).collect()
                } else if let Some(tag) = section.strip_prefix("tag:") {
                    active.into_iter().filter(|n| n.tags.iter().any(|t| t == tag)).collect()
                } else {
                    active
                }
            }
        };
        let query = self.search.text().trim().to_string();
        notes.retain(|note| note.matches(&query));
        match SORTS[(self.sort.selected() as usize).min(SORTS.len() - 1)].0 {
            "title" => notes.sort_by_key(|n| n.display_title().to_lowercase()),
            "created" => notes.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
            _ => notes.sort_by(|a, b| b.edited_at.cmp(&a.edited_at)),
        }
        notes
    }

    fn section_title(&self) -> String {
        let section = self.section.borrow().clone();
        match section.as_str() {
            "all" => "All Notes".into(),
            "pinned" => "Pinned".into(),
            "daily" => "Daily".into(),
            "archive" => "Archive".into(),
            _ => {
                if let Some(group) = section.strip_prefix("group:") {
                    app().store.group(group).map(|g| g.name).unwrap_or_else(|| "Group".into())
                } else {
                    format!("#{}", section.strip_prefix("tag:").unwrap_or(""))
                }
            }
        }
    }

    fn refresh_list(self: &Rc<Self>) {
        let notes = self.notes_in_section();
        self.title_widget.set_title(&self.section_title());
        self.title_widget.set_subtitle(&format!("{} note{}", notes.len(), if notes.len() != 1 { "s" } else { "" }));
        self.rebuilding.set(true);
        clear(&self.list);
        let mut rows = HashMap::new();
        let mut order = Vec::new();
        for note in &notes {
            let row = NoteRow::new(note);
            self.list.append(&row.row);
            order.push(note.id.clone());
            rows.insert(note.id.clone(), row);
        }
        *self.rows.borrow_mut() = rows;
        *self.row_order.borrow_mut() = order;
        self.rebuilding.set(false);
        let selected = self.selected.borrow().clone();
        let target = match selected {
            Some(id) if self.rows.borrow().contains_key(&id) => Some(id),
            _ => notes.first().map(|n| n.id.clone()),
        };
        match target {
            Some(id) => {
                let row = self.rows.borrow().get(&id).map(|r| r.row.clone());
                if let Some(row) = row {
                    self.list.select_row(Some(&row));
                }
                self.show_note(Some(id));
            }
            None => self.show_note(None),
        }
    }

    fn on_store_changed(self: &Rc<Self>, note_id: &str) {
        if !note_id.is_empty() && self.rows.borrow().contains_key(note_id) {
            if let Some(note) = app().store.get(note_id) {
                if self.notes_in_section().iter().any(|n| n.id == note.id) {
                    if let Some(row) = self.rows.borrow().get(note_id) {
                        row.update(&note);
                    }
                    self.update_actions();
                    return;
                }
            }
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(all) = weak.upgrade() {
                all.refresh_sidebar();
            }
        });
    }

    fn show_note(self: &Rc<Self>, note_id: Option<String>) {
        let current = self.editor.borrow().as_ref().map(|e| e.note_id.clone());
        if note_id.is_some() && note_id == current {
            *self.selected.borrow_mut() = note_id;
            return;
        }
        if let Some(editor) = self.editor.borrow_mut().take() {
            editor.flush();
        }
        clear(&self.note_holder);
        *self.selected.borrow_mut() = note_id.clone();
        let Some(note_id) = note_id.filter(|id| app().store.get(id).is_some()) else {
            self.detail.set_visible_child_name("empty");
            return;
        };
        self.note_holder.append(&self.note_actions);
        let editor = NoteEditor::new(&note_id);
        editor.root.add_css_class("in-pane");
        let weak = Rc::downgrade(self);
        editor.set_on_close(move || {
            if let Some(all) = weak.upgrade() {
                all.back_to_list();
            }
        });
        self.note_holder.append(&editor.root);
        *self.editor.borrow_mut() = Some(editor);
        self.update_actions();
        self.detail.set_visible_child_name("note");
        app().store.mark_opened(&note_id);
    }

    fn update_actions(self: &Rc<Self>) {
        clear(&self.note_actions);
        let Some(note) = self.selected.borrow().as_deref().and_then(|id| app().store.get(id)) else {
            return;
        };
        remove_classes_with_prefix(&self.note_holder, "paper-");
        self.note_holder.add_css_class(&format!("paper-{}", color_slug(note.color)));
        let id = note.id.clone();
        let mut buttons: Vec<(String, &str, Box<dyn Fn()>, bool)> = Vec::new();
        if note.is_active() {
            let open_id = id.clone();
            buttons.push(("Open on Deck".into(), "go-next-symbolic", Box::new(move || {
                app().open_note(&open_id, None, true);
            }), true));
            let float_id = id.clone();
            buttons.push((
                if note.floating { "Return to Deck" } else { "Float Note" }.into(),
                "view-fullscreen-symbolic",
                Box::new(move || app().toggle_float(&float_id)),
                false,
            ));
            let keep_id = id.clone();
            let keep = !note.keep_on_deck;
            buttons.push((
                if note.keep_on_deck { "Don't Keep on Deck" } else { "Keep on Deck" }.into(),
                if note.keep_on_deck { "changes-prevent-symbolic" } else { "changes-allow-symbolic" },
                Box::new(move || {
                    app().store.update(&keep_id, |note| note.keep_on_deck = keep);
                }),
                false,
            ));
            let archive_id = id.clone();
            buttons.push(("Archive".into(), "folder-symbolic", Box::new(move || app().archive_note(&archive_id)), false));
        } else {
            let back_id = id.clone();
            buttons.push(("Bring Back to Deck".into(), "edit-undo-symbolic", Box::new(move || app().store.unarchive(&back_id)), true));
        }
        let delete_id = id.clone();
        buttons.push(("Delete".into(), "user-trash-symbolic", Box::new(move || app().delete_with_undo(&delete_id)), false));
        for (label, icon, callback, primary) in buttons {
            let button = gtk::Button::builder().css_classes(["flat", "detail-button"]).tooltip_text(&label).build();
            if primary {
                button.set_child(Some(&adw::ButtonContent::builder().icon_name(icon).label(&label).build()));
            } else {
                button.set_icon_name(icon);
                set_label(&button, &label);
            }
            button.connect_clicked(move |_| callback());
            self.note_actions.append(&button);
        }
    }

    fn back_to_list(&self) {
        let selected = self.selected.borrow().clone();
        if let Some(row) = selected.and_then(|id| self.rows.borrow().get(&id).map(|r| r.row.clone())) {
            row.grab_focus();
        }
    }

    fn new_note(self: &Rc<Self>) {
        let section = self.section.borrow().clone();
        let group_id = section.strip_prefix("group:").map(str::to_string);
        if matches!(section.as_str(), "archive" | "pinned" | "daily") || section.starts_with("tag:") {
            *self.section.borrow_mut() = "all".into();
        }
        let note = app().store.create(crate::store::Draft { group_id, ..Default::default() });
        *self.selected.borrow_mut() = Some(note.id);
        self.search.set_text("");
        self.refresh_sidebar();
        if let Some(editor) = self.editor.borrow().as_ref() {
            editor.focus_text(true);
        }
    }

    fn import(&self) {
        let dialog = gtk::FileDialog::builder().title("Import a folder of .hmnote or .md files").build();
        dialog.select_folder(Some(&self.window), None::<&gio::Cancellable>, |result| {
            let Some(folder) = result.ok().and_then(|f| f.path()) else {
                return;
            };
            let kn = app();
            let (notes, groups) = import_folder(&kn.store, &kn.attachments, &folder);
            kn.announce(&format!("Imported {notes} notes and {groups} groups"), false);
        });
    }

    fn export_markdown(&self) {
        let dialog = gtk::FileDialog::builder().title("Export All as Markdown").accept_label("Export").build();
        dialog.select_folder(Some(&self.window), None::<&gio::Cancellable>, |result| {
            let Some(folder) = result.ok().and_then(|f| f.path()) else {
                return;
            };
            let kn = app();
            let written = export_notes(&kn.attachments, &kn.store.live_notes(), &folder);
            kn.announce(&format!("Exported {} notes", written.len()), false);
        });
    }

    fn export_text(&self) {
        let dialog = gtk::FileDialog::builder().title("Export All as Plain Text").initial_name("Ledge.txt").build();
        dialog.save(Some(&self.window), None::<&gio::Cancellable>, |result| {
            let Some(path) = result.ok().and_then(|f| f.path()) else {
                return;
            };
            let kn = app();
            match export_plain(&kn.store.live_notes(), &path) {
                Ok(()) => kn.announce("Exported", false),
                Err(error) => kn.announce(&format!("Export failed: {error}"), false),
            }
        });
    }
}
