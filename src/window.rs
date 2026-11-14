use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::viewer::Viewer;
use crate::recent;

type Slot = Rc<RefCell<Option<Rc<Viewer>>>>;

pub const MIN_SIZE: (i32, i32) = (360, 220);
const SHOW_TIMEOUT: Duration = Duration::from_millis(800);

pub fn new_window(app: &adw::Application, file: Option<&gio::File>) {
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
            let Some(file) = list.files().into_iter().next() else {
                return false;
            };
            load(&window, &slot, &file);
            true
        }
    ));
    window.add_controller(drop_target);

    window.connect_close_request(glib::clone!(
        #[strong]
        slot,
        move |_| {
            if let Some(viewer) = slot.borrow_mut().take() {
                viewer.shutdown();
            }
            glib::Propagation::Proceed
        }
    ));

    match file {
        Some(file) => {
            load(&window, &slot, file);
            glib::timeout_add_local_once(
                SHOW_TIMEOUT,
                glib::clone!(
                    #[weak]
                    window,
                    move || show(&window)
                ),
            );
        }
        None => {
            show_empty(&window, &slot);
            window.present();
        }
    }
}

pub fn show(window: &adw::ApplicationWindow) {
    if !window.is_visible() {
        window.present();
    }
}

fn load(window: &adw::ApplicationWindow, slot: &Slot, file: &gio::File) {
    if let Some(previous) = slot.borrow_mut().take() {
        previous.shutdown();
    }
    let title = file
        .basename()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Rajo".into());
    match Viewer::open(window, file) {
        Ok(viewer) => *slot.borrow_mut() = Some(viewer),
        Err(err) => show_error(window, &title, &err),
    }
}

fn choose_file(window: &adw::ApplicationWindow, slot: &Slot) {
    let images = gtk::FileFilter::new();
    images.set_name(Some("Images"));
    images.add_mime_type("image/*");
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

    dialog.open(
        Some(window),
        gio::Cancellable::NONE,
        glib::clone!(
            #[weak]
            window,
            #[strong]
            slot,
            move |result| {
                if let Ok(file) = result {
                    load(&window, &slot, &file);
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
