use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;

type Listener = Rc<dyn Fn(&str)>;

#[derive(Default)]
pub struct Listeners {
    next: Cell<u64>,
    items: RefCell<Vec<(u64, Listener)>>,
}

impl Listeners {
    pub fn connect(&self, callback: impl Fn(&str) + 'static) -> u64 {
        let id = self.next.get() + 1;
        self.next.set(id);
        self.items.borrow_mut().push((id, Rc::new(callback)));
        id
    }

    pub fn disconnect(&self, id: u64) {
        self.items.borrow_mut().retain(|(item, _)| *item != id);
    }

    pub fn emit(&self, value: &str) {
        let snapshot: Vec<(u64, Listener)> = self.items.borrow().iter().cloned().collect();
        for (id, callback) in snapshot {
            let alive = self.items.borrow().iter().any(|(item, _)| *item == id);
            if alive {
                callback(value);
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Timer(Rc<RefCell<Option<glib::SourceId>>>);

impl Timer {
    pub fn start(&self, milliseconds: u64, callback: impl FnOnce() + 'static) {
        self.cancel();
        let slot = self.0.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(milliseconds), move || {
            slot.borrow_mut().take();
            callback();
        });
        *self.0.borrow_mut() = Some(id);
    }

    pub fn idle(&self, callback: impl FnOnce() + 'static) {
        self.cancel();
        let slot = self.0.clone();
        let id = glib::idle_add_local_once(move || {
            slot.borrow_mut().take();
            callback();
        });
        *self.0.borrow_mut() = Some(id);
    }

    pub fn cancel(&self) {
        let pending = self.0.borrow_mut().take();
        if let Some(id) = pending {
            id.remove();
        }
    }

    pub fn active(&self) -> bool {
        self.0.borrow().is_some()
    }
}

pub fn later(milliseconds: u64, callback: impl FnOnce() + 'static) {
    glib::timeout_add_local_once(Duration::from_millis(milliseconds), callback);
}

pub fn idle(callback: impl FnOnce() + 'static) {
    glib::idle_add_local_once(callback);
}
