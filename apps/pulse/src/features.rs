//! Shared account-scoped feature windows, without blocking GTK on Telegram requests.
use crate::{actions::Scope, groups::note};
use gtk::{glib, prelude::*};
use nixe_ui::widgets as w;
use serde_json::Value;
use std::future::Future;
pub fn load<F, Fut>(title: &str, work: F)
where
    F: FnOnce(Scope, gtk::Window, gtk::Box) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let Some(scope) = Scope::current() else {
        crate::window::show_status("Connect to Telegram first.");
        return;
    };
    let (dialog, body) = w::dialog(title, 520);
    let list = crate::groups::scroll(&body);
    let status = note(&list, "Loading…");
    dialog.present();
    glib::spawn_future_local(async move {
        let result = work(scope.clone(), dialog.clone(), list).await;
        match result {
            Ok(()) => status.set_visible(false),
            Err(e) => {
                if scope.valid() && dialog.is_visible() {
                    status.set_text(&e);
                    status.set_selectable(true);
                    status.add_css_class("error-text");
                }
            }
        }
    });
}
pub fn button(body: &gtk::Box, title: &str, action: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(title);
    if let Some(label) = b.child().and_downcast::<gtk::Label>() {
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(44);
    }
    b.connect_clicked(move |_| action());
    body.append(&b);
    b
}
pub fn check(body: &gtk::Box, title: &str, value: bool) -> gtk::CheckButton {
    let c = gtk::CheckButton::with_label(title);
    if let Some(label) = c.child().and_downcast::<gtk::Label>() {
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_max_width_chars(44);
        label.set_xalign(0.0);
    }
    c.set_active(value);
    body.append(&c);
    c
}
pub fn alive(scope: &Scope, d: &gtk::Window) -> bool {
    scope.valid() && d.is_visible()
}
pub fn ids(input: &str) -> Result<Vec<i64>, String> {
    let mut ids = Vec::new();
    for part in input.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()) {
        let id = part.parse::<i64>().map_err(|_| "Enter numeric Telegram IDs separated by commas.")?;
        if id == 0 {
            return Err("An ID cannot be zero.".into());
        }
        if !ids.contains(&id) {
            ids.push(id)
        }
    }
    Ok(ids)
}
pub fn id_text(v: &Value) -> String {
    v.as_array().into_iter().flatten().filter_map(Value::as_i64).map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
}
pub fn pretty(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_uppercase() && !out.is_empty() {
            out.push(' ')
        }
        out.push(c)
    }
    out
}
pub fn summary(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Array(a) => a.iter().map(summary).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| {
                !k.starts_with('@')
                    && !matches!(k.as_str(), "json_data" | "photo" | "thumbnail" | "animation" | "sticker" | "minithumbnail")
            })
            .map(|(k, v)| format!("{}: {}", k.replace('_', " "), summary(v)))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => v.to_string(),
    }
}

pub fn hub() {
    load("Pulse tools", |_s, _d, b| async move {
        button(&b, "Profile history, passkeys and sessions", crate::account_extras::open);
        button(&b, "Accessibility and shortcuts", crate::desktop_extras::open);
        button(&b, "Gifts and collectibles", crate::gifts::open);
        button(&b, "Account, contacts and privacy", crate::account_tools::open);
        button(&b, "Downloads, storage and proxies", crate::desktop_tools::open);
        button(&b, "Stories", crate::stories::browse);
        button(&b, "Live locations", crate::locations::manage);
        button(&b, "Premium, Stars, gifts and subscriptions", crate::commerce::open);
        button(&b, "Telegram Business and quick replies", crate::business::open);
        Ok(())
    });
}

pub fn pages(
    title: &str,
    method: &'static str,
    args: Value,
    key: &'static str,
    row: impl Fn(&gtk::Box, &Scope, &Value) + 'static,
) {
    load(title, move |s, d, b| async move {
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let rows = nixe_ui::widgets::vbox(6);
        b.append(&rows);
        let cursor = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let row = std::rc::Rc::new(row);
        let weak = d.downgrade();
        more.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, args, rows, cursor, row, weak, button) =
                (s.clone(), args.clone(), rows.clone(), cursor.clone(), row.clone(), weak.clone(), button.clone());
            glib::spawn_future_local(async move {
                let previous = cursor.borrow().clone();
                let mut args = args;
                args["offset"] = serde_json::json!(previous);
                let result = s.call(method, args).await;
                button.set_sensitive(true);
                if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                match result {
                    Err(e) => {
                        note(&rows, &e);
                    }
                    Ok(data) => {
                        for value in data[key].as_array().into_iter().flatten() {
                            row(&rows, &s, value);
                        }
                        let next = data["next_offset"].as_str().unwrap_or("").to_string();
                        button.set_visible(!next.is_empty() && next != previous);
                        *cursor.borrow_mut() = next;
                    }
                }
            });
        });
        more.emit_clicked();
        Ok(())
    });
}
