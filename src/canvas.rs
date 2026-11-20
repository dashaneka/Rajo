//! A fixed-size image canvas whose natural size is independent of the texture.
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Canvas {
        pub size: Cell<(i32, i32)>,
        pub picture: RefCell<Option<gtk::Picture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "RajoCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {
        fn dispose(&self) {
            if let Some(picture) = self.picture.borrow_mut().take() {
                picture.unparent();
            }
        }
    }

    impl WidgetImpl for Canvas {
        fn measure(&self, orientation: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            let size = self.size.get();
            let value = if orientation == gtk::Orientation::Horizontal {
                size.0
            } else {
                size.1
            }
            .max(1);
            (value, value, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(picture) = self.picture.borrow().as_ref() {
                picture.allocate(width, height, baseline, None);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(picture) = self.picture.borrow().as_ref() {
                self.obj().snapshot_child(picture, snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Canvas {
    pub fn new(picture: &gtk::Picture) -> Self {
        let canvas: Self = glib::Object::new();
        canvas.set_halign(gtk::Align::Center);
        canvas.set_valign(gtk::Align::Center);
        canvas.set_hexpand(false);
        canvas.set_vexpand(false);
        picture.set_parent(&canvas);
        *canvas.imp().picture.borrow_mut() = Some(picture.clone());
        canvas
    }

    pub fn set_dimensions(&self, width: i32, height: i32) {
        if self.imp().size.replace((width, height)) != (width, height) {
            self.queue_resize();
        }
    }

    #[cfg(test)]
    pub fn dimensions(&self) -> (i32, i32) {
        self.imp().size.get()
    }
}
