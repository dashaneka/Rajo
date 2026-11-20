use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::viewer::Viewer;
use crate::{loading, recent};

#[derive(Default)]
struct WindowState {
    viewer: Option<Rc<Viewer>>,
    pending: Option<gio::Cancellable>,
    show_source: Option<glib::SourceId>,
}
type Slot = Rc<RefCell<WindowState>>;

pub const MIN_SIZE: (i32, i32) = (360, 220);
/// How long a window opened with a file waits for the image before showing
/// anyway, so a slow or remote file never leaves it hidden.
const SHOW_TIMEOUT: Duration = Duration::from_millis(1500);

pub fn new_window(app: &adw::Application, file: Option<&gio::File>) {
    match file {
        Some(file) => {
            let (window, slot) = build_window(app);
            load(&window, &slot, file);
            let source = glib::timeout_add_local_once(
                SHOW_TIMEOUT,
                glib::clone!(
                    #[weak]
                    window,
                    #[weak]
                    slot,
                    move || {
                        slot.borrow_mut().show_source = None;
                        show(&window);
                    }
                ),
            );
            slot.borrow_mut().show_source = Some(source);
        }
        None => {
            create_window(app);
        }
    }
}

fn create_window(app: &adw::Application) -> (adw::ApplicationWindow, Slot) {
    let (window, slot) = build_window(app);
    show_empty(&window, &slot);
    window.present();
    (window, slot)
}

fn build_window(app: &adw::Application) -> (adw::ApplicationWindow, Slot) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Rajo")
        .default_width(960)
        .default_height(640)
        .width_request(MIN_SIZE.0)
        .height_request(MIN_SIZE.1)
        .build();

    let slot = Slot::default();

    let open_action = gio::SimpleAction::new("open", None);
    open_action.connect_activate(glib::clone!(
        #[weak]
        window,
        #[strong]
        slot,
        move |_, _| choose_file(&window, &slot)
    ));
    window.add_action(&open_action);

    let drop_target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop_target.connect_drop(glib::clone!(
        #[weak]
        window,
        #[strong]
        slot,
        #[upgrade_or]
        false,
        move |_, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            open_files(&window, &slot, &list.files())
        }
    ));
    window.add_controller(drop_target);

    let cleanup = glib::clone!(
        #[strong]
        slot,
        move || {
            let mut state = slot.borrow_mut();
            if let Some(source) = state.show_source.take() {
                source.remove();
            }
            if let Some(cancel) = state.pending.take() {
                cancel.cancel();
            }
            if let Some(viewer) = state.viewer.take() {
                viewer.shutdown();
            }
        }
    );
    window.connect_close_request({
        let cleanup = cleanup.clone();
        move |_| {
            cleanup();
            glib::Propagation::Proceed
        }
    });
    window.connect_destroy(move |_| cleanup());

    (window, slot)
}

// All entry points share the same batch behavior: first image in this window,
// with a separate window for each additional selection.
fn open_files(window: &adw::ApplicationWindow, slot: &Slot, files: &[gio::File]) -> bool {
    let Some((first, rest)) = files.split_first() else {
        return false;
    };
    load(window, slot, first);
    if let Some(app) = window.application().and_downcast::<adw::Application>() {
        for file in rest {
            new_window(&app, Some(file));
        }
    }
    true
}

pub fn show(window: &adw::ApplicationWindow) {
    if !window.is_visible() {
        window.present();
    }
}

fn load(window: &adw::ApplicationWindow, slot: &Slot, file: &gio::File) {
    let cancel = gio::Cancellable::new();
    {
        let mut state = slot.borrow_mut();
        if let Some(previous) = state.pending.replace(cancel.clone()) {
            previous.cancel();
        }
        if let Some(viewer) = &state.viewer {
            viewer.cancel_loading();
        }
    }
    window.set_cursor_from_name(Some("wait"));
    let file = file.clone();
    let window = window.downgrade();
    let slot = Rc::downgrade(slot);
    glib::MainContext::default().spawn_local(async move {
        let result = loading::load(file.clone(), cancel.clone(), loading::PREVIEW_EDGE).await;
        if cancel.is_cancelled() {
            return;
        }
        let (Some(window), Some(slot)) = (window.upgrade(), slot.upgrade()) else {
            return;
        };
        window.set_cursor(None);
        {
            let mut state = slot.borrow_mut();
            state.pending = None;
            if let Some(source) = state.show_source.take() {
                source.remove();
            }
        }
        match result {
            Ok(image) => {
                if let Some(previous) = slot.borrow_mut().viewer.take() {
                    previous.shutdown();
                }
                let viewer = Viewer::open(&window, &file, image);
                slot.borrow_mut().viewer = Some(viewer);
            }
            Err(err) => {
                if slot.borrow().viewer.is_some() {
                    let dialog = adw::AlertDialog::builder()
                        .heading("Can't Open This Image")
                        .body(&err)
                        .build();
                    dialog.add_response("close", "Close");
                    dialog.present(Some(&window));
                } else {
                    show_error(&window, "Rajo", &err);
                }
            }
        }
    });
}

fn choose_file(window: &adw::ApplicationWindow, slot: &Slot) {
    let images = gtk::FileFilter::new();
    images.set_name(Some("Supported Images"));
    images.add_pixbuf_formats();
    let all = gtk::FileFilter::new();
    all.set_name(Some("All Files"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&images);
    filters.append(&all);

    let dialog = gtk::FileDialog::builder()
        .title("Open Image")
        .modal(true)
        .filters(&filters)
        .default_filter(&images)
        .build();
    if let Some(dir) = glib::user_special_dir(glib::UserDirectory::Pictures) {
        dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    }

    dialog.open_multiple(
        Some(window),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            window,
            #[strong]
            slot,
            move |result| {
                if let Ok(model) = result {
                    let files: Vec<gio::File> =
                        model.iter::<gio::File>().filter_map(Result::ok).collect();
                    open_files(&window, &slot, &files);
                }
            }
        ),
    );
}

pub fn header_bar(title: &str) -> adw::HeaderBar {
    let open_button = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Open Image (Ctrl+O)")
        .action_name("win.open")
        .build();
    let header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new(title, ""))
        .build();
    header.pack_start(&open_button);
    header
}

fn show_page(window: &adw::ApplicationWindow, title: &str, page: &adw::StatusPage) {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header_bar(title));
    toolbar.set_content(Some(page));
    window.set_content(Some(&toolbar));
}

fn show_empty(window: &adw::ApplicationWindow, slot: &Slot) {
    let open_button = gtk::Button::builder()
        .label("Open Image…")
        .action_name("win.open")
        .halign(gtk::Align::Center)
        .build();
    open_button.add_css_class("pill");
    open_button.add_css_class("suggested-action");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 36);
    content.append(&open_button);
    let recent = recent::list();
    if !recent.is_empty() {
        content.append(&recent_list(window, slot, &recent));
    }

    let page = adw::StatusPage::builder()
        .icon_name("image-x-generic-symbolic")
        .title("Drop an Image Here")
        .description("Or open one from your files")
        .child(&content)
        .build();
    if !recent.is_empty() {
        page.add_css_class("compact");
    }
    show_page(window, "Rajo", &page);
}

fn recent_list(
    window: &adw::ApplicationWindow,
    slot: &Slot,
    items: &[gtk::RecentInfo],
) -> adw::Clamp {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    for item in items {
        let file = gio::File::for_uri(&item.uri());
        let subtitle = match file.parent() {
            Some(dir) if dir.path() == Some(glib::home_dir()) => "Home".to_owned(),
            Some(dir) => dir
                .basename()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            None => String::new(),
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&item.display_name()))
            .subtitle(glib::markup_escape_text(&subtitle))
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("image-x-generic-symbolic"));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.connect_activated(glib::clone!(
            #[weak]
            window,
            #[strong]
            slot,
            move |_| load(&window, &slot, &file)
        ));
        list.append(&row);
    }
    let heading = gtk::Label::builder()
        .label("Recent")
        .halign(gtk::Align::Start)
        .build();
    heading.add_css_class("heading");
    let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
    section.append(&heading);
    section.append(&list);
    adw::Clamp::builder()
        .maximum_size(480)
        .child(&section)
        .build()
}

pub fn show_error(window: &adw::ApplicationWindow, title: &str, message: &str) {
    let page = adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title("Can't Open This Image")
        .description(glib::markup_escape_text(message).as_str())
        .build();
    show_page(window, title, &page);
    show(window);
}

#[cfg(test)]
pub(crate) async fn check_failed_open_preserves_viewer(
    window: &adw::ApplicationWindow,
    viewer: Rc<Viewer>,
    invalid: &gio::File,
) {
    let slot = Rc::new(RefCell::new(WindowState {
        viewer: Some(viewer.clone()),
        ..WindowState::default()
    }));
    load(window, &slot, invalid);
    for _ in 0..500 {
        if slot.borrow().pending.is_none() {
            assert!(Rc::ptr_eq(slot.borrow().viewer.as_ref().unwrap(), &viewer));
            return;
        }
        glib::timeout_future(std::time::Duration::from_millis(10)).await;
    }
    panic!("file open did not complete");
}

#[cfg(test)]
pub(crate) async fn check_file_entry_points() {
    use crate::test_support::Directory;
    use std::time::Duration;

    fn contains(widget: &gtk::Widget, predicate: &impl Fn(&gtk::Widget) -> bool) -> bool {
        if predicate(widget) {
            return true;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if contains(&widget, predicate) {
                return true;
            }
            child = widget.next_sibling();
        }
        false
    }

    async fn ready(app: &adw::Application, files: &[gio::File], errors: usize) {
        for _ in 0..500 {
            let windows = app.windows();
            let loaded = windows.len() == files.len() + errors
                && files.iter().all(|file| {
                    let title = file.basename().unwrap().to_string_lossy().into_owned();
                    windows.iter().any(|window| {
                        window.title().as_deref() == Some(title.as_str())
                            && contains(window.upcast_ref(), &|widget| {
                                widget.is::<gtk::Picture>() && widget.width() > 0
                            })
                            && contains(window.upcast_ref(), &|widget| {
                                widget
                                    .downcast_ref::<gtk::Label>()
                                    .is_some_and(|label| label.label() == "Fit")
                            })
                    })
                });
            let failed = windows
                .iter()
                .filter(|window| {
                    contains(window.upcast_ref(), &|widget| {
                        widget
                            .downcast_ref::<adw::StatusPage>()
                            .is_some_and(|page| page.title() == "Can't Open This Image")
                    })
                })
                .count();
            if loaded && failed == errors {
                return;
            }
            glib::timeout_future(Duration::from_millis(10)).await;
        }
        panic!("file entry point did not open every selection");
    }

    let dir = Directory::new();
    let spaced = dir.image("photo with spaces #1 % & <tag>.png", 48, 32);
    let unicode = dir.image("ფოტო café 图片.png", 32, 48);
    let leading_dash = dir.image("-photo.png", 64, 32);
    let files = vec![spaced, unicode, leading_dash];
    let app = crate::application();
    app.register(gio::Cancellable::NONE).unwrap();

    // Exercise the same GApplication open signal used by desktop %U launches.
    let mut selection = files.clone();
    selection.push(gio::File::for_path(dir.0.join("missing.png")));
    app.open(&selection, "");
    ready(&app, &files, 1).await;
    for window in app.windows() {
        window.close();
    }

    // File picker and drag-and-drop use this common batch route.
    let (window, slot) = create_window(&app);
    assert!(!open_files(&window, &slot, &[]));
    assert!(open_files(&window, &slot, &files));
    ready(&app, &files, 0).await;
    assert!(slot.borrow().viewer.is_some());
    assert_eq!(
        window.title().unwrap().as_str(),
        files[0].basename().unwrap().to_string_lossy().as_ref()
    );
    for window in app.windows() {
        window.close();
    }
    assert!(slot.borrow().viewer.is_none());

    // Replacing a request before completion must display only the newest file.
    let (window, slot) = create_window(&app);
    load(&window, &slot, &files[0]);
    load(&window, &slot, &files[1]);
    ready(&app, &files[1..2], 0).await;
    window.close();

    // Closing a window while its decode is queued must not resurrect it.
    new_window(&app, Some(&files[0]));
    for window in app.windows() {
        if window.is_visible() {
            window.close();
        } else {
            window.destroy();
        }
    }
    let (window, slot) = create_window(&app);
    load(&window, &slot, &files[0]);
    window.close();
    glib::timeout_future(Duration::from_millis(100)).await;
    assert!(app.windows().is_empty());
}
