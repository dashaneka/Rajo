//! Shows one image: zoom, pan, next/prev, fade controls.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::{canvas::Canvas, folder, loading, viewport::Viewport};
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
    toasts: adw::ToastOverlay,
    scrolled: gtk::ScrolledWindow,
    /// Exact pixel size of the zoomed image; the scrolled window sizes to this.
    canvas: Canvas,
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
    files: Rc<[gio::File]>,
    index: usize,
    /// Target index of an in-flight folder step so rapid arrow presses advance.
    target_index: Option<usize>,
    texture: Option<gdk::Texture>,
    original: (i32, i32),
    /// Longest edge requested for the current texture; prevents re-decode loops
    /// when a codec produces pixels slightly smaller than requested.
    decoded_edge: i32,
    pending: Option<gio::Cancellable>,
    scan: Option<gio::Cancellable>,
    stopped: bool,
    /// Multiplier of the image's own pixels. 1.0 = 100%.
    zoom: f64,
    /// When true, zoom is picked so the image fits the window.
    fit: bool,
    fitted_window: bool,
    /// Blocks zoom→resize→scroll→zoom recursion.
    busy: bool,
    pointer: Option<(f64, f64)>,
    motion_controller: Option<gtk::EventControllerMotion>,
    drag_start: Option<(f64, f64)>,
    hide_source: Option<glib::SourceId>,
    key_controller: Option<gtk::EventControllerKey>,
    fullscreen_handler: Option<glib::SignalHandlerId>,
}

impl Viewer {
    pub fn open(
        window: &adw::ApplicationWindow,
        file: &gio::File,
        image: loading::Image,
    ) -> Rc<Self> {
        let original = image.original;
        let texture = image.texture();
        let files = vec![file.clone()];
        let index = 0;
        recent::add(&file.uri());

        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Fill)
            .hexpand(true)
            .vexpand(true)
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Fill)
            .build();

        let canvas = Canvas::new(&picture);

        let scrolled = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .kinetic_scrolling(true)
            .child(&canvas)
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
        let viewport = Viewport::new(&overlay);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&viewport));
        window.set_content(Some(&toasts));
        window.set_title(Some(&title_of(file)));

        let viewer = Rc::new_cyclic(|this| Self {
            this: this.clone(),
            window: window.clone(),
            overlay,
            toasts,
            scrolled,
            canvas,
            picture,
            header_revealer,
            controls_revealer,
            zoom_label,
            prev_button: prev_button.clone(),
            next_button: next_button.clone(),
            fullscreen_button: fullscreen_button.clone(),
            state: RefCell::new(State {
                files: files.into(),
                index,
                target_index: None,
                texture: Some(texture),
                original,
                decoded_edge: loading::PREVIEW_EDGE,
                pending: None,
                scan: None,
                stopped: false,
                zoom: 1.0,
                fit: true,
                fitted_window: false,
                busy: false,
                pointer: None,
                motion_controller: None,
                drag_start: None,
                hide_source: None,
                key_controller: None,
                fullscreen_handler: None,
            }),
        });
        viewer.connect_signals(&zoom_out, &zoom_in, &zoom_fit, &zoom_100);
        let refit = viewer.with(Viewer::relayout);
        viewport.connect_resized(refit);
        viewer.apply_image();
        viewer.fit_window();
        viewer.update_nav();
        viewer.show_controls();
        viewer.scan_folder(file);
        viewer
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

        let fullscreen = self.with(Viewer::fullscreen_changed);
        let handler = self
            .window
            .connect_fullscreened_notify(move |_| fullscreen());
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
                viewer.schedule_hide();
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

        // Observe the whole window: revealing child widgets must not look like
        // the pointer leaving and re-entering the image overlay.
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = self.this.clone();
        motion.connect_motion(move |_, x, y| {
            if let Some(viewer) = this.upgrade() {
                viewer.pointer_moved(x, y);
            }
        });
        let this = self.this.clone();
        motion.connect_leave(move |_| {
            if let Some(viewer) = this.upgrade() {
                viewer.state.borrow_mut().pointer = None;
                viewer.hide_controls();
            }
        });
        self.window.add_controller(motion.clone());
        self.state.borrow_mut().motion_controller = Some(motion);
    }

    fn pointer_moved(&self, x: f64, y: f64) {
        {
            let mut state = self.state.borrow_mut();
            if state.pointer == Some((x, y)) {
                return;
            }
            state.pointer = Some((x, y));
        }
        self.show_controls();
    }

    fn pointer_over_bar(&self) -> bool {
        let Some((x, y)) = self.state.borrow().pointer else {
            return false;
        };
        let point = gtk::graphene::Point::new(x as f32, y as f32);
        [&self.header_revealer, &self.controls_revealer]
            .into_iter()
            .any(|bar| {
                bar.reveals_child()
                    && bar
                        .compute_bounds(&self.window)
                        .is_some_and(|bounds| bounds.contains_point(&point))
            })
    }

    fn apply_image(&self) {
        let texture = self.state.borrow().texture.clone();
        self.picture.set_paintable(texture.as_ref());
        self.relayout();
    }

    fn image_size(&self) -> (f64, f64) {
        let state = self.state.borrow();
        (state.original.0 as f64, state.original.1 as f64)
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
            (state.original.0 as f64, state.original.1 as f64, state.fit)
        };
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        self.state.borrow_mut().busy = true;
        let zoom = if fit {
            let zoom = self.fit_zoom_for(iw, ih);
            self.state.borrow_mut().zoom = zoom;
            zoom
        } else {
            let mut state = self.state.borrow_mut();
            state.zoom = state.zoom.clamp(
                MIN_ZOOM.min(self.fit_zoom_for(iw, ih)),
                MAX_ZOOM.max(self.fit_zoom_for(iw, ih)),
            );
            state.zoom
        };
        let (w, h) = ((iw * zoom).round() as i32, (ih * zoom).round() as i32);
        let (w, h) = (w.max(1), h.max(1));
        self.canvas.set_dimensions(w, h);
        let label = if fit {
            "Fit".to_owned()
        } else {
            format!("{}%", (zoom * 100.0).round() as i32)
        };
        if self.zoom_label.label() != label {
            self.zoom_label.set_label(&label);
        }
        self.state.borrow_mut().busy = false;
        self.ensure_detail();
    }

    fn fit_zoom_for(&self, iw: f64, ih: f64) -> f64 {
        if iw <= 0.0 || ih <= 0.0 {
            return 1.0;
        }
        let (vw, vh) = self.view_size();
        // Fit fills the available viewport, including for small images.
        (vw / iw).min(vh / ih)
    }

    fn zoom_by(&self, factor: f64) {
        if self.state.borrow().busy {
            return;
        }
        let (iw, ih) = self.image_size();
        let fit_zoom = self.fit_zoom_for(iw, ih);
        let minimum = MIN_ZOOM.min(fit_zoom);
        let maximum = MAX_ZOOM.max(fit_zoom);
        {
            let mut state = self.state.borrow_mut();
            if state.fit {
                state.fit = false;
            }
            state.zoom = (state.zoom * factor).clamp(minimum, maximum);
        }
        self.relayout();
        self.show_controls();
    }

    fn zoom_fit(&self) {
        self.cancel_loading();
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

    fn scan_folder(&self, file: &gio::File) {
        let cancel = gio::Cancellable::new();
        self.state.borrow_mut().scan = Some(cancel.clone());
        let file = file.clone();
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let worker_cancel = cancel.clone();
            let result = gio::spawn_blocking(move || folder::siblings(&file, &worker_cancel)).await;
            if cancel.is_cancelled() {
                return;
            }
            if let Some(viewer) = this.upgrade()
                && let Ok((files, index)) = result
            {
                let mut state = viewer.state.borrow_mut();
                state.files = files.into();
                state.index = index;
                state.scan = None;
                drop(state);
                viewer.update_nav();
            }
        });
    }

    pub fn cancel_loading(&self) {
        let mut state = self.state.borrow_mut();
        state.target_index = None;
        if let Some(cancel) = state.pending.take() {
            cancel.cancel();
        }
    }

    fn step(&self, delta: isize) {
        let (files, current_index, base) = {
            let state = self.state.borrow();
            if state.files.len() < 2 || state.stopped {
                return;
            }
            (
                state.files.clone(),
                state.index,
                state.target_index.unwrap_or(state.index),
            )
        };
        self.cancel_loading();
        let first = (base as isize + delta).rem_euclid(files.len() as isize) as usize;
        if first == current_index {
            return;
        }
        let cancel = gio::Cancellable::new();
        {
            let mut state = self.state.borrow_mut();
            state.pending = Some(cancel.clone());
            state.target_index = Some(first);
        }
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let mut skipped = 0;
            for offset in 1..files.len() {
                let next = (base as isize + delta * offset as isize)
                    .rem_euclid(files.len() as isize) as usize;
                if next == current_index {
                    continue;
                }
                let file = &files[next];
                let result =
                    loading::load(file.clone(), cancel.clone(), loading::PREVIEW_EDGE).await;
                if cancel.is_cancelled() {
                    return;
                }
                let Some(viewer) = this.upgrade() else { return };
                match result {
                    Ok(image) => {
                        {
                            let mut state = viewer.state.borrow_mut();
                            state.index = next;
                            state.target_index = None;
                            state.original = image.original;
                            state.decoded_edge = loading::PREVIEW_EDGE;
                            state.texture = Some(image.texture());
                            state.fit = true;
                            state.fitted_window = false;
                            state.pending = None;
                        }
                        recent::add(&file.uri());
                        let title = title_of(file);
                        viewer.window.set_title(Some(&title));
                        if let Some(header) = viewer.header_revealer.child()
                            && let Some(bar) = header.downcast_ref::<adw::HeaderBar>()
                            && let Some(title_widget) = bar
                                .title_widget()
                                .and_downcast::<adw::WindowTitle>()
                        {
                            title_widget.set_title(&title);
                        }
                        viewer.apply_image();
                        viewer.fit_window();
                        viewer.show_controls();
                        if skipped > 0 {
                            viewer.notice("Skipped unreadable images in this folder");
                        }
                        return;
                    }
                    Err(_) => skipped += 1,
                }
            }
            if let Some(viewer) = this.upgrade() {
                let mut state = viewer.state.borrow_mut();
                state.pending = None;
                state.target_index = None;
                drop(state);
                viewer.notice("No other readable images in this folder");
            }
        });
    }

    fn notice(&self, message: &str) {
        let toast = adw::Toast::builder()
            .title(message)
            .use_markup(false)
            .build();
        self.toasts.add_toast(toast);
    }

    fn ensure_detail(&self) {
        let (file, edge) = {
            let state = self.state.borrow();
            if state.pending.is_some() || state.stopped {
                return;
            }
            let Some(texture) = state.texture.as_ref() else {
                return;
            };
            let (iw, ih) = state.original;
            if iw <= 0 || ih <= 0 {
                return;
            }
            // Include the display scale so HiDPI screens stay sharp.
            let scale = self.window.scale_factor() as f64;
            let required = (iw.max(ih) as f64 * state.zoom * scale)
                .ceil()
                .min(iw.max(ih) as f64) as i32;
            let edge = required.max(loading::PREVIEW_EDGE).min(iw.max(ih));
            let current = texture.width().max(texture.height());
            // Release full-resolution pixels again when returning to Fit.
            let shrink = state.fit && current > edge;
            if (!shrink && (required <= current + 1 || edge <= state.decoded_edge))
                || (shrink && edge >= state.decoded_edge)
            {
                return;
            }
            (state.files[state.index].clone(), edge)
        };
        let cancel = gio::Cancellable::new();
        self.state.borrow_mut().pending = Some(cancel.clone());
        let this = self.this.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = loading::load(file, cancel.clone(), edge).await;
            if cancel.is_cancelled() {
                return;
            }
            if let Some(viewer) = this.upgrade() {
                viewer.state.borrow_mut().pending = None;
                match result {
                    Ok(image) => {
                        {
                            let mut state = viewer.state.borrow_mut();
                            state.decoded_edge = edge;
                            state.texture = Some(image.texture());
                        }
                        viewer.apply_image();
                    }
                    Err(err) => viewer.notice(&format!("Can't load image detail: {err}")),
                }
            }
        });
    }

    fn update_nav(&self) {
        let many = self.state.borrow().files.len() > 1;
        self.prev_button.set_sensitive(many);
        self.next_button.set_sensitive(many);
    }

    fn fit_window(&self) {
        let (iw, ih) = self.image_size();
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
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
            let (min_w, min_h) = window::MIN_SIZE;
            let (width, height) = window_size(iw, ih, area.width(), area.height(), min_w, min_h);
            self.window.set_default_size(width, height);
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
        if self.pointer_over_bar() || self.state.borrow().drag_start.is_some() {
            return;
        }
        if let Some(source) = self.state.borrow_mut().hide_source.take() {
            source.remove();
        }
        self.header_revealer.set_reveal_child(false);
        self.controls_revealer.set_reveal_child(false);
        self.overlay.set_cursor_from_name(Some("none"));
    }

    pub fn shutdown(&self) {
        let mut state = self.state.borrow_mut();
        state.stopped = true;
        for cancel in [state.pending.take(), state.scan.take()]
            .into_iter()
            .flatten()
        {
            cancel.cancel();
        }
        if let Some(source) = state.hide_source.take() {
            source.remove();
        }
        if let Some(handler) = state.fullscreen_handler.take() {
            self.window.disconnect(handler);
        }
        if let Some(motion) = state.motion_controller.take() {
            self.window.remove_controller(&motion);
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

/// Clamp window dimensions independently; preserve the image aspect ratio inside it.
fn window_size(
    iw: f64,
    ih: f64,
    screen_w: i32,
    screen_h: i32,
    min_w: i32,
    min_h: i32,
) -> (i32, i32) {
    let max_w = (screen_w as f64 * MAX_SCREEN_SHARE).round() as i32;
    let max_h = (screen_h as f64 * MAX_SCREEN_SHARE).round() as i32;
    let scale = (max_w as f64 / iw).min(max_h as f64 / ih).min(1.0);
    (
        ((iw * scale).round() as i32).max(min_w).min(max_w),
        ((ih * scale).round() as i32).max(min_h).min(max_h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for(mut ready: impl FnMut() -> bool) {
        for _ in 0..500 {
            if ready() {
                return;
            }
            glib::timeout_future(Duration::from_millis(10)).await;
        }
        panic!("viewer operation timed out");
    }

    #[test]
    fn viewer_loading_navigation_zoom_and_recovery() {
        glib::set_prgname(Some("rajo-tests"));
        adw::init().expect("graphical integration test needs a display");
        glib::set_application_name("Rajo Tests");
        let context = glib::MainContext::default();
        context.block_on(async {
            let dir = crate::test_support::Directory::new();
            let first = dir.image("a.png", 4096, 3072);
            std::fs::write(dir.0.join("b.png"), b"corrupt image").unwrap();
            let last = dir.image("c.png", 64, 32);
            let app = adw::Application::builder()
                .application_id("io.github.rajnish.Rajo.Tests")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            app.register(gio::Cancellable::NONE).unwrap();
            let window = adw::ApplicationWindow::builder()
                .application(&app)
                .default_width(800)
                .default_height(600)
                .build();
            let image = loading::load(
                first.clone(),
                gio::Cancellable::new(),
                loading::PREVIEW_EDGE,
            )
            .await
            .unwrap();
            let viewer = Viewer::open(&window, &first, image);
            wait_for(|| viewer.state.borrow().scan.is_none() && viewer.scrolled.width() > 1).await;
            wait_for(|| viewer.state.borrow().pending.is_none()).await;
            viewer.relayout();
            assert!(viewer.state.borrow().fit);
            assert_eq!(viewer.zoom_label.label(), "Fit");
            assert!(viewer.canvas.dimensions().0 <= viewer.scrolled.width());
            assert!(viewer.canvas.dimensions().1 <= viewer.scrolled.height());
            // Check allocated pixels, not just the percentage or size request.
            wait_for(|| viewer.picture.width() == viewer.canvas.dimensions().0).await;
            for factor in [0.8, 0.8, 0.8, 1.25, 1.25, 1.25] {
                let before = viewer.picture.width();
                viewer.zoom_by(factor);
                wait_for(|| viewer.picture.width() == viewer.canvas.dimensions().0).await;
                assert_ne!(
                    viewer.picture.width(),
                    before,
                    "each zoom step must resize the image"
                );
                assert_eq!(viewer.picture.height(), viewer.canvas.dimensions().1);
            }
            viewer.zoom_fit();
            wait_for(|| viewer.state.borrow().pending.is_none()).await;

            // Inject deterministic pointer coordinates without interference
            // from the real desktop pointer while this test window is visible.
            let motion = viewer.state.borrow().motion_controller.clone().unwrap();
            assert_eq!(motion.widget().as_ref(), Some(window.upcast_ref()));
            window.remove_controller(&motion);
            // Hovering either bar keeps it visible across the hide deadline.
            for bar in [&viewer.header_revealer, &viewer.controls_revealer] {
                viewer.show_controls();
                let bounds = bar.compute_bounds(&window).unwrap();
                viewer.pointer_moved(bounds.center().x() as f64, bounds.center().y() as f64);
                glib::timeout_future(HIDE_DELAY + Duration::from_millis(200)).await;
                assert!(viewer.header_revealer.reveals_child());
                assert!(viewer.controls_revealer.reveals_child());
            }
            let x = window.width() as f64 / 2.0;
            let y = window.height() as f64 / 2.0;
            viewer.pointer_moved(x, y);
            glib::timeout_future(HIDE_DELAY + Duration::from_millis(300)).await;
            assert!(!viewer.controls_revealer.reveals_child());
            // Synthetic motion from a reveal/layout change must not reopen it.
            viewer.pointer_moved(x, y);
            glib::timeout_future(Duration::from_millis(300)).await;
            assert!(!viewer.controls_revealer.reveals_child());
            viewer.pointer_moved(x + 10.0, y);
            assert!(viewer.controls_revealer.reveals_child());
            window.add_controller(motion);

            // Fit must work below the manual 5% zoom floor.
            assert!(viewer.fit_zoom_for(100000.0, 100000.0) < MIN_ZOOM);
            viewer.zoom_100();
            wait_for(|| viewer.state.borrow().pending.is_none()).await;
            wait_for(|| viewer.picture.width() == 4096).await;
            assert_eq!(
                viewer.state.borrow().texture.as_ref().unwrap().width(),
                4096
            );
            viewer.zoom_fit();
            wait_for(|| viewer.state.borrow().pending.is_none()).await;
            assert_eq!(
                viewer.state.borrow().texture.as_ref().unwrap().width(),
                loading::PREVIEW_EDGE
            );
            // Every new image resets manual zoom and fills the viewport.
            viewer.zoom_by(ZOOM_STEP);
            assert!(!viewer.state.borrow().fit);
            // Broken siblings must be skipped in both directions.
            viewer.step(1);
            wait_for(|| viewer.state.borrow().pending.is_none()).await;
            assert!(viewer.state.borrow().files[viewer.state.borrow().index].equal(&last));
            wait_for(|| {
                let (w, h) = viewer.canvas.dimensions();
                viewer.picture.width() == w
                    && viewer.picture.height() == h
                    && ((w - viewer.scrolled.width()).abs() <= 1
                        || (h - viewer.scrolled.height()).abs() <= 1)
            })
            .await;
            assert!(viewer.state.borrow().fit);
            assert!(
                viewer.state.borrow().zoom > 1.0,
                "small images should also fit the viewport"
            );
            assert_eq!(viewer.zoom_label.label(), "Fit");
            viewer.step(-1);
            wait_for(|| viewer.state.borrow().pending.is_none()).await;
            assert!(viewer.state.borrow().files[viewer.state.borrow().index].equal(&first));
            assert!(viewer.state.borrow().fit);
            assert_eq!(viewer.zoom_label.label(), "Fit");
            crate::window::check_failed_open_preserves_viewer(
                &window,
                viewer.clone(),
                &gio::File::for_path(dir.0.join("b.png")),
            )
            .await;
            // Superseded navigation cannot replace the retained image.
            viewer.step(1);
            viewer.cancel_loading();
            glib::timeout_future(Duration::from_millis(100)).await;
            assert!(viewer.state.borrow().files[viewer.state.borrow().index].equal(&first));
            // If every remaining sibling is broken or gone, retain this image.
            std::fs::remove_file(last.path().unwrap()).unwrap();
            viewer.step(1);
            wait_for(|| viewer.state.borrow().pending.is_none()).await;
            assert!(viewer.state.borrow().files[viewer.state.borrow().index].equal(&first));
            viewer.shutdown();
            assert!(viewer.state.borrow().pending.is_none());
            assert!(viewer.state.borrow().scan.is_none());
            window.close();
            crate::window::check_file_entry_points().await;
        });
    }

    #[test]
    fn extreme_aspect_ratios_stay_within_screen() {
        for (iw, ih) in [
            (100.0, 10000.0),
            (10000.0, 100.0),
            (1.0, 1.0),
            (800.0, 600.0),
        ] {
            let (w, h) = window_size(iw, ih, 1920, 1080, 360, 220);
            assert!((360..=1632).contains(&w));
            assert!((220..=918).contains(&h));
        }
    }
}
