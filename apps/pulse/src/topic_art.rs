//! Topic artwork and emoji selection. Static vector frames avoid one animation timer per topic.
use crate::{
    actions::{self, Scope},
    groups::{note, scroll},
    window,
};
use gtk::{gdk, gio, glib, prelude::*};
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
};
thread_local! {
    static STICKERS:RefCell<HashMap<(String,String),Value>>=RefCell::new(HashMap::new());
    static SLOTS:Arc<tokio::sync::Semaphore>=Arc::new(tokio::sync::Semaphore::new(4));
}
pub fn emoji_id(sticker: &Value) -> Value {
    sticker["full_type"]["custom_emoji_id"].clone()
}
fn nonzero(id: &Value) -> bool {
    id.as_i64().is_some_and(|v| v != 0) || id.as_str().is_some_and(|v| v != "0" && !v.is_empty())
}
pub fn badge(icon: &Value, general: bool) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    let area = gtk::DrawingArea::new();
    area.set_content_width(28);
    area.set_content_height(28);
    let color = icon["color"].as_u64().unwrap_or(0x6FB9F0) as u32;
    area.set_draw_func(move |_, cr, width, height| {
        let size = width.min(height) as f64;
        let (x, y) = (width as f64 / 2.0, height as f64 / 2.0);
        cr.set_source_rgb(((color >> 16) & 255) as f64 / 255., ((color >> 8) & 255) as f64 / 255., (color & 255) as f64 / 255.);
        cr.arc(x, y, size * 0.44, 0., std::f64::consts::TAU);
        let _ = cr.fill();
        cr.set_source_rgba(0., 0., 0., 0.78);
        cr.set_line_width(size * 0.065);
        if general {
            for fraction in [-0.1, 0.1] {
                cr.move_to(x - size * 0.23, y + size * fraction);
                cr.line_to(x + size * 0.23, y + size * fraction);
                cr.move_to(x + size * fraction, y - size * 0.23);
                cr.line_to(x + size * fraction, y + size * 0.23);
            }
        } else {
            for fraction in [-0.1, 0.1] {
                cr.move_to(x - size * 0.2, y + size * fraction);
                cr.line_to(x + size * 0.2, y + size * fraction);
            }
        }
        let _ = cr.stroke();
    });
    overlay.set_child(Some(&area));
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_size_request(28, 28);
    picture.set_visible(false);
    overlay.add_overlay(&picture);
    if nonzero(&icon["custom_emoji_id"]) {
        paint(&picture, icon["custom_emoji_id"].clone(), None);
    }
    overlay
}
fn active_picture(p: &gtk::Picture) -> bool {
    p.root().and_downcast::<gtk::Window>().is_some_and(|d| d.is_visible())
}
fn paint(picture: &gtk::Picture, id: Value, known: Option<Value>) {
    let Some(scope) = Scope::current() else { return };
    let generation = window::service().map(|s| s.generation).unwrap_or_default();
    let key = (generation, id.to_string());
    let weak = picture.downgrade();
    glib::spawn_future_local(async move {
        let slots = SLOTS.with(Clone::clone);
        let Ok(_permit) = slots.acquire_owned().await else { return };
        if !scope.valid() || weak.upgrade().is_none_or(|p| !active_picture(&p)) {
            return;
        }
        let cached = STICKERS.with(|s| s.borrow().get(&key).cloned());
        let mut sticker = if let Some(sticker) = known.or(cached) {
            sticker
        } else {
            let Ok(data) = scope.call("getCustomEmojiStickers", json!({"custom_emoji_ids":[id]})).await else { return };
            let Some(sticker) = data["stickers"].as_array().and_then(|v| v.first()).cloned() else { return };
            sticker
        };
        let vector = sticker["format"]["@type"] == "stickerFormatTgs";
        let video = sticker["format"]["@type"] == "stickerFormatWebm";
        let file = if video { sticker["thumbnail"]["file"].clone() } else { sticker["sticker"].clone() };
        let Some(file_id) = file["id"].as_i64() else { return };
        if file["size"].as_u64().unwrap_or(0) > 4 * 1024 * 1024 {
            return;
        }
        let file = if file["local"]["is_downloading_completed"] == true {
            file
        } else {
            match scope
                .call("downloadFile", json!({"file_id":file_id,"priority":8,"offset":0,"limit":0,"synchronous":true}))
                .await
            {
                Ok(file) => file,
                Err(_) => return,
            }
        };
        if !scope.valid() {
            return;
        }
        let Some(p) = weak.upgrade().filter(active_picture) else { return };
        let Some(path) = file["local"]["path"].as_str().filter(|s| !s.is_empty()) else { return };
        let rendered = if vector {
            let color = (sticker["full_type"]["needs_repainting"] == true).then(|| p.color());
            crate::animation::still(&p, path, color)
        } else {
            match gdk::Texture::from_file(&gio::File::for_path(path)) {
                Ok(texture) => {
                    p.set_paintable(Some(&texture));
                    true
                }
                Err(_) => false,
            }
        };
        if rendered {
            p.set_visible(true);
            if let Some(overlay) = p.parent().and_downcast::<gtk::Overlay>() {
                if let Some(base) = overlay.child() {
                    base.set_visible(false);
                }
            }
            if video {
                sticker["thumbnail"]["file"] = file;
            } else {
                sticker["sticker"] = file;
            }
            STICKERS.with(|s| {
                let mut s = s.borrow_mut();
                if s.len() >= 256 {
                    s.clear();
                }
                s.insert(key, sticker);
            });
        }
    });
}
pub fn choose(callback: impl Fn(Value) + 'static) {
    actions::run(0, "topic-icon-picker", move |scope| async move {
        let premium = scope.call("getMe", json!({})).await?["is_premium"] == true;
        let mut ids = vec![Value::Null];
        let mut titles = vec!["Default topic icons".to_string()];
        if premium {
            if let Ok(sets) =
                scope.call("getInstalledStickerSets", json!({"sticker_type":{"@type":"stickerTypeCustomEmoji"}})).await
            {
                for set in sets["sets"].as_array().into_iter().flatten() {
                    ids.push(set["id"].clone());
                    titles.push(set["title"].as_str().unwrap_or("Emoji pack").into());
                }
            }
        }
        let (dialog, body) = w::dialog("Topic icon", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        if !premium {
            note(&body, "Your account can use Telegram's default topic icons. Premium also allows installed custom emoji packs.");
        }
        let packs = gtk::DropDown::from_strings(&titles.iter().map(String::as_str).collect::<Vec<_>>());
        body.append(&packs);
        let list = scroll(&body);
        let status = note(&body, "");
        let clear = gtk::Button::with_label("Use the topic's default symbol");
        body.append(&clear);
        let callback: Rc<dyn Fn(Value)> = Rc::new(callback);
        let (cb, s, weak) = (callback.clone(), scope.clone(), dialog.downgrade());
        clear.connect_clicked(move |_| {
            if s.valid() {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                cb(json!(0));
            }
        });
        let revision = Rc::new(Cell::new(0u64));
        let weak = dialog.downgrade();
        let load: Rc<dyn Fn(usize)> = Rc::new(move |index| {
            let Some(id) = ids.get(index).cloned() else { return };
            let n = revision.get() + 1;
            revision.set(n);
            status.set_text("Loading icons…");
            let (scope, weak, list, status, revision, callback) =
                (scope.clone(), weak.clone(), list.clone(), status.clone(), revision.clone(), callback.clone());
            glib::spawn_future_local(async move {
                let result = scope
                    .call(
                        if id.is_null() { "getForumTopicDefaultIcons" } else { "getStickerSet" },
                        if id.is_null() { json!({}) } else { json!({"set_id":id}) },
                    )
                    .await;
                if !scope.valid() || revision.get() != n || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                while let Some(c) = list.first_child() {
                    list.remove(&c)
                }
                let data = match result {
                    Ok(v) => v,
                    Err(e) => {
                        status.set_text(&e);
                        return;
                    }
                };
                status.set_text("");
                let flow = gtk::FlowBox::new();
                flow.set_selection_mode(gtk::SelectionMode::None);
                flow.set_min_children_per_line(4);
                flow.set_max_children_per_line(8);
                flow.set_row_spacing(4);
                flow.set_column_spacing(4);
                list.append(&flow);
                for sticker in data["stickers"].as_array().into_iter().flatten() {
                    let id = emoji_id(sticker);
                    if !nonzero(&id) {
                        continue;
                    }
                    let b = gtk::Button::new();
                    b.set_tooltip_text(sticker["emoji"].as_str());
                    let overlay = gtk::Overlay::new();
                    overlay.set_child(Some(&w::label(sticker["emoji"].as_str().unwrap_or("◆"), "message-text")));
                    let picture = gtk::Picture::new();
                    picture.set_size_request(32, 32);
                    picture.set_can_shrink(true);
                    picture.set_visible(false);
                    overlay.add_overlay(&picture);
                    b.set_child(Some(&overlay));
                    flow.append(&b);
                    paint(&picture, id.clone(), Some(sticker.clone()));
                    let (scope, weak, callback) = (scope.clone(), weak.clone(), callback.clone());
                    b.connect_clicked(move |_| {
                        if scope.valid() {
                            if let Some(d) = weak.upgrade() {
                                d.close();
                                callback(id.clone());
                            }
                        }
                    });
                }
            });
        });
        let retry = gtk::Button::with_label("Retry icons");
        let retry_load = load.clone();
        let retry_packs = packs.clone();
        retry.connect_clicked(move |_| retry_load(retry_packs.selected() as usize));
        body.append(&retry);
        let initial = load.clone();
        packs.connect_selected_notify(move |p| load(p.selected() as usize));
        dialog.present();
        initial(0);
        Ok(())
    });
}
