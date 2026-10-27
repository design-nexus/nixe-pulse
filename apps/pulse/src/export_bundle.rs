//! Self-contained HTML/JSONL exports, with optional copied media and cancellation.
use crate::{actions::Scope, features::check, groups::note, window};
use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    io::Write,
    path::PathBuf,
    rc::Rc,
};
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}
fn originals(v: &Value, out: &mut Vec<i32>) {
    if let Some(sizes) = v.get("sizes").and_then(Value::as_array) {
        if let Some(largest) = sizes.iter().max_by_key(|s| s["width"].as_i64().unwrap_or(0) * s["height"].as_i64().unwrap_or(0)) {
            if let Some(id) = largest["photo"]["id"].as_i64() {
                out.push(id as i32);
            }
        }
    }
    if let Some(object) = v.as_object() {
        for (key, value) in object {
            if matches!(key.as_str(), "thumbnail" | "minithumbnail" | "sizes" | "alternative_videos" | "storyboards") {
                continue;
            }
            if value["@type"] == "file" {
                if let Some(id) = value["id"].as_i64() {
                    out.push(id as i32);
                }
            } else if value.is_object() || value.is_array() {
                originals(value, out);
            }
        }
    } else if let Some(array) = v.as_array() {
        for value in array {
            originals(value, out);
        }
    }
}
pub fn open(chat: i64) {
    let Some(scope) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Export conversation", 480);
    let format = gtk::DropDown::from_strings(&["HTML", "JSON Lines"]);
    b.append(&format);
    let media = check(&b, "Include downloaded copies of media", false);
    note(
        &b,
        "Exports newest messages first. Protected content and secret chats are excluded. Large exports can take time; closing this window cancels after the current request and keeps a partial export.",
    );
    let status = note(&b, "");
    let canceled = Rc::new(Cell::new(false));
    let cancel = canceled.clone();
    d.connect_close_request(move |_| {
        cancel.set(true);
        glib::Propagation::Proceed
    });
    let button = gtk::Button::with_label("Choose export file…");
    b.append(&button);
    let weak = d.downgrade();
    button.connect_clicked(move |button| {
        if !scope.valid() {
            return;
        }
        let Some(parent) = weak.upgrade() else { return };
        let html = format.selected() == 0;
        let include = media.is_active();
        let chooser = gtk::FileDialog::builder()
            .title("Export conversation")
            .initial_name(if html { "conversation.html" } else { "conversation.jsonl" })
            .build();
        let (scope, button, status, canceled) = (scope.clone(), button.clone(), status.clone(), canceled.clone());
        chooser.save(Some(&parent), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                let result = export(scope.clone(), chat, path, html, include, canceled, status.clone()).await;
                if scope.valid() {
                    match result {
                        Ok(n) => status.set_text(&format!("Export complete: {n} messages.")),
                        Err(e) => status.set_text(&format!("Export stopped; the selected file may be partial. {e}")),
                    }
                    button.set_sensitive(true);
                }
            });
        });
    });
    d.present();
}
async fn export(
    s: Scope,
    chat: i64,
    path: PathBuf,
    html: bool,
    media: bool,
    canceled: Rc<Cell<bool>>,
    status: gtk::Label,
) -> Result<usize, String> {
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
    let folder = path.with_file_name(format!("{}-media", path.file_stem().unwrap_or_default().to_string_lossy()));
    if media {
        std::fs::create_dir(&folder)
            .map_err(|e| format!("Choose another export filename; its media folder cannot be created: {e}"))?;
    }
    if html {
        write!(file,"<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{}</title><style>body{{font:16px system-ui;max-width:850px;margin:auto;padding:24px;background:#161b22;color:#eee}}article{{margin:16px 0;padding:16px;border:1px solid #46515e;border-radius:8px}}p{{white-space:pre-wrap;overflow-wrap:anywhere}}a{{color:#79b8ff}}small{{color:#b4bdc8}}</style></head><body><h1>{}</h1><p>Newest messages first.</p>",escape(c["title"].as_str().unwrap_or("Conversation")),escape(c["title"].as_str().unwrap_or("Conversation"))).map_err(|e|e.to_string())?;
    }
    let (mut from, mut count) = (0, 0);
    let mut names = HashMap::<i64, String>::new();
    let mut copied = HashMap::<i32, String>::new();
    loop {
        if canceled.get() {
            return Err("Canceled.".into());
        }
        if s.call(
            "getChat",
            json!({
            "chat_id":chat}
            ),
        )
        .await?["has_protected_content"]
            == true
        {
            return Err("Content protection was enabled.".into());
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
        let next = rows.iter().filter_map(|m| m["id"].as_i64()).min().unwrap_or(from);
        if rows.is_empty() || next == from {
            break;
        }
        let mut text = String::new();
        for mut m in rows {
            if canceled.get() {
                return Err("Canceled.".into());
            }
            if from != 0 && m["id"].as_i64().is_some_and(|id| id >= from) {
                continue;
            }
            if m["can_be_saved"] != true || m["has_protected_content"] == true {
                continue;
            }
            let mut attachments = Vec::new();
            if media {
                let mut ids = Vec::new();
                originals(&m["content"], &mut ids);
                let mut seen = HashSet::new();
                for id in ids {
                    if !seen.insert(id) {
                        continue;
                    }
                    if let Some(path) = copied.get(&id) {
                        attachments.push(path.clone());
                        continue;
                    }
                    let mut f = s
                        .call(
                            "getFile",
                            json!({
                            "file_id":id}
                            ),
                        )
                        .await?;
                    if f["local"]["is_downloading_completed"] != true {
                        f = s
                            .call(
                                "downloadFile",
                                json!({
                                "file_id":id,"priority":1,"offset":0,"limit":0,"synchronous":true}
                                ),
                            )
                            .await?;
                    }
                    if canceled.get() {
                        return Err("Canceled.".into());
                    }
                    let source = PathBuf::from(
                        f["local"]["path"].as_str().filter(|p| !p.is_empty()).ok_or("Media download returned no file.")?,
                    );
                    let name = format!("{id}-{}", source.file_name().unwrap_or_default().to_string_lossy());
                    let target = folder.join(&name);
                    gio::spawn_blocking(move || std::fs::copy(source, target).map_err(|e| e.to_string()))
                        .await
                        .map_err(|_| "Media copy stopped.")??;
                    let relative = format!("{}/{}", folder.file_name().unwrap_or_default().to_string_lossy(), name);
                    copied.insert(id, relative.clone());
                    attachments.push(relative);
                }
            }
            if html {
                let id = m["sender_id"]["user_id"].as_i64().unwrap_or(0);
                if id != 0 && !names.contains_key(&id) {
                    let u = s
                        .call(
                            "getUser",
                            json!({
                            "user_id":id}
                            ),
                        )
                        .await
                        .unwrap_or(Value::Null);
                    names.insert(
                        id,
                        format!("{} {}", u["first_name"].as_str().unwrap_or("User"), u["last_name"].as_str().unwrap_or("")),
                    );
                }
                let who = names.get(&id).cloned().unwrap_or_else(|| {
                    window::chat(m["sender_id"]["chat_id"].as_i64().unwrap_or(0))
                        .map(|c| c.title)
                        .unwrap_or_else(|| "Telegram".into())
                });
                let date = chrono::DateTime::from_timestamp(m["date"].as_i64().unwrap_or(0), 0)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default();
                let content = m["content"]["text"]["text"]
                    .as_str()
                    .or(m["content"]["caption"]["text"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| features_summary(&m["content"]));
                text.push_str(&format!(
                    "<article id=\"message-{}\"><strong>{}</strong> <small>{}</small><p>{}</p>",
                    m["id"].as_i64().unwrap_or(0),
                    escape(&who),
                    escape(&date),
                    escape(&content)
                ));
                for path in &attachments {
                    text.push_str(&format!(
                        "<p><a href=\"{}\">{}</a></p>",
                        escape(path),
                        escape(path.rsplit('/').next().unwrap_or("Media"))
                    ));
                }
                text.push_str("</article>\n");
            } else {
                m["exported_media"] = json!(attachments);
                text.push_str(&m.to_string());
                text.push('\n');
            }
            count += 1;
        }
        file = gio::spawn_blocking(move || {
            file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            Ok::<_, String>(file)
        })
        .await
        .map_err(|_| "Export writer stopped.")??;
        from = next;
        status.set_text(&format!("Exporting… {count} messages"));
    }
    if html {
        file.write_all(b"</body></html>").map_err(|e| e.to_string())?;
    }
    Ok(count)
}
fn features_summary(v: &Value) -> String {
    crate::features::summary(v)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remainder_export_safety() {
        assert_eq!(escape("<a x=\"&\">"), "&lt;a x=&quot;&amp;&quot;&gt;");
        let mut ids = Vec::new();
        originals(
            &json!({
            "photo":{
            "sizes":[{
            "width":20,"height":20,"photo":{
            "id":1}
            }
            ,{
            "width":200,"height":200,"photo":{
            "id":2}
            }
            ]}
            ,"thumbnail":{
            "file":{
            "@type":"file","id":3}
            }
            }
            ),
            &mut ids,
        );
        assert_eq!(ids, vec![2]);
    }
}
