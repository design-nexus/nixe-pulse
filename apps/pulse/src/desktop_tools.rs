//! Download queue, cache management, proxy management and protected chat exports.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub fn open() {
    features::load("Downloads and storage", |_s, _d, b| async move {
        button(&b, "Downloads", downloads);
        button(&b, "Storage usage and cache", storage);
        button(&b, "Connection and proxies", proxies);
        button(&b, "Export current conversation…", || {
            if let Some(chat) = window::active_chat_id() {
                export_chat(chat)
            } else {
                window::show_status("Open a conversation first.")
            }
        });
        Ok(())
    });
}
pub fn downloads() {
    features::load("Downloads", |s, d, b| async move {
        let query = entry(&b, "Search downloads", "", 128);
        let list = nixe_ui::widgets::vbox(6);
        b.append(&list);
        let next = gtk::Button::with_label("Load more");
        let refresh = gtk::Button::with_label("Search / refresh");
        b.append(&refresh);
        b.append(&next);
        let offset = Rc::new(RefCell::new(String::new()));
        let weak = d.downgrade();
        let seen: Rc<RefCell<std::collections::HashSet<i64>>> = Default::default();
        let o = offset.clone();
        let l = list.clone();
        let seen_ = seen.clone();
        let next_ = next.clone();
        refresh.connect_clicked(move |_| {
            o.borrow_mut().clear();
            seen_.borrow_mut().clear();
            while let Some(c) = l.first_child() {
                l.remove(&c)
            }
            next_.set_visible(true);
            next_.emit_clicked();
        });
        next.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, weak, list, offset, seen, button, query) = (
                s.clone(),
                weak.clone(),
                list.clone(),
                offset.clone(),
                seen.clone(),
                button.clone(),
                query.text().to_string(),
            );
            glib::spawn_future_local(async move {
                let result = s
                    .call(
                        "searchFileDownloads",
                        json!({
                        "query":query,"only_active":false,"only_completed":false,"offset":offset.borrow().clone(),"limit":50}
                        ),
                    )
                    .await;
                button.set_sensitive(true);
                if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                match result {
                    Err(e) => {
                        note(&list, &e);
                    }
                    Ok(data) => {
                        for item in data["files"].as_array().into_iter().flatten() {
                            let Some(id) = item["file_id"].as_i64() else { continue };
                            if !seen.borrow_mut().insert(id) {
                                continue;
                            }
                            let file = s
                                .call(
                                    "getFile",
                                    json!({
                                    "file_id":id}
                                    ),
                                )
                                .await
                                .unwrap_or(Value::Null);
                            if !s.valid() {
                                return;
                            }
                            let path = file["local"]["path"].as_str().unwrap_or("");
                            let title = std::path::Path::new(path)
                                .file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| format!("File {id}"));
                            note(
                                &list,
                                &format!(
                                    "{title} · {:.1} / {:.1} MB{}",
                                    file["local"]["downloaded_size"].as_f64().unwrap_or(0.) / 1048576.,
                                    file["size"].as_f64().or(file["expected_size"].as_f64()).unwrap_or(0.) / 1048576.,
                                    if item["complete_date"].as_i64().unwrap_or(0) > 0 {
                                        " · completed"
                                    } else {
                                        ""
                                    }
                                ),
                            );
                            let scope = s.clone();
                            let paused = item["is_paused"] == true;
                            if item["complete_date"].as_i64().unwrap_or(0) == 0 {
                                button_action(
                                    &list,
                                    if paused { "Resume" } else { "Pause" },
                                    scope.clone(),
                                    "toggleDownloadIsPaused",
                                    json!({
                                    "file_id":id,"is_paused":!paused}
                                    ),
                                );
                            }
                            let chat = item["message"]["chat_id"].as_i64().unwrap_or(0);
                            let message = item["message"]["id"].as_i64().unwrap_or(0);
                            features::button(&list, "Show message", move || {
                                if scope.valid() {
                                    actions::run(chat, "download-jump", move |s| async move {
                                        let data = s
                                            .call(
                                                "getMessage",
                                                json!({
                                                "chat_id":chat,"message_id":message}
                                                ),
                                            )
                                            .await?;
                                        if let Some(m) = nixe_telegram_core::client::parse_message(&data) {
                                            window::jump_message(&m);
                                        }
                                        Ok(())
                                    });
                                }
                            });
                            let scope = s.clone();
                            features::button(&list, "Remove from download list", move || {
                                if !scope.valid() {
                                    return;
                                }
                                actions::run(0, "download-remove", move |s| async move {
                                    s.call(
                                        "removeFileFromDownloads",
                                        json!({
                                        "file_id":id,"delete_from_cache":false}
                                        ),
                                    )
                                    .await?;
                                    Ok(())
                                });
                            });
                        }
                        let cursor = data["next_offset"].as_str().unwrap_or("").to_string();
                        button.set_visible(!cursor.is_empty() && cursor != *offset.borrow());
                        *offset.borrow_mut() = cursor;
                    }
                }
            });
        });
        next.emit_clicked();
        Ok(())
    });
}
fn button_action(b: &gtk::Box, title: &str, scope: Scope, method: &'static str, args: Value) {
    button(b, title, move || {
        if !scope.valid() {
            return;
        }
        let args = args.clone();
        actions::run(0, method, move |s| async move {
            s.call(method, args).await?;
            window::show_status("Saved. Refresh this page to see the latest state.");
            Ok(())
        });
    });
}
pub fn storage() {
    features::load("Storage and cache", |s, d, b| async move {
        let stats = s
            .call(
                "getStorageStatistics",
                json!({
                "chat_limit":100}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(
            &b,
            &format!(
                "Telegram cache · {:.1} MB · {} files",
                stats["size"].as_f64().unwrap_or(0.) / 1048576.,
                stats["count"]
            ),
        );
        note(
            &b,
            "Cache removal deletes downloaded copies. Messages and server media remain in Telegram; secret-chat media may not be downloadable again.",
        );
        for chat in stats["by_chat"].as_array().into_iter().flatten() {
            let id = chat["chat_id"].as_i64().unwrap_or(0);
            let title = window::chat(id)
                .map(|c| c.title)
                .unwrap_or_else(|| if id == 0 { "Other files".into() } else { format!("Chat {id}") });
            note(
                &b,
                &format!("{title} · {:.1} MB", chat["size"].as_f64().unwrap_or(0.) / 1048576.),
            );
            let scope = s.clone();
            button(&b, "Clear this cache…", move || {
                if scope.valid() {
                    clear_cache(vec![id]);
                }
            });
        }
        let scope = s.clone();
        button(&b, "Clear all Telegram cache…", move || {
            if scope.valid() {
                clear_cache(vec![]);
            }
        });
        Ok(())
    });
}
fn clear_cache(ids: Vec<i64>) {
    actions::operation(
        "Clear downloaded cache?",
        "Downloaded copies will be removed. Secret-chat media may be permanently unavailable after removal. Telegram messages are retained.",
        vec![],
        "Clear cache",
        move |s, _| {
            let ids = ids.clone();
            async move {
                s.call("optimizeStorage",json!({
"size":0,"ttl":0,"count":0,"immunity_delay":0,"file_types":[],"chat_ids":ids,"exclude_chat_ids":[],"return_deleted_file_statistics":true,"chat_limit":0}
)).await?;
                window::show_status("Cache cleared.");
                Ok(())
            }
        },
    );
}
pub fn proxies() {
    features::load("Proxies", |s, d, b| async move {
        let data = s.call("getProxies", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        for added in data["proxies"].as_array().into_iter().flatten() {
            let id = added["id"].as_i64().unwrap_or(0);
            let p = &added["proxy"];
            note(
                &b,
                &format!(
                    "{}:{}{}",
                    p["server"].as_str().unwrap_or(""),
                    p["port"],
                    if added["is_enabled"] == true { " · active" } else { "" }
                ),
            );
            button_action(
                &b,
                "Enable",
                s.clone(),
                "enableProxy",
                json!({
                "proxy_id":id}
                ),
            );
            let scope = s.clone();
            let proxy = p.clone();
            button(&b, "Test latency", move || {
                if !scope.valid() {
                    return;
                }
                let proxy = proxy.clone();
                actions::run(0, "proxy-ping", move |s| async move {
                    let result = s
                        .call(
                            "pingProxy",
                            json!({
                            "proxy":proxy}
                            ),
                        )
                        .await?;
                    window::show_status(&format!(
                        "Proxy latency: {:.0} ms",
                        result["seconds"].as_f64().unwrap_or(0.) * 1000.
                    ));
                    Ok(())
                });
            });
            let scope = s.clone();
            let added = added.clone();
            button(&b, "Edit…", move || {
                if scope.valid() {
                    proxy_edit(Some(added.clone()));
                }
            });
            let scope = s.clone();
            button(&b, "Delete proxy…", move || {
                if !scope.valid() {
                    return;
                }
                actions::operation(
                    "Delete proxy?",
                    "The saved proxy will be removed from this account.",
                    vec![],
                    "Delete proxy",
                    move |s, _| async move {
                        s.call(
                            "removeProxy",
                            json!({
                            "proxy_id":id}
                            ),
                        )
                        .await?;
                        Ok(())
                    },
                );
            });
        }
        button_action(&b, "Use direct connection", s.clone(), "disableProxy", json!({}));
        button(&b, "Add proxy…", || proxy_edit(None));
        Ok(())
    });
}
fn proxy_edit(original: Option<Value>) {
    let Some(scope) = Scope::current() else { return };
    let (d, b) = nixe_ui::widgets::dialog("Proxy", 440);
    let fields = nixe_ui::widgets::vbox(8);
    b.append(&fields);
    let p = original.as_ref().map(|p| p["proxy"].clone()).unwrap_or(Value::Null);
    let server = entry(&fields, "Hostname", p["server"].as_str().unwrap_or(""), 255);
    let port = entry(&fields, "Port", &p["port"].as_i64().unwrap_or(1080).to_string(), 5);
    let kind = gtk::DropDown::from_strings(&["SOCKS5", "MTProto", "HTTP"]);
    kind.set_selected(match p["type"]["@type"].as_str() {
        Some("proxyTypeMtproto") => 1,
        Some("proxyTypeHttp") => 2,
        _ => 0,
    });
    fields.append(&kind);
    let user = entry(&fields, "Username", p["type"]["username"].as_str().unwrap_or(""), 255);
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_text(p["type"]["secret"].as_str().or(p["type"]["password"].as_str()).unwrap_or(""));
    note(&fields, "Password or MTProto secret");
    fields.append(&password);
    let enable = check(
        &fields,
        "Enable this proxy",
        original.as_ref().is_none_or(|v| v["is_enabled"] == true),
    );
    submit(&d, &b, &fields, scope, "Save proxy", move |s| {
        let (server, port, user, password, kind, enable, original) = (
            server.text().trim().to_string(),
            port.text().parse::<u16>(),
            user.text().to_string(),
            password.text().to_string(),
            kind.selected(),
            enable.is_active(),
            original.clone(),
        );
        async move {
            let port = port.map_err(|_| "Enter a port between 1 and 65535.")?;
            if port == 0 || server.is_empty() {
                return Err("Enter a hostname and valid port.".into());
            }
            let typ = match kind {
                1 => json!({
                "@type":"proxyTypeMtproto","secret":password}
                ),
                2 => json!({
                "@type":"proxyTypeHttp","username":user,"password":password,"http_only":false}
                ),
                _ => json!({
                "@type":"proxyTypeSocks5","username":user,"password":password}
                ),
            };
            let proxy = json!({
            "@type":"proxy","server":server,"port":port,"type":typ}
            );
            let method = if original.is_some() { "editProxy" } else { "addProxy" };
            s.call(method,json!({
"proxy_id":original.as_ref().map(|v|v["id"].clone()).unwrap_or(Value::Null),"proxy":proxy,"enable":enable,"comment":"Pulse"}
)).await?;
            Ok(())
        }
    });
    d.present();
}
pub fn export_chat(chat: i64) {
    let Some(scope) = Scope::current() else { return };
    let (d, b) = nixe_ui::widgets::dialog("Export conversation", 440);
    note(
        &b,
        "Export message text and metadata as JSON Lines. Protected messages are omitted. Media files are not copied.",
    );
    let run = gtk::Button::with_label("Choose export file…");
    b.append(&run);
    let canceled = Rc::new(Cell::new(false));
    let cancel = canceled.clone();
    d.connect_close_request(move |_| {
        cancel.set(true);
        glib::Propagation::Proceed
    });
    let weak = d.downgrade();
    run.connect_clicked(move |button| {
        if !scope.valid() {
            return;
        }
        let Some(parent) = weak.upgrade() else { return };
        let chooser = gtk::FileDialog::builder()
            .title("Export conversation")
            .initial_name("pulse-chat.jsonl")
            .build();
        let (s, weak, button, canceled) = (scope.clone(), weak.clone(), button.clone(), canceled.clone());
        chooser.save(Some(&parent), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                let result = export(&s, chat, path, canceled).await;
                button.set_sensitive(true);
                if !s.valid() {
                    return;
                }
                match result {
                    Ok(n) => {
                        window::show_status(&format!("Exported {n} messages."));
                        if let Some(d) = weak.upgrade() {
                            d.close();
                        }
                    }
                    Err(e) => window::show_status(&e),
                }
            });
        });
    });
    d.present();
}
async fn export(s: &Scope, chat: i64, path: std::path::PathBuf, canceled: Rc<Cell<bool>>) -> Result<usize, String> {
    let c = s
        .call(
            "getChat",
            json!({
            "chat_id":chat}
            ),
        )
        .await?;
    if c["has_protected_content"] == true || c["type"]["@type"] == "chatTypeSecret" {
        return Err("This conversation cannot be exported.".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("Choose a new filename: {e}"))?;
    let mut from = 0;
    let mut count = 0;
    loop {
        if canceled.get() {
            return Err("Export canceled. The selected file contains the messages exported so far.".into());
        }
        if s.call("getChat", json!({"chat_id":chat})).await?["has_protected_content"] == true {
            return Err("Content protection was enabled. Export stopped; the selected file is partial.".into());
        }
        let data = s
            .call(
                "getChatHistory",
                json!({
                "chat_id":chat,"from_message_id":from,"offset":0,"limit":100,"only_local":false}
                ),
            )
            .await?;
        let rows = data["messages"].as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            break;
        }
        let next = rows.iter().filter_map(|m| m["id"].as_i64()).min().unwrap_or(from);
        if next == from {
            break;
        }
        let mut batch = String::new();
        for m in rows {
            if from != 0 && m["id"].as_i64().is_some_and(|id| id >= from) {
                continue;
            }
            if m["has_protected_content"] == true || m["can_be_saved"] != true {
                continue;
            }
            batch.push_str(&m.to_string());
            batch.push('\n');
            count += 1;
        }
        use std::io::Write;
        file = gio::spawn_blocking(move || file.write_all(batch.as_bytes()).map(|_| file).map_err(|e| e.to_string()))
            .await
            .map_err(|_| "Export worker stopped.")??;
        from = next;
        window::show_status(&format!("Exporting… {count} messages"));
    }
    Ok(count)
}
