mod folder;
mod recent;
mod viewer;
mod window;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

pub const APP_ID: &str = "io.github.rajnish.Rajo";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|app| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        app.set_accels_for_action("win.open", &["<Control>o"]);

        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    });

    app.connect_open(|app, files, _hint| {
        for file in files {
            window::new_window(app, Some(file));
        }
    });

    app.connect_activate(|app| window::new_window(app, None));

    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    for signal in [SIGINT, SIGTERM] {
        let app = app.downgrade();
        glib_unix::unix_signal_add_local(signal, move || {
            if let Some(app) = app.upgrade() {
                let windows = app.windows();
                if windows.is_empty() {
                    app.quit();
                }
                for window in windows {
                    window.close();
                }
            }
            glib::ControlFlow::Continue
        });
    }

    app.run()
}
