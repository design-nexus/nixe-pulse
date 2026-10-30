//! Group/channel membership and administration. Every mutation uses a fresh permission snapshot.
use crate::{
    actions::{self, Scope},
    window,
};
use gtk::{gdk, glib, prelude::*};
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    future::Future,
    rc::Rc,
};

const PERMISSIONS: &[(&str, &str)] = &[
    ("can_send_basic_messages", "Text messages"),
    ("can_send_photos", "Photos"),
    ("can_send_videos", "Videos"),
    ("can_send_audios", "Music"),
    ("can_send_documents", "Files"),
    ("can_send_voice_notes", "Voice messages"),
    ("can_send_video_notes", "Video messages"),
    ("can_send_polls", "Polls"),
    ("can_send_other_messages", "Stickers and animations"),
    ("can_add_link_previews", "Link previews"),
    ("can_react_to_messages", "Reactions"),
    ("can_edit_tag", "Edit own member tag"),
    ("can_change_info", "Change group information"),
    ("can_invite_users", "Invite members"),
    ("can_pin_messages", "Pin messages"),
    ("can_create_topics", "Create topics"),
];
const RIGHTS: &[(&str, &str)] = &[
    ("can_manage_chat", "Manage chat"),
    ("can_change_info", "Change information"),
    ("can_post_messages", "Post messages"),
    ("can_edit_messages", "Edit messages"),
    ("can_delete_messages", "Delete messages"),
    ("can_invite_users", "Invite members"),
    ("can_restrict_members", "Restrict and ban members"),
    ("can_pin_messages", "Pin messages"),
    ("can_manage_topics", "Manage topics"),
    ("can_promote_members", "Add administrators"),
    ("can_manage_video_chats", "Manage calls"),
    ("can_post_stories", "Post stories"),
    ("can_edit_stories", "Edit stories"),
    ("can_delete_stories", "Delete stories"),
    ("can_manage_direct_messages", "Manage direct messages"),
    ("can_manage_tags", "Manage member tags"),
    ("can_send_welcome_messages", "Send welcome messages"),
    ("is_anonymous", "Remain anonymous"),
];
#[derive(Clone)]
pub(crate) struct Context {
    pub(crate) chat: Value,
    pub(crate) group: Value,
    pub(crate) me: i64,
}
impl Context {
    pub(crate) fn owner(&self) -> bool {
        self.group["status"]["@type"] == "chatMemberStatusCreator"
    }
    pub(crate) fn admin(&self) -> bool {
        self.owner() || self.group["status"]["@type"] == "chatMemberStatusAdministrator"
    }
    pub(crate) fn right(&self, key: &str) -> bool {
        self.owner() || self.group["status"]["rights"][key] == true
    }
    pub(crate) fn require(&self, key: &str) -> Result<(), String> {
        if self.right(key) { Ok(()) } else { Err("Your Telegram permissions no longer allow this action.".into()) }
    }
    pub(crate) fn supergroup(&self) -> Option<i64> {
        self.chat["type"]["supergroup_id"].as_i64()
    }
    pub(crate) fn channel(&self) -> bool {
        self.group["is_channel"] == true
    }
}
pub(crate) async fn context(scope: &Scope, chat: i64) -> Result<Context, String> {
    let data = scope.call("getChat", json!({"chat_id":chat})).await?;
    let group = if let Some(id) = data["type"]["supergroup_id"].as_i64() {
        scope.call("getSupergroup", json!({"supergroup_id":id})).await?
    } else if let Some(id) = data["type"]["basic_group_id"].as_i64() {
        scope.call("getBasicGroup", json!({"basic_group_id":id})).await?
    } else {
        return Err("This action requires a group or channel.".into());
    };
    let me = scope.call("getMe", json!({})).await?["id"].as_i64().ok_or("Account unavailable")?;
    Ok(Context { chat: data, group, me })
}
pub(crate) async fn commit(scope: &Scope, method: &str, params: Value) -> Result<Value, String> {
    scope.call(method, params).await.map_err(|e| {
        if e.contains("timed out") || e.contains("shut down") || e.contains("stopped") {
            format!("The action's status is unknown. Check the chat or invite list before trying again. {e}")
        } else {
            e
        }
    })
}
fn alive(scope: &Scope, dialog: &glib::WeakRef<gtk::Window>) -> bool {
    scope.valid() && dialog.upgrade().is_some_and(|d| d.is_visible())
}
pub(crate) fn note(body: &gtk::Box, text: &str) -> gtk::Label {
    let label = w::label(text, "dim");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(48);
    body.append(&label);
    label
}
pub(crate) fn entry(body: &gtk::Box, title: &str, value: &str, max: i32) -> gtk::Entry {
    let label=note(body, title);
    let e = gtk::Entry::new();
    label.set_mnemonic_widget(Some(&e));
    e.set_text(value);
    e.set_max_length(max);
    body.append(&e);
    e
}
pub(crate) fn scroll(body: &gtk::Box) -> gtk::Box {
    let list = w::vbox(6);
    body.append(
        &gtk::ScrolledWindow::builder()
            .min_content_height(100)
            .max_content_height(340)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .build(),
    );
    list
}
fn clear(body: &gtk::Box) {
    while let Some(c) = body.first_child() {
        body.remove(&c);
    }
}
fn button(body: &gtk::Box, text: &str, scope: &Scope, callback: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    if let Some(label)=b.child().and_downcast::<gtk::Label>(){label.set_wrap(true);label.set_wrap_mode(gtk::pango::WrapMode::WordChar);label.set_max_width_chars(44);}
    let scope = scope.clone();
    b.connect_clicked(move |_| {
        if scope.valid() {
            callback();
        }
    });
    body.append(&b);
    b
}
fn refresh(chat: i64) {
    window::with_service(|s| s.load_chat(chat));
}
// Disable fields and cancellation while a request is in flight, keeping recoverable errors in the dialog.
pub(crate) fn submit<F, Fut>(dialog: &gtk::Window, body: &gtk::Box, fields: &gtk::Box, scope: Scope, title: &str, work: F)
where
    F: Fn(Scope) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let error = w::label("", "error");
    error.set_wrap(true);
    error.set_max_width_chars(48);
    body.append(&error);
    let row = w::hbox(8);
    row.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label(title);
    row.append(&cancel);
    row.append(&save);
    body.append(&row);
    let busy = Rc::new(Cell::new(false));
    let state = busy.clone();
    dialog.connect_close_request(move |_| if state.get() { glib::Propagation::Stop } else { glib::Propagation::Proceed });
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    let weak = dialog.downgrade();
    let fields = fields.clone();
    save.connect_clicked(move |b| {
        let Some(d) = weak.upgrade() else { return };
        if !scope.valid() {
            d.close();
            return;
        }
        if busy.replace(true) {
            return;
        }
        let future = work(scope.clone());
        b.set_sensitive(false);
        cancel.set_sensitive(false);
        fields.set_sensitive(false);
        error.set_text("");
        let (weak, busy, b, cancel, fields, error, scope) =
            (weak.clone(), busy.clone(), b.clone(), cancel.clone(), fields.clone(), error.clone(), scope.clone());
        glib::spawn_future_local(async move {
            let result = future.await;
            busy.set(false);
            let Some(d) = weak.upgrade() else { return };
            if !scope.valid() {
                d.close();
                return;
            }
            match result {
                Ok(()) => d.close(),
                Err(e) => {
                    let uncertain = e.contains("status is unknown");
                    error.set_text(&e);
                    b.set_sensitive(!uncertain);
                    cancel.set_sensitive(true);
                    fields.set_sensitive(!uncertain);
                }
            }
        });
    });
}
fn checkboxes(
    body: &gtk::Box,
    source: &Value,
    keys: &[(&str, &str)],
    allowed: impl Fn(&str) -> bool,
) -> Vec<(String, gtk::CheckButton)> {
    keys.iter()
        .map(|(key, title)| {
            let c = gtk::CheckButton::with_label(title);
            c.set_active(source[*key] == true);
            c.set_sensitive(allowed(key));
            body.append(&c);
            (key.to_string(), c)
        })
        .collect()
}
fn collect(mut value: Value, kind: &str, checks: &[(String, gtk::CheckButton)]) -> Value {
    if !value.is_object() {
        value = json!({});
    }
    value["@type"] = json!(kind);
    for (key, c) in checks {
        value[key] = json!(c.is_active());
    }
    value
}
fn name(user: &Value) -> String {
    format!("{} {}", user["first_name"].as_str().unwrap_or(""), user["last_name"].as_str().unwrap_or("")).trim().to_string()
}
fn status_label(status: &Value) -> &'static str {
    match status["@type"].as_str().unwrap_or("") {
        "chatMemberStatusCreator" => "Owner",
        "chatMemberStatusAdministrator" => "Administrator",
        "chatMemberStatusRestricted" => "Restricted",
        "chatMemberStatusBanned" => "Banned",
        "chatMemberStatusLeft" => "Left",
        _ => "Member",
    }
}

pub fn manage(chat: i64) {
    actions::run(chat, "manage-group", move |scope| async move {
        let c = context(&scope, chat).await?;
        let (dialog, body) = w::dialog(if c.channel() { "Channel management" } else { "Group management" }, 440);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        let body = scroll(&body);
        note(&body, c.chat["title"].as_str().unwrap_or(""));
        note(&body, status_label(&c.group["status"]));
        let status = c.group["status"]["@type"].as_str().unwrap_or("");
        if status == "chatMemberStatusLeft" {
            button(&body, "Join…", &scope, move || join_public(chat));
        }
        if c.group["is_forum"] == true {
            button(&body, "Topics…", &scope, move || crate::topics::browse(chat));
        }
        button(&body, "Group / channel settings…", &scope, move || crate::group_settings::open(chat));
        button(&body, "Members and administrators…", &scope, move || members(chat));
        if c.right("can_change_info") || c.chat["permissions"]["can_change_info"] == true {
            button(&body, "Edit name…", &scope, move || edit_info(chat, false));
            button(&body, "Edit description…", &scope, move || edit_info(chat, true));
        }
        if !c.channel() && c.right("can_restrict_members") {
            button(&body, "Default permissions…", &scope, move || permissions(chat));
        }
        if c.right("can_invite_users") || c.chat["permissions"]["can_invite_users"] == true {
            button(&body, "Add member…", &scope, move || add_member(chat));
        }
        if c.admin() && c.right("can_invite_users") {
            button(&body, "Invite links…", &scope, move || invites(chat));
            button(&body, "Join requests…", &scope, move || join_requests(chat));
        }
        if c.supergroup().is_some() && c.admin() {
            button(&body, "Recent actions…", &scope, move || crate::audit::events(chat));
        }
        if c.owner() {
            button(&body, "Transfer ownership…", &scope, move || choose_user("New owner", move |user| transfer(chat, user)));
        }
        dialog.present();
        Ok(())
    });
}
pub fn create() {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("New group or channel", 440);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    button(&body, "Open invitation…", &scope, invite_prompt);
    let fields = w::vbox(8);
    body.append(&fields);
    let kind = gtk::DropDown::from_strings(&["Group", "Forum group", "Channel"]);
    fields.append(&kind);
    let title = entry(&fields, "Name", "", 128);
    let description = entry(&fields, "Description", "", 255);
    note(&fields, "Creates a private chat. Add people or share an invitation afterward.");
    submit(&dialog, &body, &fields, scope, "Create", move |s| {
        let (title, description, kind) = (title.text().trim().to_string(), description.text().to_string(), kind.selected());
        async move {
            if title.is_empty() {
                return Err("Enter a name.".into());
            }
            let data = commit(&s,"createNewSupergroupChat", json!({"title":title,"description":description,"is_forum":kind==1,"is_channel":kind==2,"location":null,"message_auto_delete_time":0,"for_import":false})).await?;
            if let Some(id) = data["id"].as_i64() {
                refresh(id);
                window::open_chat(id);
            }
            Ok(())
        }
    });
    dialog.present();
}
fn edit_info(chat: i64, description: bool) {
    actions::run(chat, "group-info", move |s| async move {
        let c = context(&s, chat).await?;
        let initial = if description {
            let (method, args) = if let Some(id) = c.supergroup() {
                ("getSupergroupFullInfo", json!({"supergroup_id":id}))
            } else {
                ("getBasicGroupFullInfo", json!({"basic_group_id":c.chat["type"]["basic_group_id"]}))
            };
            s.call(method, args).await?["description"].as_str().unwrap_or("").to_string()
        } else {
            c.chat["title"].as_str().unwrap_or("").to_string()
        };
        let (dialog, body) = w::dialog(if description { "Edit description" } else { "Edit name" }, 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        let value =
            entry(&fields, if description { "Description" } else { "Name" }, &initial, if description { 255 } else { 128 });
        submit(&dialog, &body, &fields, s, "Save", move |s| {
            let value = value.text().trim().to_string();
            async move {
                let c = context(&s, chat).await?;
                if !c.right("can_change_info") && c.chat["permissions"]["can_change_info"] != true {
                    return Err("You cannot change this chat's information.".into());
                }
                if description {
                    s.call("setChatDescription", json!({"chat_id":chat,"description":value})).await?;
                } else {
                    if value.is_empty() {
                        return Err("Enter a name.".into());
                    }
                    s.call("setChatTitle", json!({"chat_id":chat,"title":value})).await?;
                }
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn permissions(chat: i64) {
    actions::run(chat, "group-permissions", move |s| async move {
        let c = context(&s, chat).await?;
        c.require("can_restrict_members")?;
        let (dialog, body) = w::dialog("Default member permissions", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        note(&body, "These permissions apply to all ordinary members.");
        let fields = scroll(&body);
        let original = c.chat["permissions"].clone();
        let checks = checkboxes(&fields, &original, PERMISSIONS, |_| true);
        submit(&dialog, &body, &fields, s, "Save permissions", move |s| {
            let permissions = collect(original.clone(), "chatPermissions", &checks);
            let original = original.clone();
            async move {
                let c = context(&s, chat).await?;
                c.require("can_restrict_members")?;
                if c.chat["permissions"] != original {
                    return Err("Permissions changed in another client. Reopen this dialog before saving.".into());
                }
                s.call("setChatPermissions", json!({"chat_id":chat,"permissions":permissions})).await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}

pub fn choose_user(title: &str, callback: impl Fn(Value) + 'static) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog(title, 440);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    let query = gtk::SearchEntry::new();
    query.set_placeholder_text(Some("Search contacts, or enter @username"));
    body.append(&query);
    let list = scroll(&body);
    let revision = Rc::new(Cell::new(0u64));
    let callback = Rc::new(callback);
    let weak = dialog.downgrade();
    let load: Rc<dyn Fn(String)> = Rc::new(move |query| {
        let n = revision.get() + 1;
        revision.set(n);
        let (scope, weak, list, revision, callback) =
            (scope.clone(), weak.clone(), list.clone(), revision.clone(), callback.clone());
        glib::spawn_future_local(async move {
            let result = async {
                let ids = if query.starts_with('@') && query.len() > 1 {
                    let chat = scope.call("searchPublicChat", json!({"username":query.trim_start_matches('@')})).await?;
                    vec![chat["type"]["user_id"].as_i64().ok_or("That username is not a person or bot.")?]
                } else {
                    scope.call("searchContacts", json!({"query":query,"limit":50})).await?["user_ids"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_i64)
                        .collect()
                };
                Ok::<_, String>(ids)
            }
            .await;
            if !alive(&scope, &weak) || revision.get() != n {
                return;
            }
            clear(&list);
            let ids = match result {
                Ok(v) => v,
                Err(e) => {
                    note(&list, &e);
                    return;
                }
            };
            if ids.is_empty() {
                note(&list, "No contacts found.");
            }
            for id in ids {
                let Ok(user) = scope.call("getUser", json!({"user_id":id})).await else { continue };
                if !alive(&scope, &weak) || revision.get() != n {
                    return;
                }
                let (cb, d) = (callback.clone(), weak.clone());
                let caption = name(&user);
                button(&list, &caption, &scope, move || {
                    if let Some(d) = d.upgrade() {
                        d.close();
                    }
                    cb(user.clone());
                });
            }
        });
    });
    let first = load.clone();
    query.connect_search_changed(move |q| load(q.text().to_string()));
    dialog.present();
    first(String::new());
}
fn add_member(chat: i64) {
    choose_user("Add member", move |user| {
        let id = user["id"].as_i64().unwrap_or(0);
        let title = name(&user);
        actions::operation(
            "Add member?",
            &format!("Add {title} to this chat? No earlier messages will be forwarded."),
            vec![],
            "Add member",
            move |s, _| async move {
                let c = context(&s, chat).await?;
                if !c.right("can_invite_users") && c.chat["permissions"]["can_invite_users"] != true {
                    return Err("You cannot invite members.".into());
                }
                let result = s.call("addChatMember", json!({"chat_id":chat,"user_id":id,"forward_limit":0})).await?;
                if result["failed_to_add_members"].as_array().is_some_and(|a| !a.is_empty()) {
                    return Err("Telegram could not add this person. Their privacy settings or Premium requirements may require sending an invite link instead.".into());
                }
                refresh(chat);
                Ok(())
            },
        );
    });
}

struct Members {
    scope: Scope,
    chat: i64,
    context: Context,
    dialog: glib::WeakRef<gtk::Window>,
    list: gtk::Box,
    query: gtk::SearchEntry,
    filter: gtk::DropDown,
    more: gtk::Button,
    revision: Cell<u64>,
    offset: Cell<i64>,
    seen: RefCell<HashSet<String>>,
    busy: Cell<bool>,
}
pub fn members(chat: i64) {
    actions::run(chat, "group-members", move |s| async move {
        let c = context(&s, chat).await?;
        let (dialog, body) = w::dialog("Members and administrators", 460);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        let query = gtk::SearchEntry::new();
        query.set_placeholder_text(Some("Search members"));
        body.append(&query);
        let filter = gtk::DropDown::from_strings(&["Members", "Administrators", "Restricted", "Banned", "Bots"]);
        body.append(&filter);
        let list = scroll(&body);
        let more = gtk::Button::with_label("Load more");
        body.append(&more);
        let browser = Rc::new(Members {
            scope: s,
            chat,
            context: c,
            dialog: dialog.downgrade(),
            list,
            query: query.clone(),
            filter: filter.clone(),
            more: more.clone(),
            revision: Cell::new(0),
            offset: Cell::new(0),
            seen: RefCell::new(HashSet::new()),
            busy: Cell::new(false),
        });
        let b = Rc::downgrade(&browser);
        query.connect_search_changed(move |_| {
            if let Some(b) = b.upgrade() {
                b.load(true);
            }
        });
        let b = Rc::downgrade(&browser);
        filter.connect_selected_notify(move |_| {
            if let Some(b) = b.upgrade() {
                b.load(true);
            }
        });
        let b = Rc::downgrade(&browser);
        more.connect_clicked(move |_| {
            if let Some(b) = b.upgrade() {
                b.load(false);
            }
        });
        let b = browser.clone();
        button(&body, "Refresh", &browser.scope, move || b.load(true));
        let keep = browser.clone();
        dialog.connect_destroy(move |_| {
            keep.revision.set(keep.revision.get() + 1);
        });
        dialog.present();
        browser.load(true);
        Ok(())
    });
}
impl Members {
    fn load(self: &Rc<Self>, reset: bool) {
        if reset {
            self.revision.set(self.revision.get() + 1);
            self.offset.set(0);
            self.seen.borrow_mut().clear();
            clear(&self.list);
        } else if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.more.set_sensitive(false);
        let this = self.clone();
        let n = self.revision.get();
        let query = self.query.text().to_string();
        let filter = self.filter.selected();
        let offset = self.offset.get();
        glib::spawn_future_local(async move {
            let result = if let Some(id) = this.context.supergroup() {
                let kind = match filter {
                    1 => "supergroupMembersFilterAdministrators",
                    2 => "supergroupMembersFilterRestricted",
                    3 => "supergroupMembersFilterBanned",
                    4 => "supergroupMembersFilterBots",
                    _ => {
                        if query.is_empty() {
                            "supergroupMembersFilterRecent"
                        } else {
                            "supergroupMembersFilterSearch"
                        }
                    }
                };
                this.scope
                    .call(
                        "getSupergroupMembers",
                        json!({"supergroup_id":id,"filter":{"@type":kind,"query":query},"offset":offset,"limit":50}),
                    )
                    .await
            } else {
                this.scope
                    .call("getBasicGroupFullInfo", json!({"basic_group_id":this.context.chat["type"]["basic_group_id"]}))
                    .await
            };
            if !alive(&this.scope, &this.dialog) || n != this.revision.get() {
                return;
            }
            let data = match result {
                Ok(v) => v,
                Err(e) => {
                    note(&this.list, &e);
                    this.busy.set(false);
                    this.more.set_sensitive(true);
                    return;
                }
            };
            let rows = data["members"].as_array().cloned().unwrap_or_default();
            let size = rows.len() as i64;
            this.offset.set(offset + size);
            for member in rows {
                let status = member["status"]["@type"].as_str().unwrap_or("");
                if this.context.supergroup().is_none()
                    && match filter {
                        1 => !matches!(status, "chatMemberStatusAdministrator" | "chatMemberStatusCreator"),
                        2 => status != "chatMemberStatusRestricted",
                        3 => status != "chatMemberStatusBanned",
                        _ => false,
                    }
                {
                    continue;
                }
                let member_id = member["member_id"].clone();
                let key = member_id.to_string();
                if !this.seen.borrow_mut().insert(key) {
                    continue;
                }
                let user = if let Some(id) = member_id["user_id"].as_i64() {
                    this.scope.call("getUser", json!({"user_id":id})).await.ok()
                } else {
                    None
                };
                if !alive(&this.scope, &this.dialog) || n != this.revision.get() {
                    return;
                }
                let title = if let Some(u) = &user { name(u) } else { format!("Chat {}", member_id["chat_id"]) };
                if (filter == 4 && user.as_ref().is_none_or(|u| u["type"]["@type"] != "userTypeBot"))
                    || (!query.is_empty() && matches!(filter, 1 | 4) && !title.to_lowercase().contains(&query.to_lowercase()))
                    || (this.context.supergroup().is_none() && !title.to_lowercase().contains(&query.to_lowercase()))
                {
                    continue;
                }
                let chat = this.chat;
                button(
                    &this.list,
                    &format!(
                        "{title} · {}{}",
                        status_label(&member["status"]),
                        member["tag"].as_str().filter(|tag| !tag.is_empty()).map(|tag| format!(" · {tag}")).unwrap_or_default()
                    ),
                    &this.scope,
                    move || member_actions(chat, member_id.clone(), title.clone()),
                );
            }
            this.busy.set(false);
            this.more.set_sensitive(true);
            this.more.set_visible(
                this.context.supergroup().is_some()
                    && size > 0
                    && this.offset.get() < data["total_count"].as_i64().unwrap_or(i64::MAX),
            );
            if this.list.first_child().is_none() {
                note(&this.list, "No members available in this view.");
            }
        });
    }
}
fn editable_target(c: &Context, member: &Value) -> Result<(), String> {
    if member["member_id"]["user_id"].as_i64() == Some(c.me) {
        return Err("Use your account settings or leave this chat to change your own membership.".into());
    }
    if member["status"]["@type"] == "chatMemberStatusCreator" {
        return Err("The owner cannot be moderated.".into());
    }
    if member["status"]["@type"] == "chatMemberStatusAdministrator" && member["status"]["can_be_edited"] != true {
        return Err("This administrator cannot be edited by your account.".into());
    }
    Ok(())
}
pub(crate) fn member_actions(chat: i64, member: Value, title: String) {
    actions::run(chat, "member-actions", move |s| async move {
        let c = context(&s, chat).await?;
        let data = s.call("getChatMember", json!({"chat_id":chat,"member_id":member})).await?;
        let (dialog, body) = w::dialog(&title, 420);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        note(&body, status_label(&data["status"]));
        if let Some(tag) = data["tag"].as_str().filter(|tag| !tag.is_empty()) {
            note(&body, &format!("Tag: {tag}"));
        }
        if let Some(user) = member["user_id"].as_i64() {
            if !c.channel() && can_edit_tag(&c, user) {
                button(&body, "Edit member tag…", &s, move || tag_editor(chat, user));
            }
        }

        if let Some(id) = member["user_id"].as_i64() {
            button(&body, "Open conversation", &s, move || {
                actions::run(0, "open-user", move |s| async move {
                    let d = s.call("createPrivateChat", json!({"user_id":id,"force":false})).await?;
                    if let Some(id) = d["id"].as_i64() {
                        window::open_chat(id);
                    }
                    Ok(())
                });
            });
        }
        if editable_target(&c, &data).is_ok() {
            if c.right("can_promote_members") && member["user_id"].is_number() {
                let m = member.clone();
                button(&body, "Administrator rights…", &s, move || admin_editor(chat, m.clone()));
            }
            if c.right("can_restrict_members") {
                if c.supergroup().is_some() && !c.channel() && member["user_id"].is_number() {
                    let m = member.clone();
                    button(&body, "Restrict permissions…", &s, move || restrict_editor(chat, m.clone()));
                }
                if data["status"]["@type"] == "chatMemberStatusBanned" {
                    let m = member.clone();
                    button(&body, "Unban…", &s, move || moderate(chat, m.clone(), false, false));
                } else {
                    let m = member.clone();
                    button(&body, "Remove member…", &s, move || moderate(chat, m.clone(), false, true));
                    let m = member.clone();
                    button(&body, "Ban member…", &s, move || moderate(chat, m.clone(), true, false));
                }
            }
        }
        dialog.present();
        Ok(())
    });
}
fn moderate(chat: i64, member: Value, ban: bool, remove: bool) {
    let description = if ban {
        "Ban this member indefinitely? In supergroups and channels Telegram also deletes all of their messages. This cannot be undone."
    } else if remove {
        "Remove this member? They may rejoin through an invitation."
    } else {
        "Allow this member to rejoin? This does not add them back to the chat."
    };
    actions::operation(
        if ban {
            "Ban member?"
        } else if remove {
            "Remove member?"
        } else {
            "Unban member?"
        },
        description,
        vec![],
        if ban {
            "Ban"
        } else if remove {
            "Remove"
        } else {
            "Unban"
        },
        move |s, _| {
            let member = member.clone();
            async move {
                let c = context(&s, chat).await?;
                c.require("can_restrict_members")?;
                let data = s.call("getChatMember", json!({"chat_id":chat,"member_id":member})).await?;
                editable_target(&c, &data)?;
                let status = data["status"]["@type"].as_str().unwrap_or("");
                if status == "chatMemberStatusAdministrator" {
                    return Err("Remove administrator privileges before moderating this member.".into());
                }
                if !ban && !remove && status != "chatMemberStatusBanned" {
                    return Err("This member is no longer banned. Refresh the list.".into());
                }
                if (ban || remove) && matches!(status, "chatMemberStatusBanned" | "chatMemberStatusLeft") {
                    return Err("This member's status changed. Refresh the list.".into());
                }
                if ban {
                    s.call(
                        "banChatMember",
                        json!({"chat_id":chat,"member_id":member,"banned_until_date":0,"revoke_messages":false}),
                    )
                    .await?;
                } else {
                    s.call(
                        "setChatMemberStatus",
                        json!({"chat_id":chat,"member_id":member,"status":{"@type":"chatMemberStatusLeft"}}),
                    )
                    .await?;
                }
                refresh(chat);
                Ok(())
            }
        },
    );
}
fn admin_editor(chat: i64, member: Value) {
    actions::run(chat, "admin-editor", move |s| async move {
        let c = context(&s, chat).await?;
        c.require("can_promote_members")?;
        let data = s.call("getChatMember", json!({"chat_id":chat,"member_id":member})).await?;
        editable_target(&c, &data)?;
        if !matches!(
            data["status"]["@type"].as_str(),
            Some("chatMemberStatusMember" | "chatMemberStatusRestricted" | "chatMemberStatusAdministrator")
        ) {
            return Err("Add or unban this person before promoting them.".into());
        }
        let original = data["status"].clone();
        let mut rights = original["rights"].clone();
        if !rights.is_object() {
            rights = json!({"@type":"chatAdministratorRights"});
        }
        let (dialog, body) = w::dialog("Administrator rights", 460);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        note(&body, "Only rights you can delegate are editable. Saving changes this member's administrator privileges.");
        let fields = scroll(&body);
        let checks = checkboxes(&fields, &rights, RIGHTS, |key| c.right(key));
        let demote = gtk::CheckButton::with_label("Remove administrator privileges");
        fields.append(&demote);
        submit(&dialog, &body, &fields, s, "Save administrator", move |s| {
            let (rights, demote, member, original) = (
                collect(rights.clone(), "chatAdministratorRights", &checks),
                demote.is_active(),
                member.clone(),
                original.clone(),
            );
            async move {
                let c = context(&s, chat).await?;
                c.require("can_promote_members")?;
                let fresh = s.call("getChatMember", json!({"chat_id":chat,"member_id":member})).await?;
                editable_target(&c, &fresh)?;
                if fresh["status"] != original {
                    return Err("This member's status changed. Reopen the editor.".into());
                }
                for (key, _) in RIGHTS {
                    if rights[*key] == true && !c.right(key) && original["rights"][*key] != true {
                        return Err("Your delegable administrator rights changed. Reopen the editor.".into());
                    }
                }
                let status = if demote {
                    json!({"@type":"chatMemberStatusMember","member_until_date":0})
                } else {
                    json!({"@type":"chatMemberStatusAdministrator","rights":rights})
                };
                s.call("setChatMemberStatus", json!({"chat_id":chat,"member_id":member,"status":status})).await?;
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn restrict_editor(chat: i64, member: Value) {
    actions::run(chat, "restrict-editor", move |s| async move {
        let c = context(&s, chat).await?;
        c.require("can_restrict_members")?;
        let data = s.call("getChatMember", json!({"chat_id":chat,"member_id":member})).await?;
        editable_target(&c, &data)?;
        if !matches!(data["status"]["@type"].as_str(), Some("chatMemberStatusMember" | "chatMemberStatusRestricted")) {
            return Err("Only current non-administrator members can be restricted here.".into());
        }
        let original = data["status"].clone();
        let restricted = original["@type"] == "chatMemberStatusRestricted";
        let permissions = if restricted { original["permissions"].clone() } else { c.chat["permissions"].clone() };
        let (dialog, body) = w::dialog("Member restrictions", 460);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        note(&body, "Checked permissions remain allowed, subject to the group's defaults.");
        let fields = scroll(&body);
        let checks = checkboxes(&fields, &permissions, PERMISSIONS, |_| true);
        let duration = gtk::DropDown::from_strings(&["Keep existing expiry", "Forever", "1 hour", "1 day", "1 week"]);
        fields.append(&duration);
        let lift = gtk::CheckButton::with_label("Remove restrictions");
        fields.append(&lift);
        submit(&dialog, &body, &fields, s, "Save restrictions", move |s| {
            let (permissions, which, lift, original, member) = (
                collect(permissions.clone(), "chatPermissions", &checks),
                duration.selected(),
                lift.is_active(),
                original.clone(),
                member.clone(),
            );
            async move {
                let c = context(&s, chat).await?;
                c.require("can_restrict_members")?;
                let fresh = s.call("getChatMember", json!({"chat_id":chat,"member_id":member})).await?;
                editable_target(&c, &fresh)?;
                if fresh["status"] != original {
                    return Err("This member's status changed. Reopen the editor.".into());
                }
                let is_member = if original["@type"] == "chatMemberStatusRestricted" {
                    original["is_member"] == true
                } else {
                    original["@type"] == "chatMemberStatusMember"
                };
                let status = if lift {
                    if is_member {
                        json!({"@type":"chatMemberStatusMember","member_until_date":0})
                    } else {
                        json!({"@type":"chatMemberStatusLeft"})
                    }
                } else {
                    let until = match which {
                        0 => original["restricted_until_date"].as_i64().unwrap_or(0),
                        1 => 0,
                        2 => chrono::Utc::now().timestamp() + 3600,
                        3 => chrono::Utc::now().timestamp() + 86400,
                        _ => chrono::Utc::now().timestamp() + 604800,
                    };
                    json!({"@type":"chatMemberStatusRestricted","is_member":is_member,"restricted_until_date":until,"permissions":permissions})
                };
                s.call("setChatMemberStatus", json!({"chat_id":chat,"member_id":member,"status":status})).await?;
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}

struct Invites {
    scope: Scope,
    chat: i64,
    dialog: glib::WeakRef<gtk::Window>,
    list: gtk::Box,
    more: gtk::Button,
    creator: gtk::DropDown,
    creators: Vec<i64>,
    revoked: gtk::CheckButton,
    revision: Cell<u64>,
    offset: RefCell<(i64, String)>,
    seen: RefCell<HashSet<String>>,
    busy: Cell<bool>,
}
fn invites(chat: i64) {
    actions::run(chat, "group-invites", move |s| async move {
        let c = context(&s, chat).await?;
        c.require("can_invite_users")?;
        let (dialog, body) = w::dialog("Invite links", 480);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        let mut creators = vec![c.me];
        let mut names = vec!["My links".to_string()];
        if c.owner() {
            if let Ok(counts) = s.call("getChatInviteLinkCounts", json!({"chat_id":chat})).await {
                for row in counts["invite_link_counts"].as_array().into_iter().flatten() {
                    if let Some(user) = row["user_id"].as_i64() {
                        if user != c.me {
                            let label = s
                                .call("getUser", json!({"user_id":user}))
                                .await
                                .map(|u| name(&u))
                                .unwrap_or_else(|_| format!("User {user}"));
                            creators.push(user);
                            names.push(label);
                        }
                    }
                }
            }
        }
        let creator = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
        body.append(&creator);
        let revoked = gtk::CheckButton::with_label("Show revoked links");
        body.append(&revoked);
        button(&body, "Create invite link…", &s, move || invite_editor(chat, Value::Null));
        if c.channel(){button(&body,"Create paid channel invitation…",&s,move||crate::commerce::subscription_invite(chat,Value::Null));}
        let list = scroll(&body);
        let more = gtk::Button::with_label("Load more");
        body.append(&more);
        let browser = Rc::new(Invites {
            scope: s,
            chat,
            dialog: dialog.downgrade(),
            list,
            more: more.clone(),
            creator: creator.clone(),
            creators,
            revoked: revoked.clone(),
            revision: Cell::new(0),
            offset: RefCell::new((0, String::new())),
            seen: RefCell::new(HashSet::new()),
            busy: Cell::new(false),
        });
        let b = Rc::downgrade(&browser);
        creator.connect_selected_notify(move |_| {
            if let Some(b) = b.upgrade() {
                b.load(true);
            }
        });
        let b = Rc::downgrade(&browser);
        revoked.connect_toggled(move |_| {
            if let Some(b) = b.upgrade() {
                b.load(true);
            }
        });
        let b = Rc::downgrade(&browser);
        more.connect_clicked(move |_| {
            if let Some(b) = b.upgrade() {
                b.load(false);
            }
        });
        let b = browser.clone();
        button(&body, "Refresh", &browser.scope, move || b.load(true));
        let keep = browser.clone();
        dialog.connect_destroy(move |_| {
            keep.revision.set(keep.revision.get() + 1);
        });
        dialog.present();
        browser.load(true);
        Ok(())
    });
}
impl Invites {
    fn load(self: &Rc<Self>, reset: bool) {
        if reset {
            self.revision.set(self.revision.get() + 1);
            *self.offset.borrow_mut() = (0, String::new());
            self.seen.borrow_mut().clear();
            clear(&self.list);
        } else if self.busy.get() {
            return;
        }
        let Some(creator) = self.creators.get(self.creator.selected() as usize).copied() else { return };
        self.busy.set(true);
        self.more.set_sensitive(false);
        let this = self.clone();
        let n = self.revision.get();
        let (date, link) = self.offset.borrow().clone();
        let revoked = self.revoked.is_active();
        glib::spawn_future_local(async move {
            let result=this.scope.call("getChatInviteLinks",json!({"chat_id":this.chat,"creator_user_id":creator,"is_revoked":revoked,"offset_date":date,"offset_invite_link":link,"limit":30})).await;
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
            let links = data["invite_links"].as_array().cloned().unwrap_or_default();
            let previous = this.offset.borrow().clone();
            for link in &links {
                let url = link["invite_link"].as_str().unwrap_or("").to_string();
                *this.offset.borrow_mut() = (link["date"].as_i64().unwrap_or(0), url.clone());
                if !this.seen.borrow_mut().insert(url.clone()) {
                    continue;
                }
                let card = w::vbox(4);
                this.list.append(&card);
                note(
                    &card,
                    &format!(
                        "{} · {} joined · {} pending",
                        link["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(if link["is_primary"] == true {
                            "Primary link"
                        } else {
                            "Invite link"
                        }),
                        link["member_count"],
                        link["pending_join_request_count"]
                    ),
                );
                let label = note(&card, &url);
                label.set_selectable(true);
                let chat = this.chat;
                button(&card, "Copy link", &this.scope, move || {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&url);
                    }
                });
                let used_link = link["invite_link"].as_str().unwrap_or("").to_string();
                button(&card, "View joined members…", &this.scope, move || {
                    crate::audit::invite_members(chat, used_link.clone())
                });
                if link["is_revoked"] != true {
                    if link["is_primary"] != true {
                        let link = link.clone();
                        button(&card, "Edit…", &this.scope, move || invite_editor(chat, link.clone()));
                    }
                    let link = link.clone();
                    button(&card, "Revoke…", &this.scope, move || revoke_invite(chat, link.clone(), false));
                } else {
                    let link = link.clone();
                    button(&card, "Delete revoked link…", &this.scope, move || revoke_invite(chat, link.clone(), true));
                }
            }
            this.more.set_visible(
                !links.is_empty()
                    && *this.offset.borrow() != previous
                    && this.seen.borrow().len() < (data["total_count"].as_u64().unwrap_or(u64::MAX) as usize),
            );
            if this.list.first_child().is_none() {
                note(&this.list, "No invite links in this view.");
            }
        });
    }
}
fn invite_editor(chat: i64, link: Value) {
    if link["subscription_pricing"].is_object(){crate::commerce::subscription_invite(chat,link);return}
    let Some(s) = Scope::current() else { return };
    let edit = link.is_object();
    let (dialog, body) = w::dialog(if edit { "Edit invite link" } else { "Create invite link" }, 440);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    let fields = w::vbox(8);
    body.append(&fields);
    let name = entry(&fields, "Label (only administrators see this)", link["name"].as_str().unwrap_or(""), 32);
    note(&fields, "Expires");
    let expiry = gtk::DropDown::from_strings(&["Keep current / never", "Never", "In 1 hour", "In 1 day", "In 1 week"]);
    fields.append(&expiry);
    let limit = gtk::SpinButton::with_range(0.0, 99999.0, 1.0);
    limit.set_value(link["member_limit"].as_f64().unwrap_or(0.0));
    note(&fields, "Member limit (0 means unlimited)");
    fields.append(&limit);
    let approval = gtk::CheckButton::with_label("Require administrator approval");
    approval.set_active(link["creates_join_request"] == true);
    fields.append(&approval);
    let l = limit.clone();
    approval.connect_toggled(move |c| {
        l.set_sensitive(!c.is_active());
        if c.is_active() {
            l.set_value(0.0);
        }
    });
    limit.set_sensitive(!approval.is_active());
    submit(&dialog, &body, &fields, s, if edit { "Save link" } else { "Create link" }, move |s| {
        let (name, which, limit, approval, link) =
            (name.text().to_string(), expiry.selected(), limit.value_as_int(), approval.is_active(), link.clone());
        async move {
            let c = context(&s, chat).await?;
            c.require("can_invite_users")?;
            if !c.admin() {
                return Err("Administrator privileges are required.".into());
            }
            if edit {
                let fresh = s.call("getChatInviteLink", json!({"chat_id":chat,"invite_link":link["invite_link"]})).await?;
                if fresh["is_revoked"] == true {
                    return Err("This link was revoked. Refresh the list.".into());
                }
                if !c.owner() && fresh["creator_user_id"].as_i64() != Some(c.me) {
                    return Err("Only the owner can edit another administrator's link.".into());
                }
                if fresh["expiration_date"] != link["expiration_date"]
                    || fresh["member_limit"] != link["member_limit"]
                    || fresh["creates_join_request"] != link["creates_join_request"]
                    || fresh["name"] != link["name"]
                {
                    return Err("This link changed. Reopen its editor.".into());
                }
            }
            let date = match which {
                0 => link["expiration_date"].as_i64().unwrap_or(0),
                1 => 0,
                2 => chrono::Utc::now().timestamp() + 3600,
                3 => chrono::Utc::now().timestamp() + 86400,
                _ => chrono::Utc::now().timestamp() + 604800,
            };
            let data=commit(&s,if edit{"editChatInviteLink"}else{"createChatInviteLink"},json!({"chat_id":chat,"invite_link":link["invite_link"],"name":name,"expiration_date":date,"member_limit":if approval{0}else{limit},"creates_join_request":approval})).await?;
            let (d, b) = w::dialog("Invite link", 440);
            crate::dialog_dismissal::add(&d, &b, "Close");
            let url = data["invite_link"].as_str().unwrap_or("").to_string();
            let label = note(&b, &url);
            label.set_selectable(true);
            button(&b, "Copy link", &s, move || {
                if let Some(d) = gdk::Display::default() {
                    d.clipboard().set_text(&url);
                }
            });
            d.present();
            Ok(())
        }
    });
    dialog.present();
}
fn revoke_invite(chat: i64, link: Value, delete: bool) {
    actions::operation(
        if delete { "Delete revoked link?" } else { "Revoke invite link?" },
        if delete {
            "Remove this revoked link from the administrator list?"
        } else {
            "This link will stop admitting new members. Current members stay in the chat. Refresh the list afterward."
        },
        vec![],
        if delete { "Delete" } else { "Revoke" },
        move |s, _| {
            let link = link.clone();
            async move {
                let c = context(&s, chat).await?;
                c.require("can_invite_users")?;
                s.call(
                    if delete { "deleteRevokedChatInviteLink" } else { "revokeChatInviteLink" },
                    json!({"chat_id":chat,"invite_link":link["invite_link"]}),
                )
                .await?;
                Ok(())
            }
        },
    );
}

struct Requests {
    scope: Scope,
    chat: i64,
    dialog: glib::WeakRef<gtk::Window>,
    query: gtk::SearchEntry,
    list: gtk::Box,
    more: gtk::Button,
    revision: Cell<u64>,
    offset: RefCell<Value>,
    seen: RefCell<HashSet<i64>>,
    busy: Cell<bool>,
}
fn join_requests(chat: i64) {
    actions::run(chat, "join-requests", move |s| async move {
        context(&s, chat).await?.require("can_invite_users")?;
        let (dialog, body) = w::dialog("Join requests", 460);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        let query = gtk::SearchEntry::new();
        query.set_placeholder_text(Some("Search requests"));
        body.append(&query);
        let list = scroll(&body);
        let more = gtk::Button::with_label("Load more");
        body.append(&more);
        let b = Rc::new(Requests {
            scope: s,
            chat,
            dialog: dialog.downgrade(),
            query: query.clone(),
            list,
            more: more.clone(),
            revision: Cell::new(0),
            offset: RefCell::new(Value::Null),
            seen: RefCell::new(HashSet::new()),
            busy: Cell::new(false),
        });
        let c = Rc::downgrade(&b);
        query.connect_search_changed(move |_| {
            if let Some(c) = c.upgrade() {
                c.load(true);
            }
        });
        let c = Rc::downgrade(&b);
        more.connect_clicked(move |_| {
            if let Some(c) = c.upgrade() {
                c.load(false);
            }
        });
        let c = b.clone();
        button(&body, "Refresh", &b.scope, move || c.load(true));
        let keep = b.clone();
        dialog.connect_destroy(move |_| {
            keep.revision.set(keep.revision.get() + 1);
        });
        dialog.present();
        b.load(true);
        Ok(())
    });
}
impl Requests {
    fn load(self: &Rc<Self>, reset: bool) {
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
        let this = self.clone();
        let n = self.revision.get();
        let offset = self.offset.borrow().clone();
        let query = self.query.text().to_string();
        glib::spawn_future_local(async move {
            let result = this
                .scope
                .call(
                    "getChatJoinRequests",
                    json!({"chat_id":this.chat,"invite_link":"","query":query,"offset_request":offset,"limit":30}),
                )
                .await;
            if !alive(&this.scope, &this.dialog) || n != this.revision.get() {
                return;
            }
            let data = match result {
                Ok(v) => v,
                Err(e) => {
                    note(&this.list, &e);
                    this.busy.set(false);
                    this.more.set_sensitive(true);
                    return;
                }
            };
            let requests = data["requests"].as_array().cloned().unwrap_or_default();
            for request in &requests {
                *this.offset.borrow_mut() = request.clone();
                let Some(user) = request["user_id"].as_i64() else { continue };
                if !this.seen.borrow_mut().insert(user) {
                    continue;
                }
                let title = this
                    .scope
                    .call("getUser", json!({"user_id":user}))
                    .await
                    .map(|u| name(&u))
                    .unwrap_or_else(|_| format!("User {user}"));
                if !alive(&this.scope, &this.dialog) || n != this.revision.get() {
                    return;
                }
                let card = w::vbox(4);
                this.list.append(&card);
                note(&card, &title);
                note(&card, request["bio"].as_str().unwrap_or(""));
                for approve in [true, false] {
                    let chat = this.chat;
                    let weak = card.downgrade();
                    let browser = Rc::downgrade(&this);
                    let title = title.clone();
                    button(&card, if approve { "Approve…" } else { "Decline…" }, &this.scope, move || {
                        let (weak, browser) = (weak.clone(), browser.clone());
                        actions::operation(
                            if approve { "Approve request?" } else { "Decline request?" },
                            &format!("{} {title}'s request to join?", if approve { "Approve" } else { "Decline" }),
                            vec![],
                            if approve { "Approve" } else { "Decline" },
                            move |s, _| {
                                let (weak, browser) = (weak.clone(), browser.clone());
                                async move {
                                    context(&s, chat).await?.require("can_invite_users")?;
                                    s.call("processChatJoinRequest", json!({"chat_id":chat,"user_id":user,"approve":approve}))
                                        .await?;
                                    if let Some(card) = weak.upgrade() {
                                        card.set_sensitive(false);
                                        note(&card, if approve { "Approved" } else { "Declined" });
                                    }
                                    if let Some(b) = browser.upgrade() {
                                        b.seen.borrow_mut().remove(&user);
                                    }
                                    Ok(())
                                }
                            },
                        );
                    });
                }
            }
            this.busy.set(false);
            this.more.set_sensitive(true);
            this.more.set_visible(!requests.is_empty() && *this.offset.borrow() != offset);
            if this.list.first_child().is_none() {
                note(&this.list, "No pending requests.");
            }
        });
    }
}
fn transfer(chat: i64, user: Value) {
    actions::run(chat, "transfer-owner", move |s| async move {
        let c = context(&s, chat).await?;
        if !c.owner() {
            return Err("Only the owner can transfer ownership.".into());
        }
        if user["type"]["@type"] != "userTypeRegular" || user["id"].as_i64() == Some(c.me) {
            return Err("Choose another active person as the new owner.".into());
        }
        let eligibility = s.call("canTransferOwnership", json!({})).await?;
        match eligibility["@type"].as_str().unwrap_or("") {
            "canTransferOwnershipResultOk" => {}
            "canTransferOwnershipResultPasswordNeeded" => {
                return Err("Enable two-step verification in Telegram before transferring ownership.".into());
            }
            _ => {
                return Err(format!(
                    "Telegram requires a security waiting period before ownership can be transferred ({} seconds remaining).",
                    eligibility["retry_after"]
                ));
            }
        }
        let (dialog, body) = w::dialog("Transfer ownership", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        note(&fields, &format!("{} will become the owner. You will give up ownership of this chat.", name(&user)));
        note(&fields, "Your two-step verification password");
        let password = gtk::PasswordEntry::new();
        password.set_show_peek_icon(true);
        fields.append(&password);
        let confirm = gtk::CheckButton::with_label("I understand that I am giving up ownership");
        fields.append(&confirm);
        submit(&dialog, &body, &fields, s, "Transfer ownership", move |s| {
            let (password, confirm, user) = (password.text().to_string(), confirm.is_active(), user["id"].clone());
            async move {
                if !confirm || password.is_empty() {
                    return Err("Confirm the transfer and enter your two-step verification password.".into());
                }
                if !context(&s, chat).await?.owner() {
                    return Err("You are no longer the owner.".into());
                }
                if s.call("canTransferOwnership", json!({})).await?["@type"] != "canTransferOwnershipResultOk" {
                    return Err("Telegram no longer permits this transfer. Reopen this dialog.".into());
                }
                s.call("transferChatOwnership", json!({"chat_id":chat,"user_id":user,"password":password})).await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn joined(data: &Value) {
    match data["@type"].as_str().unwrap_or("") {
        "chatJoinResultSuccess" => {
            if let Some(id) = data["chat_id"].as_i64() {
                refresh(id);
                window::open_chat(id);
            }
        }
        "chatJoinResultRequestSent" => window::show_status("Your join request was sent for administrator approval."),
        "chatJoinResultGuardBotApprovalRequired" => window::show_status(
            "This chat requires approval through its guard bot. Complete verification in Telegram before joining.",
        ),
        "chatJoinResultDeclined" => window::show_status("Telegram declined the join request."),
        _ => window::show_status("Telegram returned an unfamiliar join result. Check your chat list before retrying."),
    }
}
pub fn join_public(chat: i64) {
    let title = window::chat(chat).map(|c| c.title).unwrap_or_else(|| "this chat".into());
    actions::operation(
        "Join chat?",
        &format!("Join {title}? Telegram may require administrator approval."),
        vec![],
        "Join",
        move |s, _| async move {
            let result = s.call("joinChat", json!({"chat_id":chat})).await?;
            joined(&result);
            Ok(())
        },
    );
}
fn invite_prompt() {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Open invitation", 440);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    let fields = w::vbox(8);
    body.append(&fields);
    let link = entry(&fields, "Telegram invite link", "", 2048);
    submit(&dialog, &body, &fields, scope, "Preview invitation", move |s| {
        let link = link.text().trim().to_string();
        async move {
            let kind = s.call("getInternalLinkType", json!({"link":link})).await?;
            match kind["@type"].as_str() {
                Some("internalLinkTypeChatInvite") => {
                    if let Some(url) = kind["invite_link"].as_str() {
                        import_link(url.into());
                    }
                }
                Some("internalLinkTypeChatFolderInvite") => {
                    if let Some(url) = kind["invite_link"].as_str() {
                        crate::folders::import_link(url.into());
                    }
                }
                _ => return Err("Enter a group, channel or shared-folder invite link.".into()),
            }
            Ok(())
        }
    });
    dialog.present();
}
pub fn update(value: &Value) {
    let (field, id) = match value["@type"].as_str() {
        Some("updateSupergroup") => ("supergroup_id", value["supergroup"]["id"].as_i64()),
        Some("updateBasicGroup") => ("basic_group_id", value["basic_group"]["id"].as_i64()),
        _ => return,
    };
    if let Some(id) = id {
        for chat in window::list_chats(usize::MAX) {
            if chat.data["type"][field].as_i64() == Some(id) {
                refresh(chat.id);
            }
        }
    }
}
pub fn import_link(link: String) {
    actions::run(0, "chat-invite-preview", move |s| async move {
        let info = s.call("checkChatInviteLink", json!({"invite_link":link})).await?;
        let (dialog, body) = w::dialog("Chat invitation", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        note(&body, info["title"].as_str().unwrap_or("Telegram chat"));
        note(&body, info["description"].as_str().unwrap_or(""));
        note(&body, &format!("{} members", info["member_count"]));
        let fields = w::vbox(8);
        body.append(&fields);
        if !info["subscription_info"].is_null() {
            note(
                &fields,
                "This invitation requires a paid subscription. Open it in the official Telegram app to review and purchase.",
            );
        } else {
            note(
                &fields,
                if info["creates_join_request"] == true {
                    "Joining sends a request for administrator approval."
                } else {
                    "Join this group or channel?"
                },
            );
            submit(&dialog, &body, &fields, s, "Join", move |s| {
                let link = link.clone();
                async move {
                    let info = s.call("checkChatInviteLink", json!({"invite_link":link})).await?;
                    if !info["subscription_info"].is_null() {
                        return Err("This invitation now requires payment. Review it in the official Telegram app.".into());
                    }
                    let result = s.call("joinChatByInviteLink", json!({"invite_link":link})).await?;
                    joined(&result);
                    Ok(())
                }
            });
        }
        dialog.present();
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permissions_and_member_protection() {
        let c = Context {
            chat: json!({}),
            group: json!({"status":{"@type":"chatMemberStatusAdministrator","rights":{"can_invite_users":true}}}),
            me: 7,
        };
        assert!(c.require("can_invite_users").is_ok());
        assert!(c.require("can_restrict_members").is_err());
        assert!(editable_target(&c, &json!({"member_id":{"user_id":7},"status":{"@type":"chatMemberStatusMember"}})).is_err());
        assert!(editable_target(&c, &json!({"member_id":{"user_id":8},"status":{"@type":"chatMemberStatusCreator"}})).is_err());
        assert!(
            editable_target(
                &c,
                &json!({"member_id":{"user_id":8},"status":{"@type":"chatMemberStatusAdministrator","can_be_edited":false}})
            )
            .is_err()
        );
        assert!(editable_target(&c, &json!({"member_id":{"user_id":8},"status":{"@type":"chatMemberStatusMember"}})).is_ok());
    }
}

fn can_edit_tag(c: &Context, user: i64) -> bool {
    if user != c.me {
        return c.right("can_manage_tags");
    }
    if c.admin() {
        return true;
    }
    if c.group["status"]["@type"] == "chatMemberStatusRestricted" {
        c.group["status"]["permissions"]["can_edit_tag"] == true
    } else {
        c.chat["permissions"]["can_edit_tag"] == true
    }
}
fn tag_editor(chat: i64, user: i64) {
    actions::run(chat, "member-tag", move |s| async move {
        let c = context(&s, chat).await?;
        if c.channel() || !can_edit_tag(&c, user) {
            return Err("You cannot edit this member's tag.".into());
        }
        let member =
            s.call("getChatMember", json!({"chat_id":chat,"member_id":{"@type":"messageSenderUser","user_id":user}})).await?;
        let original = member["tag"].as_str().unwrap_or("").to_string();
        let (dialog, body) = w::dialog("Member tag", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        let tag = entry(&fields, "Tag or administrator title", &original, 16);
        note(&fields, "Up to 16 characters, without emoji. Leave blank to remove the tag.");
        submit(&dialog, &body, &fields, s, "Save tag", move |s| {
            let tag = tag.text().trim().to_string();
            let original = original.clone();
            async move {
                let c = context(&s, chat).await?;
                if c.channel() || !can_edit_tag(&c, user) {
                    return Err("Your permissions changed. You cannot edit this tag.".into());
                }
                let member = s
                    .call("getChatMember", json!({"chat_id":chat,"member_id":{"@type":"messageSenderUser","user_id":user}}))
                    .await?;
                if member["tag"].as_str().unwrap_or("") != original {
                    return Err("This tag changed elsewhere. Reopen the editor.".into());
                }
                s.call("setChatMemberTag", json!({"chat_id":chat,"user_id":user,"tag":tag})).await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
#[cfg(test)]
mod tag_tests {
    use super::*;
    #[test]
    fn stage3d_tag_permissions_respect_self_restrictions() {
        let mut c = Context {
            chat: json!({"permissions":{"can_edit_tag":true}}),
            group: json!({"status":{"@type":"chatMemberStatusRestricted","permissions":{"can_edit_tag":false}}}),
            me: 7,
        };
        assert!(!can_edit_tag(&c, 7));
        assert!(!can_edit_tag(&c, 8));
        c.group = json!({"status":{"@type":"chatMemberStatusMember"}});
        assert!(can_edit_tag(&c, 7));
        assert!(!can_edit_tag(&c, 8));
        c.group = json!({"status":{"@type":"chatMemberStatusAdministrator","rights":{"can_manage_tags":true}}});
        assert!(can_edit_tag(&c, 8));
    }
}
