use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::graphene;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VerticalLabel {
        pub text: RefCell<String>,
        pub clockwise: Cell<bool>,
        pub max_length: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VerticalLabel {
        const NAME: &'static str = "LedgeVerticalLabel";
        type Type = super::VerticalLabel;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for VerticalLabel {}

    impl WidgetImpl for VerticalLabel {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (width, height) = self.obj().layout().pixel_size();
            let size = if orientation == gtk::Orientation::Horizontal { height } else { width };
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let layout = widget.layout();
            let (text_width, text_height) = layout.pixel_size();
            let width = widget.width() as f32;
            let height = widget.height() as f32;
            snapshot.save();
            if self.clockwise.get() {
                snapshot.translate(&graphene::Point::new(width, 0.0));
                snapshot.rotate(90.0);
            } else {
                snapshot.translate(&graphene::Point::new(0.0, height));
                snapshot.rotate(-90.0);
            }
            snapshot.translate(&graphene::Point::new(
                (height - text_width as f32) / 2.0,
                (width - text_height as f32) / 2.0,
            ));
            snapshot.append_layout(&layout, &widget.color());
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct VerticalLabel(ObjectSubclass<imp::VerticalLabel>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VerticalLabel {
    pub fn new(text: &str, clockwise: bool, max_length: i32, classes: &[&str]) -> Self {
        let label: VerticalLabel = glib::Object::new();
        label.add_css_class("vlabel");
        for class in classes {
            label.add_css_class(class);
        }
        label.imp().clockwise.set(clockwise);
        label.imp().max_length.set(max_length);
        label.set_text(text);
        label
    }

    fn layout(&self) -> pango::Layout {
        let layout = self.create_pango_layout(Some(&self.imp().text.borrow()));
        let max_length = self.imp().max_length.get();
        if max_length > 0 {
            layout.set_width(max_length * pango::SCALE);
            layout.set_ellipsize(pango::EllipsizeMode::End);
        }
        layout
    }

    pub fn set_text(&self, text: &str) {
        *self.imp().text.borrow_mut() = text.to_string();
        self.update_property(&[gtk::accessible::Property::Label(text)]);
        self.queue_resize();
    }
}

pub fn clear(container: &impl IsA<gtk::Widget>) {
    let container = container.as_ref();
    while let Some(child) = container.first_child() {
        if let Some(list) = container.downcast_ref::<gtk::ListBox>() {
            list.remove(&child);
        } else if let Some(boxed) = container.downcast_ref::<gtk::Box>() {
            boxed.remove(&child);
        } else {
            child.unparent();
        }
    }
}

pub fn remove_classes_with_prefix(widget: &impl IsA<gtk::Widget>, prefix: &str) {
    for class in widget.css_classes() {
        if class.starts_with(prefix) {
            widget.remove_css_class(&class);
        }
    }
}

pub fn set_label(widget: &impl IsA<gtk::Accessible>, text: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(text)]);
}

pub fn prompt_name(
    anchor: &impl IsA<gtk::Widget>,
    heading: &str,
    initial: &str,
    on_accept: impl Fn(String) + 'static,
    on_state: Option<Box<dyn Fn(bool)>>,
) -> gtk::Popover {
    let popover = gtk::Popover::builder().has_arrow(true).css_classes(["name-prompt"]).build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&gtk::Label::builder().label(heading).xalign(0.0).css_classes(["heading"]).build());
    let entry = gtk::Entry::builder().text(initial).width_chars(22).max_length(60).build();
    content.append(&entry);
    popover.set_child(Some(&content));
    popover.set_parent(anchor);
    let on_state: std::rc::Rc<Option<Box<dyn Fn(bool)>>> = std::rc::Rc::new(on_state);
    entry.connect_activate(glib::clone!(
        #[weak]
        popover,
        move |entry| {
            let text = entry.text().trim().to_string();
            popover.popdown();
            if !text.is_empty() {
                on_accept(text);
            }
        }
    ));
    let closed_state = on_state.clone();
    popover.connect_closed(move |popover| {
        if let Some(callback) = closed_state.as_ref() {
            callback(false);
        }
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    if let Some(callback) = on_state.as_ref() {
        callback(true);
    }
    popover.popup();
    entry.grab_focus();
    popover
}
