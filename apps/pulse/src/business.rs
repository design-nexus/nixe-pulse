//! Telegram Business profile, automation, chat links and quick replies.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::prelude::*;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap};
thread_local! {
static SHORTCUTS:RefCell<BTreeMap<i64,Value>>=const{
RefCell::new(BTreeMap::new())}
;
}
thread_local! {static MESSAGES:RefCell<BTreeMap<i64,Vec<Value>>>=const{RefCell::new(BTreeMap::new())};static ORDER:RefCell<Vec<i64>>=const{RefCell::new(Vec::new())};}
pub fn reset() {
    MESSAGES.with(|v| v.borrow_mut().clear());
    ORDER.with(|v| v.borrow_mut().clear());
    SHORTCUTS.with(|v| v.borrow_mut().clear());
}
pub fn update(v: &Value) {
    match v["@type"].as_str() {
        Some("updateQuickReplyShortcutMessages") => {
            if let (Some(id), Some(messages)) = (v["shortcut_id"].as_i64(), v["messages"].as_array()) {
                MESSAGES.with(|v| v.borrow_mut().insert(id, messages.clone()));
            }
        }
        Some("updateQuickReplyShortcuts") => {
            ORDER.with(|o| {
                *o.borrow_mut() = v["shortcut_ids"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect()
            });
        }
        Some("updateQuickReplyShortcut") => {
            if let Some(id) = v["shortcut"]["id"].as_i64() {
                SHORTCUTS.with(|s| s.borrow_mut().insert(id, v["shortcut"].clone()));
            }
        }
        Some("updateQuickReplyShortcutDeleted") => {
            if let Some(id) = v["shortcut_id"].as_i64() {
                SHORTCUTS.with(|s| s.borrow_mut().remove(&id));
            }
        }
        _ => {}
    }
}
async fn info(s: &Scope) -> Result<Value, String> {
    let me = s.call("getMe", json!({})).await?;
    let info = s
        .call(
            "getUserFullInfo",
            json!({
            "user_id":me["id"]}
            ),
        )
        .await?;
    Ok(info["business_info"].clone())
}
pub fn open() {
    features::load("Telegram Business", |_s, _d, b| async move {
        note(&b, "Telegram determines which Business capabilities your account can use.");
        button(&b, "Business address", location);
        button(&b, "Opening hours", crate::business_extras::hours);
        button(&b, "Greeting message", || automation(false));
        button(&b, "Away message", || automation(true));
        button(&b, "Start page", start_page);
        button(&b, "Quick replies", quick_replies);
        button(&b, "Business chat links", links);
        button(&b, "Connected business bot", crate::business_extras::connected_bot);
        Ok(())
    });
}
fn location() {
    features::load("Business location", |s, d, b| async move {
        let original = info(&s).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let address = entry(&fields, "Address", original["location"]["address"].as_str().unwrap_or(""), 512);
        let lat = entry(
            &fields,
            "Latitude (optional)",
            &original["location"]["location"]["latitude"].as_f64().map(|n| n.to_string()).unwrap_or_default(),
            32,
        );
        let lon = entry(
            &fields,
            "Longitude (optional)",
            &original["location"]["location"]["longitude"].as_f64().map(|n| n.to_string()).unwrap_or_default(),
            32,
        );
        submit(&d, &b, &fields, s, "Save location", move |s| {
            let (address, lat, lon) = (address.text().trim().to_string(), lat.text().to_string(), lon.text().to_string());
            async move {
                let location = if lat.is_empty() && lon.is_empty() {
                    Value::Null
                } else {
                    let (a, b) = (
                        lat.parse::<f64>().map_err(|_| "Invalid latitude.")?,
                        lon.parse::<f64>().map_err(|_| "Invalid longitude.")?,
                    );
                    if !a.is_finite() || !b.is_finite() || !(-90.0..=90.).contains(&a) || !(-180.0..=180.).contains(&b) {
                        return Err("Coordinates are outside the valid range.".into());
                    }
                    json!({
                    "@type":"location","latitude":a,"longitude":b,"horizontal_accuracy":0.}
                    )
                };
                s.call(
                    "setBusinessLocation",
                    json!({
                    "location":if address.is_empty(){
                    Value::Null}
                    else{
                    json!({
                    "@type":"businessLocation","address":address,"location":location}
                    )}
                    }
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
#[cfg(test)]
fn minute(input: &str) -> Result<i32, String> {
    let Some((h, m)) = input.trim().split_once(':') else {
        return Err("Use 24-hour HH:MM times.".into());
    };
    let (h, m) = (h.parse::<i32>().map_err(|_| "Invalid hour.")?, m.parse::<i32>().map_err(|_| "Invalid minute.")?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err("Use times between 00:00 and 23:59.".into());
    }
    Ok(h * 60 + m)
}
fn automation(away: bool) {
    features::load(if away { "Away message" } else { "Greeting message" }, move |s, d, b| async move {
        let original = info(&s).await?;
        let key = if away { "away_message_settings" } else { "greeting_message_settings" };
        let original = original[key].clone();
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let enabled = check(&fields, "Enable automatic message", !original.is_null());
        let shortcut = entry(
            &fields,
            "Quick-reply shortcut ID",
            &original["shortcut_id"].as_i64().map(|v| v.to_string()).unwrap_or_default(),
            12,
        );
        button(&fields, "Manage quick replies", quick_replies);
        let contacts = check(&fields, "Send to contacts", original["recipients"]["select_contacts"] != false);
        let noncontacts = check(&fields, "Send to non-contacts", original["recipients"]["select_non_contacts"] != false);
        let except = entry(&fields, "Excluded chat IDs", &features::id_text(&original["recipients"]["excluded_chat_ids"]), 4096);
        let days = gtk::SpinButton::with_range(7., 28., 7.);
        days.set_value(original["inactivity_days"].as_f64().unwrap_or(7.));
        if !away {
            note(&fields, "Send greeting after inactive days");
            fields.append(&days);
        }
        let offline = check(&fields, "Away: send only while offline", original["offline_only"] == true);
        let schedule = gtk::DropDown::from_strings(&["Always", "Outside opening hours", "Custom period"]);
        schedule.set_selected(if original["schedule"]["@type"] == "businessAwayMessageScheduleCustom" {
            2
        } else if original["schedule"]["@type"] == "businessAwayMessageScheduleOutsideOfOpeningHours" {
            1
        } else {
            0
        });
        if away {
            fields.append(&schedule);
        }
        let start = entry(
            &fields,
            "Custom start (Unix timestamp)",
            &original["schedule"]["start_date"].as_i64().unwrap_or(0).to_string(),
            12,
        );
        let end = entry(
            &fields,
            "Custom end (Unix timestamp)",
            &original["schedule"]["end_date"].as_i64().unwrap_or(0).to_string(),
            12,
        );
        submit(&d, &b, &fields, s, "Save automation", move |s| {
            let (enabled, shortcut, contacts, noncontacts, except, days, offline, schedule, original) = (
                enabled.is_active(),
                shortcut.text().parse::<i32>(),
                contacts.is_active(),
                noncontacts.is_active(),
                features::ids(&except.text()),
                days.value_as_int(),
                offline.is_active(),
                schedule.selected(),
                original.clone(),
            );
            let (start, end) = (start.text().parse::<i64>(), end.text().parse::<i64>());
            async move {
                if info(&s).await?[key] != original {
                    return Err("Automation changed. Reopen the editor.".into());
                }
                let schedule = if schedule == 2 {
                    let (start, end) = (start.map_err(|_| "Invalid start time.")?, end.map_err(|_| "Invalid end time.")?);
                    if start <= 0 || end <= start {
                        return Err("The custom period must have an end after its start.".into());
                    }
                    json!({
                    "@type":"businessAwayMessageScheduleCustom","start_date":start,"end_date":end}
                    )
                } else {
                    json!({
                    "@type":if schedule==1{
                    "businessAwayMessageScheduleOutsideOfOpeningHours"}
                    else{
                    "businessAwayMessageScheduleAlways"}
                    }
                    )
                };
                let mut recipients = if original["recipients"].is_object() {
                    original["recipients"].clone()
                } else {
                    json!({
                    "@type":"businessRecipients","chat_ids":[],"excluded_chat_ids":[],"select_existing_chats":true,"select_new_chats":true,"select_contacts":true,"select_non_contacts":true,"exclude_selected":true}
                    )
                };
                recipients["select_contacts"] = json!(contacts);
                recipients["select_non_contacts"] = json!(noncontacts);
                recipients["excluded_chat_ids"] = json!(except?);
                let settings = if !enabled {
                    Value::Null
                } else {
                    let id = shortcut.map_err(|_| "Choose a quick-reply shortcut ID.")?;
                    if away {
                        json!({
                        "@type":"businessAwayMessageSettings","shortcut_id":id,"recipients":recipients,"schedule":schedule
                        ,"offline_only":offline}
                        )
                    } else {
                        json!({
                        "@type":"businessGreetingMessageSettings","shortcut_id":id,"recipients":recipients,"inactivity_days":days}
                        )
                    }
                };
                if away {
                    s.call(
                        "setBusinessAwayMessageSettings",
                        json!({
                        "away_message_settings":settings}
                        ),
                    )
                    .await?;
                } else {
                    s.call(
                        "setBusinessGreetingMessageSettings",
                        json!({
                        "greeting_message_settings":settings}
                        ),
                    )
                    .await?;
                }
                Ok(())
            }
        });
        Ok(())
    });
}
fn start_page() {
    features::load("Business start page", |s, d, b| async move {
        let current = info(&s).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let title = entry(&fields, "Title", current["start_page"]["title"].as_str().unwrap_or(""), 64);
        let message = entry(&fields, "Description", current["start_page"]["message"].as_str().unwrap_or(""), 512);
        let original = current["start_page"].clone();
        let remove = check(&fields, "Remove start-page sticker", false);
        let replacement = crate::poll_media::sticker_picker(&fields, &d, &s, "New start-page sticker");
        submit(&d, &b, &fields, s, "Save start page", move |s| {
            let (title, message, original, remove) =
                (title.text().to_string(), message.text().to_string(), original.clone(), remove.is_active());
            let replacement = replacement.borrow().as_ref().map(|a| a.input()).transpose();
            async move {
                if info(&s).await?["start_page"] != original {
                    return Err("The start page changed. Reopen the editor.".into());
                }
                let sticker = if let Some(replacement) = replacement? {
                    if remove {
                        return Err("Choose either a replacement sticker or removal.".into());
                    }
                    replacement["sticker"]["sticker"].clone()
                } else if remove {
                    Value::Null
                } else {
                    original["sticker"]["sticker"]["id"]
                        .as_i64()
                        .map(|id| {
                            json!({
                            "@type":"inputFileId","id":id}
                            )
                        })
                        .unwrap_or(Value::Null)
                };
                s.call(
                    "setBusinessStartPage",
                    json!({
                    "start_page":if title.is_empty()&&message.is_empty(){
                    Value::Null}
                    else{
                    json!({
                    "@type":"inputBusinessStartPage","title":title,"message":message,"sticker":sticker}
                    )}
                    }
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
pub fn quick_replies() {
    features::load("Quick replies", |s, _d, b| async move {
        s.call("loadQuickReplyShortcuts", json!({})).await?;
        let list = nyx_ui::widgets::vbox(6);
        b.append(&list);
        let refresh = gtk::Button::with_label("Refresh shortcuts");
        b.append(&refresh);
        refresh.connect_clicked(move |_| {
            if !s.valid() {
                return;
            }
            while let Some(c) = list.first_child() {
                list.remove(&c)
            }
            let values = SHORTCUTS.with(|v| v.borrow().values().cloned().collect::<Vec<_>>());
            for shortcut in values {
                let id = shortcut["id"].as_i64().unwrap_or(0);
                note(
                    &list,
                    &format!("/{} · ID {id} · {} messages", shortcut["name"].as_str().unwrap_or(""), shortcut["message_count"]),
                );
                let name = shortcut["name"].as_str().unwrap_or("").to_string();
                button(&list, "Edit shortcut and messages…", move || edit_shortcut(id, name.clone()));
                let scope = s.clone();
                button(&list, "Send to current conversation…", move || {
                    if !scope.valid() {
                        return;
                    }
                    let Some(chat) = window::active_chat_id() else {
                        window::show_status("Open a conversation first.");
                        return;
                    };
                    actions::operation(
                        "Send quick reply?",
                        "The saved shortcut messages will be sent to the currently selected conversation.",
                        vec![],
                        "Send",
                        move |s, _| async move {
                            crate::groups::commit(
                                &s,
                                "sendQuickReplyShortcutMessages",
                                json!({
                                "chat_id":chat,"shortcut_id":id,"sending_id":0}
                                ),
                            )
                            .await?;
                            Ok(())
                        },
                    );
                });
                let scope = s.clone();
                button(&list, "Delete shortcut…", move || {
                    if !scope.valid() {
                        return;
                    }
                    actions::operation(
                        "Delete quick reply?",
                        "All messages in this shortcut will be removed.",
                        vec![],
                        "Delete",
                        move |s, _| async move {
                            s.call(
                                "deleteQuickReplyShortcut",
                                json!({
                                "shortcut_id":id}
                                ),
                            )
                            .await?;
                            Ok(())
                        },
                    );
                });
            }
        });
        refresh.emit_clicked();
        button(&b, "Create or append to shortcut…", new_shortcut);
        Ok(())
    });
}
fn new_shortcut() {
    new_shortcut_named("")
}
fn new_shortcut_named(initial_name: &str) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Quick reply message", 440);
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let name = entry(&fields, "Shortcut name", initial_name, 32);
    let text = entry(&fields, "Message or caption (Markdown)", "", 4096);
    let media = crate::poll_media::picker(&fields, &d, &s, "Optional attachment", false);
    submit(&d, &b, &fields, s, "Save message", move |s| {
        let (name, text) = (name.text().trim().trim_start_matches('/').to_string(), text.text().to_string());
        let media = media.borrow().as_ref().map(|a| a.input()).transpose();
        async move {
            if name.is_empty() || (text.trim().is_empty() && media.as_ref().is_ok_and(|v| v.is_none())) {
                return Err("Enter a name and message.".into());
            }
            let formatted = s
                .call("parseTextEntities", json!({"text":text,"parse_mode":{"@type":"textParseModeMarkdown","version":2}}))
                .await?;
            let content = if let Some(media) = media? {
                crate::message_editing::content(media, formatted)?
            } else {
                json!({"@type":"inputMessageText","text":formatted,"clear_draft":false})
            };
            crate::groups::commit(
                &s,
                "addQuickReplyShortcutMessage",
                json!({"shortcut_name":name,"reply_to_message_id":0,"input_message_content":content}),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
fn links() {
    features::load("Business chat links", |s, d, b| async move {
        let data = s.call("getBusinessChatLinks", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        for link in data["links"].as_array().into_iter().flatten() {
            note(&b, &format!("{} · {} views", link["title"].as_str().unwrap_or("Link"), link["view_count"]));
            let url = link["link"].as_str().unwrap_or("").to_string();
            let editable = link.clone();
            button(&b, "Edit link…", move || edit_link(editable.clone()));
            let copy = url.clone();
            button(&b, "Copy link", move || {
                if let Some(d) = gtk::gdk::Display::default() {
                    d.clipboard().set_text(&copy);
                }
            });
            let scope = s.clone();
            button(&b, "Delete link…", move || {
                if !scope.valid() {
                    return;
                }
                let url = url.clone();
                actions::operation(
                    "Delete business link?",
                    "This link will stop opening your chat.",
                    vec![],
                    "Delete",
                    move |s, _| {
                        let url = url.clone();
                        async move {
                            s.call(
                                "deleteBusinessChatLink",
                                json!({
                                "link":url}
                                ),
                            )
                            .await?;
                            Ok(())
                        }
                    },
                );
            });
        }
        button(&b, "Create link…", || {
            let Some(s) = Scope::current() else { return };
            let (d, b) = nyx_ui::widgets::dialog("Business chat link", 440);
            let fields = nyx_ui::widgets::vbox(8);
            b.append(&fields);
            let title = entry(&fields, "Title", "", 64);
            let text = entry(&fields, "Prefilled message", "", 4096);
            submit(&d, &b, &fields, s, "Create link", move |s| {
                let (title, text) = (title.text().to_string(), text.text().to_string());
                async move {
                    let result = crate::groups::commit(
                        &s,
                        "createBusinessChatLink",
                        json!({
                        "link_info":{
                        "@type":"inputBusinessChatLink","title":title,"text":{
                        "@type":"formattedText","text":text,"entities":[]}
                        }
                        }
                        ),
                    )
                    .await?;
                    if let (Some(d), Some(link)) = (gtk::gdk::Display::default(), result["link"].as_str()) {
                        d.clipboard().set_text(link);
                    }
                    Ok(())
                }
            });
            d.present();
        });
        Ok(())
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combined_business_times() {
        assert_eq!(minute("09:30").unwrap(), 570);
        assert!(minute("24:00").is_err());
        assert!(minute("12:70").is_err());
    }
}

fn edit_shortcut(id: i64, name: String) {
    features::load("Edit quick reply", move |s, d, b| async move {
        s.call("loadQuickReplyShortcutMessages", json!({"shortcut_id":id})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let rename = entry(&fields, "Shortcut name", &name, 32);
        let scope = s.clone();
        button(&fields, "Rename", move || {
            let name = rename.text().trim().trim_start_matches('/').to_string();
            if name.is_empty() {
                return;
            }
            let scope = scope.clone();
            actions::run(0, "quick-reply-rename", move |_| async move {
                scope.call("setQuickReplyShortcutName", json!({"shortcut_id":id,"name":name})).await?;
                Ok(())
            });
        });
        let rows = crate::groups::scroll(&b);
        let refresh = gtk::Button::with_label("Refresh messages");
        b.append(&refresh);
        let scope = s.clone();
        refresh.connect_clicked(move |_| {
            if !scope.valid() {
                return;
            }
            while let Some(c) = rows.first_child() {
                rows.remove(&c)
            }
            let messages = MESSAGES.with(|m| m.borrow().get(&id).cloned().unwrap_or_default());
            for message in messages {
                let mid = message["id"].as_i64().unwrap_or(0);
                note(
                    &rows,
                    message["content"]["text"]["text"]
                        .as_str()
                        .or(message["content"]["caption"]["text"].as_str())
                        .unwrap_or("Media message"),
                );
                if message["can_be_edited"] == true {
                    let m = message.clone();
                    button(&rows, "Edit message…", move || edit_quick_message(id, m.clone()));
                }
                button(&rows, "Delete message…", move || {
                    actions::operation(
                        "Delete quick reply message?",
                        "This removes the saved message from the shortcut.",
                        vec![],
                        "Delete",
                        move |s, _| async move {
                            s.call("deleteQuickReplyShortcutMessages", json!({"shortcut_id":id,"message_ids":[mid]})).await?;
                            Ok(())
                        },
                    )
                });
            }
        });
        refresh.emit_clicked();
        let (scope, name) = (s.clone(), name.clone());
        button(&b, "Append text or media…", move || {
            if scope.valid() {
                new_shortcut_named(&name);
            }
        });
        let order = ORDER.with(|o| o.borrow().clone());
        if let Some(index) = order.iter().position(|n| *n == id) {
            for (title, delta) in [("Move shortcut up", -1isize), ("Move shortcut down", 1)] {
                let target = index as isize + delta;
                if target < 0 || target as usize >= order.len() {
                    continue;
                }
                let original = order.clone();
                let mut changed = order.clone();
                changed.swap(index, target as usize);
                button(&b, title, move || {
                    let (original, changed) = (original.clone(), changed.clone());
                    actions::run(0, "quick-reply-order", move |s| async move {
                        if ORDER.with(|o| *o.borrow() != original) {
                            return Err("Shortcut order changed. Reopen the editor.".into());
                        }
                        s.call("reorderQuickReplyShortcuts", json!({"shortcut_ids":changed})).await?;
                        Ok(())
                    });
                });
            }
        }
        Ok(())
    });
}

fn edit_quick_message(shortcut: i64, message: Value) {
    features::load("Edit quick reply message", move |s, d, b| async move {
        let id = message["id"].as_i64().unwrap_or(0);
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let original = if message["content"]["@type"] == "messageText" {
            message["content"]["text"].clone()
        } else {
            message["content"]["caption"].clone()
        };
        let text = entry(&fields, "Text or caption", original["text"].as_str().unwrap_or(""), 4096);
        let media = crate::poll_media::picker(&fields, &d, &s, "Replacement media (required for media messages)", false);
        submit(&d, &b, &fields, s, "Save message", move |s| {
            let (text, original, message, media) = (
                text.text().to_string(),
                original.clone(),
                message.clone(),
                media.borrow().as_ref().map(|a| a.input()).transpose(),
            );
            async move {
                if MESSAGES.with(|m| {
                    m.borrow()
                        .get(&shortcut)
                        .and_then(|messages| messages.iter().find(|m| m["id"].as_i64() == Some(id)))
                        .is_none_or(|m| m["content"] != message["content"])
                }) {
                    return Err("The shortcut message changed. Reopen it.".into());
                }
                let formatted = if text == original["text"].as_str().unwrap_or("") {
                    original
                } else {
                    json!({"@type":"formattedText","text":text,"entities":[]})
                };
                let content = if let Some(media) = media? {
                    crate::message_editing::content(media, formatted)?
                } else if message["content"]["@type"] == "messageText" {
                    json!({"@type":"inputMessageText","text":formatted,"clear_draft":false})
                } else {
                    return Err("Choose replacement media to edit this media message.".into());
                };
                crate::groups::commit(
                    &s,
                    "editQuickReplyMessage",
                    json!({"shortcut_id":shortcut,"message_id":id,"input_message_content":content}),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}

fn edit_link(original: Value) {
    features::load("Edit Business chat link", move |s, d, b| async move {
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let title = entry(&fields, "Title", original["title"].as_str().unwrap_or(""), 64);
        let text = entry(&fields, "Prefilled message", original["text"]["text"].as_str().unwrap_or(""), 4096);
        submit(&d, &b, &fields, s, "Save link", move |s| {
            let (title, text, original) = (title.text().to_string(), text.text().to_string(), original.clone());
            async move {
                let data = s.call("getBusinessChatLinks", json!({})).await?;
                let current = data["links"]
                    .as_array()
                    .and_then(|links| links.iter().find(|l| l["link"] == original["link"]))
                    .ok_or("This link no longer exists.")?;
                if current["title"] != original["title"] || current["text"] != original["text"] {
                    return Err("The link changed. Reopen its editor.".into());
                }
                let formatted = if text == original["text"]["text"].as_str().unwrap_or("") {
                    original["text"].clone()
                } else {
                    json!({"@type":"formattedText","text":text,"entities":[]})
                };
                s.call(
                    "editBusinessChatLink",
                    json!({"link":original["link"],"link_info":{"@type":"inputBusinessChatLink","title":title,"text":formatted}}),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
