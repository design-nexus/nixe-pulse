//! Formatted Telegram text backed by GTK tags, preserving UTF-16 entity offsets.
use gtk::{gdk, pango, prelude::*};
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
const PREFIX: &str = "pulse-entity:";
fn tag(buffer: &gtk::TextBuffer, kind: &Value) -> gtk::TextTag {
    let name = format!("{PREFIX}{kind}");
    if let Some(tag) = buffer.tag_table().lookup(&name) {
        return tag;
    }
    let tag = gtk::TextTag::new(Some(&name));
    match kind["@type"].as_str() {
        Some("textEntityTypeBold") => tag.set_weight(700),
        Some("textEntityTypeItalic") => tag.set_style(pango::Style::Italic),
        Some("textEntityTypeUnderline" | "textEntityTypeTextUrl" | "textEntityTypeMentionName") => {
            tag.set_underline(pango::Underline::Single)
        }
        Some("textEntityTypeStrikethrough") => tag.set_strikethrough(true),
        Some("textEntityTypeCode" | "textEntityTypePre" | "textEntityTypePreCode") => tag.set_family(Some("monospace")),
        Some("textEntityTypeBlockQuote" | "textEntityTypeExpandableBlockQuote") => {
            tag.set_left_margin(16);
            tag.set_style(pango::Style::Italic);
        }
        Some("textEntityTypeSpoiler") => tag.set_underline(pango::Underline::Double),
        _ => {}
    }
    buffer.tag_table().add(&tag);
    tag
}
fn char_offset(text: &str, utf16: usize) -> Option<i32> {
    let mut units = 0;
    for (i, c) in text.chars().enumerate() {
        if units == utf16 {
            return Some(i as i32);
        }
        units += c.len_utf16();
        if units > utf16 {
            return None;
        }
    }
    (units == utf16).then_some(text.chars().count() as i32)
}
pub fn load(buffer: &gtk::TextBuffer, formatted: &Value) {
    let text = formatted["text"].as_str().unwrap_or("");
    buffer.set_text(text);
    for entity in formatted["entities"].as_array().into_iter().flatten() {
        let Some(start) = entity["offset"].as_u64().and_then(|n| char_offset(text, n as usize)) else { continue };
        let Some(end) = entity["offset"]
            .as_u64()
            .zip(entity["length"].as_u64())
            .and_then(|(a, b)| char_offset(text, a.saturating_add(b) as usize))
        else {
            continue;
        };
        if end > start {
            buffer.apply_tag(&tag(buffer, &entity["type"]), &buffer.iter_at_offset(start), &buffer.iter_at_offset(end));
        }
    }
}
pub fn formatted(buffer: &gtk::TextBuffer) -> Value {
    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
    let mut active = BTreeMap::<String, (usize, usize)>::new();
    let mut entities = Vec::new();
    let mut offset = 0;
    for (index, c) in text.chars().enumerate() {
        let names = buffer
            .iter_at_offset(index as i32)
            .tags()
            .into_iter()
            .filter_map(|t| t.name())
            .filter_map(|n| n.strip_prefix(PREFIX).map(str::to_owned))
            .collect::<HashSet<_>>();
        let ended = active.keys().filter(|name| !names.contains(*name)).cloned().collect::<Vec<_>>();
        for name in ended {
            let (start, end) = active.remove(&name).unwrap();
            if let Ok(kind) = serde_json::from_str::<Value>(&name) {
                entities.push(json!({"@type":"textEntity","offset":start,"length":end-start,"type":kind}));
            }
        }
        for name in names {
            active.entry(name).or_insert((offset, offset)).1 = offset + c.len_utf16();
        }
        offset += c.len_utf16();
    }
    for (name, (start, end)) in active {
        if let Ok(kind) = serde_json::from_str::<Value>(&name) {
            entities.push(json!({"@type":"textEntity","offset":start,"length":end-start,"type":kind}));
        }
    }
    entities.sort_by_key(|e| (e["offset"].as_u64().unwrap_or(0), std::cmp::Reverse(e["length"].as_u64().unwrap_or(0))));
    json!({"@type":"formattedText","text":text,"entities":entities})
}
pub fn apply(buffer: &gtk::TextBuffer, kind: Option<Value>) {
    let Some((start, end)) = buffer.selection_bounds() else {
        crate::window::show_status("Select text to format");
        return;
    };
    if let Some(kind) = kind {
        let tag = tag(buffer, &kind);
        if start.has_tag(&tag) { buffer.remove_tag(&tag, &start, &end) } else { buffer.apply_tag(&tag, &start, &end) }
    } else {
        buffer.remove_all_tags(&start, &end)
    }
}
pub fn key(view: &gtk::TextView, key: gdk::Key, mods: gdk::ModifierType) -> bool {
    if !mods.contains(gdk::ModifierType::CONTROL_MASK) || mods.contains(gdk::ModifierType::ALT_MASK) {
        return false;
    }
    let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
    let kind = match key {
        gdk::Key::b | gdk::Key::B => Some("textEntityTypeBold"),
        gdk::Key::i | gdk::Key::I => Some("textEntityTypeItalic"),
        gdk::Key::u | gdk::Key::U => Some("textEntityTypeUnderline"),
        gdk::Key::x | gdk::Key::X if shift => Some("textEntityTypeStrikethrough"),
        gdk::Key::m | gdk::Key::M if shift => Some("textEntityTypeCode"),
        gdk::Key::p | gdk::Key::P if shift => Some("textEntityTypeSpoiler"),
        gdk::Key::backslash => {
            apply(&view.buffer(), None);
            return true;
        }
        _ => return false,
    };
    apply(&view.buffer(), kind.map(|kind| json!({"@type":kind})));
    true
}
pub fn menu(parent: &gtk::Widget, view: &gtk::TextView) {
    let (p, body) = crate::actions::popover(parent, None);
    let scope = crate::actions::Scope::current();
    for (title, kind) in [
        ("Bold · Ctrl+B", "textEntityTypeBold"),
        ("Italic · Ctrl+I", "textEntityTypeItalic"),
        ("Underline · Ctrl+U", "textEntityTypeUnderline"),
        ("Strikethrough · Ctrl+Shift+X", "textEntityTypeStrikethrough"),
        ("Monospace · Ctrl+Shift+M", "textEntityTypeCode"),
        ("Spoiler · Ctrl+Shift+P", "textEntityTypeSpoiler"),
        ("Block quote", "textEntityTypeBlockQuote"),
    ] {
        let buffer = view.buffer();
        let scope = scope.clone();
        crate::actions::menu_item(&p, &body, title, move || {
            if scope.as_ref().is_none_or(|s| s.valid()) {
                apply(&buffer, Some(json!({"@type":kind})))
            }
        });
    }
    let buffer = view.buffer();
    let scope_link = scope.clone();
    crate::actions::menu_item(&p, &body, "Create link…", move || {
        if !scope_link.as_ref().is_none_or(|s| s.valid()) {
            return;
        }
        let Some((start, end)) = buffer.selection_bounds() else {
            crate::window::show_status("Select text for the link");
            return;
        };
        let selected = buffer.text(&start, &end, false).to_string();
        let start = buffer.create_mark(None, &start, true);
        let end = buffer.create_mark(None, &end, false);
        let (dialog, body) = w::dialog("Create link", 420);
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("https://…"));
        body.append(&entry);
        let save = gtk::Button::with_label("Apply link");
        let weak = dialog.downgrade();
        let buffer = buffer.clone();
        let scope = scope_link.clone();
        let cleanup_buffer = buffer.clone();
        let cleanup_start = start.clone();
        let cleanup_end = end.clone();
        dialog.connect_close_request(move |_| {
            if !cleanup_start.is_deleted() {
                cleanup_buffer.delete_mark(&cleanup_start);
            }
            if !cleanup_end.is_deleted() {
                cleanup_buffer.delete_mark(&cleanup_end);
            }
            gtk::glib::Propagation::Proceed
        });
        save.connect_clicked(move |_| {
            let url = entry.text().to_string();
            if !(url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("tg://")
                || url.starts_with("mailto:"))
            {
                crate::window::show_status("Enter an HTTP, Telegram, or email link");
                return;
            }
            if scope.as_ref().is_none_or(|s| s.valid()) {
                if buffer.text(&buffer.iter_at_mark(&start), &buffer.iter_at_mark(&end), false).as_str() != selected {
                    crate::window::show_status("The selected text changed. Select it again before creating a link.");
                    if let Some(d) = weak.upgrade() {
                        d.close();
                    }
                    return;
                }
                buffer.apply_tag(
                    &tag(&buffer, &json!({"@type":"textEntityTypeTextUrl","url":url})),
                    &buffer.iter_at_mark(&start),
                    &buffer.iter_at_mark(&end),
                );
            }
            buffer.delete_mark(&start);
            buffer.delete_mark(&end);
            if let Some(d) = weak.upgrade() {
                d.close();
            }
        });
        body.append(&save);
        dialog.present();
    });
    let buffer = view.buffer();
    crate::actions::menu_item(&p, &body, "Clear formatting · Ctrl+\\", move || {
        if scope.as_ref().is_none_or(|s| s.valid()) {
            apply(&buffer, None)
        }
    });
    p.popup();
}
pub fn insert_mention(buffer: &gtk::TextBuffer, name: &str, user: i64) {
    let mut cursor = buffer.iter_at_offset(buffer.cursor_position());
    let start = cursor.offset();
    buffer.insert(&mut cursor, name);
    let end = cursor.offset();
    buffer.apply_tag(
        &tag(buffer, &json!({"@type":"textEntityTypeMentionName","user_id":user})),
        &buffer.iter_at_offset(start),
        &buffer.iter_at_offset(end),
    );
    buffer.insert(&mut cursor, " ");
}
#[cfg(test)]
pub fn check_roundtrip() {
    let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let value = json!({"@type":"formattedText","text":"😀 bold and link","entities":[{"@type":"textEntity","offset":3,"length":4,"type":{"@type":"textEntityTypeBold"}},{"@type":"textEntity","offset":12,"length":4,"type":{"@type":"textEntityTypeTextUrl","url":"https://example.org"}}]});
    load(&buffer, &value);
    assert_eq!(formatted(&buffer), value);
    let mut iter = buffer.start_iter();
    buffer.insert(&mut iter, "Hi ");
    let value = formatted(&buffer);
    assert_eq!(value["entities"][0]["offset"], 6);
}
