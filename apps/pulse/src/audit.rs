//! Read-only membership and recent administrator history, with explicit moderation actions.
use crate::{
    actions::{self, Scope},
    groups::{self, note, scroll},
    identities, window,
};
use gtk::{glib, prelude::*};
use nyx_telegram_core::client::parse_message;
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};
#[derive(Clone)]
enum Kind {
    Events,
    Invite(String),
}
struct Browser {
    scope: Scope,
    chat: i64,
    kind: Kind,
    dialog: glib::WeakRef<gtk::Window>,
    list: gtk::Box,
    status: gtk::Label,
    query: gtk::SearchEntry,
    filter: gtk::DropDown,
    member: RefCell<Option<i64>>,
    expired: gtk::CheckButton,
    more: gtk::Button,
    offset: RefCell<Value>,
    seen: RefCell<HashSet<String>>,
    revision: Cell<u64>,
    busy: Cell<bool>,
}
fn alive(scope: &Scope, dialog: &glib::WeakRef<gtk::Window>) -> bool {
    scope.valid() && dialog.upgrade().is_some_and(|d| d.is_visible())
}
fn clear(list: &gtk::Box) {
    while let Some(c) = list.first_child() {
        list.remove(&c)
    }
}
fn date(value: &Value) -> String {
    value
        .as_i64()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.with_timezone(&chrono::Local).format("%b %d, %Y · %H:%M").to_string())
        .unwrap_or_default()
}
fn number_id(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|v| v.to_string()))
}
fn words(value: &str) -> String {
    let mut out = String::new();
    for (i, c) in value.chars().enumerate() {
        if c == '_' {
            out.push(' ')
        } else {
            if i > 0 && c.is_uppercase() {
                out.push(' ');
            }
            out.push(c);
        }
    }
    out.trim().to_string()
}
fn value_text(v: &Value) -> String {
    if let Some(text) = v.as_str() {
        return text.to_string();
    }
    if let Some(on) = v.as_bool() {
        return if on { "Enabled" } else { "Disabled" }.into();
    }
    if v.is_number() {
        return v.to_string();
    }
    if v.is_null() {
        return "None".into();
    }
    if let Some(text) = v["text"].as_str() {
        return text.to_string();
    }
    match v["@type"].as_str().unwrap_or("") {
        "chatMemberStatusCreator" => "Owner".into(),
        "chatMemberStatusAdministrator" => format!("Administrator · {}", permissions(&v["rights"])),
        "chatMemberStatusMember" => "Member".into(),
        "chatMemberStatusLeft" => "Left".into(),
        "chatMemberStatusBanned" => "Banned".into(),
        "chatMemberStatusRestricted" => format!("Restricted · {}", permissions(&v["permissions"])),
        "chatPermissions" | "chatAdministratorRights" => permissions(v),
        "forumTopicInfo" => v["name"].as_str().unwrap_or("Topic").into(),
        "chatInviteLink" => {
            v["name"].as_str().filter(|s| !s.is_empty()).or(v["invite_link"].as_str()).unwrap_or("Invite link").into()
        }
        _ => {
            if let Some(array) = v.as_array() {
                array.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
            } else {
                "Updated".into()
            }
        }
    }
}
fn permissions(v: &Value) -> String {
    let enabled = v
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, val)| key.as_str() != "@type" && **val == Value::Bool(true))
        .map(|(key, _)| words(key.trim_start_matches("can_")))
        .collect::<Vec<_>>();
    if enabled.is_empty() { "No permissions".into() } else { enabled.join(", ") }
}
fn message_preview(v: &Value) -> String {
    parse_message(v).map(|m| crate::chat_view::preview(&m)).unwrap_or_else(|| "Message unavailable".into())
}
fn describe(action: &Value) -> (String, Vec<String>) {
    let kind = action["@type"].as_str().unwrap_or("");
    let title = words(kind.trim_start_matches("chatEvent"));
    let mut details = Vec::new();
    if kind == "chatEventMessageEdited" {
        details.push(format!("Before: {}", message_preview(&action["old_message"])));
        details.push(format!("After: {}", message_preview(&action["new_message"])));
    } else if action["message"].is_object() {
        details.push(message_preview(&action["message"]));
    }
    if let Some(object) = action.as_object() {
        for (key, old) in object {
            if let Some(property) = key.strip_prefix("old_") {
                if property == "message" {
                    continue;
                }
                if let Some(new) = object.get(&format!("new_{property}")) {
                    details.push(format!("{}\nBefore: {}\nAfter: {}", words(property), value_text(old), value_text(new)));
                }
            }
        }
    }
    for key in [
        "is_all_history_available",
        "has_protected_content",
        "has_aggressive_anti_spam_enabled",
        "sign_messages",
        "show_message_sender",
        "is_forum",
        "can_invite_users",
        "join_to_send_messages",
        "join_by_request",
        "mute_new_participants",
        "has_hidden_members",
    ] {
        if action.get(key).is_some() {
            details.push(format!("{}: {}", words(key), value_text(&action[key])));
        }
    }
    if let Some(name) = action["topic_info"]["name"].as_str() {
        details.push(format!("Topic: {name}"));
    }
    if action["invite_link"].is_object() {
        details.push(format!("Invitation: {}", value_text(&action["invite_link"])));
    }
    (if title.is_empty() { "Administrator event".into() } else { title }, details)
}
fn filters(index: u32) -> Value {
    let keys: &[&str] = match index {
        1 => &["message_edits", "message_deletions", "message_pins"],
        2 => &[
            "member_joins",
            "member_leaves",
            "member_invites",
            "member_restrictions",
            "member_tag_changes",
            "subscription_extensions",
        ],
        3 => &["member_promotions"],
        4 => &["invite_link_changes"],
        5 => &["info_changes", "setting_changes"],
        6 => &["forum_changes"],
        7 => &["video_chat_changes"],
        _ => return Value::Null,
    };
    let mut f = json!({"@type":"chatEventLogFilters"});
    for key in keys {
        f[*key] = json!(true);
    }
    f
}
pub fn events(chat: i64) {
    open(chat, Kind::Events)
}
pub fn invite_members(chat: i64, link: String) {
    open(chat, Kind::Invite(link))
}
fn open(chat: i64, kind: Kind) {
    actions::run(chat, "audit-browser", move |s| async move {
        let c = groups::context(&s, chat).await?;
        let subscription = match &kind {
            Kind::Events => {
                if c.supergroup().is_none() || !c.admin() {
                    return Err("Recent actions require administrator access to a supergroup or channel.".into());
                }
                false
            }
            Kind::Invite(link) => {
                c.require("can_invite_users")?;
                !s.call("getChatInviteLink", json!({"chat_id":chat,"invite_link":link})).await?["subscription_pricing"].is_null()
            }
        };
        let is_events = matches!(kind, Kind::Events);
        let (dialog, body) = w::dialog(if is_events { "Recent actions" } else { "Invite use history" }, 520);
        note(
            &body,
            if is_events {
                "Administrator and member actions from Telegram's last 48 hours."
            } else {
                "People who joined through this invitation. Telegram controls which membership records are available."
            },
        );
        let query = gtk::SearchEntry::new();
        query.set_placeholder_text(Some("Search recent actions"));
        query.set_visible(is_events);
        body.append(&query);
        let filter = gtk::DropDown::from_strings(&[
            "All actions",
            "Messages",
            "Members",
            "Administrator rights",
            "Invitations",
            "Chat settings",
            "Topics",
            "Calls",
        ]);
        filter.set_visible(is_events);
        body.append(&filter);
        let expired = gtk::CheckButton::with_label("Only expired subscriptions");
        expired.set_visible(!is_events && subscription);
        body.append(&expired);
        let member_button = gtk::Button::with_label("Filter by related member…");
        member_button.set_visible(is_events);
        body.append(&member_button);
        let list = scroll(&body);
        let status = note(&body, "");
        let more = gtk::Button::with_label("Load more");
        body.append(&more);
        let browser = Rc::new(Browser {
            scope: s,
            chat,
            kind,
            dialog: dialog.downgrade(),
            list,
            status,
            query: query.clone(),
            filter: filter.clone(),
            member: RefCell::new(None),
            expired: expired.clone(),
            more: more.clone(),
            offset: RefCell::new(Value::Null),
            seen: RefCell::new(HashSet::new()),
            revision: Cell::new(0),
            busy: Cell::new(false),
        });
        let weak = Rc::downgrade(&browser);
        query.connect_search_changed(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(true)
            }
        });
        let weak = Rc::downgrade(&browser);
        filter.connect_selected_notify(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(true)
            }
        });
        let weak = Rc::downgrade(&browser);
        expired.connect_toggled(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(true)
            }
        });
        let weak = Rc::downgrade(&browser);
        more.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(false)
            }
        });
        let weak = Rc::downgrade(&browser);
        member_button.connect_clicked(move |button| {
            let Some(b) = weak.upgrade().filter(|b| alive(&b.scope, &b.dialog)) else { return };
            if b.member.borrow().is_some() {
                *b.member.borrow_mut() = None;
                button.set_label("Filter by related member…");
                b.load(true);
                return;
            }
            let weak = Rc::downgrade(&b);
            let button = button.downgrade();
            crate::message_tools::choose_member(b.chat, move |user| {
                if let Some(b) = weak.upgrade().filter(|b| alive(&b.scope, &b.dialog)) {
                    *b.member.borrow_mut() = user["id"].as_i64();
                    if let Some(button) = button.upgrade() {
                        button.set_label(&format!("{} · click to clear", user["first_name"].as_str().unwrap_or("Member")));
                    }
                    b.load(true);
                }
            });
        });
        let refresh = gtk::Button::with_label("Refresh");
        let weak = Rc::downgrade(&browser);
        refresh.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(true)
            }
        });
        body.append(&refresh);
        let keep = browser.clone();
        dialog.connect_destroy(move |_| keep.revision.set(keep.revision.get() + 1));
        dialog.present();
        browser.load(true);
        Ok(())
    });
}
impl Browser {
    fn load(self: &Rc<Self>, reset: bool) {
        if !alive(&self.scope, &self.dialog) {
            return;
        }
        if reset {
            self.revision.set(self.revision.get() + 1);
            *self.offset.borrow_mut() = Value::Null;
            self.seen.borrow_mut().clear();
            clear(&self.list);
        } else if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.more.set_sensitive(false);
        self.status.set_text("Loading…");
        let this = self.clone();
        let revision = self.revision.get();
        let offset = self.offset.borrow().clone();
        let query = self.query.text().to_string();
        let filter = filters(self.filter.selected());
        let member = *self.member.borrow();
        let expired = self.expired.is_active();
        glib::spawn_future_local(async move {
            let result=match &this.kind{
                Kind::Events=>this.scope.call("getChatEventLog",json!({"chat_id":this.chat,"query":query,"from_event_id":if offset.is_null(){json!(0)}else{offset.clone()},"limit":50,"filters":filter,"user_ids":member.into_iter().collect::<Vec<_>>()})).await,
                Kind::Invite(link)=>this.scope.call("getChatInviteLinkMembers",json!({"chat_id":this.chat,"invite_link":link,"only_with_expired_subscription":expired,"offset_member":offset,"limit":50})).await,
            };
            if !alive(&this.scope, &this.dialog) || this.revision.get() != revision {
                return;
            }
            let data = match result {
                Ok(data) => data,
                Err(e) => {
                    this.busy.set(false);
                    this.more.set_sensitive(true);
                    this.status.set_text(&e);
                    return;
                }
            };
            let rows = data[if matches!(this.kind, Kind::Events) { "events" } else { "members" }]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut next = Value::Null;
            for row in &rows {
                let key = match &this.kind {
                    Kind::Events => {
                        next = row["id"].clone();
                        number_id(&row["id"])
                    }
                    Kind::Invite(_) => {
                        next = row.clone();
                        Some(format!("{}:{}", row["user_id"], row["joined_chat_date"]))
                    }
                };
                let Some(key) = key else { continue };
                if !this.seen.borrow_mut().insert(key) {
                    continue;
                }
                let actor = match &this.kind {
                    Kind::Events => row["member_id"].clone(),
                    Kind::Invite(_) => json!({"@type":"messageSenderUser","user_id":row["user_id"]}),
                };
                let name = identities::name(&this.scope, &actor).await.unwrap_or_else(|_| "Unavailable account".into());
                if !alive(&this.scope, &this.dialog) || this.revision.get() != revision {
                    return;
                }
                let card = w::vbox(4);
                match &this.kind {
                    Kind::Events => {
                        let (title, details) = describe(&row["action"]);
                        note(&card, &format!("{name} · {}", date(&row["date"])));
                        let heading = w::label(&title, "section-title");
                        heading.set_wrap(true);
                        heading.set_max_width_chars(46);
                        card.append(&heading);
                        for detail in details {
                            note(&card, &detail);
                        }
                        let action = &row["action"];
                        let subject = if action["member_id"].is_object() {
                            action["member_id"].clone()
                        } else if let Some(id) = action["user_id"].as_i64() {
                            json!({"@type":"messageSenderUser","user_id":id})
                        } else {
                            actor.clone()
                        };
                        let subject_name = identities::name(&this.scope, &subject).await.unwrap_or_else(|_| "Member".into());
                        if subject != actor {
                            note(&card, &format!("Member: {subject_name}"));
                        }
                        if !alive(&this.scope, &this.dialog) || this.revision.get() != revision {
                            return;
                        }
                        this.member_action(&card, subject, subject_name);
                        let message = if action["new_message"].is_object() { &action["new_message"] } else { &action["message"] };
                        if let (Some(chat), Some(id)) = (message["chat_id"].as_i64(), message["id"].as_i64()) {
                            let b = gtk::Button::with_label("Open message");
                            let scope = this.scope.clone();
                            b.connect_clicked(move |_| {
                                if scope.valid() {
                                    actions::run(chat, "audit-message", move |s| async move {
                                        let data = s.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
                                        if let Some(message) = parse_message(&data) {
                                            window::jump_message(&message);
                                        }
                                        Ok(())
                                    });
                                }
                            });
                            card.append(&b);
                            if action["can_report_anti_spam_false_positive"] == true {
                                let b = gtk::Button::with_label("Report incorrect anti-spam removal…");
                                let scope = this.scope.clone();
                                let source = this.chat;
                                b.connect_clicked(move |_| {
                                    if scope.valid() {
                                        false_positive(source, id);
                                    }
                                });
                                card.append(&b);
                            }
                        }
                    }
                    Kind::Invite(_) => {
                        note(&card, &name);
                        note(
                            &card,
                            &format!(
                                "Joined {}{}",
                                date(&row["joined_chat_date"]),
                                if row["via_chat_folder_invite_link"] == true { " · Shared folder invitation" } else { "" }
                            ),
                        );
                        if let Some(id) = row["approver_user_id"].as_i64().filter(|id| *id != 0) {
                            let approver = identities::name(&this.scope, &json!({"@type":"messageSenderUser","user_id":id}))
                                .await
                                .unwrap_or_else(|_| "Unavailable administrator".into());
                            if !alive(&this.scope, &this.dialog) || this.revision.get() != revision {
                                return;
                            }
                            note(&card, &format!("Approved by {approver}"));
                        }
                        this.member_action(&card, actor, name);
                    }
                }
                this.list.append(&card);
            }
            this.busy.set(false);
            this.more.set_sensitive(true);
            *this.offset.borrow_mut() = next.clone();
            this.more.set_visible(!rows.is_empty() && !next.is_null() && next != offset);
            this.status.set_text(if this.seen.borrow().is_empty() { "No records available in this view." } else { "" });
        });
    }
    fn member_action(&self, body: &gtk::Box, sender: Value, name: String) {
        let button = gtk::Button::with_label("Member actions…");
        let (scope, chat) = (self.scope.clone(), self.chat);
        button.connect_clicked(move |_| {
            if scope.valid() {
                groups::member_actions(chat, sender.clone(), name.clone());
            }
        });
        body.append(&button);
    }
}
fn false_positive(chat: i64, message: i64) {
    actions::operation(
        "Report incorrect removal?",
        "Tell Telegram this message was incorrectly removed by aggressive anti-spam?",
        vec![],
        "Report",
        move |s, _| async move {
            let c = groups::context(&s, chat).await?;
            if !c.admin() {
                return Err("Administrator rights are required.".into());
            }
            s.call("reportSupergroupAntiSpamFalsePositive", json!({"supergroup_id":c.supergroup(),"message_id":message})).await?;
            Ok(())
        },
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage3c_event_ids_filters_and_edit_descriptions() {
        let id = json!("9223372036854775806");
        assert_eq!(number_id(&id).as_deref(), Some("9223372036854775806"));
        assert!(filters(0).is_null());
        assert_eq!(filters(3)["member_promotions"], true);
        assert!(filters(3)["message_deletions"].is_null());
        let (title, details) = describe(&json!({"@type":"chatEventTitleChanged","old_title":"Before","new_title":"After"}));
        assert_eq!(title, "Title Changed");
        assert!(details[0].contains("Before: Before"));
        assert!(details[0].contains("After: After"));
        assert_eq!(value_text(&json!({"@type":"chatMemberStatusBanned"})), "Banned");
    }
}
