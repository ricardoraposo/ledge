use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::app::app;
use crate::compositor::TITLE_SUFFIX;
use crate::editor::NoteEditor;
use crate::model::color_slug;
use crate::signals::{Timer, later};
use crate::widgets::{VerticalLabel, remove_classes_with_prefix, set_label};

const CARD_WIDTH: i32 = 430;
const CARD_HEIGHT: i32 = 340;
const MIN_WIDTH: i32 = 300;
const MIN_HEIGHT: i32 = 220;
const SLIDE_MS: u64 = 200;

fn debug(message: &str) {
    if std::env::var_os("LEDGE_DEBUG").is_some() {
        println!("card: {message}");
    }
}

pub fn monitor_for(window: Option<&gtk::Window>) -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    if let Some(surface) = window.and_then(|w| w.surface()) {
        if let Some(monitor) = display.monitor_at_surface(&surface) {
            return Some(monitor);
        }
    }
    display.monitors().item(0).and_downcast::<gdk::Monitor>()
}

pub struct NoteSheet {
    pub root: gtk::Box,
    pub editor: Rc<NoteEditor>,
    spine_label: VerticalLabel,
    note_id: String,
    handler: Cell<u64>,
}

impl NoteSheet {
    fn new(note_id: &str, floating: bool, on_close: Rc<dyn Fn()>) -> Rc<NoteSheet> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("card");
        let editor = NoteEditor::new(note_id);
        editor.root.set_hexpand(true);
        let close_editor = on_close.clone();
        editor.set_on_close(move || close_editor());

        let spine = gtk::Box::new(gtk::Orientation::Vertical, 10);
        spine.add_css_class("card-spine");
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["spine-close"])
            .tooltip_text("Close")
            .build();
        set_label(&close, "Close note");
        close.connect_clicked(move |_| on_close());
        let label = if floating { "Return to Deck" } else { "Float Note" };
        let float_button = gtk::Button::builder()
            .css_classes(["spine-float"])
            .tooltip_text(label)
            .icon_name(if floating { "view-restore-symbolic" } else { "view-fullscreen-symbolic" })
            .build();
        set_label(&float_button, label);
        let id = note_id.to_string();
        float_button.connect_clicked(move |_| app().toggle_float(&id));
        let spine_first = floating || !app().settings.right_edge();
        let spine_label = VerticalLabel::new("", !spine_first, 220, &["spine-title"]);
        spine_label.set_vexpand(true);
        spine_label.set_valign(gtk::Align::Center);
        spine.append(&close);
        spine.append(&float_button);
        spine.append(&spine_label);
        let spine_holder: gtk::Widget = if floating {
            gtk::WindowHandle::builder().child(&spine).build().upcast()
        } else {
            spine.upcast()
        };
        let perforation = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        perforation.add_css_class("perforation");
        let editor_widget: gtk::Widget = editor.root.clone().upcast();
        let order = if spine_first {
            [spine_holder, perforation.upcast(), editor_widget]
        } else {
            [editor_widget, perforation.upcast(), spine_holder]
        };
        for child in order {
            root.append(&child);
        }
        root.add_css_class(if floating {
            "floating"
        } else if spine_first {
            "anchored-left"
        } else {
            "anchored-right"
        });
        let sheet = Rc::new(NoteSheet { root, editor, spine_label, note_id: note_id.to_string(), handler: Cell::new(0) });
        let weak = Rc::downgrade(&sheet);
        sheet.handler.set(app().store.changed.connect(move |changed| {
            if let Some(sheet) = weak.upgrade() {
                if changed.is_empty() || changed == sheet.note_id {
                    sheet.refresh();
                }
            }
        }));
        let weak = Rc::downgrade(&sheet);
        sheet.root.connect_destroy(move |_| {
            if let Some(sheet) = weak.upgrade() {
                sheet.dispose();
            }
        });
        sheet.refresh();
        sheet
    }

    fn dispose(&self) {
        let handler = self.handler.replace(0);
        if handler != 0 {
            app().store.changed.disconnect(handler);
        }
        self.editor.dispose();
    }

    pub fn refresh(&self) {
        let Some(note) = app().store.get(&self.note_id) else {
            return;
        };
        remove_classes_with_prefix(&self.root, "paper-");
        self.root.add_css_class(&format!("paper-{}", color_slug(note.color)));
        let title = note.display_title();
        self.spine_label.set_text(&title);
        if let Some(window) = self.root.root().and_downcast::<gtk::Window>() {
            window.set_title(Some(&format!("{title}{TITLE_SUFFIX}")));
        }
    }
}

fn click_catcher(monitor: Option<&gdk::Monitor>, on_click: Rc<dyn Fn()>) -> gtk::Window {
    let window = gtk::Window::builder().application(&app().gtk).title("Ledge").build();
    window.add_css_class("catcher-window");
    window.init_layer_shell();
    window.set_namespace(Some("ledge-catcher"));
    window.set_layer(Layer::Top);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    if let Some(monitor) = monitor {
        window.set_monitor(Some(monitor));
    }
    window.set_child(Some(&gtk::Box::builder().hexpand(true).vexpand(true).build()));
    let click = gtk::GestureClick::builder().button(0).build();
    let pressed = on_click.clone();
    click.connect_pressed(move |_, _, _, _| {
        debug("catcher pressed");
        pressed();
    });
    window.add_controller(click);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    scroll.connect_scroll(move |_, _, _| {
        on_click();
        glib::Propagation::Stop
    });
    window.add_controller(scroll);
    window
}

pub struct Card {
    pub note_id: String,
    pub floating: bool,
    pub window: gtk::Window,
    pub editor: Rc<NoteEditor>,
    sheet: Rc<NoteSheet>,
    slider: gtk::Box,
    catcher: RefCell<Option<gtk::Window>>,
    monitor: Option<gdk::Monitor>,
    anchor_y: Option<i32>,
    closing: Cell<bool>,
    handler: Cell<u64>,
    reveal: Timer,
}

impl Card {
    fn closer(weak: &Weak<Card>) -> Rc<dyn Fn()> {
        let weak = weak.clone();
        Rc::new(move || {
            if let Some(card) = weak.upgrade() {
                card.close_card(true);
            }
        })
    }

    pub fn docked(note_id: &str, anchor_y: Option<i32>) -> Rc<Card> {
        let card = Rc::new_cyclic(|weak: &Weak<Card>| {
            let window = gtk::Window::builder().application(&app().gtk).title("Ledge").build();
            window.add_css_class("card-window");
            let deck_window = app().deck_window();
            let monitor = monitor_for(deck_window.as_ref());
            let weak_catch = weak.clone();
            let catcher = click_catcher(
                monitor.as_ref(),
                Rc::new(move || {
                    if let Some(card) = weak_catch.upgrade() {
                        card.on_catcher_click();
                    }
                }),
            );
            window.init_layer_shell();
            window.set_namespace(Some("ledge-note"));
            window.set_layer(Layer::Overlay);
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::Exclusive);
            if let Some(monitor) = &monitor {
                window.set_monitor(Some(monitor));
            }
            let sheet = NoteSheet::new(note_id, false, Card::closer(weak));
            let slider = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            slider.add_css_class("card-slider");
            slider.append(&sheet.root);
            window.set_child(Some(&slider));
            Card {
                note_id: note_id.to_string(),
                floating: false,
                window,
                editor: sheet.editor.clone(),
                sheet,
                slider,
                catcher: RefCell::new(Some(catcher)),
                monitor,
                anchor_y,
                closing: Cell::new(false),
                handler: Cell::new(0),
                reveal: Timer::default(),
            }
        });
        card.connect_common();
        card.place();
        card.sheet.refresh();
        card
    }

    pub fn floating(note_id: &str) -> Rc<Card> {
        let note = app().store.get(note_id);
        let width = note.as_ref().and_then(|n| n.float_w).map(|w| w as i32).unwrap_or(CARD_WIDTH).max(MIN_WIDTH);
        let height = note.as_ref().and_then(|n| n.float_h).map(|h| h as i32).unwrap_or(CARD_HEIGHT).max(MIN_HEIGHT);
        let card = Rc::new_cyclic(|weak: &Weak<Card>| {
            let window = gtk::Window::builder()
                .application(&app().gtk)
                .title("Ledge")
                .decorated(false)
                .default_width(width)
                .default_height(height)
                .build();
            window.add_css_class("floating-window");
            let sheet = NoteSheet::new(note_id, true, Card::closer(weak));
            let grip = gtk::Box::builder().css_classes(["resize-grip"]).halign(gtk::Align::End).valign(gtk::Align::End).build();
            grip.set_size_request(18, 18);
            grip.set_cursor_from_name(Some("se-resize"));
            let press = gtk::GestureClick::new();
            let weak_resize = weak.clone();
            press.connect_pressed(move |gesture, _, x, y| {
                if let Some(card) = weak_resize.upgrade() {
                    card.begin_resize(gesture, x, y);
                }
            });
            grip.add_controller(press);
            let overlay = gtk::Overlay::builder().child(&sheet.root).build();
            overlay.add_overlay(&grip);
            window.set_child(Some(&overlay));
            Card {
                note_id: note_id.to_string(),
                floating: true,
                window,
                editor: sheet.editor.clone(),
                sheet,
                slider: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                catcher: RefCell::new(None),
                monitor: None,
                anchor_y: None,
                closing: Cell::new(false),
                handler: Cell::new(0),
                reveal: Timer::default(),
            }
        });
        card.connect_common();
        card.sheet.refresh();
        card
    }

    fn connect_common(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(card) = weak.upgrade() {
                card.close_card(false);
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        self.handler.set(app().store.changed.connect(move |note_id| {
            if let Some(card) = weak.upgrade() {
                card.on_store_changed(note_id);
            }
        }));
    }

    fn on_catcher_click(self: &Rc<Self>) {
        if app().dialog_open.get() || self.editor.has_open_popover() {
            return;
        }
        debug("click outside");
        self.close_card(true);
    }

    fn place(&self) {
        let right = app().settings.right_edge();
        let height = self.monitor.as_ref().map(|m| m.geometry().height()).unwrap_or(1080);
        let card_height = CARD_HEIGHT.min(height - 48);
        let anchor = self.anchor_y.unwrap_or((height - card_height) / 2);
        let top = (anchor - 34).min(height - card_height - 40).max(8);
        self.window.set_anchor(Edge::Top, true);
        self.window.set_anchor(Edge::Left, !right);
        self.window.set_anchor(Edge::Right, right);
        self.window.set_margin(Edge::Top, top);
        self.slider.remove_css_class(if right { "slide-left" } else { "slide-right" });
        self.slider.add_css_class(if right { "slide-right" } else { "slide-left" });
        self.sheet.root.set_size_request(CARD_WIDTH, card_height);
        self.window.set_default_size(CARD_WIDTH + 18, card_height + 32);
    }

    fn begin_resize(&self, gesture: &gtk::GestureClick, x: f64, y: f64) {
        let Some(surface) = self.window.surface() else {
            return;
        };
        let Some(event) = gesture.current_event() else {
            return;
        };
        let Some(widget) = gesture.widget() else {
            return;
        };
        let point = widget
            .compute_point(&self.window, &graphene::Point::new(x as f32, y as f32))
            .unwrap_or(graphene::Point::new(x as f32, y as f32));
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() {
            toplevel.begin_resize(
                gdk::SurfaceEdge::SouthEast,
                event.device().as_ref(),
                gesture.current_button() as i32,
                point.x() as f64,
                point.y() as f64,
                event.time(),
            );
        }
    }

    pub fn present_card(self: &Rc<Self>, focus: bool) {
        if self.floating {
            self.window.present();
            if focus {
                let editor = self.editor.clone();
                later(40, move || editor.focus_text(true));
            }
        } else {
            let animate = !self.window.is_mapped() && !app().reduce_motion();
            if let Some(catcher) = self.catcher.borrow().as_ref() {
                if !catcher.is_mapped() {
                    catcher.present();
                }
            }
            if animate {
                self.slider.add_css_class("hidden-card");
            }
            self.window.present();
            if animate {
                let slider = self.slider.clone();
                self.reveal.start(16, move || slider.remove_css_class("hidden-card"));
            }
            let editor = self.editor.clone();
            later(40, move || editor.focus_text(true));
        }
        app().store.mark_opened(&self.note_id);
    }

    fn on_store_changed(self: &Rc<Self>, note_id: &str) {
        if !(note_id.is_empty() || note_id == self.note_id) {
            return;
        }
        let note = app().store.get(&self.note_id);
        let gone = note.as_ref().is_none_or(|note| !note.is_active());
        if self.floating {
            if gone || note.as_ref().is_some_and(|note| !note.floating) {
                self.close_floating(true);
            }
        } else if gone || note.as_ref().is_some_and(|note| note.floating) {
            self.close_card(false);
        }
    }

    pub fn close_card(self: &Rc<Self>, animate: bool) {
        if self.floating {
            self.close_floating(false);
            return;
        }
        if self.closing.replace(true) {
            return;
        }
        debug("close");
        self.editor.flush();
        if let Some(catcher) = self.catcher.borrow_mut().take() {
            catcher.destroy();
        }
        if animate && !app().reduce_motion() && self.window.is_mapped() {
            self.slider.add_css_class("hidden-card");
            let card = self.clone();
            later(SLIDE_MS, move || card.finish_close());
        } else {
            self.finish_close();
        }
    }

    fn finish_close(self: &Rc<Self>) {
        self.disconnect();
        app().card_closed(self);
        self.window.destroy();
    }

    fn disconnect(&self) {
        let handler = self.handler.replace(0);
        if handler != 0 {
            app().store.changed.disconnect(handler);
        }
        self.reveal.cancel();
    }

    fn close_floating(self: &Rc<Self>, keep_floating: bool) {
        if self.closing.replace(true) {
            return;
        }
        self.editor.flush();
        if app().store.get(&self.note_id).is_some() {
            let width = self.window.width() as i64;
            let height = self.window.height() as i64;
            app().store.update(&self.note_id, |note| {
                if width > 0 {
                    note.float_w = Some(width);
                    note.float_h = Some(height);
                }
                if !keep_floating {
                    note.floating = false;
                }
            });
        }
        self.disconnect();
        app().card_closed(self);
        self.window.destroy();
    }

    pub fn save_size(&self) {
        if self.floating && self.window.width() > 0 {
            let width = self.window.width() as i64;
            let height = self.window.height() as i64;
            app().store.update_silently(&self.note_id, |note| {
                note.float_w = Some(width);
                note.float_h = Some(height);
            });
        }
    }
}

pub struct Toast {
    pub window: gtk::Window,
    timer: Timer,
    action: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl Toast {
    pub fn show(text: &str, action_label: Option<&str>, action: Option<Box<dyn FnOnce()>>, seconds: f64) -> Rc<Toast> {
        let window = gtk::Window::builder().application(&app().gtk).title("Ledge").build();
        window.init_layer_shell();
        window.set_namespace(Some("ledge-toast"));
        window.set_layer(Layer::Overlay);
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, 40);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        if let Some(monitor) = monitor_for(app().deck_window().as_ref()) {
            window.set_monitor(Some(&monitor));
        }
        window.add_css_class("toast-window");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        content.add_css_class("toast");
        content.append(&gtk::Label::new(Some(text)));
        let toast = Rc::new(Toast { window, timer: Timer::default(), action: RefCell::new(action) });
        if let Some(label) = action_label {
            let button = gtk::Button::builder().label(label).css_classes(["toast-action"]).build();
            let weak = Rc::downgrade(&toast);
            button.connect_clicked(move |_| {
                if let Some(toast) = weak.upgrade() {
                    let action = toast.action.borrow_mut().take();
                    if let Some(action) = action {
                        action();
                    }
                    toast.dismiss();
                }
            });
            content.append(&button);
        }
        toast.window.set_child(Some(&content));
        set_label(&toast.window, text);
        let weak = Rc::downgrade(&toast);
        toast.timer.start((seconds * 1000.0) as u64, move || {
            if let Some(toast) = weak.upgrade() {
                toast.dismiss();
            }
        });
        toast.window.present();
        toast
    }

    pub fn dismiss(self: &Rc<Self>) {
        self.timer.cancel();
        app().toast_closed(self);
        self.window.destroy();
    }
}
