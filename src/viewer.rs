//! Shows one image: zoom, pan, next/prev, fade controls.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::folder;
use crate::{recent, window};

const HIDE_DELAY: Duration = Duration::from_secs(2);
const ZOOM_STEP: f64 = 1.25;
const MIN_ZOOM: f64 = 0.05;
const MAX_ZOOM: f64 = 32.0;
const MAX_SCREEN_SHARE: f64 = 0.85;

pub struct Viewer {
    this: Weak<Viewer>,
    window: adw::ApplicationWindow,
    overlay: gtk::Overlay,
    scrolled: gtk::ScrolledWindow,
    picture: gtk::Picture,
    header_revealer: gtk::Revealer,
    controls_revealer: gtk::Revealer,
    zoom_label: gtk::Label,
    prev_button: gtk::Button,
    next_button: gtk::Button,
    fullscreen_button: gtk::Button,
    state: RefCell<State>,
}

struct State {
    files: Vec<gio::File>,
    index: usize,
    texture: Option<gdk::Texture>,
    /// Multiplier of the image's own pixels. 1.0 = 100%.
    zoom: f64,
    /// When true, zoom is picked so the image fits the window.
    fit: bool,
    fitted_window: bool,
    /// Blocks zoom→resize→scroll→zoom recursion.
    busy: bool,
    over_controls: bool,
    drag_start: Option<(f64, f64)>,
    hide_source: Option<glib::SourceId>,
    key_controller: Option<gtk::EventControllerKey>,
    fullscreen_handler: Option<glib::SignalHandlerId>,
}

impl Viewer {
    pub fn open(window: &adw::ApplicationWindow, file: &gio::File) -> Result<Rc<Self>, String> {
        let texture = gdk::Texture::from_file(file).map_err(|err| err.to_string())?;
        let (files, index) = folder::siblings(file);
        recent::add(&file.uri());

        let picture = gtk::Picture::builder()
            .can_shrink(false)
            .content_fit(gtk::ContentFit::Fill)
            .build();

        let scrolled = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .kinetic_scrolling(true)
            .child(&picture)
            .build();
        scrolled.add_css_class("rajo-canvas");

        let header = window::header_bar(&title_of(file));
        header.add_css_class("rajo-header");
        let header_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .reveal_child(true)
            .valign(gtk::Align::Start)
            .child(&header)
            .build();

        let prev_button = icon_button("go-previous-symbolic", "Previous");
        let next_button = icon_button("go-next-symbolic", "Next");
        let zoom_out = icon_button("zoom-out-symbolic", "Zoom Out");
        let zoom_label = gtk::Label::new(Some("Fit"));
        zoom_label.add_css_class("numeric");
        zoom_label.set_width_chars(5);
        let zoom_in = icon_button("zoom-in-symbolic", "Zoom In");
        let zoom_fit = icon_button("zoom-fit-best-symbolic", "Fit");
        let zoom_100 = icon_button("zoom-original-symbolic", "100%");
        let fullscreen_button = icon_button("view-fullscreen-symbolic", "Fullscreen");

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.add_css_class("osd");
        controls.add_css_class("rajo-controls");
        controls.set_margin_start(12);
        controls.set_margin_end(12);
        controls.set_margin_bottom(12);
        controls.set_halign(gtk::Align::Center);
        controls.append(&prev_button);
        controls.append(&next_button);
        controls.append(&zoom_out);
        controls.append(&zoom_label);
        controls.append(&zoom_in);
        controls.append(&zoom_fit);
        controls.append(&zoom_100);
        controls.append(&fullscreen_button);

        let controls_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .reveal_child(true)
            .valign(gtk::Align::End)
            .child(&controls)
            .build();

        let overlay = gtk::Overlay::builder().child(&scrolled).build();
        overlay.add_overlay(&header_revealer);
        overlay.add_overlay(&controls_revealer);
        window.set_content(Some(&overlay));
        window.set_title(Some(&title_of(file)));

        let viewer = Rc::new_cyclic(|this| Self {
            this: this.clone(),
            window: window.clone(),
            overlay,
            scrolled,
            picture,
            header_revealer,
            controls_revealer,
            zoom_label,
            prev_button: prev_button.clone(),
            next_button: next_button.clone(),
            fullscreen_button: fullscreen_button.clone(),
            state: RefCell::new(State {
                files,
                index,
                texture: Some(texture),
                zoom: 1.0,
                fit: true,
                fitted_window: false,
                busy: false,
                over_controls: false,
                drag_start: None,
                hide_source: None,
                key_controller: None,
                fullscreen_handler: None,
            }),
        });
        viewer.connect_signals(&controls, &zoom_out, &zoom_in, &zoom_fit, &zoom_100);
        viewer.apply_image();
        viewer.fit_window();
        viewer.update_nav();
        viewer.show_controls();
        Ok(viewer)
    }

    fn with(&self, f: impl Fn(&Viewer) + 'static) -> impl Fn() + 'static {
        let this = self.this.clone();
        move || {
            if let Some(viewer) = this.upgrade() {
                f(&viewer);
            }
        }
    }

    fn connect_signals(
        self: &Rc<Self>,
        controls: &gtk::Box,
        zoom_out: &gtk::Button,
        zoom_in: &gtk::Button,
        zoom_fit: &gtk::Button,
        zoom_100: &gtk::Button,
    ) {
        let prev = self.with(|v| v.step(-1));
        self.prev_button.connect_clicked(move |_| prev());
        let next = self.with(|v| v.step(1));
        self.next_button.connect_clicked(move |_| next());
        let out = self.with(|v| v.zoom_by(1.0 / ZOOM_STEP));
        zoom_out.connect_clicked(move |_| out());
        let inn = self.with(|v| v.zoom_by(ZOOM_STEP));
        zoom_in.connect_clicked(move |_| inn());
        let fit = self.with(Viewer::zoom_fit);
        zoom_fit.connect_clicked(move |_| fit());
        let original = self.with(Viewer::zoom_100);
        zoom_100.connect_clicked(move |_| original());
        let toggle = self.with(Viewer::toggle_fullscreen);
        self.fullscreen_button.connect_clicked(move |_| toggle());

        let handler = self.window.connect_fullscreened_notify(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_| {
                if let Some(viewer) = this.this.upgrade() {
                    viewer.fullscreen_changed();
                }
            }
        ));
        self.state.borrow_mut().fullscreen_handler = Some(handler);
        self.fullscreen_changed();

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = self.this.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let handled = !modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) && this.upgrade().is_some_and(|v| v.handle_key(key));
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.window.add_controller(keys.clone());
        self.state.borrow_mut().key_controller = Some(keys);

        // Ctrl+scroll zooms; plain scroll is left to the scrolled window to pan.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = self.this.clone();
        scroll.connect_scroll(move |controller, _, dy| {
            let Some(viewer) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);
            if !ctrl || dy == 0.0 || viewer.state.borrow().busy {
                return glib::Propagation::Proceed;
            }
            if dy < 0.0 {
                viewer.zoom_by(ZOOM_STEP);
            } else {
                viewer.zoom_by(1.0 / ZOOM_STEP);
            }
            glib::Propagation::Stop
        });
        self.overlay.add_controller(scroll);

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let this = self.this.clone();
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(viewer) = this.upgrade() {
                let hadj = viewer.scrolled.hadjustment().value();
                let vadj = viewer.scrolled.vadjustment().value();
                viewer.state.borrow_mut().drag_start = Some((hadj, vadj));
                viewer.show_controls();
            }
        });
        let this = self.this.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(viewer) = this.upgrade() {
                let Some((hx, vy)) = viewer.state.borrow().drag_start else {
                    return;
                };
                viewer.scrolled.hadjustment().set_value(hx - dx);
                viewer.scrolled.vadjustment().set_value(vy - dy);
            }
        });
        let this = self.this.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(viewer) = this.upgrade() {
                viewer.state.borrow_mut().drag_start = None;
            }
        });
        self.picture.add_controller(drag);

        let click = gtk::GestureClick::new();
        let toggle = self.with(Viewer::toggle_fullscreen);
        click.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                toggle();
            }
        });
        self.picture.add_controller(click);

        let motion = gtk::EventControllerMotion::new();
        let show = self.with(Viewer::show_controls);
        motion.connect_motion(move |_, _, _| show());
        let hide = self.with(Viewer::hide_controls);
        motion.connect_leave(move |_| hide());
        self.overlay.add_controller(motion);

        let controls_motion = gtk::EventControllerMotion::new();
        let this = self.this.clone();
        controls_motion.connect_enter(move |_, _, _| {
            if let Some(viewer) = this.upgrade() {
                viewer.state.borrow_mut().over_controls = true;
            }
        });
        let this = self.this.clone();
        controls_motion.connect_leave(move |_| {
            if let Some(viewer) = this.upgrade() {
                viewer.state.borrow_mut().over_controls = false;
                viewer.schedule_hide();
            }
        });
        controls.add_controller(controls_motion);

        // Refit when the window is resized, not on every child size tweak.
        let this = self.this.clone();
        self.window.connect_default_width_notify(move |_| {
            if let Some(viewer) = this.upgrade() {
                if viewer.state.borrow().fit {
                    viewer.relayout();
                }
            }
        });
        let this = self.this.clone();
        self.window.connect_default_height_notify(move |_| {
            if let Some(viewer) = this.upgrade() {
                if viewer.state.borrow().fit {
                    viewer.relayout();
                }
            }
        });
    }

    fn apply_image(&self) {
        let texture = self.state.borrow().texture.clone();
        self.picture.set_paintable(texture.as_ref());
        self.relayout();
    }

    fn image_size(&self) -> Option<(f64, f64)> {
        let texture = self.state.borrow().texture.clone()?;
        Some((texture.width() as f64, texture.height() as f64))
    }

    fn view_size(&self) -> (f64, f64) {
        let width = self.scrolled.width().max(1) as f64;
        let height = self.scrolled.height().max(1) as f64;
        (width, height)
    }

    fn relayout(&self) {
        let (iw, ih, fit) = {
            let state = self.state.borrow();
            if state.busy {
                return;
            }
            let Some(texture) = state.texture.as_ref() else {
                return;
            };
            (texture.width() as f64, texture.height() as f64, state.fit)
        };
        self.state.borrow_mut().busy = true;
        let zoom = if fit {
            let zoom = self.fit_zoom_for(iw, ih);
            self.state.borrow_mut().zoom = zoom;
            zoom
        } else {
            let mut state = self.state.borrow_mut();
            state.zoom = state.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
            state.zoom
        };
        let (w, h) = ((iw * zoom).round() as i32, (ih * zoom).round() as i32);
        let (w, h) = (w.max(1), h.max(1));
        if self.picture.width_request() != w || self.picture.height_request() != h {
            self.picture.set_size_request(w, h);
        }
        let label = format!("{}%", (zoom * 100.0).round() as i32);
        if self.zoom_label.label() != label {
            self.zoom_label.set_label(&label);
        }
        self.state.borrow_mut().busy = false;
    }

    fn fit_zoom_for(&self, iw: f64, ih: f64) -> f64 {
        let (vw, vh) = self.view_size();
        (vw / iw).min(vh / ih).min(1.0).max(MIN_ZOOM)
    }

    fn zoom_by(&self, factor: f64) {
        if self.state.borrow().busy {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            if state.fit {
                state.fit = false;
            }
            state.zoom = (state.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        self.relayout();
        self.show_controls();
    }

    fn zoom_fit(&self) {
        self.state.borrow_mut().fit = true;
        self.relayout();
        self.show_controls();
    }

    fn zoom_100(&self) {
        let mut state = self.state.borrow_mut();
        state.fit = false;
        state.zoom = 1.0;
        drop(state);
        self.relayout();
        self.show_controls();
    }

    fn step(&self, delta: isize) {
        let (file, texture) = {
            let mut state = self.state.borrow_mut();
            let len = state.files.len() as isize;
            if len == 0 {
                return;
            }
            let next = (state.index as isize + delta).rem_euclid(len) as usize;
            if next == state.index {
                return;
            }
            let file = state.files[next].clone();
            match gdk::Texture::from_file(&file) {
                Ok(texture) => {
                    state.index = next;
                    state.texture = Some(texture.clone());
                    state.fit = true;
                    state.fitted_window = false;
                    (file, texture)
                }
                Err(err) => {
                    eprintln!("rajo: {err}");
                    return;
                }
            }
        };
        recent::add(&file.uri());
        self.window.set_title(Some(&title_of(&file)));
        // Refresh the translucent header title.
        if let Some(header) = self.header_revealer.child()
            && let Some(bar) = header.downcast_ref::<adw::HeaderBar>()
        {
            bar.set_title_widget(Some(&adw::WindowTitle::new(&title_of(&file), "")));
        }
        let _ = texture;
        self.apply_image();
        self.fit_window();
        self.update_nav();
        self.show_controls();
    }

    fn update_nav(&self) {
        let many = self.state.borrow().files.len() > 1;
        self.prev_button.set_sensitive(many);
        self.next_button.set_sensitive(many);
    }

    fn fit_window(&self) {
        let Some((iw, ih)) = self.image_size() else {
            return;
        };
        if self.state.borrow().fitted_window {
            return;
        }
        self.state.borrow_mut().fitted_window = true;
        if self.window.is_maximized() || self.window.is_fullscreen() {
            window::show(&self.window);
            return;
        }
        let display = gtk::prelude::WidgetExt::display(&self.window);
        let monitor = self
            .window
            .surface()
            .and_then(|surface| display.monitor_at_surface(&surface))
            .or_else(|| display.monitors().item(0).and_downcast::<gdk::Monitor>());
        if let Some(monitor) = monitor {
            let area = monitor.geometry();
            let scale = (area.width() as f64 * MAX_SCREEN_SHARE / iw)
                .min(area.height() as f64 * MAX_SCREEN_SHARE / ih)
                .min(1.0);
            let (min_w, min_h) = window::MIN_SIZE;
            let scale = scale.max(min_w as f64 / iw).max(min_h as f64 / ih);
            self.window
                .set_default_size((iw * scale).round() as i32, (ih * scale).round() as i32);
        }
        window::show(&self.window);
        // Fit zoom needs the new size.
        let this = self.this.clone();
        glib::idle_add_local_once(move || {
            if let Some(viewer) = this.upgrade() {
                viewer.relayout();
            }
        });
    }

    fn handle_key(&self, key: gdk::Key) -> bool {
        match key.to_lower() {
            gdk::Key::Left | gdk::Key::BackSpace => self.step(-1),
            gdk::Key::Right | gdk::Key::space => self.step(1),
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => self.zoom_by(ZOOM_STEP),
            gdk::Key::minus | gdk::Key::KP_Subtract => self.zoom_by(1.0 / ZOOM_STEP),
            gdk::Key::_0 | gdk::Key::KP_0 => self.zoom_fit(),
            gdk::Key::_1 | gdk::Key::KP_1 => self.zoom_100(),
            gdk::Key::f => self.toggle_fullscreen(),
            gdk::Key::Escape if self.window.is_fullscreen() => self.window.unfullscreen(),
            _ => return false,
        }
        true
    }

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    fn fullscreen_changed(&self) {
        let (icon, tip) = if self.window.is_fullscreen() {
            ("view-restore-symbolic", "Leave Fullscreen")
        } else {
            ("view-fullscreen-symbolic", "Fullscreen")
        };
        self.fullscreen_button.set_icon_name(icon);
        self.fullscreen_button.set_tooltip_text(Some(tip));
        self.relayout();
    }

    fn show_controls(&self) {
        self.header_revealer.set_reveal_child(true);
        self.controls_revealer.set_reveal_child(true);
        self.overlay.set_cursor(None);
        self.schedule_hide();
    }

    fn schedule_hide(&self) {
        let mut state = self.state.borrow_mut();
        if let Some(source) = state.hide_source.take() {
            source.remove();
        }
        let this = self.this.clone();
        state.hide_source = Some(glib::timeout_add_local_once(HIDE_DELAY, move || {
            if let Some(viewer) = this.upgrade() {
                viewer.state.borrow_mut().hide_source = None;
                viewer.hide_controls();
            }
        }));
    }

    fn hide_controls(&self) {
        if self.state.borrow().over_controls {
            return;
        }
        self.header_revealer.set_reveal_child(false);
        self.controls_revealer.set_reveal_child(false);
        self.overlay.set_cursor_from_name(Some("none"));
    }

    pub fn shutdown(&self) {
        let mut state = self.state.borrow_mut();
        if let Some(source) = state.hide_source.take() {
            source.remove();
        }
        if let Some(handler) = state.fullscreen_handler.take() {
            self.window.disconnect(handler);
        }
        if let Some(keys) = state.key_controller.take() {
            self.window.remove_controller(&keys);
        }
    }
}

fn title_of(file: &gio::File) -> String {
    file.basename()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Rajo".into())
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button.add_css_class("circular");
    button
}
