//! Forum navigation and management, and channel comment/message threads.
use crate::{
    actions::{self, Scope},
    groups::{self, entry, note, scroll, submit},
    window,
};
use gtk::{gdk, glib, prelude::*};
use nixe_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

pub fn suffix(topic: &Value) -> String {
    match topic["@type"].as_str() {
        Some("messageTopicForum") => format!(":f:{}", topic["forum_topic_id"]),
        Some("messageTopicThread") => format!(":t:{}", topic["message_thread_id"]),
        _ => String::new(),
    }
}
pub fn draft_key(account: &str, chat: i64, topic: &Value) -> String {
    format!("{account}:{chat}{}", suffix(topic))
}
pub fn parse_key(account: &str, key: &str) -> Option<(i64, Value)> {
    let rest = key.strip_prefix(&format!("{account}:"))?;
    let parts = rest.split(':').collect::<Vec<_>>();
    let chat = parts.first()?.parse().ok()?;
    let topic = match parts.as_slice() {
        [_] => Value::Null,
        [_, "f", id] => json!({"@type":"messageTopicForum","forum_topic_id":id.parse::<i32>().ok()?}),
        [_, "t", id] => json!({"@type":"messageTopicThread","message_thread_id":id.parse::<i64>().ok()?}),
        _ => return None,
    };
    Some((chat, topic))
}
pub fn matches_message(topic: &Value, message: &Value) -> bool {
    if topic.is_null() {
        return true;
    }
    if message["topic_id"] == *topic {
        return true;
    }
    topic["@type"] == "messageTopicThread" && message["id"] == topic["message_thread_id"]
}
pub fn thread(chat: i64, message: i64) {
    let navigation = window::navigation_version();
    actions::run(chat, "open-thread", move |s| async move {
        if s.call("getMessageProperties", json!({"chat_id":chat,"message_id":message})).await?["can_get_message_thread"] != true {
            return Err("This message has no accessible thread or discussion.".into());
        }
        let info = s.call("getMessageThread", json!({"chat_id":chat,"message_id":message})).await?;
        if window::navigation_version() != navigation {
            return Ok(());
        }
        let chat = info["chat_id"].as_i64().ok_or("Missing discussion chat")?;
        let id = info["message_thread_id"].as_i64().ok_or("Missing thread")?;
        window::open_context(
            chat,
            json!({"@type":"messageTopicThread","message_thread_id":id}),
            "Replies and comments".into(),
            info["draft_message"].clone(),
        );
        Ok(())
    });
}
fn alive(s: &Scope, d: &glib::WeakRef<gtk::Window>) -> bool {
    s.valid() && d.upgrade().is_some_and(|d| d.is_visible())
}
fn clear(list: &gtk::Box) {
    while let Some(c) = list.first_child() {
        list.remove(&c)
    }
}
type TopicChoice = Rc<dyn Fn(Value, String)>;
struct Topics {
    on_select: Option<TopicChoice>,
    scope: Scope,
    chat: i64,
    dialog: glib::WeakRef<gtk::Window>,
    list: gtk::Box,
    query: gtk::SearchEntry,
    more: gtk::Button,
    offset: RefCell<(i64, i64, i64)>,
    seen: RefCell<HashSet<i64>>,
    revision: Cell<u64>,
    busy: Cell<bool>,
}
pub fn browse(chat: i64) {
    browse_inner(chat, None);
}
pub fn choose_destination(chat: i64, callback: impl Fn(Value, String) + 'static) {
    browse_inner(chat, Some(Rc::new(callback)));
}
fn browse_inner(chat: i64, on_select: Option<TopicChoice>) {
    actions::run(chat, "forum-topics", move |s| async move {
        let c = groups::context(&s, chat).await?;
        if c.group["is_forum"] != true {
            return Err("This group does not have forum topics enabled.".into());
        }
        let (dialog, body) = w::dialog("Topics", 460);
        let query = gtk::SearchEntry::new();
        query.set_placeholder_text(Some("Search topics"));
        body.append(&query);
        if on_select.is_none() && (c.right("can_manage_topics") || c.chat["permissions"]["can_create_topics"] == true) {
            let b = gtk::Button::with_label("New topic…");
            let scope = s.clone();
            b.connect_clicked(move |_| {
                if scope.valid() {
                    edit(chat, None);
                }
            });
            body.append(&b);
        }
        if on_select.is_none() && c.right("can_manage_topics") {
            let button = gtk::Button::with_label("Arrange pinned topics…");
            let scope = s.clone();
            button.connect_clicked(move |_| {
                if scope.valid() {
                    arrange(chat);
                }
            });
            body.append(&button);
        }
        let list = scroll(&body);
        let more = gtk::Button::with_label("Load more");
        body.append(&more);
        let b = Rc::new(Topics {
            on_select,
            scope: s,
            chat,
            dialog: dialog.downgrade(),
            list,
            query: query.clone(),
            more: more.clone(),
            offset: RefCell::new((0, 0, 0)),
            seen: RefCell::new(HashSet::new()),
            revision: Cell::new(0),
            busy: Cell::new(false),
        });
        let weak = Rc::downgrade(&b);
        query.connect_search_changed(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(true)
            }
        });
        let weak = Rc::downgrade(&b);
        more.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(false)
            }
        });
        let refresh = gtk::Button::with_label("Refresh");
        let weak = Rc::downgrade(&b);
        refresh.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(true)
            }
        });
        body.append(&refresh);
        let keep = b.clone();
        dialog.connect_destroy(move |_| keep.revision.set(keep.revision.get() + 1));
        dialog.present();
        b.load(true);
        Ok(())
    });
}
impl Topics {
    fn load(self: &Rc<Self>, reset: bool) {
        if reset {
            self.revision.set(self.revision.get() + 1);
            *self.offset.borrow_mut() = (0, 0, 0);
            self.seen.borrow_mut().clear();
            clear(&self.list);
        } else if self.busy.get() {
            return;
        }
        if !alive(&self.scope, &self.dialog) {
            return;
        }
        self.busy.set(true);
        self.more.set_sensitive(false);
        let this = self.clone();
        let n = self.revision.get();
        let offset = *self.offset.borrow();
        let query = self.query.text().to_string();
        glib::spawn_future_local(async move {
            let result=this.scope.call("getForumTopics",json!({"chat_id":this.chat,"query":query,"offset_date":offset.0,"offset_message_id":offset.1,"offset_forum_topic_id":offset.2,"limit":50})).await;
            if !alive(&this.scope, &this.dialog) || n != this.revision.get() {
                return;
            }
            this.busy.set(false);
            this.more.set_sensitive(true);
            let data = match result {
                Ok(v) => v,
                Err(e) => {
                    note(&this.list, &e);
                    return;
                }
            };
            let topics = data["topics"].as_array().cloned().unwrap_or_default();
            for topic in &topics {
                let Some(id) = topic["info"]["forum_topic_id"].as_i64() else { continue };
                if !this.seen.borrow_mut().insert(id) {
                    continue;
                }
                let row = w::hbox(6);
                row.append(&crate::topic_art::badge(&topic["info"]["icon"], topic["info"]["is_general"] == true));
                let title = topic["info"]["name"].as_str().unwrap_or("Topic");
                let b = gtk::Button::with_label(&format!(
                    "{}{}{} · {} unread",
                    if topic["is_pinned"] == true { "📌 " } else { "" },
                    title,
                    if topic["info"]["is_closed"] == true { " · Closed" } else { "" },
                    topic["unread_count"]
                ));
                b.set_hexpand(true);
                if let Some(label) = b.child().and_downcast::<gtk::Label>() {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_max_width_chars(32);
                }
                let (scope, weak, chat) = (this.scope.clone(), this.dialog.clone(), this.chat);
                let on_select = this.on_select.clone();
                b.connect_clicked(move |_| {
                    if !alive(&scope, &weak) {
                        return;
                    }
                    let close = weak.clone();
                    let on_select = on_select.clone();
                    actions::run(chat, "open-forum-topic", move |s| {
                        let close = close.clone();
                        let on_select = on_select.clone();
                        async move {
                            let topic = s.call("getForumTopic", json!({"chat_id":chat,"forum_topic_id":id})).await?;
                            if close.upgrade().is_none_or(|d| !d.is_visible()) {
                                return Ok(());
                            }
                            if let Some(d) = close.upgrade() {
                                d.close();
                            }
                            if let Some(callback) = on_select {
                                callback(
                                    json!({"@type":"messageTopicForum","forum_topic_id":id}),
                                    topic["info"]["name"].as_str().unwrap_or("Topic").into(),
                                );
                                return Ok(());
                            }
                            window::open_context(
                                chat,
                                json!({"@type":"messageTopicForum","forum_topic_id":id}),
                                topic["info"]["name"].as_str().unwrap_or("Topic").into(),
                                topic["draft_message"].clone(),
                            );
                            Ok(())
                        }
                    });
                });
                row.append(&b);
                let menu = w::bar_button("open-menu-symbolic", "Topic actions");
                let (scope, chat) = (this.scope.clone(), this.chat);
                menu.connect_clicked(move |_| {
                    if scope.valid() {
                        manage(chat, id)
                    }
                });
                menu.set_visible(this.on_select.is_none());
                row.append(&menu);
                this.list.append(&row);
            }
            let next = (
                data["next_offset_date"].as_i64().unwrap_or(0),
                data["next_offset_message_id"].as_i64().unwrap_or(0),
                data["next_offset_forum_topic_id"].as_i64().unwrap_or(0),
            );
            *this.offset.borrow_mut() = next;
            this.more.set_visible(!topics.is_empty() && next != (0, 0, 0) && next != offset);
            if this.list.first_child().is_none() {
                note(&this.list, "No topics found.");
            }
        });
    }
}
fn manage(chat: i64, id: i64) {
    actions::run(chat, "manage-topic", move |s| async move {
        let c = groups::context(&s, chat).await?;
        let topic = s.call("getForumTopic", json!({"chat_id":chat,"forum_topic_id":id})).await?;
        let (dialog, body) = w::dialog("Topic actions", 420);
        note(&body, topic["info"]["name"].as_str().unwrap_or("Topic"));
        let add = |title: &str, work: Box<dyn Fn()>| {
            let b = gtk::Button::with_label(title);
            let scope = s.clone();
            b.connect_clicked(move |_| {
                if scope.valid() {
                    work();
                }
            });
            body.append(&b);
        };
        add(
            "Copy topic link",
            Box::new(move || {
                actions::run(chat, "topic-link", move |s| async move {
                    let link = s.call("getForumTopicLink", json!({"chat_id":chat,"forum_topic_id":id})).await?;
                    if let (Some(d), Some(url)) = (gdk::Display::default(), link["link"].as_str()) {
                        d.clipboard().set_text(url);
                    }
                    Ok(())
                })
            }),
        );
        let muted = if topic["notification_settings"]["use_default_mute_for"] == true {
            if c.chat["notification_settings"]["use_default_mute_for"] == true {
                s.call("getScopeNotificationSettings",json!({"scope":{"@type":"notificationSettingsScopeGroupChats"}})).await?["mute_for"].as_i64().unwrap_or(0)>0
            } else {
                c.chat["notification_settings"]["mute_for"].as_i64().unwrap_or(0) > 0
            }
        } else {
            topic["notification_settings"]["mute_for"].as_i64().unwrap_or(0) > 0
        };
        add(
            if muted { "Unmute topic" } else { "Mute topic" },
            Box::new(move || {
                actions::run(chat, "topic-mute", move |s| async move {
                    let topic = s.call("getForumTopic", json!({"chat_id":chat,"forum_topic_id":id})).await?;
                    let mut settings = topic["notification_settings"].clone();
                    settings["@type"] = json!("chatNotificationSettings");
                    settings["use_default_mute_for"] = json!(false);
                    settings["mute_for"] = json!(if muted { 0 } else { i32::MAX });
                    s.call(
                        "setForumTopicNotificationSettings",
                        json!({"chat_id":chat,"forum_topic_id":id,"notification_settings":settings}),
                    )
                    .await?;
                    Ok(())
                })
            }),
        );
        if c.right("can_manage_topics") || topic["info"]["is_outgoing"] == true {
            add("Edit topic…", Box::new(move || edit(chat, Some(id))));
            let closed = topic["info"]["is_closed"] == true;
            add(
                if closed { "Reopen topic…" } else { "Close topic…" },
                Box::new(move || toggle(chat, id, "toggleForumTopicIsClosed", "is_closed", !closed)),
            );
        }
        if c.right("can_manage_topics") {
            let pinned = topic["is_pinned"] == true;
            add(
                if pinned { "Unpin topic…" } else { "Pin topic…" },
                Box::new(move || toggle(chat, id, "toggleForumTopicIsPinned", "is_pinned", !pinned)),
            );
            if topic["info"]["is_general"] == true {
                let hidden = topic["info"]["is_hidden"] == true;
                add(
                    if hidden { "Show General topic…" } else { "Hide General topic…" },
                    Box::new(move || toggle(chat, id, "toggleGeneralForumTopicIsHidden", "is_hidden", !hidden)),
                );
            }
        }
        if c.right("can_delete_messages") || topic["info"]["is_outgoing"] == true {
            add(
                "Delete topic…",
                Box::new(move || {
                    actions::operation(
                        "Delete topic?",
                        "Delete this topic and its messages for everyone? This cannot be undone.",
                        vec![],
                        "Delete topic",
                        move |s, _| async move {
                            s.call("deleteForumTopic", json!({"chat_id":chat,"forum_topic_id":id})).await?;
                            window::topic_deleted(chat, id);
                            Ok(())
                        },
                    )
                }),
            );
        }
        note(&body, "Refresh the topic list to see changes.");
        dialog.present();
        Ok(())
    });
}
fn toggle(chat: i64, id: i64, method: &'static str, key: &'static str, value: bool) {
    actions::operation("Change topic?", "Apply this topic setting for the group?", vec![], "Apply", move |s, _| async move {
        let mut params = json!({"chat_id":chat,"forum_topic_id":id});
        params[key] = json!(value);
        s.call(method, params).await?;
        Ok(())
    });
}
fn edit(chat: i64, id: Option<i64>) {
    actions::run(chat, "edit-topic", move |s| async move {
        let original = if let Some(id) = id {
            s.call("getForumTopic", json!({"chat_id":chat,"forum_topic_id":id})).await?["info"].clone()
        } else {
            Value::Null
        };
        let (dialog, body) = w::dialog(if id.is_some() { "Edit topic" } else { "New topic" }, 440);
        let fields = w::vbox(8);
        body.append(&fields);
        let name = entry(&fields, "Name", original["name"].as_str().unwrap_or(""), 128);
        let chosen = Rc::new(RefCell::new(original["icon"]["custom_emoji_id"].clone()));
        let color = Rc::new(Cell::new(original["icon"]["color"].as_u64().unwrap_or(COLORS[0] as u64) as u32));
        let preview = w::hbox(0);
        fields.append(&preview);
        let general = original["is_general"] == true;
        let redraw: Rc<dyn Fn()> = Rc::new({
            let (chosen, color, preview) = (chosen.clone(), color.clone(), preview.clone());
            move || {
                clear(&preview);
                preview.append(&crate::topic_art::badge(
                    &json!({"color":color.get(),"custom_emoji_id":chosen.borrow().clone()}),
                    general,
                ));
            }
        });
        redraw();
        if original["is_general"] != true {
            let picker = gtk::Button::with_label("Choose icon…");
            fields.append(&picker);
            let (chosen, redraw, scope, weak) = (chosen.clone(), redraw.clone(), s.clone(), dialog.downgrade());
            picker.connect_clicked(move |_| {
                if !scope.valid() {
                    return;
                }
                let (chosen, redraw, scope, weak) = (chosen.clone(), redraw.clone(), scope.clone(), weak.clone());
                crate::topic_art::choose(move |id| {
                    if alive(&scope, &weak) {
                        *chosen.borrow_mut() = id;
                        redraw();
                    }
                });
            });
        }
        if id.is_none() {
            let dropdown = gtk::DropDown::from_strings(&["Blue", "Yellow", "Purple", "Green", "Pink", "Red"]);
            fields.append(&dropdown);
            let (state, redraw) = (color.clone(), redraw.clone());
            dropdown.connect_selected_notify(move |d| {
                if let Some(color) = COLORS.get(d.selected() as usize) {
                    state.set(*color);
                    redraw();
                }
            });
        }
        submit(&dialog, &body, &fields, s, "Save topic", move |s| {
            let (name, chosen, color, original) =
                (name.text().trim().to_string(), chosen.borrow().clone(), color.get(), original.clone());
            async move {
                if name.is_empty() {
                    return Err("Enter a topic name.".into());
                }
                if (id.is_none() || chosen != original["icon"]["custom_emoji_id"]) && !icon_allowed(&s, &chosen).await? {
                    return Err("Your account can no longer use the selected icon. Choose a default topic icon.".into());
                }
                if let Some(id) = id {
                    let current = s.call("getForumTopic", json!({"chat_id":chat,"forum_topic_id":id})).await?;
                    if current["info"]["name"] != original["name"] || current["info"]["icon"] != original["icon"] {
                        return Err("This topic changed elsewhere. Reopen the editor.".into());
                    }
                    let change_icon = original["is_general"] != true && chosen != original["icon"]["custom_emoji_id"];
                    s.call("editForumTopic",json!({"chat_id":chat,"forum_topic_id":id,"name":name,"edit_icon_custom_emoji":change_icon,"icon_custom_emoji_id":if chosen.is_null(){json!(0)}else{chosen}})).await?;
                } else {
                    groups::commit(&s,"createForumTopic",json!({"chat_id":chat,"name":name,"is_name_implicit":false,"icon":{"@type":"forumTopicIcon","color":color,"custom_emoji_id":if chosen.is_null(){json!(0)}else{chosen}}})).await?;
                }
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
const COLORS: [u32; 6] = [0x6FB9F0, 0xFFD67E, 0xCB86DB, 0x8EEE98, 0xFF93B2, 0xFB6F5F];
async fn icon_allowed(s: &Scope, id: &Value) -> Result<bool, String> {
    if id.is_null() || id.as_i64() == Some(0) || id.as_str() == Some("0") {
        return Ok(true);
    }
    if s.call("getMe", json!({})).await?["is_premium"] == true {
        return Ok(true);
    }
    let defaults = s.call("getForumTopicDefaultIcons", json!({})).await?;
    Ok(defaults["stickers"].as_array().into_iter().flatten().any(|sticker| crate::topic_art::emoji_id(sticker) == *id))
}
pub fn update(v: &Value) {
    if v["@type"] == "updateForumTopic" {
        if let Some(chat) = v["chat_id"].as_i64() {
            window::remote_topic_draft(
                chat,
                &json!({"@type":"messageTopicForum","forum_topic_id":v["forum_topic_id"]}),
                &v["draft_message"],
            );
        }
    }
    if v["@type"] == "updateChatDraftMessage" && !v["topic_id"].is_null() {
        if let Some(chat) = v["chat_id"].as_i64() {
            window::remote_topic_draft(chat, &v["topic_id"], &v["draft_message"]);
        }
    }
    if v["@type"] == "updateForumTopicInfo" {
        let info = &v["info"];
        if let (Some(chat), Some(id), Some(name)) =
            (info["chat_id"].as_i64(), info["forum_topic_id"].as_i64(), info["name"].as_str())
        {
            window::topic_name(chat, id, name);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_identity_and_drafts() {
        let f = json!({"@type":"messageTopicForum","forum_topic_id":3});
        let t = json!({"@type":"messageTopicThread","message_thread_id":3145728});
        assert_ne!(draft_key("a", -1, &f), draft_key("a", -1, &t));
        assert_eq!(parse_key("a", &draft_key("a", -1, &f)), Some((-1, f.clone())));
        assert_eq!(parse_key("a", "a:-1"), Some((-1, Value::Null)));
        assert!(parse_key("b", "a:-1:f:3").is_none());
        assert!(matches_message(&f, &json!({"topic_id":f})));
        assert!(!matches_message(&f, &json!({"topic_id":t})));
        assert!(matches_message(&t, &json!({"id":3145728})));
        assert!(!matches_message(&t, &json!({"id":7})));
    }
}

pub fn open_link(info: Value) {
    let Some(chat) = info["chat_id"].as_i64() else { return };
    let topic = info["topic_id"].clone();
    let jump = info["message"]["id"].as_i64();
    let navigation = window::navigation_version();
    actions::run(chat, "open-topic-link", move |s| async move {
        let (title, draft) = match topic["@type"].as_str() {
            Some("messageTopicForum") => {
                let t = s.call("getForumTopic", json!({"chat_id":chat,"forum_topic_id":topic["forum_topic_id"]})).await?;
                (t["info"]["name"].as_str().unwrap_or("Topic").to_string(), t["draft_message"].clone())
            }
            Some("messageTopicThread") => {
                let t = s.call("getMessageThread", json!({"chat_id":chat,"message_id":topic["message_thread_id"]})).await?;
                ("Replies and comments".into(), t["draft_message"].clone())
            }
            None => (String::new(), Value::Null),
            _ => return Err("This Telegram topic type is not supported yet.".into()),
        };
        if window::navigation_version() == navigation {
            window::open_context_at(chat, topic, title, draft, jump);
        }
        Ok(())
    });
}

async fn pinned_topics(scope: &Scope, chat: i64, dialog: &glib::WeakRef<gtk::Window>) -> Result<Vec<(i64, String)>, String> {
    let mut offset = (0i64, 0i64, 0i64);
    let mut cursors = HashSet::new();
    let mut seen = HashSet::new();
    let mut pinned = Vec::new();
    loop {
        if !alive(scope, dialog) {
            return Err("Topic arrangement closed".into());
        }
        if !cursors.insert(offset) {
            return Err("Telegram repeated a topic page. Reopen the arrangement before saving.".into());
        }
        let page=scope.call("getForumTopics",json!({"chat_id":chat,"query":"","offset_date":offset.0,"offset_message_id":offset.1,"offset_forum_topic_id":offset.2,"limit":100})).await?;
        for topic in page["topics"].as_array().into_iter().flatten() {
            let Some(id) = topic["info"]["forum_topic_id"].as_i64() else { continue };
            if topic["is_pinned"] == true && seen.insert(id) {
                let order = topic["order"].as_i64().or_else(|| topic["order"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
                pinned.push((id, topic["info"]["name"].as_str().unwrap_or("Topic").to_string(), order));
            }
        }
        offset = (
            page["next_offset_date"].as_i64().unwrap_or(0),
            page["next_offset_message_id"].as_i64().unwrap_or(0),
            page["next_offset_forum_topic_id"].as_i64().unwrap_or(0),
        );
        if offset == (0, 0, 0) {
            break;
        }
    }
    pinned.sort_by_key(|row| std::cmp::Reverse(row.2));
    Ok(pinned.into_iter().map(|(id, name, _)| (id, name)).collect())
}
fn reorder_rows(list: &gtk::Box, model: &Rc<RefCell<Vec<(i64, String)>>>, scope: &Scope) {
    clear(list);
    let rows = model.borrow().clone();
    for (index, (_, name)) in rows.iter().enumerate() {
        let row = w::hbox(6);
        let label = w::label(name, "");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(28);
        label.set_hexpand(true);
        row.append(&label);
        for up in [true, false] {
            let button =
                w::bar_button(if up { "go-up-symbolic" } else { "go-down-symbolic" }, if up { "Move up" } else { "Move down" });
            button.set_sensitive(if up { index > 0 } else { index + 1 < rows.len() });
            let (model, list, scope) = (model.clone(), list.downgrade(), scope.clone());
            button.connect_clicked(move |_| {
                if !scope.valid() {
                    return;
                }
                {
                    let mut rows = model.borrow_mut();
                    move_row(&mut rows, index, up);
                }
                if let Some(list) = list.upgrade() {
                    reorder_rows(&list, &model, &scope);
                }
            });
            row.append(&button);
        }
        list.append(&row);
    }
}
fn move_row<T>(rows: &mut [T], index: usize, up: bool) {
    let other = if up { index.checked_sub(1) } else { index.checked_add(1) };
    if let Some(other) = other.filter(|i| *i < rows.len() && index < rows.len()) {
        rows.swap(index, other);
    }
}
fn arrange(chat: i64) {
    actions::run(chat, "pinned-topic-order", move |scope| async move {
        groups::context(&scope, chat).await?.require("can_manage_topics")?;
        let (dialog, body) = w::dialog("Arrange pinned topics", 440);
        let status = note(&body, "Loading pinned topics…");
        let fields = scroll(&body);
        dialog.present();
        let weak = dialog.downgrade();
        let original = match pinned_topics(&scope, chat, &weak).await {
            Ok(rows) => rows,
            Err(e) => {
                if alive(&scope, &weak) {
                    status.set_text(&e);
                }
                return Ok(());
            }
        };
        if !alive(&scope, &weak) {
            return Ok(());
        }
        if original.is_empty() {
            status.set_text("No pinned topics. Pin topics from their actions menu first.");
            return Ok(());
        }
        status.set_text("Move topics, then save the order for this group.");
        let model = Rc::new(RefCell::new(original.clone()));
        reorder_rows(&fields, &model, &scope);
        submit(&dialog, &body, &fields, scope, "Save order", move |s| {
            let chosen = model.borrow().iter().map(|(id, _)| *id).collect::<Vec<_>>();
            let original = original.iter().map(|(id, _)| *id).collect::<Vec<_>>();
            let weak = weak.clone();
            async move {
                groups::context(&s, chat).await?.require("can_manage_topics")?;
                if pinned_topics(&s, chat, &weak).await?.iter().map(|(id, _)| *id).collect::<Vec<_>>() != original {
                    return Err("Pinned topics changed elsewhere. Reopen the arrangement.".into());
                }
                s.call("setPinnedForumTopics", json!({"chat_id":chat,"forum_topic_ids":chosen})).await?;
                Ok(())
            }
        });
        Ok(())
    });
}
#[cfg(test)]
mod ordering_tests {
    use super::*;
    #[test]
    fn stage3d_reordering_preserves_membership_and_bounds() {
        let mut rows = vec![1, 2, 3];
        move_row(&mut rows, 0, true);
        assert_eq!(rows, vec![1, 2, 3]);
        move_row(&mut rows, 1, true);
        assert_eq!(rows, vec![2, 1, 3]);
        move_row(&mut rows, 0, false);
        assert_eq!(rows, vec![1, 2, 3]);
        move_row(&mut rows, 99, false);
        move_row(&mut rows, 2, false);
        assert_eq!(rows, vec![1, 2, 3]);
    }
}
