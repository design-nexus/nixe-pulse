//! Telegram custom emoji occupy text anchors at their original UTF-16 positions.
use gtk::{gdk, gio, glib, pango, prelude::*};
use nyx_telegram_core::models::{FormattedText, TextEntityKind};
use serde_json::json;
use std::{cell::RefCell, collections::HashMap};
thread_local! {static EMOJI:RefCell<HashMap<i64,String>>=RefCell::new(HashMap::new());}
pub fn widget(text: &FormattedText) -> gtk::Widget {
    let mut custom = text
        .entities
        .iter()
        .filter_map(|e| {
            if let TextEntityKind::CustomEmoji(id) = &e.kind {
                Some((e.offset, e.length, id.trim_matches('"').parse::<i64>().ok()?))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    custom.sort_by_key(|e| e.0);
    custom.truncate(32);
    if custom.is_empty() {
        let label = gtk::Label::new(None);
        label.add_css_class("message-text");
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_max_width_chars(62);
        label.set_xalign(0.0);
        label.set_selectable(true);
        label.set_markup(&text.to_pango_markup());
        return label.upcast();
    }
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.add_css_class("message-inline");
    view.add_css_class("message-text");
    view.set_size_request(260, -1);
    let buffer = view.buffer();
    let utf16 = text.text.encode_utf16().collect::<Vec<_>>();
    let mut position = 0;
    let mut widgets = Vec::new();
    for (offset, length, id) in &custom {
        let end = offset.saturating_add(*length).min(utf16.len());
        if *offset < position || *offset > utf16.len() {
            continue;
        }
        let Ok(prefix) = String::from_utf16(&utf16[position..*offset]) else { continue };
        buffer.insert(&mut buffer.end_iter(), &prefix);
        let anchor = buffer.create_child_anchor(&mut buffer.end_iter());
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        container.set_size_request(24, 24);
        let fallback = String::from_utf16(&utf16[*offset..end]).unwrap_or_else(|_| "◌".into());
        container.append(&gtk::Label::new(Some(&fallback)));
        view.add_child_at_anchor(&container, &anchor);
        widgets.push((*id, container.downgrade()));
        position = end;
    }
    if let Ok(suffix) = String::from_utf16(&utf16[position..]) {
        buffer.insert(&mut buffer.end_iter(), &suffix);
    }
    let to_offset = |offset: usize| -> Option<i32> {
        let offset = offset.min(utf16.len());
        let mut result = String::from_utf16(&utf16[..offset]).ok()?.chars().count() as i32;
        for (start, len, _) in &custom {
            if start + len <= offset {
                let count = String::from_utf16(&utf16[*start..start + len]).ok()?.chars().count() as i32;
                result -= count - 1;
            }
        }
        Some(result)
    };
    let mut links = Vec::new();
    for entity in &text.entities {
        let (Some(start), Some(end)) = (to_offset(entity.offset), to_offset(entity.offset.saturating_add(entity.length))) else {
            continue;
        };
        let tag = match &entity.kind {
            TextEntityKind::Bold => gtk::TextTag::builder().weight(700).build(),
            TextEntityKind::Italic => gtk::TextTag::builder().style(pango::Style::Italic).build(),
            TextEntityKind::Underline => gtk::TextTag::builder().underline(pango::Underline::Single).build(),
            TextEntityKind::Strikethrough => gtk::TextTag::builder().strikethrough(true).build(),
            TextEntityKind::Code | TextEntityKind::Pre(_) => gtk::TextTag::builder().family("monospace").build(),
            TextEntityKind::TextUrl(url) => {
                links.push((start, end, url.clone()));
                gtk::TextTag::builder()
                    .foreground(crate::theme::current_palette().accent)
                    .underline(pango::Underline::Single)
                    .build()
            }
            _ => continue,
        };
        buffer.tag_table().add(&tag);
        buffer.apply_tag(&tag, &buffer.iter_at_offset(start), &buffer.iter_at_offset(end));
    }
    let gesture = gtk::GestureClick::new();
    let weak = view.downgrade();
    gesture.connect_released(move |_, _, x, y| {
        if let Some(view) = weak.upgrade() {
            let (x, y) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            if let Some(iter) = view.iter_at_location(x, y) {
                if let Some((_, _, url)) = links.iter().find(|(start, end, _)| *start <= iter.offset() && iter.offset() < *end) {
                    if url.starts_with("tg://") || url.starts_with("https://t.me/") {
                        crate::window::resolve_link(url)
                    } else if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
                        let _ = gio::AppInfo::launch_default_for_uri(url, gio::AppLaunchContext::NONE);
                    }
                }
            }
        }
    });
    view.add_controller(gesture);
    let mut missing = Vec::new();
    for (id, weak) in &widgets {
        if let Some(path) = EMOJI.with(|c| c.borrow().get(id).cloned()) {
            if let Some(container) = weak.upgrade() {
                paint(&container, &path);
            }
        } else {
            missing.push(*id);
        }
    }
    if !missing.is_empty() {
        if let Some(service) = crate::window::service() {
            let generation = service.generation.clone();
            let rx = service.request("getCustomEmojiStickers", json!({"custom_emoji_ids":missing}));
            glib::spawn_future_local(async move {
                let Ok(Ok(data)) = rx.recv().await else { return };
                if !crate::window::service_is_current(&generation) {
                    return;
                }
                for (index, item) in data["stickers"].as_array().into_iter().flatten().enumerate() {
                    if widgets.iter().all(|(_, w)| w.upgrade().is_none()) {
                        return;
                    }
                    let Some(id) = missing.get(index) else { continue };
                    let file = &item["sticker"];
                    let mut path = file["local"]["path"]
                        .as_str()
                        .filter(|_| file["local"]["is_downloading_completed"] == true)
                        .map(str::to_owned);
                    if path.is_none() {
                        let rx = service.request(
                            "downloadFile",
                            json!({"file_id":file["id"],"priority":16,"offset":0,"limit":0,"synchronous":true}),
                        );
                        if let Ok(Ok(file)) = rx.recv().await {
                            path = file["local"]["path"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
                        }
                    }
                    if !crate::window::service_is_current(&generation) { return; }
                    if let Some(path) = path {
                        EMOJI.with(|c| {
                            c.borrow_mut().insert(*id, path.clone());
                        });
                        for (_, weak) in widgets.iter().filter(|(widget_id, _)| widget_id == id) {
                            if let Some(container) = weak.upgrade() {
                                paint(&container, &path);
                            }
                        }
                    }
                }
            });
        }
    }
    view.upcast()
}
fn paint(container: &gtk::Box, path: &str) {
    let child: gtk::Widget = if path.ends_with(".tgs") {
        let pic = gtk::Picture::new();
        pic.set_size_request(24, 24);
        if !crate::animation::play(&pic, path) { return; }
        pic.upcast()
    } else if let Ok(texture) = gdk::Texture::from_file(&gio::File::for_path(path)) {
        let pic = gtk::Picture::for_paintable(&texture);
        pic.set_can_shrink(true);
        pic.set_size_request(24, 24);
        pic.upcast()
    } else {
        let pic = gtk::Picture::new();
        crate::clip::play(&pic, path, 24, 24);
        pic.upcast()
    };
    while let Some(c) = container.first_child() {
        container.remove(&c);
    }
    container.append(&child);
}
pub fn clear_cache() {
    EMOJI.with(|c| c.borrow_mut().clear());
}
