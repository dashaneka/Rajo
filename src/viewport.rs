//! Notify the viewer after actual allocation changes, without a polling timer.
use std::cell::{Cell, RefCell};

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Viewport {
        pub size: Cell<(i32, i32)>,
        pub pending: Cell<bool>,
        pub child: RefCell<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Viewport {
        const NAME: &'static str = "RajoViewport";
        type Type = super::Viewport;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Viewport {
        fn dispose(&self) {
            if let Some(child) = self.child.borrow_mut().take() {
                child.unparent();
            }
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("resized").build()])
        }
    }

    impl WidgetImpl for Viewport {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.child
                .borrow()
                .as_ref()
                .map_or((0, 0, -1, -1), |child| child.measure(orientation, for_size))
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(child) = self.child.borrow().as_ref() {
                self.obj().snapshot_child(child, snapshot);
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                child.allocate(width, height, baseline, None);
            }
            if self.size.replace((width, height)) != (width, height) && !self.pending.replace(true)
            {
                // Descendants can finish allocation after this parent returns.
                // Refit once their actual viewport dimensions are available.
                let obj = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(obj) = obj.upgrade() {
                        obj.imp().pending.set(false);
                        obj.emit_by_name::<()>("resized", &[]);
                    }
                });
            }
        }
    }
}

glib::wrapper! {
    pub struct Viewport(ObjectSubclass<imp::Viewport>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Viewport {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let obj: Self = glib::Object::new();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        child.set_parent(&obj);
        *obj.imp().child.borrow_mut() = Some(child.as_ref().clone());
        obj
    }

    pub fn connect_resized(&self, f: impl Fn() + 'static) {
        self.connect_local("resized", false, move |_| {
            f();
            None
        });
    }
}
