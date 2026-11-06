use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use chrono::Local;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::app::app;
use crate::markdown as md;
use crate::model::{COLOR_ORDER, Note, Stamp, color_name, color_slug, next_color, parse_tags, swatch, today_date};
use crate::signals::{Timer, idle};
use crate::widgets::{prompt_name, remove_classes_with_prefix, set_label};

const STYLE_TAGS: [&str; 18] = [
    "h1", "h2", "h3", "bold", "italic", "strike", "highlight", "code", "codeblock", "quote", "link", "marker",
    "checkbox", "done", "syntax", "hidden", "hr", "table",
];
const IMAGE_GAP: i32 = 8;
const IMAGE_MAX_HEIGHT: f64 = 420.0;
const PASTED_IMAGE_LABEL: &str = "Pasted image";

pub fn relative_edited(stamp: &Stamp) -> String {
    let local = stamp.with_timezone(&Local);
    let seconds = (Local::now() - local).num_seconds();
    let today = today_date();
    if seconds < 60 {
        return "Edited just now".into();
    }
    if seconds < 3600 {
        return format!("Edited {} min ago", seconds / 60);
    }
    if local.date_naive() == today {
        return format!("Edited {} h ago", seconds / 3600);
    }
    if (today - local.date_naive()).num_days() == 1 {
        return "Edited yesterday".into();
    }
    format!("Edited {}", local.format("%-d %b"))
}

pub fn rgba(hex: &str, alpha: f32) -> gdk::RGBA {
    let mut color = gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::BLACK);
    color.set_alpha(alpha);
    color
}

pub struct PlacedImage {
    pub line: i32,
    pub texture: gdk::Texture,
    pub width: i32,
    pub height: i32,
    pub digest: String,
}

mod view {
    use super::*;

    #[derive(Default)]
    pub struct NoteTextView {
        pub boxes: RefCell<Vec<(i32, bool)>>,
        pub images: RefCell<Vec<PlacedImage>>,
        pub image_rects: RefCell<Vec<(graphene::Rect, String)>>,
        pub box_color: RefCell<Option<gdk::RGBA>>,
        pub paper_color: RefCell<Option<gdk::RGBA>>,
        pub on_width_changed: RefCell<Option<Box<dyn Fn()>>>,
        pub last_width: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteTextView {
        const NAME: &'static str = "LedgeTextView";
        type Type = super::NoteTextView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for NoteTextView {}

    impl WidgetImpl for NoteTextView {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if width != self.last_width.get() {
                self.last_width.set(width);
                if let Some(callback) = self.on_width_changed.borrow().as_ref() {
                    callback();
                }
            }
        }
    }

    impl TextViewImpl for NoteTextView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            if layer != gtk::TextViewLayer::AboveText {
                return;
            }
            let view = self.obj();
            view.snapshot_images(&snapshot);
            if !self.boxes.borrow().is_empty() {
                view.snapshot_boxes(&snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct NoteTextView(ObjectSubclass<view::NoteTextView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

impl NoteTextView {
    fn new(buffer: &gtk::TextBuffer) -> Self {
        let view: NoteTextView = glib::Object::builder()
            .property("buffer", buffer)
            .property("wrap-mode", gtk::WrapMode::WordChar)
            .property("vexpand", true)
            .property("top-margin", 4)
            .property("bottom-margin", 12)
            .property("left-margin", 2)
            .property("right-margin", 2)
            .property("pixels-above-lines", 2)
            .build();
        view.add_css_class("note-text");
        *view.imp().box_color.borrow_mut() = Some(rgba("#1F5C99", 1.0));
        *view.imp().paper_color.borrow_mut() = Some(rgba("#F7E3A1", 1.0));
        view
    }

    pub fn image_width_limit(&self) -> i32 {
        (self.width() - self.left_margin() - self.right_margin() - 6).max(0)
    }

    pub fn image_at(&self, x: f64, y: f64) -> Option<String> {
        let (bx, by) = self.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let point = graphene::Point::new(bx as f32, by as f32);
        self.imp()
            .image_rects
            .borrow()
            .iter()
            .find(|(rect, _)| rect.contains_point(&point))
            .map(|(_, digest)| digest.clone())
    }

    fn snapshot_images(&self, snapshot: &gtk::Snapshot) {
        let buffer = self.buffer();
        let mut rects = Vec::new();
        for image in self.imp().images.borrow().iter() {
            let Some(start) = buffer.iter_at_line(image.line) else {
                continue;
            };
            let x = self.iter_location(&start).x() as f32;
            let (line_top, _) = self.line_yrange(&start);
            let y = line_top as f32 + IMAGE_GAP as f32 / 2.0;
            let rect = graphene::Rect::new(x, y, image.width as f32, image.height as f32);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 8.0));
            snapshot.append_scaled_texture(&image.texture, gsk::ScalingFilter::Trilinear, &rect);
            snapshot.pop();
            rects.push((rect, image.digest.clone()));
        }
        *self.imp().image_rects.borrow_mut() = rects;
    }

    fn snapshot_boxes(&self, snapshot: &gtk::Snapshot) {
        let buffer = self.buffer();
        let color = self.imp().box_color.borrow().unwrap_or(gdk::RGBA::BLACK);
        let paper = self.imp().paper_color.borrow().unwrap_or(gdk::RGBA::WHITE);
        let (r, g, b) = (color.red() as f64, color.green() as f64, color.blue() as f64);
        for (offset, done) in self.imp().boxes.borrow().iter() {
            let rect = self.iter_location(&buffer.iter_at_offset(*offset));
            let size = ((rect.height() as f64 * 0.66) as i32).clamp(11, 16) as f64;
            let x = rect.x() as f64 + 1.0;
            let y = rect.y() as f64 + (rect.height() as f64 - size) / 2.0;
            let bounds = graphene::Rect::new((x - 1.0) as f32, (y - 1.0) as f32, (size + 2.0) as f32, (size + 2.0) as f32);
            let cr = snapshot.append_cairo(&bounds);
            let radius = 3.5;
            cr.new_sub_path();
            for (cx, cy, start) in [
                (x + size - radius, y + radius, -std::f64::consts::FRAC_PI_2),
                (x + size - radius, y + size - radius, 0.0),
                (x + radius, y + size - radius, std::f64::consts::FRAC_PI_2),
                (x + radius, y + radius, std::f64::consts::PI),
            ] {
                cr.arc(cx, cy, radius, start, start + std::f64::consts::FRAC_PI_2);
            }
            cr.close_path();
            if *done {
                cr.set_source_rgba(r, g, b, 1.0);
                let _ = cr.fill();
                cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                cr.set_line_width(2.0);
                cr.move_to(x + size * 0.24, y + size * 0.52);
                cr.line_to(x + size * 0.43, y + size * 0.71);
                cr.line_to(x + size * 0.77, y + size * 0.30);
                let _ = cr.stroke();
            } else {
                cr.set_source_rgba(paper.red() as f64, paper.green() as f64, paper.blue() as f64, 1.0);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.5);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(r, g, b, 0.9);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
        }
    }
}

type KeyCommand = ((bool, bool, bool, &'static str), &'static str);

const KEY_TABLE: [KeyCommand; 18] = [
    ((true, false, false, "b"), "bold"),
    ((true, false, false, "i"), "italic"),
    ((true, true, false, "x"), "strike"),
    ((true, true, false, "h"), "highlight"),
    ((true, false, false, "e"), "code"),
    ((true, false, false, "k"), "link"),
    ((true, false, true, "1"), "h1"),
    ((true, false, true, "2"), "h2"),
    ((true, false, true, "3"), "h3"),
    ((true, true, false, "7"), "bullet"),
    ((true, true, false, "8"), "bullet"),
    ((true, true, false, "9"), "number"),
    ((true, true, false, "l"), "check"),
    ((true, true, false, "b"), "quote"),
    ((true, false, true, "k"), "codeblock"),
    ((true, false, true, "t"), "table"),
    ((true, false, true, "r"), "divider"),
    ((true, true, false, "d"), "date"),
];

const MENU_COMMANDS: [&str; 24] = [
    "bold", "italic", "strike", "highlight", "code", "link", "h1", "h2", "h3", "bullet", "number", "check", "quote",
    "codeblock", "table", "divider", "date", "copy", "find", "float", "keep", "archive", "delete", "export",
];

pub struct NoteEditor {
    pub root: gtk::Box,
    pub note_id: String,
    title_entry: gtk::Text,
    edited_label: gtk::Label,
    pin_button: gtk::ToggleButton,
    tools_button: gtk::MenuButton,
    group_menu: gio::Menu,
    pub buffer: gtk::TextBuffer,
    pub text_view: NoteTextView,
    find_revealer: gtk::Revealer,
    find_entry: gtk::SearchEntry,
    find_count: gtk::Label,
    tags_entry: gtk::Text,
    tag_suggestions: gtk::Box,
    color_buttons: Vec<(i64, gtk::ToggleButton)>,
    save_timer: Timer,
    style_timer: Timer,
    cursor_line: Cell<Option<i32>>,
    links: RefCell<Vec<(i32, i32, String)>>,
    checkboxes: RefCell<Vec<i32>>,
    loading: Cell<bool>,
    syncing: Cell<bool>,
    last_saved: RefCell<Option<String>>,
    normalize_range: Cell<Option<(i32, i32)>>,
    open_popovers: Cell<i32>,
    image_space_tags: RefCell<HashSet<String>>,
    matches: RefCell<Vec<i32>>,
    match_index: Cell<i32>,
    match_length: Cell<i32>,
    store_handler: Cell<u64>,
    settings_handler: Cell<u64>,
    clock: RefCell<Option<glib::SourceId>>,
    on_close: RefCell<Option<Rc<dyn Fn()>>>,
    disposed: Cell<bool>,
}

impl NoteEditor {
    pub fn new(note_id: &str) -> Rc<NoteEditor> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("note-editor");

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.add_css_class("note-header");
        let title_entry = gtk::Text::builder().placeholder_text("Title").hexpand(true).css_classes(["note-title"]).build();
        header.append(&title_entry);
        let edited_label = gtk::Label::builder().css_classes(["note-edited"]).build();
        header.append(&edited_label);
        let pin_button = gtk::ToggleButton::builder()
            .icon_name("view-pin-symbolic")
            .css_classes(["flat", "note-tool"])
            .tooltip_text("Pin to Center")
            .build();
        set_label(&pin_button, "Pin to Center");
        header.append(&pin_button);
        let tools_button = gtk::MenuButton::builder()
            .icon_name("applications-engineering-symbolic")
            .css_classes(["flat", "note-tool"])
            .tooltip_text("Tools")
            .build();
        set_label(&tools_button, "Tools");
        let group_menu = gio::Menu::new();
        tools_button.set_menu_model(Some(&tools_menu(&group_menu)));
        header.append(&tools_button);
        root.append(&header);

        let buffer = gtk::TextBuffer::new(None);
        let text_view = NoteTextView::new(&buffer);
        set_label(&text_view, "Note text");
        let table = buffer.tag_table();
        for name in STYLE_TAGS.iter().chain(["search", "search-current"].iter()) {
            table.add(&gtk::TextTag::new(Some(name)));
        }
        let scroller = gtk::ScrolledWindow::builder()
            .child(&text_view)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["note-scroll"])
            .build();
        root.append(&scroller);

        let find_revealer = gtk::Revealer::builder().transition_type(gtk::RevealerTransitionType::SlideUp).build();
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.add_css_class("note-find");
        let find_entry = gtk::SearchEntry::builder().hexpand(true).placeholder_text("Find in note").build();
        let find_count = gtk::Label::builder().css_classes(["dim-label", "caption"]).build();
        bar.append(&find_entry);
        bar.append(&find_count);
        let previous = gtk::Button::builder().icon_name("go-up-symbolic").css_classes(["flat"]).tooltip_text("Previous match").build();
        let next = gtk::Button::builder().icon_name("go-down-symbolic").css_classes(["flat"]).tooltip_text("Next match").build();
        bar.append(&previous);
        bar.append(&next);
        find_revealer.set_child(Some(&bar));
        root.append(&find_revealer);

        let footer = gtk::Box::new(gtk::Orientation::Vertical, 8);
        footer.add_css_class("note-footer");
        let tags_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tags_row.append(&gtk::Label::builder().label("#").css_classes(["note-hash"]).build());
        let tags_entry = gtk::Text::builder().placeholder_text("Add tags").hexpand(true).css_classes(["note-tags"]).build();
        set_label(&tags_entry, "Tags");
        tags_row.append(&tags_entry);
        let tag_suggestions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tag_suggestions.add_css_class("tag-suggestions");
        footer.append(&tags_row);
        footer.append(&tag_suggestions);
        let colors = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        colors.add_css_class("note-colors");
        let mut color_buttons = Vec::new();
        for color in COLOR_ORDER {
            let button = gtk::ToggleButton::builder()
                .css_classes(["swatch".to_string(), format!("swatch-{}", color_slug(color))])
                .tooltip_text(color_name(color))
                .valign(gtk::Align::Center)
                .build();
            set_label(&button, color_name(color));
            colors.append(&button);
            color_buttons.push((color, button));
        }
        footer.append(&colors);
        root.append(&footer);

        let editor = Rc::new(NoteEditor {
            root,
            note_id: note_id.to_string(),
            title_entry,
            edited_label,
            pin_button,
            tools_button,
            group_menu,
            buffer,
            text_view,
            find_revealer,
            find_entry,
            find_count,
            tags_entry,
            tag_suggestions,
            color_buttons,
            save_timer: Timer::default(),
            style_timer: Timer::default(),
            cursor_line: Cell::new(None),
            links: RefCell::new(Vec::new()),
            checkboxes: RefCell::new(Vec::new()),
            loading: Cell::new(false),
            syncing: Cell::new(false),
            last_saved: RefCell::new(None),
            normalize_range: Cell::new(None),
            open_popovers: Cell::new(0),
            image_space_tags: RefCell::new(HashSet::new()),
            matches: RefCell::new(Vec::new()),
            match_index: Cell::new(-1),
            match_length: Cell::new(0),
            store_handler: Cell::new(0),
            settings_handler: Cell::new(0),
            clock: RefCell::new(None),
            on_close: RefCell::new(None),
            disposed: Cell::new(false),
        });
        editor.connect_widgets(&previous, &next);
        editor.install_actions();
        editor.install_keys();
        editor.load();
        editor
    }

    pub fn note(&self) -> Option<Note> {
        app().store.get(&self.note_id)
    }

    pub fn set_on_close(&self, callback: impl Fn() + 'static) {
        *self.on_close.borrow_mut() = Some(Rc::new(callback));
    }

    fn request_close(&self) {
        let callback = self.on_close.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }

    fn connect_widgets(self: &Rc<Self>, previous: &gtk::Button, next: &gtk::Button) {
        let this = self;
        this.title_entry.connect_changed(glib::clone!(#[weak] this, move |_| {
            if !this.loading.get() {
                this.schedule_save();
            }
        }));
        this.title_entry.connect_activate(glib::clone!(#[weak] this, move |_| {
            this.text_view.grab_focus();
        }));
        this.pin_button.connect_toggled(glib::clone!(#[weak] this, move |button| this.on_pin_toggled(button)));
        this.tools_button.set_create_popup_func(glib::clone!(#[weak] this, move |_| this.refresh_group_menu()));
        if let Some(popover) = this.tools_button.popover() {
            popover.connect_show(glib::clone!(#[weak] this, move |_| this.popover_changed(1)));
            popover.connect_hide(glib::clone!(#[weak] this, move |_| this.popover_changed(-1)));
        }

        this.buffer.connect_changed(glib::clone!(#[weak] this, move |_| {
            if this.loading.get() {
                return;
            }
            this.schedule_save();
            this.schedule_restyle();
        }));
        this.buffer.connect_local("insert-text", true, glib::clone!(#[weak] this, #[upgrade_or] None, move |values| {
            if let (Some(Ok(location)), Some(Ok(text))) =
                (values.get(1).map(|v| v.get::<gtk::TextIter>()), values.get(2).map(|v| v.get::<String>()))
            {
                this.on_inserted(&location, &text);
            }
            None
        }));
        this.buffer.connect_mark_set(glib::clone!(#[weak] this, move |buffer, iter, mark| {
            if *mark == buffer.get_insert() && Some(iter.line()) != this.cursor_line.get() {
                this.schedule_restyle();
            }
        }));

        let text_focus = gtk::EventControllerFocus::new();
        text_focus.connect_enter(glib::clone!(#[weak] this, move |_| this.schedule_restyle()));
        text_focus.connect_leave(glib::clone!(#[weak] this, move |_| this.schedule_restyle()));
        this.text_view.add_controller(text_focus);
        let click = gtk::GestureClick::builder().button(1).build();
        click.connect_released(glib::clone!(#[weak] this, move |gesture, presses, x, y| {
            this.on_text_click(gesture, presses, x, y)
        }));
        this.text_view.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(#[weak] this, move |_, x, y| this.on_text_motion(x, y)));
        this.text_view.add_controller(motion);
        let drop = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        drop.set_types(&[gdk::FileList::static_type(), gdk::Texture::static_type()]);
        drop.connect_drop(glib::clone!(#[weak] this, #[upgrade_or] false, move |_, value, _, _| this.on_drop(value)));
        this.text_view.add_controller(drop);
        this.text_view.connect_paste_clipboard(glib::clone!(#[weak] this, move |view| this.on_paste(view)));
        let weak = Rc::downgrade(this);
        *this.text_view.imp().on_width_changed.borrow_mut() = Some(Box::new(move || {
            if let Some(editor) = weak.upgrade() {
                editor.schedule_restyle();
            }
        }));

        this.find_entry.connect_search_changed(glib::clone!(#[weak] this, move |_| this.find(0)));
        this.find_entry.connect_activate(glib::clone!(#[weak] this, move |_| this.find(1)));
        this.find_entry.connect_next_match(glib::clone!(#[weak] this, move |_| this.find(1)));
        this.find_entry.connect_previous_match(glib::clone!(#[weak] this, move |_| this.find(-1)));
        this.find_entry.connect_stop_search(glib::clone!(#[weak] this, move |_| this.hide_find()));
        previous.connect_clicked(glib::clone!(#[weak] this, move |_| this.find(-1)));
        next.connect_clicked(glib::clone!(#[weak] this, move |_| this.find(1)));

        this.tags_entry.connect_changed(glib::clone!(#[weak] this, move |_| this.refresh_suggestions()));
        this.tags_entry.connect_activate(glib::clone!(#[weak] this, move |_| this.commit_tags(true)));
        let tags_focus = gtk::EventControllerFocus::new();
        tags_focus.connect_leave(glib::clone!(#[weak] this, move |_| {
            this.commit_tags(true);
            this.tags_entry.select_region(0, 0);
        }));
        this.tags_entry.add_controller(tags_focus);
        for (color, button) in &this.color_buttons {
            let color = *color;
            button.connect_toggled(glib::clone!(#[weak] this, move |button| this.on_color_toggled(button, color)));
        }

        let store = app().store.clone();
        let weak = Rc::downgrade(this);
        this.store_handler.set(store.changed.connect(move |note_id| {
            if let Some(editor) = weak.upgrade() {
                editor.on_store_changed(note_id);
            }
        }));
        let weak = Rc::downgrade(this);
        this.settings_handler.set(app().settings.changed.connect(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.sync_chrome();
            }
        }));
        let weak = Rc::downgrade(this);
        let clock = glib::timeout_add_seconds_local(30, move || match weak.upgrade() {
            Some(editor) => {
                editor.tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        *this.clock.borrow_mut() = Some(clock);
        this.root.connect_destroy(glib::clone!(#[weak] this, move |_| this.dispose()));
    }

    pub fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        self.flush();
        app().store.changed.disconnect(self.store_handler.get());
        app().settings.changed.disconnect(self.settings_handler.get());
        if let Some(clock) = self.clock.borrow_mut().take() {
            clock.remove();
        }
        self.style_timer.cancel();
    }

    fn popover_changed(&self, delta: i32) {
        self.open_popovers.set((self.open_popovers.get() + delta).max(0));
    }

    pub fn has_open_popover(&self) -> bool {
        self.open_popovers.get() > 0
    }

    fn install_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        for name in MENU_COMMANDS.iter().copied().chain(["insert-image"]) {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(editor) = weak.upgrade() {
                    editor.run_command(name);
                }
            });
            group.add_action(&action);
        }
        let new_group = gio::SimpleAction::new("new-group", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        new_group.connect_activate(move |_, _| {
            if let Some(editor) = weak.upgrade() {
                editor.run_command("new-group");
            }
        });
        group.add_action(&new_group);
        let move_to = gio::SimpleAction::new("move-to", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        move_to.connect_activate(move |_, parameter| {
            let Some(editor) = weak.upgrade() else {
                return;
            };
            let value = parameter.and_then(|p| p.str().map(str::to_string)).unwrap_or_default();
            let group_id = value.split_once('|').map(|(_, group)| group.to_string()).filter(|g| !g.is_empty());
            app().move_to_group(&editor.note_id, group_id);
        });
        group.add_action(&move_to);
        self.root.insert_action_group("note", Some(&group));
    }

    fn run_command(self: &Rc<Self>, name: &str) {
        match name {
            "bold" => self.wrap("**"),
            "italic" => self.wrap("*"),
            "strike" => self.wrap("~~"),
            "highlight" => self.wrap("=="),
            "code" => self.wrap("`"),
            "link" => self.insert_link(),
            "h1" | "h2" | "h3" | "bullet" | "number" | "check" | "quote" => self.set_block(name),
            "codeblock" => self.insert_block("```\n\n```", 4),
            "table" => self.insert_block("| Column | Column |\n| --- | --- |\n|  |  |\n", 0),
            "divider" => self.insert_block("---\n", 0),
            "date" => self.buffer.insert_at_cursor(&Local::now().format("%A, %-d %B %Y").to_string()),
            "copy" => app().copy_note(&self.note_id),
            "find" => self.show_find(),
            "float" => app().toggle_float(&self.note_id),
            "keep" => self.toggle_keep(),
            "archive" => self.archive(),
            "delete" => self.delete(),
            "export" => app().export_note(&self.note_id, self.window()),
            "insert-image" => self.pick_image(),
            "new-group" => {
                let weak = Rc::downgrade(self);
                let note_id = self.note_id.clone();
                prompt_name(
                    &self.tools_button,
                    "New group",
                    "",
                    move |name| app().new_group_with(Some(&note_id), &name),
                    Some(Box::new(move |shown| {
                        if let Some(editor) = weak.upgrade() {
                            editor.popover_changed(if shown { 1 } else { -1 });
                        }
                    })),
                );
            }
            _ => {}
        }
    }

    fn window(&self) -> Option<gtk::Window> {
        self.root.root().and_downcast::<gtk::Window>()
    }

    fn refresh_group_menu(&self) {
        self.group_menu.remove_all();
        app().fill_group_menu(&self.group_menu, &self.note_id, "note");
    }

    fn archive(&self) {
        let note = self.note();
        self.request_close();
        if let Some(note) = note {
            if note.is_archived() {
                app().store.unarchive(&note.id);
            } else {
                app().store.archive(&note.id, false);
                app().announce(&format!("Archived “{}”", note.display_title()), false);
            }
        }
    }

    fn delete(&self) {
        self.request_close();
        app().delete_with_undo(&self.note_id);
    }

    fn install_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, keyval, keycode, state| match weak.upgrade() {
            Some(editor) if editor.on_key(keyval, keycode, state) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        self.root.add_controller(keys);
    }

    fn base_key_name(keyval: gdk::Key, keycode: u32) -> String {
        let base = gdk::Display::default()
            .and_then(|display| display.translate_key(keycode, gdk::ModifierType::empty(), 0))
            .map(|(key, _, _, _)| key)
            .unwrap_or(keyval);
        base.to_lower().name().map(|name| name.to_lowercase()).unwrap_or_default()
    }

    fn on_key(self: &Rc<Self>, keyval: gdk::Key, keycode: u32, state: gdk::ModifierType) -> bool {
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let name = Self::base_key_name(keyval, keycode);
        let in_text = self.text_view.has_focus();
        let is_return = keyval == gdk::Key::Return || keyval == gdk::Key::KP_Enter;

        if keyval == gdk::Key::Escape && !ctrl {
            if self.find_revealer.reveals_child() {
                self.hide_find();
            } else {
                self.request_close();
            }
            return true;
        }
        if ctrl && !alt && !shift && name == "w" {
            self.request_close();
            return true;
        }
        if ctrl && !alt && name == "f" {
            self.show_find();
            return true;
        }
        if ctrl && !alt && name == "g" {
            self.find(if shift { -1 } else { 1 });
            return true;
        }
        if ctrl && alt && name == "c" {
            if let Some(note) = self.note() {
                self.set_color(next_color(note.color, if shift { -1 } else { 1 }));
            }
            return true;
        }
        if ctrl && alt && name == "p" {
            app().toggle_float(&self.note_id);
            return true;
        }
        if ctrl && shift && !alt && name == "e" {
            self.archive();
            return true;
        }
        if ctrl && shift && (keyval == gdk::Key::BackSpace || keyval == gdk::Key::Delete) {
            self.delete();
            return true;
        }
        if !in_text {
            return false;
        }
        if keyval == gdk::Key::BackSpace && !(ctrl || alt || shift) && self.remove_image_above() {
            return true;
        }
        if ctrl && is_return {
            self.toggle_check_at_cursor();
            return true;
        }
        if is_return && !(ctrl || alt || shift) {
            return self.continue_list();
        }
        if (keyval == gdk::Key::Tab || keyval == gdk::Key::ISO_Left_Tab) && !ctrl {
            return self.indent(shift || keyval == gdk::Key::ISO_Left_Tab);
        }
        if let Some((_, command)) = KEY_TABLE.iter().find(|(key, _)| *key == (ctrl, shift, alt, name.as_str())) {
            self.run_command(command);
            return true;
        }
        false
    }

    pub fn load(self: &Rc<Self>) {
        let Some(note) = self.note() else {
            return;
        };
        self.loading.set(true);
        self.title_entry.set_text(&note.title);
        self.tags_entry.set_text(&tags_text(&note.tags));
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&md::to_display(&note.body));
        self.buffer.end_irreversible_action();
        self.buffer.place_cursor(&self.buffer.end_iter());
        self.tags_entry.select_region(0, 0);
        *self.last_saved.borrow_mut() = Some(note.body.clone());
        self.text_view.set_editable(!note.locked);
        self.title_entry.set_editable(!note.locked);
        self.loading.set(false);
        self.sync_chrome();
        self.restyle();
    }

    fn sync_chrome(&self) {
        let Some(note) = self.note() else {
            return;
        };
        self.edited_label.set_text(&relative_edited(&note.edited_at));
        self.syncing.set(true);
        for (color, button) in &self.color_buttons {
            button.set_active(*color == note.color);
        }
        self.pin_button.set_active(note.is_pinned());
        self.syncing.set(false);
        remove_classes_with_prefix(&self.root, "paper-");
        self.root.add_css_class(&format!("paper-{}", color_slug(note.color)));
        self.apply_tag_colors(&note);
        let colors = swatch(note.color);
        *self.text_view.imp().box_color.borrow_mut() = Some(rgba(colors.accent, 1.0));
        *self.text_view.imp().paper_color.borrow_mut() = Some(rgba(colors.paper, 1.0));
        self.text_view.queue_draw();
        self.refresh_suggestions();
    }

    fn apply_tag_colors(&self, note: &Note) {
        let colors = swatch(note.color);
        let table = self.buffer.tag_table();
        let tag = |name: &str| table.lookup(name).expect("style tag");
        tag("h1").set_scale(1.45);
        tag("h1").set_weight(800);
        tag("h2").set_scale(1.25);
        tag("h2").set_weight(700);
        tag("h3").set_scale(1.1);
        tag("h3").set_weight(700);
        tag("bold").set_weight(700);
        tag("italic").set_style(pango::Style::Italic);
        tag("strike").set_strikethrough(true);
        tag("highlight").set_background_rgba(Some(&rgba("#FFE45C", 0.65)));
        tag("code").set_family(Some("monospace"));
        tag("code").set_background_rgba(Some(&rgba("#000000", 0.07)));
        tag("codeblock").set_family(Some("monospace"));
        tag("codeblock").set_paragraph_background_rgba(Some(&rgba("#000000", 0.06)));
        tag("table").set_family(Some("monospace"));
        let quote = tag("quote");
        quote.set_foreground_rgba(Some(&rgba(colors.secondary, 1.0)));
        quote.set_style(pango::Style::Italic);
        quote.set_left_margin(14);
        tag("link").set_foreground_rgba(Some(&rgba(colors.accent, 1.0)));
        tag("link").set_underline(pango::Underline::Single);
        tag("marker").set_foreground_rgba(Some(&rgba(colors.accent, 1.0)));
        tag("marker").set_weight(700);
        let checkbox = tag("checkbox");
        checkbox.set_foreground_rgba(Some(&rgba(colors.paper, 1.0)));
        checkbox.set_family(Some("monospace"));
        checkbox.set_letter_spacing(4 * pango::SCALE);
        tag("done").set_foreground_rgba(Some(&rgba(colors.secondary, 0.75)));
        tag("done").set_strikethrough(true);
        tag("syntax").set_foreground_rgba(Some(&rgba(colors.secondary, 0.45)));
        tag("hidden").set_invisible(true);
        tag("hr").set_foreground_rgba(Some(&rgba(colors.secondary, 0.5)));
        tag("search").set_background_rgba(Some(&rgba(colors.spine, 0.7)));
        tag("search-current").set_background_rgba(Some(&rgba(colors.accent, 0.45)));
    }

    fn tick(&self) {
        if let Some(note) = self.note() {
            self.edited_label.set_text(&relative_edited(&note.edited_at));
        }
    }

    fn on_store_changed(self: &Rc<Self>, note_id: &str) {
        if !(note_id.is_empty() || note_id == self.note_id) || self.disposed.get() {
            return;
        }
        let Some(note) = self.note() else {
            return;
        };
        if self.last_saved.borrow().as_deref() != Some(note.body.as_str()) && !self.save_timer.active() {
            let offset = self.cursor_iter().offset();
            self.loading.set(true);
            self.buffer.set_text(&md::to_display(&note.body));
            let length = self.buffer.char_count();
            self.buffer.place_cursor(&self.buffer.iter_at_offset(offset.min(length)));
            *self.last_saved.borrow_mut() = Some(note.body.clone());
            self.loading.set(false);
            self.restyle();
        }
        if note.title != self.title_entry.text().as_str() && !self.title_entry.has_focus() {
            self.loading.set(true);
            self.title_entry.set_text(&note.title);
            self.loading.set(false);
        }
        self.sync_chrome();
    }

    pub fn text(&self) -> String {
        let (start, end) = self.buffer.bounds();
        self.buffer.slice(&start, &end, true).to_string()
    }

    fn cursor_iter(&self) -> gtk::TextIter {
        self.buffer.iter_at_mark(&self.buffer.get_insert())
    }

    fn commit_tags(&self, normalize: bool) {
        let Some(note) = self.note() else {
            return;
        };
        let tags = parse_tags(&self.tags_entry.text());
        if tags != note.tags {
            let next = tags.clone();
            app().store.update(&self.note_id, move |note| note.tags = next);
        }
        if normalize {
            let text = tags_text(&tags);
            if text != self.tags_entry.text().as_str() {
                self.tags_entry.set_text(&text);
            }
        }
    }

    fn refresh_suggestions(&self) {
        crate::widgets::clear(&self.tag_suggestions);
        let text = self.tags_entry.text().to_string();
        let current = parse_tags(&text);
        let fragment = if !text.is_empty() && !text.ends_with(' ') {
            text.split_whitespace().last().unwrap_or("").trim_start_matches('#').to_lowercase()
        } else {
            String::new()
        };
        let mut candidates: Vec<String> = app()
            .store
            .all_tags()
            .into_iter()
            .map(|(tag, _)| tag)
            .filter(|tag| !current.contains(tag) && (fragment.is_empty() || tag.starts_with(&fragment)))
            .collect();
        if !self.tags_entry.has_focus() {
            candidates.clear();
        }
        for tag in candidates.iter().take(5) {
            let chip = gtk::Button::builder().label(format!("#{tag}")).css_classes(["tag-chip", "flat"]).build();
            let tag = tag.clone();
            let fragment = fragment.clone();
            let entry = self.tags_entry.clone();
            let note_id = self.note_id.clone();
            chip.connect_clicked(move |_| {
                let mut words: Vec<String> = entry.text().split_whitespace().map(str::to_string).collect();
                if !fragment.is_empty() && !words.is_empty() {
                    words.pop();
                }
                words.push(format!("#{tag}"));
                entry.set_text(&(words.join(" ") + " "));
                entry.set_position(-1);
                entry.grab_focus();
                let tags = parse_tags(&entry.text());
                app().store.update(&note_id, move |note| note.tags = tags);
            });
            self.tag_suggestions.append(&chip);
        }
        self.tag_suggestions.set_visible(!candidates.is_empty());
    }

    fn schedule_save(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.save_timer.start(350, move || {
            if let Some(editor) = weak.upgrade() {
                editor.save();
            }
        });
    }

    fn save(&self) {
        let Some(note) = self.note() else {
            return;
        };
        if note.locked {
            return;
        }
        let body = md::to_markdown(&self.text());
        let title = self.title_entry.text().to_string();
        *self.last_saved.borrow_mut() = Some(body.clone());
        app().store.update(&self.note_id, move |note| {
            note.title = title;
            note.body = body;
        });
    }

    pub fn flush(&self) {
        if self.save_timer.active() {
            self.save_timer.cancel();
            self.save();
        }
        self.commit_tags(false);
    }

    fn schedule_restyle(self: &Rc<Self>) {
        if self.style_timer.active() || self.disposed.get() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.style_timer.idle(move || {
            if let Some(editor) = weak.upgrade() {
                editor.restyle();
            }
        });
    }

    fn restyle(self: &Rc<Self>) {
        self.style_timer.cancel();
        if self.disposed.get() {
            return;
        }
        if self.normalize_range.get().is_some() {
            self.normalize();
        }
        let (start, end) = self.buffer.bounds();
        for name in STYLE_TAGS {
            self.buffer.remove_tag_by_name(name, &start, &end);
        }
        for name in self.image_space_tags.borrow().iter() {
            self.buffer.remove_tag_by_name(name, &start, &end);
        }
        let cursor_line = if self.text_view.has_focus() { Some(self.cursor_iter().line()) } else { None };
        self.cursor_line.set(cursor_line);
        let mut links = Vec::new();
        let mut checkboxes = Vec::new();
        let mut images = Vec::new();
        let text = self.text();
        for span in md::style_spans(&text, cursor_line.map(|line| line as usize)) {
            let s = self.buffer.iter_at_offset(span.start as i32);
            let e = self.buffer.iter_at_offset(span.end as i32);
            if span.tag == "image" {
                images.push((s.line(), span.data.clone().unwrap_or_default(), s));
                continue;
            }
            self.buffer.apply_tag_by_name(span.tag, &s, &e);
            if span.tag == "link" {
                links.push((span.start as i32, span.end as i32, span.data.clone().unwrap_or_default()));
            } else if span.tag == "checkbox" {
                checkboxes.push(span.start as i32);
            }
        }
        let characters: Vec<char> = text.chars().collect();
        let boxes = checkboxes
            .iter()
            .filter(|offset| (**offset as usize) < characters.len())
            .map(|offset| (*offset, characters[*offset as usize].to_string() == md::DONE_BOX))
            .collect();
        *self.text_view.imp().boxes.borrow_mut() = boxes;
        *self.links.borrow_mut() = links;
        *self.checkboxes.borrow_mut() = checkboxes;
        self.layout_images(images);
        self.text_view.queue_draw();
    }

    fn layout_images(&self, images: Vec<(i32, String, gtk::TextIter)>) {
        let limit = self.text_view.image_width_limit() as f64;
        let mut placed = Vec::new();
        let table = self.buffer.tag_table();
        for (line, digest, start) in images {
            let Some(texture) = app().image_texture(&digest) else {
                continue;
            };
            if limit <= 0.0 || texture.width() <= 0 {
                continue;
            }
            let mut width = limit.min(texture.width() as f64);
            let mut height = texture.height() as f64 * width / texture.width() as f64;
            if height > IMAGE_MAX_HEIGHT {
                width = width * IMAGE_MAX_HEIGHT / height;
                height = IMAGE_MAX_HEIGHT;
            }
            let (width, height) = (width as i32, height as i32);
            let tag_name = format!("image-space-{}", height + IMAGE_GAP);
            if table.lookup(&tag_name).is_none() {
                let tag = gtk::TextTag::new(Some(&tag_name));
                tag.set_pixels_above_lines(height + IMAGE_GAP);
                table.add(&tag);
                self.image_space_tags.borrow_mut().insert(tag_name.clone());
            }
            let mut line_end = start;
            if !line_end.ends_line() {
                line_end.forward_to_line_end();
            }
            self.buffer.apply_tag_by_name(&tag_name, &start, &line_end);
            placed.push(PlacedImage { line, texture, width, height, digest });
        }
        *self.text_view.imp().images.borrow_mut() = placed;
    }

    fn remove_image_above(&self) -> bool {
        if self.buffer.has_selection() {
            return false;
        }
        let cursor = self.cursor_iter();
        if cursor.line_offset() != 0 || cursor.line() == 0 {
            return false;
        }
        let Some(mut above) = self.buffer.iter_at_line(cursor.line() - 1) else {
            return false;
        };
        let line = self.buffer.slice(&above, &cursor, true).trim_end_matches('\n').to_string();
        if md::image_digest(&line).is_none() {
            return false;
        }
        let mut cursor = cursor;
        self.buffer.begin_user_action();
        self.buffer.delete(&mut above, &mut cursor);
        self.buffer.end_user_action();
        true
    }

    fn on_inserted(&self, location: &gtk::TextIter, text: &str) {
        if self.loading.get() {
            return;
        }
        let end = location.offset();
        let start = end - text.chars().count() as i32;
        let range = match self.normalize_range.get() {
            Some((a, b)) => (a.min(start), b.max(end)),
            None => (start, end),
        };
        self.normalize_range.set(Some(range));
    }

    fn normalize(self: &Rc<Self>) {
        let Some((start, end)) = self.normalize_range.take() else {
            return;
        };
        let mut first = self.buffer.iter_at_offset(start);
        first.set_line_offset(0);
        let mut last = self.buffer.iter_at_offset(end);
        if !last.ends_line() {
            last.forward_to_line_end();
        }
        let original = self.buffer.slice(&first, &last, true).to_string();
        let mut changed = Vec::new();
        for line in original.split('\n') {
            let mut converted = md::to_display(line);
            let stripped = converted.trim_start().to_string();
            let indent = converted[..converted.len() - stripped.len()].to_string();
            for shortcut in ["[] ", "[ ] ", "• [] ", "• [ ] "] {
                if let Some(rest) = stripped.strip_prefix(shortcut) {
                    converted = format!("{indent}{} {rest}", md::OPEN_BOX);
                    break;
                }
            }
            for shortcut in ["• [x] ", "[x] ", "• [X] ", "[X] "] {
                if let Some(rest) = stripped.strip_prefix(shortcut) {
                    converted = format!("{indent}{} {rest}", md::DONE_BOX);
                    break;
                }
            }
            changed.push(converted);
        }
        let result = changed.join("\n");
        if result == original {
            return;
        }
        let mut cursor = self.cursor_iter().offset();
        let delta = result.chars().count() as i32 - original.chars().count() as i32;
        let base = first.offset();
        self.loading.set(true);
        self.buffer.begin_user_action();
        self.buffer.delete(&mut first, &mut last);
        self.buffer.insert(&mut self.buffer.iter_at_offset(base), &result);
        self.buffer.end_user_action();
        self.loading.set(false);
        if cursor >= base {
            cursor = base.max(cursor + delta);
        }
        self.buffer.place_cursor(&self.buffer.iter_at_offset(cursor));
        self.schedule_save();
    }

    fn current_line_bounds(&self) -> (gtk::TextIter, gtk::TextIter, gtk::TextIter) {
        let cursor = self.cursor_iter();
        let mut start = cursor;
        start.set_line_offset(0);
        let mut end = cursor;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        (start, end, cursor)
    }

    fn line_text(&self, start: &gtk::TextIter, end: &gtk::TextIter) -> String {
        self.buffer.slice(start, end, true).to_string()
    }

    fn replace_line(&self, line_number: i32, new_text: &str, cursor_in_line: Option<i32>) {
        let Some(mut start) = self.buffer.iter_at_line(line_number) else {
            return;
        };
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let base = start.offset();
        self.buffer.begin_user_action();
        self.buffer.delete(&mut start, &mut end);
        self.buffer.insert(&mut self.buffer.iter_at_offset(base), new_text);
        self.buffer.end_user_action();
        if let Some(column) = cursor_in_line {
            let length = new_text.chars().count() as i32;
            self.buffer.place_cursor(&self.buffer.iter_at_offset(base + column.min(length).max(0)));
        }
    }

    fn continue_list(&self) -> bool {
        let (start, end, cursor) = self.current_line_bounds();
        let line = self.line_text(&start, &end);
        let before = self.line_text(&start, &cursor);
        let Some(prefix) = md::continuation(&line) else {
            return false;
        };
        let item = md::parse_list_line(&line);
        if item.as_ref().is_some_and(|item| before.chars().count() < item.prefix_len) {
            return false;
        }
        if md::is_empty_item(&line) {
            self.replace_line(start.line(), "", Some(0));
            return true;
        }
        let line_number = start.line();
        self.buffer.begin_user_action();
        self.buffer.insert_at_cursor(&format!("\n{prefix}"));
        self.buffer.end_user_action();
        if item.is_some_and(|item| item.number.is_some()) {
            self.renumber_from(line_number + 1);
        }
        self.text_view.scroll_mark_onscreen(&self.buffer.get_insert());
        true
    }

    fn renumber_from(&self, line_number: i32) {
        let lines: Vec<String> = self.text().split('\n').map(str::to_string).collect();
        let updated = md::renumber(&lines, line_number as usize);
        for (index, (old, new)) in lines.iter().zip(updated.iter()).enumerate() {
            if old != new {
                let cursor = self.cursor_iter();
                let keep = (cursor.line() == index as i32).then(|| cursor.line_offset());
                self.replace_line(index as i32, new, keep);
            }
        }
    }

    fn indent(&self, outdent: bool) -> bool {
        let (start, end, cursor) = self.current_line_bounds();
        let line = self.line_text(&start, &end);
        if md::parse_list_line(&line).is_none() {
            return false;
        }
        let column = cursor.line_offset();
        let indent_width = md::INDENT.len() as i32;
        if outdent {
            let leading = (line.len() - line.trim_start_matches(' ').len()) as i32;
            let removed = if line.starts_with(md::INDENT) { indent_width } else { leading.min(indent_width) };
            if removed == 0 {
                return true;
            }
            self.replace_line(start.line(), &line[removed as usize..], Some((column - removed).max(0)));
        } else {
            self.replace_line(start.line(), &format!("{}{line}", md::INDENT), Some(column + indent_width));
        }
        true
    }

    fn toggle_check_at_cursor(&self) {
        let (start, end, cursor) = self.current_line_bounds();
        let line = self.line_text(&start, &end);
        let new = md::toggled_check(&line);
        let shift = new.chars().count() as i32 - line.chars().count() as i32;
        self.replace_line(start.line(), &new, Some(cursor.line_offset() + shift));
    }

    fn toggle_check_line(&self, line_number: i32) {
        let Some(start) = self.buffer.iter_at_line(line_number) else {
            return;
        };
        let mut end = start;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let line = self.line_text(&start, &end);
        self.replace_line(line_number, &md::toggled_check(&line), None);
    }

    fn wrap(&self, marker: &str) {
        let marker_length = marker.chars().count() as i32;
        self.buffer.begin_user_action();
        if let Some((mut start, mut end)) = self.buffer.selection_bounds() {
            let text = self.buffer.slice(&start, &end, true).to_string();
            let offset = start.offset();
            if text.starts_with(marker) && text.ends_with(marker) && text.len() >= 2 * marker.len() {
                let inner = text[marker.len()..text.len() - marker.len()].to_string();
                self.buffer.delete(&mut start, &mut end);
                self.buffer.insert(&mut self.buffer.iter_at_offset(offset), &inner);
                let length = inner.chars().count() as i32;
                self.buffer.select_range(&self.buffer.iter_at_offset(offset), &self.buffer.iter_at_offset(offset + length));
            } else {
                self.buffer.delete(&mut start, &mut end);
                self.buffer.insert(&mut self.buffer.iter_at_offset(offset), &format!("{marker}{text}{marker}"));
                let length = text.chars().count() as i32;
                self.buffer.select_range(
                    &self.buffer.iter_at_offset(offset + marker_length),
                    &self.buffer.iter_at_offset(offset + marker_length + length),
                );
            }
        } else {
            self.buffer.insert_at_cursor(&marker.repeat(2));
            let mut cursor = self.cursor_iter();
            cursor.backward_chars(marker_length);
            self.buffer.place_cursor(&cursor);
        }
        self.buffer.end_user_action();
        self.text_view.grab_focus();
    }

    fn insert_link(&self) {
        let selection = self.buffer.selection_bounds();
        let text = selection.map(|(s, e)| self.buffer.slice(&s, &e, true).to_string()).unwrap_or_default();
        self.buffer.begin_user_action();
        if let Some((mut start, mut end)) = selection {
            self.buffer.delete(&mut start, &mut end);
        }
        let back = if text.starts_with("http://") || text.starts_with("https://") {
            self.buffer.insert_at_cursor(&format!("[]({text})"));
            text.chars().count() as i32 + 3
        } else {
            self.buffer.insert_at_cursor(&format!("[{text}]()"));
            1
        };
        let mut cursor = self.cursor_iter();
        cursor.backward_chars(back);
        self.buffer.place_cursor(&cursor);
        self.buffer.end_user_action();
        self.text_view.grab_focus();
    }

    fn set_block(&self, kind: &str) {
        let (start, end, cursor) = self.current_line_bounds();
        let line = self.line_text(&start, &end);
        let new = md::set_block(&line, kind);
        let shift = new.chars().count() as i32 - line.chars().count() as i32;
        self.replace_line(start.line(), &new, Some((cursor.line_offset() + shift).max(0)));
        self.text_view.grab_focus();
    }

    fn insert_block(&self, text: &str, cursor_back: i32) {
        let (start, end, _) = self.current_line_bounds();
        let line = self.line_text(&start, &end);
        let filled = !line.trim().is_empty();
        self.buffer.begin_user_action();
        if filled {
            self.buffer.place_cursor(&end);
        }
        self.buffer.insert_at_cursor(&format!("{}{text}", if filled { "\n" } else { "" }));
        self.buffer.end_user_action();
        if cursor_back > 0 {
            let mut cursor = self.cursor_iter();
            cursor.backward_chars(cursor_back);
            self.buffer.place_cursor(&cursor);
        }
        self.text_view.grab_focus();
    }

    fn pick_image(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        filter.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Insert Image").filters(&filters).build();
        app().dialog_open.set(true);
        let weak = Rc::downgrade(self);
        dialog.open(self.window().as_ref(), None::<&gio::Cancellable>, move |result| {
            app().dialog_open.set(false);
            if let (Some(editor), Ok(file)) = (weak.upgrade(), result) {
                editor.attach_file(&file);
            }
        });
    }

    fn on_drop(self: &Rc<Self>, value: &glib::Value) -> bool {
        if let Ok(texture) = value.get::<gdk::Texture>() {
            self.attach_texture(&texture);
            return true;
        }
        if let Ok(files) = value.get::<gdk::FileList>() {
            for file in files.files() {
                self.attach_file(&file);
            }
            return true;
        }
        false
    }

    fn on_paste(self: &Rc<Self>, view: &NoteTextView) {
        let clipboard = view.clipboard();
        let formats = clipboard.formats();
        if formats.contains_type(gdk::FileList::static_type()) {
            view.stop_signal_emission_by_name("paste-clipboard");
            let weak = Rc::downgrade(self);
            clipboard.read_value_async(
                gdk::FileList::static_type(),
                glib::Priority::DEFAULT,
                None::<&gio::Cancellable>,
                move |result| {
                    let (Some(editor), Ok(value)) = (weak.upgrade(), result) else {
                        return;
                    };
                    if let Ok(files) = value.get::<gdk::FileList>() {
                        for file in files.files() {
                            editor.attach_file(&file);
                        }
                    }
                },
            );
            return;
        }
        let has_text = formats.contains_type(glib::Type::STRING) || formats.contain_mime_type("text/plain");
        if formats.contains_type(gdk::Texture::static_type()) && !has_text {
            view.stop_signal_emission_by_name("paste-clipboard");
            let weak = Rc::downgrade(self);
            clipboard.read_texture_async(None::<&gio::Cancellable>, move |result| {
                let Some(editor) = weak.upgrade() else {
                    return;
                };
                match result {
                    Ok(Some(texture)) => editor.attach_texture(&texture),
                    Ok(None) => {}
                    Err(error) => app().announce(&format!("Could not paste the image: {}", error.message()), false),
                }
            });
        }
    }

    fn attach_texture(&self, texture: &gdk::Texture) {
        let bytes = texture.save_to_png_bytes();
        match app().attachments.add_bytes(&bytes, "image.png") {
            Some(reference) => self.insert_attachment(&format!("![{PASTED_IMAGE_LABEL}]({reference})")),
            None => app().announce("Images can be up to 25 MB", false),
        }
    }

    fn attach_file(&self, file: &gio::File) {
        let name = file.basename().and_then(|n| n.to_str().map(str::to_string)).unwrap_or_else(|| "file".into());
        let reference = match file.load_contents(None::<&gio::Cancellable>) {
            Ok((data, _)) => app().attachments.add_bytes(&data, &name),
            Err(_) => None,
        };
        let Some(reference) = reference else {
            app().announce("Files can be up to 25 MB", false);
            return;
        };
        let label = name.replace('[', "(").replace(']', ")");
        let (content_type, _) = gio::content_type_guess(Some(name.as_str()), None::<&[u8]>);
        let bang = if content_type.starts_with("image/") { "!" } else { "" };
        self.insert_attachment(&format!("{bang}[{label}]({reference})"));
    }

    fn insert_attachment(&self, markdown_line: &str) {
        let (start, _, cursor) = self.current_line_bounds();
        let before = self.line_text(&start, &cursor);
        let lead = if before.trim().is_empty() { "" } else { "\n" };
        self.buffer.begin_user_action();
        self.buffer.insert_at_cursor(&format!("{lead}{markdown_line}\n"));
        self.buffer.end_user_action();
        self.text_view.grab_focus();
        self.text_view.scroll_mark_onscreen(&self.buffer.get_insert());
    }

    fn set_color(&self, color: i64) {
        app().store.update(&self.note_id, |note| note.color = color);
    }

    fn toggle_keep(&self) {
        if let Some(note) = self.note() {
            let keep = !note.keep_on_deck;
            app().store.update(&self.note_id, |note| note.keep_on_deck = keep);
            app().announce(if keep { "Kept on deck" } else { "No longer kept on deck" }, false);
        }
    }

    fn on_color_toggled(&self, button: &gtk::ToggleButton, color: i64) {
        if self.syncing.get() {
            return;
        }
        if button.is_active() {
            self.set_color(color);
        } else if self.note().is_some_and(|note| note.color == color) {
            self.syncing.set(true);
            button.set_active(true);
            self.syncing.set(false);
        }
    }

    fn on_pin_toggled(&self, button: &gtk::ToggleButton) {
        if self.syncing.get() {
            return;
        }
        if !app().store.set_pinned(&self.note_id, button.is_active()) {
            app().announce("Up to five notes can be pinned to the center", false);
            self.syncing.set(true);
            button.set_active(false);
            self.syncing.set(false);
        }
    }

    fn offset_at(&self, x: f64, y: f64) -> Option<i32> {
        let (bx, by) = self.text_view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        self.text_view.iter_at_location(bx, by).map(|iter| iter.offset())
    }

    fn link_at(&self, offset: i32) -> Option<String> {
        self.links.borrow().iter().find(|(start, end, _)| *start <= offset && offset < *end).map(|(_, _, url)| url.clone())
    }

    fn is_checkbox(&self, offset: i32) -> bool {
        let boxes = self.checkboxes.borrow();
        boxes.contains(&offset) || boxes.contains(&(offset - 1))
    }

    fn on_text_click(self: &Rc<Self>, gesture: &gtk::GestureClick, presses: i32, x: f64, y: f64) {
        if let Some(digest) = self.text_view.image_at(x, y) {
            if presses == 2 {
                let window = self.window();
                idle(move || app().open_link(&format!("attachment:{digest}"), window.as_ref()));
                return;
            }
        }
        if presses != 1 {
            return;
        }
        let Some(offset) = self.offset_at(x, y) else {
            return;
        };
        if self.is_checkbox(offset) {
            let line = self.buffer.iter_at_offset(offset).line();
            let weak = Rc::downgrade(self);
            idle(move || {
                if let Some(editor) = weak.upgrade() {
                    editor.toggle_check_line(line);
                }
            });
            return;
        }
        if gesture.current_event_state().contains(gdk::ModifierType::ALT_MASK) || self.buffer.has_selection() {
            return;
        }
        if let Some(url) = self.link_at(offset) {
            let window = self.window();
            idle(move || app().open_link(&url, window.as_ref()));
        }
    }

    fn on_text_motion(&self, x: f64, y: f64) {
        if self.text_view.image_at(x, y).is_some() {
            self.text_view.set_cursor_from_name(Some("zoom-in"));
            return;
        }
        let pointer = self.offset_at(x, y).is_some_and(|offset| self.is_checkbox(offset) || self.link_at(offset).is_some());
        self.text_view.set_cursor_from_name(Some(if pointer { "pointer" } else { "text" }));
    }

    fn show_find(&self) {
        self.find_revealer.set_reveal_child(true);
        if let Some((start, end)) = self.buffer.selection_bounds() {
            self.find_entry.set_text(&self.buffer.slice(&start, &end, true));
        }
        self.find_entry.grab_focus();
    }

    fn hide_find(&self) {
        self.find_revealer.set_reveal_child(false);
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag_by_name("search", &start, &end);
        self.buffer.remove_tag_by_name("search-current", &start, &end);
        self.find_count.set_text("");
        self.text_view.grab_focus();
    }

    fn find(&self, step: i32) {
        let query = self.find_entry.text().to_string();
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag_by_name("search", &start, &end);
        self.buffer.remove_tag_by_name("search-current", &start, &end);
        self.matches.borrow_mut().clear();
        if query.is_empty() {
            self.find_count.set_text("");
            return;
        }
        let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
        let haystack: Vec<char> = self.text().chars().map(fold).collect();
        let needle: Vec<char> = query.chars().map(fold).collect();
        let length = needle.len() as i32;
        let mut found = Vec::new();
        let mut index = 0;
        while index + needle.len() <= haystack.len() {
            if haystack[index..index + needle.len()] == needle[..] {
                found.push(index as i32);
                self.buffer.apply_tag_by_name(
                    "search",
                    &self.buffer.iter_at_offset(index as i32),
                    &self.buffer.iter_at_offset(index as i32 + length),
                );
                index += needle.len().max(1);
            } else {
                index += 1;
            }
        }
        if found.is_empty() {
            self.find_count.set_text("No matches");
            return;
        }
        let count = found.len() as i32;
        let next = if step == 0 { 0 } else { (self.match_index.get() + step).rem_euclid(count) };
        self.match_index.set(next);
        self.match_length.set(length);
        let at = found[next as usize];
        let mut s = self.buffer.iter_at_offset(at);
        let e = self.buffer.iter_at_offset(at + length);
        self.buffer.apply_tag_by_name("search-current", &s, &e);
        self.buffer.select_range(&s, &e);
        self.text_view.scroll_to_iter(&mut s, 0.2, false, 0.0, 0.0);
        self.find_count.set_text(&format!("{} of {count}", next + 1));
        *self.matches.borrow_mut() = found;
    }

    pub fn focus_text(&self, at_end: bool) {
        if let Some(note) = self.note() {
            if note.title.is_empty() && note.body.is_empty() {
                self.title_entry.grab_focus();
                return;
            }
        }
        self.text_view.grab_focus();
        if at_end {
            self.buffer.place_cursor(&self.buffer.end_iter());
        }
    }
}

fn tags_text(tags: &[String]) -> String {
    tags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" ")
}

fn tools_menu(group_menu: &gio::Menu) -> gio::Menu {
    let menu = gio::Menu::new();
    let format = gio::Menu::new();
    for (label, action) in [
        ("Bold", "bold"),
        ("Italic", "italic"),
        ("Strikethrough", "strike"),
        ("Highlight", "highlight"),
        ("Inline Code", "code"),
        ("Link", "link"),
    ] {
        format.append(Some(label), Some(&format!("note.{action}")));
    }
    menu.append_section(None, &format);
    let blocks = gio::Menu::new();
    for (label, action) in [
        ("Heading 1", "h1"),
        ("Heading 2", "h2"),
        ("Heading 3", "h3"),
        ("Bulleted List", "bullet"),
        ("Numbered List", "number"),
        ("Checklist", "check"),
        ("Quote", "quote"),
        ("Code Block", "codeblock"),
        ("Table", "table"),
        ("Divider", "divider"),
        ("Today's Date", "date"),
    ] {
        blocks.append(Some(label), Some(&format!("note.{action}")));
    }
    menu.append_submenu(Some("Format"), &blocks);
    let note = gio::Menu::new();
    note.append(Some("Copy Note"), Some("note.copy"));
    note.append(Some("Insert Image…"), Some("note.insert-image"));
    note.append(Some("Find…"), Some("note.find"));
    note.append(Some("Float Note / Return to Deck"), Some("note.float"));
    note.append(Some("Keep on Deck"), Some("note.keep"));
    note.append_submenu(Some("Move to Group"), group_menu);
    note.append(Some("Export as Markdown…"), Some("note.export"));
    menu.append_section(None, &note);
    let danger = gio::Menu::new();
    danger.append(Some("Archive"), Some("note.archive"));
    danger.append(Some("Delete"), Some("note.delete"));
    menu.append_section(None, &danger);
    menu
}
