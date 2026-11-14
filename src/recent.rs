//! Recent images from GNOME's recent files list.

use gtk::prelude::*;

const SHOWN: usize = 5;

pub fn add(uri: &str) {
    gtk::RecentManager::default().add_item(uri);
}

pub fn list() -> Vec<gtk::RecentInfo> {
    let mut items: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
        .items()
        .into_iter()
        .filter(|item| item.mime_type().starts_with("image/") && item.exists())
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.modified().to_unix()));
    items.truncate(SHOWN);
    items
}
