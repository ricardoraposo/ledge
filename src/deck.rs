use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::cairo;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::graphene;
use gtk::pango;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::app::app;
use crate::card::monitor_for;
use crate::model::{Note, color_name, color_slug, next_color, plain_text};
use crate::signals::Timer;
use crate::store::{NoteGroup, Store};
use crate::widgets::{VerticalLabel, clear, prompt_name, set_label};

const STRIPE_WIDTH: i32 = 10;
const SEGMENT_HEIGHT: i32 = 18;
const SEGMENT_GAP: i32 = 3;
const STRIPE_MAX: i32 = 420;
const TAB_WIDTH: i32 = 40;
const TAB_MIN: i32 = 52;
const TAB_MAX: i32 = 138;
const TAB_GAP: i32 = 4;
const SHADOW: i32 = 14;
const PEEK_WIDTH: i32 = 300;
const FOLD_DELAY: u64 = 320;
const PEEK_DELAY: u64 = 420;
const DAILY_GROUP: &str = "daily";

fn debug(message: &str) {
    if std::env::var_os("LEDGE_DEBUG").is_some() {
        println!("deck: {message}");
    }
}

#[derive(Clone)]
pub enum GroupRef {
    Daily,
    Group(NoteGroup),
}

impl GroupRef {
    fn id(&self) -> String {
        match self {
            GroupRef::Daily => DAILY_GROUP.into(),
            GroupRef::Group(group) => group.id.clone(),
        }
    }

    fn name(&self) -> String {
        match self {
            GroupRef::Daily => "Daily".into(),
            GroupRef::Group(group) => group.name.clone(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Note,
    Group,
    Header,
}

#[derive(Clone)]
pub struct DeckItem {
    pub kind: Kind,
    pub note: Option<Note>,
    pub group: Option<GroupRef>,
    pub notes: Vec<Note>,
}

impl DeckItem {
    fn note(note: Note) -> Self {
        DeckItem { kind: Kind::Note, note: Some(note), group: None, notes: Vec::new() }
    }

    fn group(kind: Kind, group: GroupRef, notes: Vec<Note>) -> Self {
        DeckItem { kind, note: None, group: Some(group), notes }
    }

    fn key(&self) -> String {
        if let Some(note) = &self.note {
            return format!("note:{}", note.id);
        }
        if let Some(group) = &self.group {
            return format!("group:{}", group.id());
        }
        "item".into()
    }
}

pub fn deck_plan(store: &Store, open_group: Option<&str>) -> Vec<DeckItem> {
    let active = store.active_notes();
    let mut pinned: Vec<Note> = active.iter().filter(|n| n.is_pinned()).cloned().collect();
    pinned.sort_by_key(|n| n.pinned_at);
    let loose: Vec<Note> = active.iter().filter(|n| !n.is_pinned()).cloned().collect();
    let groups = store.live_groups();
    let mut items = Vec::new();
    for group in &groups {
        let members: Vec<Note> = loose
            .iter()
            .filter(|n| n.group_id.as_deref() == Some(group.id.as_str()) && !n.is_daily())
            .cloned()
            .collect();
        if open_group == Some(group.id.as_str()) {
            items.push(DeckItem::group(Kind::Header, GroupRef::Group(group.clone()), members.clone()));
            items.extend(members.into_iter().map(DeckItem::note));
        } else {
            items.push(DeckItem::group(Kind::Group, GroupRef::Group(group.clone()), members));
        }
    }
    let mut dailies: Vec<Note> = loose.iter().filter(|n| n.is_daily()).cloned().collect();
    dailies.sort_by(|a, b| (b.daily_day.clone(), b.created_at).cmp(&(a.daily_day.clone(), a.created_at)));
    if !dailies.is_empty() {
        if open_group == Some(DAILY_GROUP) {
            items.push(DeckItem::group(Kind::Header, GroupRef::Daily, dailies.clone()));
            items.extend(dailies.into_iter().map(DeckItem::note));
        } else {
            items.push(DeckItem::group(Kind::Group, GroupRef::Daily, dailies));
        }
    }
    let rest = loose.into_iter().filter(|n| {
        !n.is_daily() && !n.group_id.as_deref().is_some_and(|id| groups.iter().any(|g| g.id == id))
    });
    items.extend(rest.map(DeckItem::note));
    let middle = items.len() / 2;
    let pinned_items: Vec<DeckItem> = pinned.into_iter().map(DeckItem::note).collect();
    items.splice(middle..middle, pinned_items);
    items
}

pub struct PeekWindow {
    window: gtk::Window,
    holder: gtk::Box,
}

fn passthrough(window: &gtk::Window) {
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&cairo::Region::create()));
    }
}

impl PeekWindow {
    fn new() -> PeekWindow {
        let window = gtk::Window::builder().application(&app().gtk).title("Ledge Peek").build();
        window.add_css_class("deck-window");
        window.init_layer_shell();
        window.set_namespace(Some("ledge-peek"));
        window.set_layer(Layer::Top);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_anchor(Edge::Top, true);
        let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        window.set_child(Some(&holder));
        window.connect_realize(passthrough);
        window.connect_map(passthrough);
        PeekWindow { window, holder }
    }

    fn show_note(&self, note: &Note, y: i32, right: bool, offset: i32, screen_height: i32) {
        clear(&self.holder);
        let card = build_peek_card(note);
        card.set_size_request(PEEK_WIDTH, -1);
        self.holder.append(&card);
        self.window.set_anchor(Edge::Right, right);
        self.window.set_anchor(Edge::Left, !right);
        self.window.set_margin(if right { Edge::Right } else { Edge::Left }, offset - 20);
        let (_, natural) = card.preferred_size();
        let height = natural.height();
        self.window.set_margin(Edge::Top, (y - 38).min(screen_height - height).max(0));
        self.window.set_default_size(PEEK_WIDTH + 44, height);
        self.window.present();
        passthrough(&self.window);
    }

    fn hide(&self) {
        self.window.set_visible(false);
    }
}

pub fn build_peek_card(note: &Note) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("peek");
    card.add_css_class(&format!("paper-{}", color_slug(note.color)));
    let title = note.display_title();
    card.append(&gtk::Label::builder().label(&title).xalign(0.0).css_classes(["peek-title"]).wrap(true).build());
    let mut lines: Vec<String> = plain_text(&note.body).lines().map(str::to_string).collect();
    if note.title.trim().is_empty() && lines.first().is_some_and(|line| line.trim() == title) {
        lines.remove(0);
    }
    let text = lines.join("\n").trim().to_string();
    if !text.is_empty() {
        card.append(
            &gtk::Label::builder()
                .label(&text)
                .xalign(0.0)
                .yalign(0.0)
                .wrap(true)
                .lines(10)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["peek-body"])
                .build(),
        );
    }
    if !note.tags.is_empty() {
        let tags = note.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join("  ");
        card.append(&gtk::Label::builder().label(&tags).xalign(0.0).css_classes(["peek-tags"]).build());
    }
    card
}

pub struct Deck {
    pub window: gtk::Window,
    stack: gtk::Stack,
    stripe: gtk::Box,
    fan_box: gtk::Box,
    column: gtk::Box,
    scroller: gtk::ScrolledWindow,
    tabs_box: gtk::Box,
    buttons: gtk::Box,
    pub fanned: Cell<bool>,
    focus_mode: Cell<bool>,
    open_group: RefCell<Option<String>>,
    fold_timer: Timer,
    fan_timer: Timer,
    peek_timer: Timer,
    reveal_timer: Timer,
    peek_note: RefCell<Option<String>>,
    menu_open: Cell<bool>,
    inside: Cell<bool>,
    tabs: RefCell<Vec<(String, gtk::Button, DeckItem)>>,
    peek: RefCell<Option<PeekWindow>>,
    fan_width: Cell<i32>,
    stripe_height: Cell<i32>,
}

impl Deck {
    pub fn new() -> Rc<Deck> {
        let window = gtk::Window::builder().application(&app().gtk).title("Ledge Deck").build();
        window.add_css_class("deck-window");
        window.init_layer_shell();
        window.set_namespace(Some("ledge-deck"));
        window.set_layer(Layer::Top);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::None)
            .hhomogeneous(false)
            .vhomogeneous(false)
            .interpolate_size(false)
            .build();
        let stripe = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(SEGMENT_GAP)
            .css_classes(["stripe"])
            .valign(gtk::Align::Center)
            .build();
        set_label(&stripe, "Open the deck");
        stack.add_named(&stripe, Some("rest"));
        let fan_box = gtk::Box::builder().css_classes(["fan"]).build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .valign(gtk::Align::Center)
            .css_classes(["deck-column"])
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .propagate_natural_height(true)
            .build();
        let tabs_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(TAB_GAP)
            .css_classes(["tabs"])
            .build();
        scroller.set_child(Some(&tabs_box));
        column.append(&scroller);
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["deck-buttons"])
            .build();
        let new_button = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .css_classes(["deck-round"])
            .tooltip_text("New Note")
            .build();
        set_label(&new_button, "New Note");
        let daily_button = gtk::Button::builder()
            .icon_name("x-office-calendar-symbolic")
            .css_classes(["deck-round"])
            .tooltip_text("Today's Daily")
            .build();
        set_label(&daily_button, "Today's Daily");
        buttons.append(&new_button);
        buttons.append(&daily_button);
        column.append(&buttons);
        stack.add_named(&fan_box, Some("fan"));
        window.set_child(Some(&stack));

        let deck = Rc::new(Deck {
            window,
            stack,
            stripe,
            fan_box,
            column,
            scroller,
            tabs_box,
            buttons,
            fanned: Cell::new(false),
            focus_mode: Cell::new(false),
            open_group: RefCell::new(None),
            fold_timer: Timer::default(),
            fan_timer: Timer::default(),
            peek_timer: Timer::default(),
            reveal_timer: Timer::default(),
            peek_note: RefCell::new(None),
            menu_open: Cell::new(false),
            inside: Cell::new(false),
            tabs: RefCell::new(Vec::new()),
            peek: RefCell::new(None),
            fan_width: Cell::new(TAB_WIDTH),
            stripe_height: Cell::new(60),
        });

        let weak = Rc::downgrade(&deck);
        new_button.connect_clicked(move |_| {
            if let Some(deck) = weak.upgrade() {
                deck.fold(true);
                app().new_note("");
            }
        });
        let weak = Rc::downgrade(&deck);
        daily_button.connect_clicked(move |_| {
            if let Some(deck) = weak.upgrade() {
                deck.fold(true);
                app().open_daily();
            }
        });

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&deck);
        motion.connect_enter(move |_, _, _| {
            if let Some(deck) = weak.upgrade() {
                deck.on_enter();
            }
        });
        let weak = Rc::downgrade(&deck);
        motion.connect_leave(move |_| {
            if let Some(deck) = weak.upgrade() {
                deck.on_leave();
            }
        });
        deck.window.add_controller(motion);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&deck);
        keys.connect_key_pressed(move |_, keyval, _, _| match weak.upgrade() {
            Some(deck) if deck.on_key(keyval) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        deck.window.add_controller(keys);

        deck.install_actions();
        let weak = Rc::downgrade(&deck);
        app().store.changed.connect(move |_| {
            if let Some(deck) = weak.upgrade() {
                deck.rebuild();
            }
        });
        let weak = Rc::downgrade(&deck);
        app().settings.changed.connect(move |key| {
            if let Some(deck) = weak.upgrade() {
                deck.on_settings(key);
            }
        });
        deck.apply_edge();
        deck.rebuild();
        deck.show_rest();
        deck
    }

    fn right(&self) -> bool {
        app().settings.right_edge()
    }

    fn edge_align(&self) -> gtk::Align {
        if self.right() { gtk::Align::End } else { gtk::Align::Start }
    }

    fn apply_edge(&self) {
        let right = self.right();
        self.window.set_anchor(Edge::Left, !right);
        self.window.set_anchor(Edge::Right, right);
        self.window.remove_css_class("edge-left");
        self.window.remove_css_class("edge-right");
        self.window.add_css_class(if right { "edge-right" } else { "edge-left" });
        if self.column.parent().is_none() {
            self.fan_box.append(&self.column);
        }
        self.column.set_hexpand(true);
        self.column.set_halign(self.edge_align());
        self.tabs_box.set_halign(self.edge_align());
        self.buttons.set_halign(self.edge_align());
    }

    fn on_settings(self: &Rc<Self>, key: &str) {
        match key {
            "edge" => {
                self.fold(true);
                self.apply_edge();
                self.rebuild();
            }
            "magnification" | "show_deck_buttons" => self.rebuild(),
            _ => {}
        }
    }

    fn install_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        let commands: [&str; 12] = [
            "open", "float", "pin", "keep", "color", "move-to", "new-group", "archive", "delete", "copy",
            "rename-group", "delete-group",
        ];
        for name in commands {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, parameter| {
                let Some(deck) = weak.upgrade() else {
                    return;
                };
                let value = parameter.and_then(|p| p.str().map(str::to_string)).unwrap_or_default();
                if !matches!(name, "new-group" | "rename-group") {
                    deck.fold(true);
                }
                deck.run_action(name, &value);
            });
            group.add_action(&action);
        }
        self.window.insert_action_group("deck", Some(&group));
    }

    fn run_action(self: &Rc<Self>, name: &str, value: &str) {
        let store = &app().store;
        match name {
            "open" => {
                app().open_note(value, None, true);
            }
            "float" => app().toggle_float(value),
            "pin" => {
                if let Some(note) = store.get(value) {
                    if !store.set_pinned(value, !note.is_pinned()) {
                        app().announce("Up to five notes can be pinned to the center", false);
                    }
                }
            }
            "keep" => {
                if let Some(note) = store.get(value) {
                    let keep = !note.keep_on_deck;
                    store.update(value, |note| note.keep_on_deck = keep);
                }
            }
            "color" => {
                if let Some(note) = store.get(value) {
                    let color = next_color(note.color, 1);
                    store.update(value, |note| note.color = color);
                }
            }
            "move-to" => {
                let (note_id, group_id) = value.split_once('|').unwrap_or((value, ""));
                app().move_to_group(note_id, Some(group_id.to_string()).filter(|g| !g.is_empty()));
            }
            "new-group" => {
                let note_id = value.to_string();
                self.prompt_on(&format!("note:{value}"), "New group", "", move |name| {
                    app().new_group_with(Some(&note_id), &name)
                });
            }
            "archive" => app().archive_note(value),
            "delete" => app().delete_with_undo(value),
            "copy" => app().copy_note(value),
            "rename-group" => {
                let initial = store.group(value).map(|g| g.name).unwrap_or_default();
                let group_id = value.to_string();
                self.prompt_on(&format!("group:{value}"), "Rename group", &initial, move |name| {
                    let name: String = name.chars().take(60).collect();
                    app().store.update_group(&group_id, |group| group.name = name);
                });
            }
            "delete-group" => store.delete_group(value),
            _ => {}
        }
    }

    fn prompt_on(self: &Rc<Self>, key: &str, heading: &str, initial: &str, on_accept: impl Fn(String) + 'static) {
        let anchor = self.tabs.borrow().iter().find(|(k, _, _)| k == key).map(|(_, button, _)| button.clone());
        let Some(anchor) = anchor else {
            return;
        };
        self.menu_open.set(true);
        let heading = heading.to_string();
        let initial = initial.to_string();
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            prompt_name(
                &anchor,
                &heading,
                &initial,
                on_accept,
                Some(Box::new(move |open| {
                    if let Some(deck) = weak.upgrade() {
                        deck.set_menu_open(open);
                    }
                })),
            );
        });
    }

    pub fn rebuild(self: &Rc<Self>) {
        debug("rebuild");
        self.hide_peek();
        let open = self.open_group.borrow().clone();
        let mut items = deck_plan(&app().store, open.as_deref());
        if open.is_some() && !items.iter().any(|item| item.kind == Kind::Header) {
            *self.open_group.borrow_mut() = None;
            items = deck_plan(&app().store, None);
        }
        self.build_stripe(&items);
        self.build_tabs(&items);
        self.buttons.set_visible(app().settings.flag("show_deck_buttons"));
        self.resize();
    }

    fn build_stripe(&self, items: &[DeckItem]) {
        clear(&self.stripe);
        let count = items.len().max(1) as i32;
        let height = SEGMENT_HEIGHT.min(((STRIPE_MAX - SEGMENT_GAP * (count - 1)) / count).max(6));
        if items.is_empty() {
            let segment = gtk::Box::builder().css_classes(["segment", "segment-empty"]).build();
            segment.set_size_request(5, 36);
            self.stripe.append(&segment);
        }
        for item in items {
            let segment = gtk::Box::builder().css_classes(["segment"]).build();
            let class = match (&item.note, &item.group) {
                (Some(note), _) => format!("spine-{}", color_slug(note.color)),
                (None, Some(GroupRef::Daily)) => "spine-daily".into(),
                (None, Some(GroupRef::Group(group))) => format!("group-color-{}", group.color_index()),
                _ => "segment-empty".into(),
            };
            segment.add_css_class(&class);
            segment.set_size_request(5, height);
            self.stripe.append(&segment);
        }
        self.stripe_height.set(count * height + (count - 1) * SEGMENT_GAP + 16);
    }

    fn tab_height(&self, label: &str) -> i32 {
        let (width, _) = self.window.create_pango_layout(Some(label)).pixel_size();
        (width + 26).clamp(TAB_MIN, TAB_MAX)
    }

    fn build_tabs(self: &Rc<Self>, items: &[DeckItem]) {
        clear(&self.tabs_box);
        let mut tabs = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let tab = self.make_tab(item, index);
            tab.set_size_request(TAB_WIDTH, -1);
            self.tabs_box.append(&tab);
            tabs.push((item.key(), tab, item.clone()));
        }
        *self.tabs.borrow_mut() = tabs;
        let extra = if self.open_group.borrow().is_some() { 14 } else { 0 };
        self.fan_width.set(TAB_WIDTH + app().settings.lift() + SHADOW + extra);
    }

    fn make_tab(self: &Rc<Self>, item: &DeckItem, index: usize) -> gtk::Button {
        let button = gtk::Button::builder()
            .css_classes(["tab".to_string(), format!("stagger-{}", index.min(15))])
            .focus_on_click(false)
            .halign(self.edge_align())
            .build();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).halign(gtk::Align::Center).build();
        let right = self.right();
        let open_group = self.open_group.borrow().clone();
        if let Some(note) = &item.note {
            let label = note.spine_label();
            button.add_css_class(&format!("spine-{}", color_slug(note.color)));
            let in_open_group = open_group.is_some() && note.group_id == open_group;
            let in_daily = open_group.as_deref() == Some(DAILY_GROUP) && note.is_daily();
            if (item.kind == Kind::Note && in_open_group) || in_daily {
                button.add_css_class("in-group");
            }
            let mut states: Vec<String> = Vec::new();
            if note.is_pinned() {
                states.push("pinned".into());
                content.append(&gtk::Label::builder().label("●").css_classes(["tab-badge"]).build());
            }
            let after = app().settings.int("archive_after_days");
            if let Some(left) = app().store.days_until_archive(note, after).filter(|left| *left <= 2) {
                content.append(&gtk::Label::builder().label("◷").css_classes(["tab-badge"]).build());
                states.push(format!("archives in {} days", left.max(0)));
            }
            if note.floating {
                states.push("floating".into());
                button.add_css_class("floating");
            }
            if note.keep_on_deck {
                states.push("kept on deck".into());
            }
            if note.is_daily() {
                states.push("daily".into());
            }
            content.append(&VerticalLabel::new(&label, right, TAB_MAX - 26, &[]));
            button.set_size_request(-1, self.tab_height(&label));
            let mut description = vec![label.clone(), color_name(note.color).to_string()];
            description.extend(states);
            set_label(&button, &description.join(", "));
            let note_id = note.id.clone();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |button| {
                if let Some(deck) = weak.upgrade() {
                    deck.on_note_clicked(button, &note_id);
                }
            });
            self.attach_menu(&button, &self.note_menu(note));
            self.attach_peek(&button, &note.id);
            self.attach_drag(&button, &note.id);
        } else if let Some(group) = &item.group {
            let is_daily = matches!(group, GroupRef::Daily);
            let label = group.name();
            match group {
                GroupRef::Daily => button.add_css_class("spine-daily"),
                GroupRef::Group(group) => button.add_css_class(&format!("group-color-{}", group.color_index())),
            }
            button.add_css_class(if item.kind == Kind::Group { "group-tab" } else { "group-header" });
            content.append(&gtk::Label::builder().label(item.notes.len().to_string()).css_classes(["tab-count"]).build());
            content.append(&VerticalLabel::new(&label, right, TAB_MAX - 30, &[]));
            button.set_size_request(-1, self.tab_height(&label) + 18);
            let state = if item.kind == Kind::Header { "open" } else { "closed" };
            set_label(&button, &format!("{label}, group of {}, {state}", item.notes.len()));
            let group_id = group.id();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(deck) = weak.upgrade() {
                    deck.on_group_clicked(&group_id);
                }
            });
            if !is_daily {
                let menu = gio::Menu::new();
                menu.append(Some("Rename Group…"), Some(&format!("deck.rename-group::{}", group.id())));
                menu.append(Some("Delete Group"), Some(&format!("deck.delete-group::{}", group.id())));
                self.attach_menu(&button, &menu);
            }
            self.attach_group_drop(&button, if is_daily { None } else { Some(group.id()) });
        }
        button.set_child(Some(&content));
        button
    }

    fn note_menu(&self, note: &Note) -> gio::Menu {
        let id = &note.id;
        let menu = gio::Menu::new();
        let first = gio::Menu::new();
        first.append(Some("Open"), Some(&format!("deck.open::{id}")));
        first.append(Some(if note.floating { "Return to Deck" } else { "Float Note" }), Some(&format!("deck.float::{id}")));
        first.append(Some("Copy Note"), Some(&format!("deck.copy::{id}")));
        menu.append_section(None, &first);
        let second = gio::Menu::new();
        second.append(
            Some(if note.is_pinned() { "Unpin from Center" } else { "Pin to Center" }),
            Some(&format!("deck.pin::{id}")),
        );
        second.append(
            Some(if note.keep_on_deck { "Don't Keep on Deck" } else { "Keep on Deck" }),
            Some(&format!("deck.keep::{id}")),
        );
        second.append(Some("Next Color"), Some(&format!("deck.color::{id}")));
        let groups = gio::Menu::new();
        app().fill_group_menu(&groups, id, "deck");
        second.append_submenu(Some("Move to Group"), &groups);
        menu.append_section(None, &second);
        let third = gio::Menu::new();
        third.append(Some("Archive"), Some(&format!("deck.archive::{id}")));
        third.append(Some("Delete"), Some(&format!("deck.delete::{id}")));
        menu.append_section(None, &third);
        menu
    }

    fn attach_menu(self: &Rc<Self>, button: &gtk::Button, menu: &gio::Menu) {
        let popover = gtk::PopoverMenu::from_model(Some(menu));
        popover.set_has_arrow(false);
        popover.set_parent(button);
        popover.set_position(if self.right() { gtk::PositionType::Left } else { gtk::PositionType::Right });
        let weak = Rc::downgrade(self);
        popover.connect_show(move |_| {
            if let Some(deck) = weak.upgrade() {
                deck.set_menu_open(true);
            }
        });
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |_| {
            if let Some(deck) = weak.upgrade() {
                deck.set_menu_open(false);
            }
        });
        let child = popover.clone();
        button.connect_destroy(move |_| child.unparent());
        let click = gtk::GestureClick::builder().button(3).build();
        let weak = Rc::downgrade(self);
        let target = popover.downgrade();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(deck) = weak.upgrade() {
                deck.hide_peek();
            }
            if let Some(popover) = target.upgrade() {
                popover.popup();
            }
        });
        button.add_controller(click);
    }

    fn set_menu_open(self: &Rc<Self>, open: bool) {
        self.menu_open.set(open);
        if !open && !self.inside.get() {
            self.schedule_fold();
        }
    }

    fn attach_peek(self: &Rc<Self>, button: &gtk::Button, note_id: &str) {
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        let id = note_id.to_string();
        let target = button.downgrade();
        motion.connect_enter(move |_, _, _| {
            if let (Some(deck), Some(button)) = (weak.upgrade(), target.upgrade()) {
                debug("tab enter");
                deck.schedule_peek(&id, &button, PEEK_DELAY);
            }
        });
        let weak = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(deck) = weak.upgrade() {
                debug("tab leave");
                deck.peek_timer.cancel();
            }
        });
        button.add_controller(motion);
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        let id = note_id.to_string();
        let target = button.downgrade();
        focus.connect_enter(move |_| {
            if let (Some(deck), Some(button)) = (weak.upgrade(), target.upgrade()) {
                if deck.focus_mode.get() {
                    deck.schedule_peek(&id, &button, 600);
                }
            }
        });
        button.add_controller(focus);
    }

    fn attach_drag(self: &Rc<Self>, button: &gtk::Button, note_id: &str) {
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::MOVE);
        let payload = format!("ledge:{note_id}");
        source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&payload.to_value())));
        let weak = Rc::downgrade(self);
        let target = button.downgrade();
        source.connect_drag_begin(move |source, _| {
            if let Some(deck) = weak.upgrade() {
                deck.hide_peek();
            }
            if let Some(button) = target.upgrade() {
                source.set_icon(Some(&gtk::WidgetPaintable::new(Some(&button))), 20, 20);
            }
        });
        button.add_controller(source);
        let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        let target_id = note_id.to_string();
        drop.connect_drop(move |_, value, _, _| drop_on_note(value, &target_id));
        button.add_controller(drop);
    }

    fn attach_group_drop(&self, button: &gtk::Button, group_id: Option<String>) {
        let drop = gtk::DropTarget::new(String::static_type(), gdk::DragAction::MOVE);
        drop.connect_drop(move |_, value, _, _| {
            let Some(note_id) = value.get::<String>().ok().and_then(|v| v.strip_prefix("ledge:").map(str::to_string)) else {
                return false;
            };
            let Some(group_id) = group_id.clone() else {
                return false;
            };
            app().store.update(&note_id, |note| note.group_id = Some(group_id));
            true
        });
        button.add_controller(drop);
    }

    fn on_note_clicked(self: &Rc<Self>, button: &gtk::Button, note_id: &str) {
        let y = self.absolute_y(button);
        self.fold(true);
        app().open_note(note_id, y, true);
    }

    fn on_group_clicked(self: &Rc<Self>, group_id: &str) {
        let current = self.open_group.borrow().clone();
        *self.open_group.borrow_mut() = if current.as_deref() == Some(group_id) { None } else { Some(group_id.to_string()) };
        self.rebuild();
        self.apply_fanned_classes(false);
    }

    fn absolute_y(&self, widget: &impl IsA<gtk::Widget>) -> Option<i32> {
        widget.compute_point(&self.window, &graphene::Point::new(0.0, 0.0)).map(|point| point.y() as i32)
    }

    fn monitor_height(&self) -> i32 {
        monitor_for(Some(&self.window)).map(|m| m.geometry().height()).unwrap_or(1080)
    }

    fn on_enter(self: &Rc<Self>) {
        debug("enter");
        self.inside.set(true);
        self.fold_timer.cancel();
        if !self.fanned.get() && !self.fan_timer.active() {
            let weak = Rc::downgrade(self);
            self.fan_timer.start(70, move || {
                if let Some(deck) = weak.upgrade() {
                    if deck.inside.get() {
                        deck.fan();
                    }
                }
            });
        }
    }

    fn on_leave(self: &Rc<Self>) {
        debug("leave");
        self.inside.set(false);
        self.fan_timer.cancel();
        self.schedule_fold();
    }

    fn schedule_fold(self: &Rc<Self>) {
        if self.focus_mode.get() || self.menu_open.get() || !self.fanned.get() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.fold_timer.start(FOLD_DELAY, move || {
            if let Some(deck) = weak.upgrade() {
                if !deck.inside.get() && !deck.menu_open.get() {
                    deck.fold(false);
                }
            }
        });
    }

    pub fn fan(self: &Rc<Self>) {
        if self.fanned.get() {
            return;
        }
        self.fanned.set(true);
        self.rebuild();
        self.stack.set_visible_child_name("fan");
        self.window.set_anchor(Edge::Top, true);
        self.window.set_anchor(Edge::Bottom, true);
        self.resize();
        self.apply_fanned_classes(true);
    }

    fn apply_fanned_classes(&self, animate: bool) {
        if animate && !app().reduce_motion() {
            self.tabs_box.add_css_class("folded");
            self.tabs_box.remove_css_class("open");
            let tabs = self.tabs_box.clone();
            self.reveal_timer.start(16, move || {
                tabs.remove_css_class("folded");
                tabs.add_css_class("open");
            });
        } else {
            self.reveal_timer.cancel();
            self.tabs_box.remove_css_class("folded");
            self.tabs_box.add_css_class("open");
        }
    }

    pub fn fold(self: &Rc<Self>, immediate: bool) {
        self.fold_timer.cancel();
        if !self.fanned.get() {
            return;
        }
        self.hide_peek();
        if self.focus_mode.get() {
            self.exit_focus_mode();
        }
        self.fanned.set(false);
        if self.open_group.borrow().is_some() && !immediate {
            *self.open_group.borrow_mut() = None;
        }
        if immediate || app().reduce_motion() {
            self.show_rest();
        } else {
            self.reveal_timer.cancel();
            self.tabs_box.add_css_class("folded");
            self.tabs_box.remove_css_class("open");
            let weak = Rc::downgrade(self);
            crate::signals::later(170, move || {
                if let Some(deck) = weak.upgrade() {
                    if !deck.fanned.get() {
                        deck.show_rest();
                    }
                }
            });
        }
    }

    fn show_rest(&self) {
        self.stack.set_visible_child_name("rest");
        self.window.set_anchor(Edge::Top, false);
        self.window.set_anchor(Edge::Bottom, false);
        self.resize();
    }

    fn resize(&self) {
        if self.fanned.get() {
            self.scroller.set_max_content_height((self.monitor_height() - 140).max(200));
            self.window.set_default_size(self.fan_width.get(), 1);
        } else {
            self.window.set_default_size(STRIPE_WIDTH, self.stripe_height.get());
        }
    }

    fn schedule_peek(self: &Rc<Self>, note_id: &str, button: &gtk::Button, delay: u64) {
        if !app().settings.flag("peek") || self.menu_open.get() {
            return;
        }
        let showing_other = self.peek_note.borrow().as_deref().is_some_and(|current| current != note_id);
        let delay = if showing_other { 60 } else { delay };
        let weak = Rc::downgrade(self);
        let id = note_id.to_string();
        let target = button.downgrade();
        self.peek_timer.start(delay, move || {
            if let (Some(deck), Some(button)) = (weak.upgrade(), target.upgrade()) {
                deck.show_peek(&id, &button);
            }
        });
    }

    fn show_peek(&self, note_id: &str, button: &gtk::Button) {
        self.peek_timer.cancel();
        let Some(note) = app().store.get(note_id) else {
            return;
        };
        if !self.fanned.get() {
            return;
        }
        let y = self.absolute_y(button).unwrap_or(0);
        if self.peek.borrow().is_none() {
            *self.peek.borrow_mut() = Some(PeekWindow::new());
        }
        if let Some(peek) = self.peek.borrow().as_ref() {
            peek.show_note(&note, y, self.right(), self.fan_width.get(), self.monitor_height());
        }
        *self.peek_note.borrow_mut() = Some(note_id.to_string());
    }

    fn hide_peek(&self) {
        self.peek_timer.cancel();
        if self.peek_note.borrow_mut().take().is_some() {
            if let Some(peek) = self.peek.borrow().as_ref() {
                peek.hide();
            }
        }
    }

    pub fn enter_focus_mode(self: &Rc<Self>) {
        self.focus_mode.set(true);
        self.window.set_keyboard_mode(KeyboardMode::Exclusive);
        self.fan();
        if let Some(first) = self.tabs_box.first_child() {
            glib::idle_add_local_once(move || {
                first.grab_focus();
            });
        }
        app().announce("Deck focused: arrows move, Return opens, Space peeks, Esc leaves", true);
    }

    fn exit_focus_mode(&self) {
        self.focus_mode.set(false);
        self.window.set_keyboard_mode(KeyboardMode::None);
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.fanned.get() {
            self.fold(true);
        } else {
            self.fan();
        }
    }

    fn on_key(self: &Rc<Self>, keyval: gdk::Key) -> bool {
        if !self.focus_mode.get() {
            return false;
        }
        if keyval == gdk::Key::Escape {
            if self.peek_note.borrow().is_some() {
                self.hide_peek();
            } else {
                self.fold(true);
            }
            return true;
        }
        if keyval == gdk::Key::Down {
            self.window.child_focus(gtk::DirectionType::TabForward);
            return true;
        }
        if keyval == gdk::Key::Up {
            self.window.child_focus(gtk::DirectionType::TabBackward);
            return true;
        }
        if keyval == gdk::Key::space {
            let Some(focus) = gtk::prelude::GtkWindowExt::focus(&self.window) else {
                return false;
            };
            let found = self
                .tabs
                .borrow()
                .iter()
                .find(|(_, tab, _)| tab.upcast_ref::<gtk::Widget>() == &focus)
                .and_then(|(_, tab, item)| item.note.as_ref().map(|note| (tab.clone(), note.id.clone())));
            if let Some((tab, note_id)) = found {
                if self.peek_note.borrow().as_deref() == Some(note_id.as_str()) {
                    self.hide_peek();
                } else {
                    self.show_peek(&note_id, &tab);
                }
                return true;
            }
        }
        false
    }
}

fn drop_on_note(value: &glib::Value, target_id: &str) -> bool {
    let Some(dragged) = value.get::<String>().ok().and_then(|v| v.strip_prefix("ledge:").map(str::to_string)) else {
        return false;
    };
    if dragged == target_id {
        return false;
    }
    let store = &app().store;
    let mut order: Vec<String> = store.active_notes().into_iter().map(|n| n.id).filter(|id| *id != dragged).collect();
    let index = order.iter().position(|id| id == target_id).unwrap_or(order.len());
    order.insert(index, dragged.clone());
    if let (Some(target), Some(moved)) = (store.get(target_id), store.get(&dragged)) {
        if moved.group_id != target.group_id {
            let group = target.group_id.clone();
            store.update_silently(&dragged, |note| note.group_id = group);
        }
    }
    store.reorder(&order);
    true
}
