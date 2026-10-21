//! Shared Telegram actions for sidebar menus, conversation menus, and message menus.
use crate::{service::TelegramService, window};
use gtk::{gdk, gio, glib, prelude::*};
use nixe_telegram_core::models::*;
use nixe_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    future::Future,
    rc::Rc,
};

thread_local! { static DEFAULT_REACTION: RefCell<Value> = const { RefCell::new(Value::Null) }; static PENDING: RefCell<HashSet<(String, i64, String)>> = RefCell::new(HashSet::new()); }
#[derive(Clone)]
pub struct Scope {
    service: TelegramService,
}
impl Scope {
    pub fn current() -> Option<Self> {
        window::service().map(|service| Self { service })
    }
    pub fn valid(&self) -> bool {
        window::service_is_current(&self.service.generation)
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        if !self.valid() {
            return Err("Account changed".into());
        }
        let chat = params["chat_id"].as_i64();
        let result = self.service.request(method, params).recv().await.map_err(|_| "Telegram service stopped".to_owned())?;
        if !self.valid() {
            return Err("Account changed".into());
        }
        if result.is_ok()
            && matches!(method, "editMessageText" | "editMessageCaption" | "deleteMessages" | "editMessageSchedulingState")
        {
            if let Some(chat) = chat {
                crate::message_tools::changed(chat);
            }
        }
        result
    }
    pub fn error(&self, error: &str) {
        if self.valid() {
            window::show_status(error);
        }
    }
}

pub fn run<F, Fut>(chat: i64, key: &str, work: F)
where
    F: FnOnce(Scope) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let Some(scope) = Scope::current() else {
        window::show_status("Connect to Telegram first");
        return;
    };
    let key = (scope.service.generation.clone(), chat, key.to_owned());
    if !PENDING.with(|p| p.borrow_mut().insert(key.clone())) {
        return;
    }
    glib::spawn_future_local(async move {
        let result = work(scope.clone()).await;
        PENDING.with(|p| p.borrow_mut().remove(&key));
        if let Err(error) = result {
            scope.error(&error);
        }
    });
}

pub fn request(chat: i64, method: &'static str, params: Value) {
    run(chat, method, move |scope| async move {
        scope.call(method, params).await?;
        scope.service.load_chat(chat);
        Ok(())
    });
}

pub fn popover(parent: &gtk::Widget, position: Option<(f64, f64)>) -> (gtk::Popover, gtk::Box) {
    let p = gtk::Popover::new();
    p.set_parent(parent);
    p.set_has_arrow(false);
    p.add_css_class("menu-popover");
    if let Some((x, y)) = position {
        p.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    }
    let body = w::vbox(2);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(520)
        .child(&body)
        .build();
    p.set_child(Some(&scroll));
    p.connect_closed(|p| p.unparent());
    let weak = p.downgrade();
    parent.connect_unmap(move |_| {
        if let Some(p) = weak.upgrade() {
            p.popdown();
        }
    });
    (p, body)
}
pub fn menu_item(p: &gtk::Popover, body: &gtk::Box, title: &str, action: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(title);
    button.add_css_class("flat");
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        label.set_xalign(0.0);label.set_wrap(true);label.set_wrap_mode(gtk::pango::WrapMode::WordChar);label.set_max_width_chars(34);
    }
    button.set_halign(gtk::Align::Fill);
    let weak = p.downgrade();
    button.connect_clicked(move |_| {
        if let Some(p) = weak.upgrade() {
            p.popdown();
        }
        action();
    });
    body.append(&button);
    button
}

pub fn show_chat_menu(parent: &gtk::Widget, id: i64, position: Option<(f64, f64)>) {
    let Some(chat) = window::chat(id) else { return };
    let (p, body) = popover(parent, position);
    menu_item(&p, &body, "View information", move || window::show_chat_info(id));
    menu_item(&p, &body, "Search messages", move || window::search_conversation(id));
    menu_item(&p, &body, "Bot commands / keyboard…", move || crate::bots::menu(id));
    menu_item(&p, &body, "Inline bot search…", move || crate::bots::inline(id));
    menu_item(&p, &body, "Stories…", move || crate::stories::chat_stories(id,false));
    menu_item(&p, &body, "Open in new window", move || crate::conversation_windows::open(id));
    menu_item(&p, &body, "Notification settings…", move || crate::notifications::chat(id));
    menu_item(&p, &body, "Export conversation…", move || crate::export_bundle::open(id));
    if matches!(chat.chat_type,ChatType::BasicGroup|ChatType::Supergroup|ChatType::Channel) {
        menu_item(&p,&body,"Group call…",move||crate::group_calls::open(id));
        menu_item(&p,&body,"Statistics…",move||crate::statistics::open(id,None));
    }
    if chat.chat_type==ChatType::Channel {menu_item(&p,&body,"Boosts…",move||crate::commerce::boost(id));}
    if chat.data["pulse_is_forum"]==true {menu_item(&p,&body,"Topics…",move ||crate::topics::browse(id));}
    let unread = chat.unread_count > 0 || chat.is_marked_unread();
    menu_item(&p, &body, if unread { "Mark as read" } else { "Mark as unread" }, move || {
        run(id, "mark-unread", move |scope| async move {
            if unread {
                let chat = scope.call("getChat", json!({"chat_id":id})).await?;
                if let Some(last) = chat["last_message"]["id"].as_i64() {
                    scope.call("viewMessages", json!({"chat_id":id,"message_ids":[last],"source":{"@type":"messageSourceChatHistory"},"force_read":true})).await?;
                }
            } else if window::active_chat_id() == Some(id) {
                window::leave_conversation(id);
            }
            scope.call("toggleChatIsMarkedAsUnread", json!({"chat_id":id,"is_marked_as_unread":!unread})).await?;
            scope.service.load_chat(id);
            Ok(())
        });
    });
    if matches!(chat.chat_type, ChatType::BasicGroup | ChatType::Supergroup | ChatType::Channel) {
        menu_item(&p, &body, "Group / channel management…", move || crate::groups::manage(id));
    }
    if matches!(chat.chat_type,ChatType::Supergroup|ChatType::Channel){menu_item(&p,&body,"Send as…",move ||crate::identities::choose(id));}
    menu_item(&p, &body, "Folders…", move || crate::folders::membership(id));
    menu_item(&p, &body, "Scheduled messages", move || crate::message_tools::scheduled(id));
    menu_item(&p, &body, "Auto-delete messages…", move || crate::message_tools::auto_delete(id));
    let draft_topic = window::composer_topic(id);
    let draft_key = crate::topics::draft_key(&crate::prefs::get().active_account_id,id,&draft_topic);
    if (draft_topic.is_null() && !chat.data["draft_message"].is_null()) || crate::prefs::get().drafts.contains_key(&draft_key) {
        menu_item(&p, &body, "Discard draft…", move || {
            let topic=draft_topic.clone();
            operation("Discard draft?","This removes the unsent draft from this conversation or topic. Sent messages are kept.",vec![],"Discard draft",move |scope,_|{
                let topic=topic.clone();async move {
                    window::with_service(|s|s.cancel_topic_draft(id,&topic));
                    scope.call("setChatDraftMessage",json!({"chat_id":id,"topic_id":topic,"draft_message":null})).await?;
                    window::discard_topic_draft(id,&topic);Ok(())
                }
            });
        });
    }
    let list = window::current_chat_list();
    let pinned = chat.list_position(list).is_some_and(|(_, p)| p);
    menu_item(&p, &body, if pinned { "Unpin chat" } else { "Pin chat" }, move || {
        request(id, "toggleChatIsPinned", json!({"chat_id":id,"chat_list":list.to_td(),"is_pinned":!pinned}))
    });
    let archived = chat.list_position(ChatListId::Archive).is_some();
    menu_item(&p, &body, if archived { "Unarchive" } else { "Archive" }, move || {
        request(
            id,
            "addChatToList",
            json!({"chat_id":id,"chat_list":if archived { ChatListId::Main } else { ChatListId::Archive }.to_td()}),
        );
    });
    if chat.is_muted {
        menu_item(&p, &body, "Unmute", move || set_mute(id, 0));
    } else {
        menu_item(&p, &body, "Mute…", move || mute_dialog(id));
    }
    menu_item(&p, &body, "Notification settings…", move || mute_dialog(id));
    menu_item(&p, &body, "Pinned messages", move || pinned_messages(id));
    if chat.data["can_be_deleted_only_for_self"] == true || chat.data["can_be_deleted_for_all_users"] == true {
        menu_item(&p, &body, "Clear history…", move || clear_history(id));
    }
    if let Some(user) = chat.data["type"]["user_id"].as_i64() {
        let blocked = chat.data["block_list"]["@type"] == "blockListMain";
        menu_item(&p, &body, if blocked { "Unblock" } else { "Block…" }, move || block_user(id, user, !blocked));
    }
    if chat.data["can_be_reported"] == true {
        menu_item(&p, &body, "Report…", move || report(id, Vec::new(), String::new(), String::new()));
    }
    let button = menu_item(
        &p,
        &body,
        if chat.chat_type == ChatType::Private { "Delete chat…" } else { "Leave and delete…" },
        move || window::request_delete_chat(id),
    );
    button.add_css_class("destructive-action");
    p.popup();
}

fn set_mute(id: i64, seconds: i32) {
    run(id, "mute", move |scope| async move {
        let chat = scope.call("getChat", json!({"chat_id":id})).await?;
        let mut settings = chat["notification_settings"].clone();
        settings["@type"] = json!("chatNotificationSettings");
        settings["use_default_mute_for"] = json!(false);
        settings["mute_for"] = json!(seconds);
        scope.call("setChatNotificationSettings", json!({"chat_id":id,"notification_settings":settings})).await?;
        scope.service.load_chat(id);
        if seconds > 0 && seconds < i32::MAX {
            let service = scope.service.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(seconds as u64 + 1), move || {
                if window::service_is_current(&service.generation) {
                    service.load_chat(id);
                }
            });
        }
        Ok(())
    });
}
fn mute_dialog(id: i64) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Notifications", 360);
    for (title, seconds) in [
        ("Unmute", 0),
        ("Mute for 1 hour", 3600),
        ("Mute for 8 hours", 28800),
        ("Mute for 2 days", 172800),
        ("Mute forever", i32::MAX),
    ] {
        let button = gtk::Button::with_label(title);
        let weak = dialog.downgrade();
        let scope = scope.clone();
        button.connect_clicked(move |_| {
            if scope.valid() {
                set_mute(id, seconds);
            }
            if let Some(d) = weak.upgrade() {
                d.close();
            }
        });
        body.append(&button);
    }
    let custom = gtk::SpinButton::with_range(1.0, 525600.0, 1.0);
    custom.set_value(60.0);
    body.append(&w::row("Custom duration", "Minutes", Some(custom.upcast_ref())));
    let button = gtk::Button::with_label("Mute for custom duration");
    let weak = dialog.downgrade();
    button.connect_clicked(move |_| {
        if scope.valid() {
            set_mute(id, custom.value_as_int().saturating_mul(60));
        }
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    body.append(&button);
    dialog.present();
}

pub(crate) fn operation<F, Fut>(title: &str, description: &str, options: Vec<(&str, bool, bool)>, action_label: &str, work: F)
where
    F: Fn(Scope, Vec<bool>) -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog(title, 440);
    let label = w::label(description, "dim");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(42);
    body.append(&label);
    let checks = options
        .into_iter()
        .map(|(text, active, sensitive)| {
            let c = gtk::CheckButton::with_label(text);
            c.set_active(active);
            c.set_sensitive(sensitive);
            body.append(&c);
            c
        })
        .collect::<Vec<_>>();
    let error = w::label("", "error");
    error.set_wrap(true);
    error.set_max_width_chars(42);
    body.append(&error);
    let actions = w::hbox(8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let confirm = gtk::Button::with_label(action_label);
    confirm.add_css_class("destructive-action");
    actions.append(&cancel);
    actions.append(&confirm);
    body.append(&actions);
    let busy = Rc::new(Cell::new(false));
    let state = busy.clone();
    dialog.connect_close_request(move |_| if state.get() { glib::Propagation::Stop } else { glib::Propagation::Proceed });
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close()
        }
    });
    let weak = dialog.downgrade();
    confirm.connect_clicked(move |confirm| {
        let Some(dialog) = weak.upgrade() else { return };
        if !scope.valid() {
            dialog.close();
            return;
        }
        if busy.replace(true) {
            return;
        }
        let future = work(scope.clone(), checks.iter().map(|c| c.is_active()).collect());
        confirm.set_sensitive(false);
        cancel.set_sensitive(false);
        error.set_text("");
        let (confirm, cancel, error, busy, scope) = (confirm.clone(), cancel.clone(), error.clone(), busy.clone(), scope.clone());
        glib::spawn_future_local(async move {
            let result = future.await;
            busy.set(false);
            if !scope.valid() {
                dialog.close();
                return;
            }
            match result {
                Ok(()) => dialog.close(),
                Err(e) => {
                    error.set_text(&e);
                    confirm.set_sensitive(true);
                    cancel.set_sensitive(true);
                }
            }
        });
    });
    dialog.present();
    cancel_focus(&dialog);
}
fn cancel_focus(dialog: &gtk::Window) {
    dialog.set_default_widget(gtk::Widget::NONE);
}
fn clear_history(id: i64) {
    let Some(chat) = window::chat(id) else { return };
    let all = chat.data["can_be_deleted_for_all_users"] == true;
    let local = chat.data["can_be_deleted_only_for_self"] == true;
    operation(
        "Clear history?",
        &format!("Clear messages in {}? The conversation remains in your list. This cannot be undone.", chat.title),
        vec![("Also delete for other participants", false, all)],
        "Clear history",
        move |scope, options| async move {
            let revoke = options[0];
            let chat = scope.call("getChat", json!({"chat_id":id})).await?;
            if chat[if revoke { "can_be_deleted_for_all_users" } else { "can_be_deleted_only_for_self" }] != true {
                return Err(if !local {
                    "Only deletion for all participants is available; select it explicitly."
                } else {
                    "Telegram does not allow this operation."
                }
                .into());
            }
            scope.call("deleteChatHistory", json!({"chat_id":id,"remove_from_chat_list":false,"revoke":revoke})).await?;
            window::refresh_conversation(id);
            Ok(())
        },
    );
}
fn block_user(chat: i64, user: i64, block: bool) {
    if !block {
        request(
            chat,
            "setMessageSenderBlockList",
            json!({"sender_id":{"@type":"messageSenderUser","user_id":user},"block_list":null}),
        );
        return;
    }
    operation(
        "Block contact?",
        "This contact will be blocked. You can unblock them later.",
        vec![],
        "Block",
        move |scope, _| async move {
            scope
                .call(
                    "setMessageSenderBlockList",
                    json!({"sender_id":{"@type":"messageSenderUser","user_id":user},"block_list":{"@type":"blockListMain"}}),
                )
                .await?;
            scope.service.load_chat(chat);
            Ok(())
        },
    );
}

pub fn report(chat: i64, ids: Vec<i64>, option: String, text: String) {
    run(chat, "report", move |scope| async move {
        let result = scope.call("reportChat", json!({"chat_id":chat,"option_id":option,"message_ids":ids,"text":text})).await?;
        match result["@type"].as_str() {
            Some("reportChatResultOk") => window::show_status("Report submitted"),
            Some("reportChatResultOptionRequired") => {
                let (dialog, body) = w::dialog(result["title"].as_str().unwrap_or("Report"), 400);
                for option in result["options"].as_array().into_iter().flatten() {
                    let Some(id) = option["id"].as_str() else { continue };
                    let id = id.to_owned();
                    let ids = ids.clone();
                    let scope = scope.clone();
                    let weak = dialog.downgrade();
                    let b = gtk::Button::with_label(option["text"].as_str().unwrap_or("Report"));
                    b.connect_clicked(move |_| {
                        if scope.valid() {
                            report(chat, ids.clone(), id.clone(), String::new());
                        }
                        if let Some(d) = weak.upgrade() {
                            d.close()
                        }
                    });
                    body.append(&b);
                }
                dialog.present();
            }
            Some("reportChatResultTextRequired") => {
                let option = result["option_id"].as_str().unwrap_or("").to_owned();
                let optional = result["is_optional"] == true;
                let (dialog, body) = w::dialog("Report details", 420);
                let entry = gtk::Entry::new();
                entry.set_placeholder_text(Some("Describe the problem"));
                body.append(&entry);
                let send = gtk::Button::with_label("Submit report");
                let weak = dialog.downgrade();
                send.connect_clicked(move |_| {
                    let text = entry.text().to_string();
                    if !optional && text.trim().is_empty() {
                        return;
                    }
                    if scope.valid() {
                        report(chat, ids.clone(), option.clone(), text)
                    }
                    if let Some(d) = weak.upgrade() {
                        d.close()
                    }
                });
                body.append(&send);
                dialog.present();
            }
            Some("reportChatResultMessagesRequired") => {
                window::open_chat(chat);
                window::select_messages_for_report();
                window::show_status("Select the messages to report, then choose Report");
            }
            _ => return Err("Telegram returned an unsupported reporting step".into()),
        }
        Ok(())
    });
}

pub fn pinned_messages(chat: i64) {
    crate::message_tools::pinned(chat);
}

pub fn message_text(message: &MessageSummary) -> Value {
    let c = &message.data["content"];
    if c["@type"] == "messageText" {
        c["text"].clone()
    } else {
        c.get("caption").cloned().unwrap_or(json!({"@type":"formattedText","text":"","entities":[]}))
    }
}
fn protected(message: &MessageSummary) -> bool {
    message.data["has_protected_content"] == true
        || window::chat(message.chat_id).is_some_and(|c| c.data["has_protected_content"] == true)
}
fn selected_text(widget: &gtk::Widget) -> Option<String> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        if let Some((a, b)) = label.selection_bounds() {
            let text = label.text();
            return Some(text.chars().skip(a.min(b) as usize).take(a.abs_diff(b) as usize).collect());
        }
    }
    if let Some(view) = widget.downcast_ref::<gtk::TextView>() {
        let b = view.buffer();
        if let Some((a, z)) = b.selection_bounds() {
            return Some(b.text(&a, &z, false).to_string());
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(text) = selected_text(&c) {
            return Some(text);
        }
        child = c.next_sibling()
    }
    None
}

pub fn show_message_menu(view: &Rc<crate::chat_view::ChatView>, parent: &gtk::Widget, id: i64, position: Option<(f64, f64)>) {
    let Some(message) = view.message(id) else { return };
    let Some(scope) = Scope::current() else { return };
    let quote = selected_text(parent);
    let (p, body) = popover(parent, position);
    body.append(&w::label("Loading actions…", "dim"));
    p.popup();
    let weak = p.downgrade();
    let view = Rc::downgrade(view);
    glib::spawn_future_local(async move {
        let result = scope.call("getMessageProperties", json!({"chat_id":message.chat_id,"message_id":id})).await;
        let (Some(p), Some(view)) = (weak.upgrade(), view.upgrade()) else { return };
        if !p.is_visible() || view.current_chat_id.borrow().as_ref() != Some(&message.chat_id) {
            return;
        }
        while let Some(c) = body.first_child() {
            body.remove(&c)
        }
        let props = match result {
            Ok(v) => v,
            Err(e) => {
                body.append(&w::label(&e, "error"));
                return;
            }
        };
        if props["can_get_message_thread"]==true {
            let chat=message.chat_id;menu_item(&p,&body,"Replies / comments…",move ||crate::topics::thread(chat,id));
        }
        let chat = message.chat_id;
        if message.data["content"]["@type"]=="messagePaymentSuccessful"{menu_item(&p,&body,"Payment receipt…",move||crate::commerce::receipt(chat,id));}
        menu_item(&p,&body,"Translate…",move||crate::message_extras::translate(chat,id));
        if message.data["content"]["@type"]=="messageVoiceNote" || message.data["content"]["@type"]=="messageVideoNote" {
            menu_item(&p,&body,"Transcribe speech",move||request(chat,"recognizeSpeech",json!({"chat_id":chat,"message_id":id})));
        }
        if window::chat(chat).is_some_and(|c|c.chat_type==ChatType::Channel){menu_item(&p,&body,"Paid reaction…",move||crate::commerce::paid_reaction(chat,id));}
        if props["can_get_statistics"]==true {menu_item(&p,&body,"Message statistics…",move||crate::statistics::open(chat,Some(id)));}
        if props["can_be_approved"]==true || props["can_be_declined"]==true {menu_item(&p,&body,"Review suggested post…",move||crate::message_extras::suggested(chat,id));}
        if props["can_be_deleted_for_all_users"]==true&&!message.is_outgoing&&window::chat(chat).is_some_and(|c|matches!(c.chat_type,ChatType::Supergroup|ChatType::Channel)){
            let sender=message.data["sender_id"].clone();menu_item(&p,&body,"Delete all messages from this sender…",move||{let sender=sender.clone();operation("Delete all sender messages?","Every message sent by this person or channel in this conversation will be deleted for everyone. This cannot be undone.",vec![],"Delete all",move|s,_|{let sender=sender.clone();async move{crate::groups::context(&s,chat).await?.require("can_delete_messages")?;crate::groups::commit(&s,"deleteChatMessagesBySender",json!({"chat_id":chat,"sender_id":sender})).await?;Ok(())}});});}
        if props["can_edit_media"]==true{menu_item(&p,&body,"Replace media…",move||crate::message_editing::replace(chat,id));}
        if props["can_add_offer"]==true{menu_item(&p,&body,"Add offer…",move||crate::message_editing::offer(chat,id));}
        if message.is_outgoing&&message.data["content"]["@type"]=="messageLiveLocation"{menu_item(&p,&body,"Proximity alert…",move||crate::message_editing::proximity(chat,id));}
        let permissions: MessagePermissions = serde_json::from_value(props.clone()).unwrap_or_default();
        let m = message.clone();
        let v = view.clone();
        if permissions.can_be_replied {
            menu_item(&p, &body, "Reply", move || v.set_reply_to(&m));
        }
        if let Some(quote) = quote.filter(|t| !t.is_empty()) {
            if props["can_be_replied"] == true && !protected(&message) {
                let m = message.clone();
                let v = view.clone();
                menu_item(&p, &body, "Quote and reply", move || v.set_quote_to(&m, &quote));
            }
        }
        if !protected(&message) && props["can_be_copied"] == true {
            let text = message_text(&message)["text"].as_str().unwrap_or("").to_owned();
            if !text.is_empty() {
                menu_item(&p, &body, "Copy text", move || {
                    if let Some(d) = gdk::Display::default() {
                        d.clipboard().set_text(&text)
                    }
                });
            }
        }
        if permissions.can_get_link {
            let m = message.clone();
            menu_item(&p, &body, "Copy message link", move || copy_link(&m));
        }
        if permissions.can_be_forwarded && !protected(&message) {
            let group = view.forward_group(message.id);
            menu_item(&p, &body, "Forward…", move || forward(group.clone(), false));
            let group = view.forward_group(message.id);
            menu_item(&p, &body, "Save to Saved Messages", move || forward(group.clone(), true));
        }
        if permissions.can_be_edited
            && (message.data["content"]["@type"] == "messageText" || message.data["content"].get("caption").is_some())
        {
            let m = message.clone();
            menu_item(&p, &body, "Edit…", move || edit_message(m.clone()));
        }
        if message.data["content"]["@type"] == "messagePoll" && message.data["content"]["poll"]["is_closed"] != true && props["can_be_edited"] == true {
            let chat = message.chat_id;
            menu_item(&p, &body, "Stop poll…", move || crate::rich_messages::stop(chat, id));
        }
        let quick = message.clone();
        menu_item(&p, &body, "Quick reaction", move || quick_reaction(quick.clone()));
        let m = message.clone();
        menu_item(&p, &body, "React…", move || reaction_picker(m.clone()));
        if message.data["interaction_info"]["reactions"]["can_get_added_reactions"] == true {
            let m = message.clone();
            menu_item(&p, &body, "View reactions", move || reaction_people(m.clone(), None, String::new()));
        }
        if permissions.can_be_pinned {
            let m = message.clone();
            menu_item(&p, &body, if message.is_pinned { "Unpin message" } else { "Pin message…" }, move || {
                pin_message(m.clone())
            });
        }
        let m = message.clone();
        let v = view.clone();
        menu_item(&p, &body, "Select message", move || v.toggle_selection(m.id));
        if !view.selected_messages().is_empty() {
            let messages = view.selected_messages();
            menu_item(&p, &body, "Forward selected…", move || forward(messages.clone(), false));
            let messages = view.selected_messages();
            let v = view.clone();
            menu_item(&p, &body, "Delete selected…", move || delete_messages(messages.clone(), Some(v.clone())));
        }
        if props["can_be_deleted_only_for_self"] == true || props["can_be_deleted_for_all_users"] == true {
            let m = message.clone();
            let v = view.clone();
            let button = menu_item(&p, &body, "Delete message…", move || delete_messages(vec![m.clone()], Some(v.clone())));
            button.add_css_class("destructive-action");
        }
        if props["can_be_saved"] == true && !protected(&message) {
            if !message.files.is_empty() {
                let m = message.clone();
                menu_item(&p, &body, "Save attachment…", move || save_attachment(m.clone()));
            }
            if let Some(path) = message.files.iter().find(|f| f.role == "original").and_then(|f| f.path.clone()) {
                if matches!(message.content, MessageContent::Photo { .. } | MessageContent::Sticker { .. }) {
                    let m = message.clone();
                    menu_item(
                        &p,
                        &body,
                        if matches!(message.content, MessageContent::Photo { .. }) { "Copy image" } else { "Copy attachment" },
                        move || copy_attachment(m.clone()),
                    );
                }
                menu_item(&p, &body, "Show in folder", move || show_in_folder(&path));
            }
        }
        if let Some(file) = message.files.iter().find(|f| f.active) {
            let file = file.id;
            let chat = message.chat_id;
            menu_item(&p, &body, "Cancel download", move || {
                request(chat, "cancelDownloadFile", json!({"file_id":file,"only_if_pending":false}))
            });
        }
        let report_chat_id = message.chat_id;
        if message.sending_state.is_some() {
            let chat = message.chat_id;
            if message.sending_state.as_deref().is_some_and(|s| s.contains("Failed")) {
                menu_item(&p, &body, "Retry send", move || {
                    request(chat, "resendMessages", json!({"chat_id":chat,"message_ids":[id]}))
                });
            } else {
                menu_item(&p, &body, "Cancel send", move || delete_messages(vec![message.clone()], None));
            }
        }
        if permissions.can_report_chat {
            menu_item(&p, &body, "Report message…", move || report(report_chat_id, vec![id], String::new(), String::new()));
        }
    });
}

fn copy_link(message: &MessageSummary) {
    let (chat, id) = (message.chat_id, message.id);
    run(chat, "message-link", move |scope| async move {
        let data = scope
            .call(
                "getMessageLink",
                json!({"chat_id":chat,"message_id":id,"media_timestamp":0,"for_album":true,"in_message_thread":false}),
            )
            .await?;
        if let Some(link) = data["link"].as_str() {
            if let Some(d) = gdk::Display::default() {
                d.clipboard().set_text(link)
            }
        }
        Ok(())
    });
}
fn pin_message(message: MessageSummary) {
    let (chat, id) = (message.chat_id, message.id);
    if message.is_pinned {
        request(chat, "unpinChatMessage", json!({"chat_id":chat,"message_id":id}));
        return;
    }
    let private = window::chat(chat).is_some_and(|c| c.chat_type == ChatType::Private);
    operation(
        "Pin message",
        "Pin this message in the conversation?",
        vec![("Only pin for me", false, private), ("Notify members", true, !private)],
        "Pin",
        move |scope, options| async move {
            let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":id})).await?;
            if props["can_be_pinned"] != true {
                return Err("You can no longer pin this message".into());
            }
            scope
                .call(
                    "pinChatMessage",
                    json!({"chat_id":chat,"message_id":id,"only_for_self":options[0],"disable_notification":!options[1]}),
                )
                .await?;
            Ok(())
        },
    );
}
pub(crate) fn edit_message(message: MessageSummary) {
    let Some(scope) = Scope::current() else { return };
    let original = message_text(&message);
    let (dialog, body) = w::dialog("Edit message", 480);
    let editor = gtk::TextView::new();
    editor.set_wrap_mode(gtk::WrapMode::WordChar);
    crate::compose::load(&editor.buffer(), &original);
    let format = gtk::Button::with_label("Format selected text");
    let text_view = editor.clone();
    format.connect_clicked(move |button| crate::compose::menu(button.upcast_ref(), &text_view));
    body.append(&format);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(140)
        .max_content_height(400)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&editor)
        .build();
    body.append(&scroll);
    let error = w::label("", "error");
    error.set_wrap(true);
    body.append(&error);
    let save = gtk::Button::with_label("Save changes");
    body.append(&save);
    let weak = dialog.downgrade();
    let edit_buffer = editor.buffer();
    save.connect_clicked(move|button|{
        let Some(dialog)=weak.upgrade()else{return};if !scope.valid(){dialog.close();return}
        let formatted=crate::compose::formatted(&edit_buffer);
        let(scope,message,error,button)=(scope.clone(),message.clone(),error.clone(),button.clone());button.set_sensitive(false);
        glib::spawn_future_local(async move{
            let result=async{
                let props=scope.call("getMessageProperties",json!({"chat_id":message.chat_id,"message_id":message.id})).await?;
                if props["can_be_edited"]!=true{return Err("This message can no longer be edited".into())}
                if message.data["content"]["@type"]=="messageText"{scope.call("editMessageText",json!({"chat_id":message.chat_id,"message_id":message.id,"input_message_content":{"@type":"inputMessageText","text":formatted}})).await?;}
                else{scope.call("editMessageCaption",json!({"chat_id":message.chat_id,"message_id":message.id,"caption":formatted,"show_caption_above_media":message.data["content"]["show_caption_above_media"]==true})).await?;}Ok::<(),String>(())
            }.await;
            if !scope.valid(){dialog.close();return}match result{Ok(())=>dialog.close(),Err(e)=>{error.set_text(&e);button.set_sensitive(true);}}
        });
    });
    dialog.present();
    editor.grab_focus();
}

pub fn delete_messages(messages: Vec<MessageSummary>, view: Option<Rc<crate::chat_view::ChatView>>) {
    let Some(first) = messages.first() else { return };
    let chat = first.chat_id;
    run(chat, "delete-properties", move |scope| async move {
        let mut local = true;
        let mut all = true;
        for m in &messages {
            let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":m.id})).await?;
            local &= props["can_be_deleted_only_for_self"] == true;
            all &= props["can_be_deleted_for_all_users"] == true;
        }
        if !local && !all {
            return Err("The selected messages cannot be deleted together with the same scope".into());
        }
        let ids = messages.iter().map(|m| m.id).collect::<Vec<_>>();
        operation(
            "Delete messages?",
            &format!("Delete {} message(s)? This cannot be undone.", ids.len()),
            vec![("Also delete for other participants", false, all)],
            "Delete",
            move |scope, options| {
                let ids = ids.clone();
                let view = view.clone();
                async move {
                    if !options[0] && !local {
                        return Err(
                            "These messages can only be deleted for all participants. Select that option explicitly.".into()
                        );
                    }
                    for id in &ids {
                        let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":id})).await?;
                        if props[if options[0] { "can_be_deleted_for_all_users" } else { "can_be_deleted_only_for_self" }] != true
                        {
                            return Err("Message permissions changed. Reopen the menu and try again.".into());
                        }
                    }
                    scope.call("deleteMessages", json!({"chat_id":chat,"message_ids":ids,"revoke":options[0]})).await?;
                    if let Some(view) = view {
                        if *view.current_chat_id.borrow() == Some(chat) {
                            view.delete_messages(&ids);
                            view.clear_selection();
                        }
                    }
                    scope.service.load_chat(chat);
                    Ok(())
                }
            },
        );
        Ok(())
    });
}

pub fn reaction_label(reaction: &Value) -> String {
    match reaction["@type"].as_str() {
        Some("reactionTypeEmoji") => reaction["emoji"].as_str().unwrap_or("◌").to_owned(),
        Some("reactionTypeCustomEmoji") => "◇".into(),
        Some("reactionTypePaid") => "★".into(),
        _ => "◌".into(),
    }
}
pub fn toggle_reaction(message: MessageSummary, reaction: Value) {
    let (chat, id) = (message.chat_id, message.id);
    if reaction["@type"] == "reactionTypePaid" {
        window::show_status("Paid reactions require an explicit Stars payment and are not enabled yet");
        return;
    }
    run(chat, &format!("reaction-{id}"), move |scope| async move {
        let data = scope.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
        let chosen = data["interaction_info"]["reactions"]["reactions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|r| r["type"] == reaction && r["is_chosen"] == true);
        scope
            .call(
                if chosen { "removeMessageReaction" } else { "addMessageReaction" },
                json!({"chat_id":chat,"message_id":id,"reaction_type":reaction,"is_big":false,"update_recent_reactions":true}),
            )
            .await?;
        Ok(())
    });
}
pub fn update_default_reaction(reaction: Value) {
    DEFAULT_REACTION.with(|r| *r.borrow_mut() = reaction);
}
pub fn quick_reaction(message: MessageSummary) {
    let reaction = DEFAULT_REACTION.with(|r| r.borrow().clone());
    if reaction.is_null() {
        reaction_picker(message);
    } else {
        toggle_reaction(message, reaction);
    }
}
fn reaction_picker(message: MessageSummary) {
    let (chat, id) = (message.chat_id, message.id);
    run(chat, "reaction-picker", move |scope| async move {
        let data = scope.call("getMessageAvailableReactions", json!({"chat_id":chat,"message_id":id,"row_size":8})).await?;
        let (dialog, body) = w::dialog("React to message", 400);
        let default = gtk::CheckButton::with_label("Use selected reaction as my quick reaction");
        body.append(&default);
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_max_children_per_line(8);
        flow.set_min_children_per_line(4);
        let mut seen = HashSet::new();
        let mut count = 0;
        for item in ["top_reactions", "recent_reactions", "popular_reactions"]
            .into_iter()
            .flat_map(|key| data[key].as_array().into_iter().flatten())
        {
            let reaction = &item["type"];
            if reaction["@type"] == "reactionTypePaid" || !seen.insert(reaction.to_string()) {
                continue;
            }
            let button = gtk::Button::new();
            if reaction["@type"] == "reactionTypeCustomEmoji" {
                let text = FormattedText {
                    text: "◇".into(),
                    entities: vec![TextEntity {
                        offset: 0,
                        length: 1,
                        kind: TextEntityKind::CustomEmoji(
                            reaction["custom_emoji_id"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| reaction["custom_emoji_id"].to_string()),
                        ),
                    }],
                };
                button.set_child(Some(&crate::rich_text::widget(&text)));
            } else {
                button.set_label(&reaction_label(reaction));
            }
            if item["needs_premium"] == true {
                button.set_tooltip_text(Some("Telegram Premium may be required"));
            }
            let chosen = message.data["interaction_info"]["reactions"]["reactions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| r["type"] == *reaction && r["is_chosen"] == true);
            if chosen {
                button.add_css_class("suggested-action");
            }
            let reaction = reaction.clone();
            let m = message.clone();
            let weak = dialog.downgrade();
            let scope = scope.clone();
            let default = default.clone();
            button.connect_clicked(move |_| {
                if scope.valid() {
                    if default.is_active() {
                        request(m.chat_id, "setDefaultReactionType", json!({"reaction_type":reaction}));
                    }
                    toggle_reaction(m.clone(), reaction.clone())
                }
                if let Some(d) = weak.upgrade() {
                    d.close()
                }
            });
            flow.insert(&button, -1);
            count += 1;
        }
        if count == 0 {
            body.append(&w::label("No reactions are available for this message", "dim"));
        }
        let scroll = gtk::ScrolledWindow::builder()
            .max_content_height(340)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&flow)
            .build();
        body.append(&scroll);
        dialog.present();
        Ok(())
    });
}
fn reaction_people(message: MessageSummary, reaction: Option<Value>, offset: String) {
    let (chat, id) = (message.chat_id, message.id);
    run(chat, "reaction-people", move |scope| async move {
        let data = scope
            .call(
                "getMessageAddedReactions",
                json!({"chat_id":chat,"message_id":id,"reaction_type":reaction,"offset":offset,"limit":50}),
            )
            .await?;
        let (dialog, body) = w::dialog("Reactions", 400);
        let list = w::vbox(4);
        let scroll = gtk::ScrolledWindow::builder()
            .max_content_height(400)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .build();
        body.append(&scroll);
        for item in data["reactions"].as_array().into_iter().flatten() {
            let sender = &item["sender_id"];
            let name = if let Some(user) = sender["user_id"].as_i64() {
                let u = scope.call("getUser", json!({"user_id":user})).await?;
                format!("{} {}", u["first_name"].as_str().unwrap_or(""), u["last_name"].as_str().unwrap_or(""))
            } else if let Some(chat) = sender["chat_id"].as_i64() {
                scope.call("getChat", json!({"chat_id":chat})).await?["title"].as_str().unwrap_or("Chat").to_owned()
            } else {
                "Unknown sender".into()
            };
            let label = w::label(&format!("{}  {}", reaction_label(&item["type"]), name.trim()), "");
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(36);
            list.append(&label);
        }
        if let Some(next) = data["next_offset"].as_str().filter(|s| !s.is_empty()) {
            let next = next.to_owned();
            let button = gtk::Button::with_label("More reactions");
            let weak = dialog.downgrade();
            let scope = scope.clone();
            button.connect_clicked(move |_| {
                if !scope.valid() {
                    if let Some(d) = weak.upgrade() {
                        d.close();
                    }
                    return;
                }
                reaction_people(message.clone(), reaction.clone(), next.clone());
                if let Some(d) = weak.upgrade() {
                    d.close()
                }
            });
            body.append(&button);
        }
        dialog.present();
        Ok(())
    });
}

type ForwardAccepted=Rc<RefCell<std::collections::HashMap<(i64,String),HashSet<i64>>>>;
type ForwardTopics=Rc<RefCell<std::collections::HashMap<i64,(Value,String)>>>;
fn forward_key(chat:i64,topic:&Value)->(i64,String){(chat,crate::topics::suffix(topic))}
fn recipient_row(list:&gtk::Box,selected:&Rc<RefCell<HashSet<i64>>>,id:i64,title:&str,forum:bool,topics:&ForwardTopics,accepted:&ForwardAccepted,scope:&Scope){
    let row=w::hbox(6);row.set_widget_name(&format!("forward-recipient-{id}"));let check=gtk::CheckButton::with_label(title);check.set_hexpand(true);
    if let Some(label)=check.child().and_downcast::<gtk::Label>(){label.set_ellipsize(gtk::pango::EllipsizeMode::End);label.set_max_width_chars(30);}
    check.set_active(selected.borrow().contains(&id));
    let selection=selected.clone();check.connect_toggled(move |c|{if c.is_active(){selection.borrow_mut().insert(id);}else{selection.borrow_mut().remove(&id);}});row.append(&check);
    if forum{
        topics.borrow_mut().entry(id).or_insert((Value::Null,"Choose topic…".into()));
        let topic_button=gtk::Button::with_label(&topics.borrow().get(&id).map(|(_,name)|name.clone()).unwrap_or_else(||"Choose topic…".into()));
        if let Some(label)=topic_button.child().and_downcast::<gtk::Label>(){label.set_max_width_chars(18);label.set_ellipsize(gtk::pango::EllipsizeMode::End);}
        let (topics,accepted,scope)=(topics.clone(),accepted.clone(),scope.clone());
        topic_button.connect_clicked(move |button|{
            if !scope.valid(){return}
            if accepted.borrow().iter().any(|((chat,_),ids)|*chat==id&&!ids.is_empty()){window::show_status("This destination is locked because some messages have already been accepted.");return}
            let (topics,accepted,scope,weak,check)=(topics.clone(),accepted.clone(),scope.clone(),button.downgrade(),check.downgrade());
            crate::topics::choose_destination(id,move |topic,title|{
                if !scope.valid()||accepted.borrow().iter().any(|((chat,_),ids)|*chat==id&&!ids.is_empty()){return}
                if let Some(button)=weak.upgrade(){if !button.is_sensitive(){return}button.set_label(&title);topics.borrow_mut().insert(id,(topic,title));if let Some(check)=check.upgrade(){check.set_active(true);}}
            });
        });row.append(&topic_button);
    }
    list.append(&row);
}
fn sync_forward_selection(list:&gtk::Box,selected:&Rc<RefCell<HashSet<i64>>>){
    let mut row=list.first_child();
    while let Some(widget)=row{
        row=widget.next_sibling();
        if let Some(id)=widget.widget_name().strip_prefix("forward-recipient-").and_then(|id|id.parse::<i64>().ok()){
            if let Some(check)=widget.first_child().and_downcast::<gtk::CheckButton>(){
                let active=selected.borrow().contains(&id);
                check.set_active(active);
            }
        }
    }
}
#[cfg(test)]
pub fn check_forward_selection(){
    let list=w::vbox(0);let row=w::hbox(0);row.set_widget_name("forward-recipient-7");let check=gtk::CheckButton::new();check.set_active(true);row.append(&check);list.append(&row);
    let selected=Rc::new(RefCell::new(HashSet::<i64>::new()));let state=selected.clone();
    check.connect_toggled(move |c|{if c.is_active(){state.borrow_mut().insert(7);}else{state.borrow_mut().remove(&7);}});
    sync_forward_selection(&list,&selected);assert!(!check.is_active());assert!(selected.borrow().is_empty());
}
pub fn forward(mut messages: Vec<MessageSummary>, saved: bool) {
    if messages.is_empty() {
        return;
    }
    messages.sort_by_key(|m| m.id);
    messages.dedup_by_key(|m| m.id);
    let chat = messages[0].chat_id;
    run(chat, "forward-picker", move |scope| async move {
        let mut can_copy = true;
        let mut can_secret = true;
        for m in &messages {
            if m.chat_id != chat || protected(m) {
                return Err("These messages cannot be forwarded".into());
            }
            let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":m.id})).await?;
            if props["can_be_forwarded"] != true {
                return Err("One or more messages cannot be forwarded".into());
            }
            can_copy &= props["can_be_copied"] == true;
            can_secret &= props["can_be_copied_to_secret_chat"] == true;
        }
        let me = scope.call("getMe", json!({})).await?;
        let own = scope.call("createPrivateChat", json!({"user_id":me["id"],"force":false})).await?;
        let own_id = own["id"].as_i64().ok_or("Saved Messages is unavailable")?;
        let ids = messages.iter().map(|m| m.id).collect::<Vec<_>>();
        let accepted:ForwardAccepted=Default::default();
        let destination_topics:ForwardTopics=Default::default();
        if saved {
            send_forward(&scope, chat, &ids, own_id, &Value::Null, false, false, can_copy, can_secret, &accepted).await?;
            window::show_status("Forward accepted for Saved Messages");
            return Ok(());
        }
        let (dialog, body) = w::dialog("Forward messages", 500);
        let preview = w::vbox(4);
        for message in &messages {
            let label = w::label(&crate::chat_view::preview(message), "dim");
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_max_width_chars(44);
            preview.append(&label);
            if let Some(path) = message
                .files
                .iter()
                .find(|f| f.role == "thumbnail")
                .or_else(|| message.files.iter().find(|f| f.role == "original"))
                .and_then(|f| f.path.as_ref())
            {
                if let Ok(texture) = gdk::Texture::from_file(&gio::File::for_path(path)) {
                    let pic = gtk::Picture::for_paintable(&texture);
                    pic.set_can_shrink(true);
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    pic.set_size_request(100, 70);
                    preview.append(&pic);
                }
            }
        }
        let sc = gtk::ScrolledWindow::builder()
            .max_content_height(100)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&preview)
            .build();
        body.append(&sc);
        let hide = gtk::CheckButton::with_label("Hide original sender names");
        hide.set_sensitive(can_copy);
        body.append(&hide);
        let captions = gtk::CheckButton::with_label("Remove media captions (sends copies)");
        captions.set_sensitive(can_copy);
        body.append(&captions);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search recipients"));
        body.append(&search);
        let recipients = w::vbox(4);
        let selected = Rc::new(RefCell::new(HashSet::new()));
        recipient_row(&recipients,&selected,own_id,"Saved Messages",false,&destination_topics,&accepted,&scope);
        for chat in window::list_chats(usize::MAX).into_iter().filter(|c| c.id != own_id) {
            recipient_row(&recipients,&selected,chat.id,&chat.title,chat.data["pulse_is_forum"]==true,&destination_topics,&accepted,&scope);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .min_content_height(120)
            .max_content_height(220)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&recipients)
            .build();
        body.append(&scroll);
        let revision = Rc::new(Cell::new(0u64));
        let list = recipients.clone();
        let selected_search = selected.clone();
        let scope_search = scope.clone();
        let weak_dialog = dialog.downgrade();
        let search_topics=destination_topics.clone();let search_accepted=accepted.clone();
        search.connect_search_changed(move |entry| {
            let n = revision.get() + 1;
            revision.set(n);
            let revision = revision.clone();
            let list = list.clone();
            let selected = selected_search.clone();
            let scope = scope_search.clone();
            let weak = weak_dialog.clone();
            let query = entry.text().to_string();
            let (destination_topics,accepted)=(search_topics.clone(),search_accepted.clone());
            glib::spawn_future_local(async move {
                let result = scope.call("searchChats", json!({"query":query,"limit":50})).await;
                if revision.get() != n || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                let data = match result {
                    Ok(data) => data,
                    Err(e) => {
                        scope.error(&e);
                        return;
                    }
                };
                let mut rows = Vec::new();
                for id in data["chat_ids"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
                    if id == own_id {
                        continue;
                    }
                    if let Ok(chat) = scope.call("getChat", json!({"chat_id":id})).await {
                        let forum=if let Some(group)=chat["type"]["supergroup_id"].as_i64(){scope.call("getSupergroup",json!({"supergroup_id":group})).await.is_ok_and(|g|g["is_forum"]==true)}else{false};
                        rows.push((id, chat["title"].as_str().unwrap_or("Chat").to_owned(),forum));
                    }
                    if revision.get() != n || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                        return;
                    }
                }
                while let Some(child) = list.first_child() {
                    list.remove(&child)
                }
                recipient_row(&list,&selected,own_id,"Saved Messages",false,&destination_topics,&accepted,&scope);
                for (id, title, forum) in rows {
                    recipient_row(&list,&selected,id,&title,forum,&destination_topics,&accepted,&scope)
                }
            });
        });
        let error = w::label("", "error");
        error.set_wrap(true);
        error.set_max_width_chars(44);
        body.append(&gtk::ScrolledWindow::builder().max_content_height(96).propagate_natural_height(true).hscrollbar_policy(gtk::PolicyType::Never).vscrollbar_policy(gtk::PolicyType::External).child(&error).build());
        let status = w::label("", "dim");
        status.set_wrap(true);status.set_max_width_chars(44);
        body.append(&status);
        let send = gtk::Button::with_label("Forward to selected chats");
        body.append(&send);
        let weak = dialog.downgrade();
        let busy = Rc::new(Cell::new(false));
        let b = busy.clone();
        dialog.connect_close_request(move |_| if b.get() { glib::Propagation::Stop } else { glib::Propagation::Proceed });
        send.connect_clicked(move |button| {
            if selected.borrow().is_empty() {
                error.set_text("Choose at least one recipient");
                return;
            }
            if selected.borrow().iter().any(|id|destination_topics.borrow().get(id).is_some_and(|(topic,_)|topic.is_null())){error.set_text("Choose a destination topic for each selected forum.");return}
            if busy.replace(true) {
                return;
            }
            let Some(dialog) = weak.upgrade() else {
                busy.set(false);
                return;
            };
            if !scope.valid() {
                busy.set(false);
                dialog.close();
                return;
            }
            let copy = hide.is_active() || captions.is_active();
            let remove = captions.is_active();
            let targets = selected.borrow().iter().map(|id|(*id,destination_topics.borrow().get(id).map(|(topic,_)|topic.clone()).unwrap_or(Value::Null))).collect::<Vec<_>>();
            let (scope, ids, selected, error, status, button, busy, accepted) = (
                scope.clone(),
                ids.clone(),
                selected.clone(),
                error.clone(),
                status.clone(),
                button.clone(),
                busy.clone(),
                accepted.clone(),
            );
            button.set_sensitive(false);
            let controls=vec![hide.clone().upcast::<gtk::Widget>(),captions.clone().upcast(),search.clone().upcast(),recipients.clone().upcast()];
            for control in &controls{control.set_sensitive(false);}
            let recipient_rows=recipients.clone();
            glib::spawn_future_local(async move {
                let mut failures = Vec::new();
                let mut sent = 0;
                for (target,topic) in targets {
                    match send_forward(&scope, chat, &ids, target, &topic, copy, remove, can_copy, can_secret, &accepted).await {
                        Ok(()) => {
                            selected.borrow_mut().remove(&target);
                            sent += 1;
                        }
                        Err(e) => {
                            failures.push(format!(
                                "{}: {e}",
                                window::chat(target).map(|c| c.title).unwrap_or_else(|| target.to_string())
                            ));
                        }
                    }
                }
                busy.set(false);
                if !scope.valid() {
                    dialog.close();
                    return;
                }
                sync_forward_selection(&recipient_rows,&selected);
                if failures.is_empty() {
                    window::show_status("Forwarding accepted");
                    dialog.close();
                } else {
                    let unknown = failures.iter().any(|e| e.contains("status is unknown"));
                    status.set_text(&format!("Accepted for {sent} chat(s). Confirmed messages are excluded from retries."));
                    error.set_text(&failures.join("\n"));
                    button.set_sensitive(!unknown);
                    for control in &controls{control.set_sensitive(!unknown);}
                }
            });
        });
        dialog.present();
        Ok(())
    });
}
async fn send_forward(
    scope: &Scope,
    from: i64,
    ids: &[i64],
    target: i64,
    topic: &Value,
    copy: bool,
    remove: bool,
    can_copy: bool,
    can_secret: bool,
    accepted: &ForwardAccepted,
) -> Result<(), String> {
    let chat = scope.call("getChat", json!({"chat_id":target})).await?;
    let secret = chat["type"]["@type"] == "chatTypeSecret";
    if secret && !can_secret {
        return Err("These messages cannot be copied into a secret chat".into());
    }
    if (copy || remove) && !can_copy {
        return Err("These messages cannot be sent as copies".into());
    }
    // Validate immediately before sending; original permissions can change while the picker is open.
    for id in ids {
        let props = scope.call("getMessageProperties", json!({"chat_id":from,"message_id":id})).await?;
        if props["can_be_forwarded"] != true
            || ((copy || remove) && props["can_be_copied"] != true)
            || (secret && props["can_be_copied_to_secret_chat"] != true)
        {
            return Err("Forwarding permission changed".into());
        }
    }
    if topic["@type"]=="messageTopicForum"{scope.call("getForumTopic",json!({"chat_id":target,"forum_topic_id":topic["forum_topic_id"]})).await?;}
    let key=forward_key(target,topic);
    let remaining = ids.iter().copied().filter(|id|!accepted.borrow().get(&key).is_some_and(|set|set.contains(id))).collect::<Vec<_>>();
    for chunk in remaining.chunks(100) {
        let data=scope.call("forwardMessages",json!({"chat_id":target,"topic_id":topic,"from_chat_id":from,"message_ids":chunk,"send_copy":copy||remove||secret,"remove_caption":remove,"options":{"@type":"messageSendOptions","paid_message_star_count":0}})).await.map_err(|e|if e.contains("timed out")||e.contains("shut down")||e.contains("stopped"){format!("Forwarding status is unknown. Check the destination before sending again: {e}")}else{e})?;
        let Some(messages) = data["messages"].as_array() else {
            return Err("Forwarding status is unknown. Check the destination before sending again.".into());
        };
        let (confirmed, complete) = accepted_forward_ids(chunk, messages);
        accepted.borrow_mut().entry(key.clone()).or_default().extend(confirmed);
        if !complete {
            return Err(
                "Telegram could not forward every selected message. Retry sends only messages that were not accepted.".into()
            );
        }
    }
    scope.service.load_chat(target);
    Ok(())
}

pub fn save_attachment(message: MessageSummary) {
    let chat = message.chat_id;
    run(chat, "save-attachment", move |scope| async move {
        let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":message.id})).await?;
        if props["can_be_saved"] != true || protected(&message) {
            return Err("Saving is not allowed for this message".into());
        }
        let file = message.files.iter().find(|f| f.role == "original").ok_or("No attachment")?;
        let path = if let Some(path) = file.path.as_ref() {
            path.clone()
        } else {
            let data = scope
                .call("downloadFile", json!({"file_id":file.id,"priority":16,"offset":0,"limit":0,"synchronous":true}))
                .await?;
            if data["local"]["is_downloading_completed"] != true {
                return Err("Attachment download has not completed".into());
            }
            data["local"]["path"].as_str().ok_or("Downloaded file is unavailable")?.to_owned()
        };
        let name = message.data["content"]["document"]["file_name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().into());
        let parent = w::main_window();
        let picker = gtk::FileDialog::builder().title("Save attachment").initial_name(name).build();
        match picker.save_future(parent.as_ref()).await {
            Ok(dest) => {
                if !scope.valid() {
                    return Ok(());
                }
                gio::File::for_path(path)
                    .copy_future(&dest, gio::FileCopyFlags::OVERWRITE, glib::Priority::DEFAULT)
                    .0
                    .await
                    .map_err(|e| e.to_string())?;
                window::show_status("Attachment saved");
            }
            Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
            Err(e) => return Err(e.to_string()),
        }
        Ok(())
    });
}
fn show_in_folder(path: &str) {
    let uri = gio::File::for_path(path).uri().to_string();
    glib::spawn_future_local(async move {
        let result = async {
            let bus = gio::bus_get_future(gio::BusType::Session).await?;
            bus.call_future(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&(vec![uri], String::new()).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
            )
            .await?;
            Ok::<(), glib::Error>(())
        }
        .await;
        if let Err(e) = result {
            window::show_status(&format!("Could not show file: {e}"));
        }
    });
}

pub fn copy_messages(messages: Vec<MessageSummary>) {
    let Some(chat) = messages.first().map(|m| m.chat_id) else { return };
    run(chat, "copy-selection", move |scope| async move {
        for m in &messages {
            if protected(m) {
                return Err("Copying protected messages is not allowed".into());
            }
            let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":m.id})).await?;
            if props["can_be_copied"] != true {
                return Err("One or more messages cannot be copied".into());
            }
        }
        let text =
            messages.iter().map(|m| message_text(m)["text"].as_str().unwrap_or("").to_owned()).collect::<Vec<_>>().join("\n\n");
        if let Some(d) = gdk::Display::default() {
            d.clipboard().set_text(&text)
        }
        Ok(())
    });
}
pub fn report_messages(messages: Vec<MessageSummary>) {
    let Some(chat) = messages.first().map(|m| m.chat_id) else { return };
    run(chat, "report-permissions", move |scope| async move {
        for m in &messages {
            let props = scope.call("getMessageProperties", json!({"chat_id":chat,"message_id":m.id})).await?;
            if props["can_report_chat"] != true {
                return Err("One or more selected messages cannot be reported".into());
            }
        }
        report(chat, messages.iter().map(|m| m.id).collect(), String::new(), String::new());
        Ok(())
    });
}

fn accepted_forward_ids(source: &[i64], results: &[Value]) -> (Vec<i64>, bool) {
    (
        source.iter().zip(results).filter_map(|(id, result)| (!result.is_null()).then_some(*id)).collect(),
        source.len() == results.len() && results.iter().all(|result| !result.is_null()),
    )
}
#[cfg(test)]
mod regression {
    use super::*;
    use nixe_telegram_core::client::parse_message;
    #[test]
    fn stage3c_forward_destinations_are_independent() {
        let a=json!({"@type":"messageTopicForum","forum_topic_id":1});let b=json!({"@type":"messageTopicForum","forum_topic_id":2});
        let mut accepted=std::collections::HashMap::<(i64,String),HashSet<i64>>::new();
        accepted.entry(forward_key(-7,&a)).or_default().insert(10);
        assert!(accepted[&forward_key(-7,&a)].contains(&10));
        assert!(!accepted.contains_key(&forward_key(-7,&b)));
        assert!(!accepted.contains_key(&forward_key(-8,&a)));
        assert_ne!(forward_key(-7,&a),forward_key(-7,&Value::Null));
    }
    #[test]
    fn partial_forward_tracks_source_ids_and_permissions_default_to_denied() {
        let (ids, complete) = accepted_forward_ids(&[10, 20, 30], &[json!({"id":900}), Value::Null, json!({"id":902})]);
        assert_eq!(ids, vec![10, 30]);
        assert!(!complete);
        assert!(!accepted_forward_ids(&[10, 20], &[json!({"id":900})]).1);
        let permissions: MessagePermissions = serde_json::from_value(json!({"can_be_forwarded":true})).unwrap();
        assert!(permissions.can_be_forwarded);
        assert!(!permissions.can_be_copied);
        assert!(!permissions.can_be_deleted_for_all_users);
        let mut message = parse_message(
            &json!({"id":7,"chat_id":9,"date":1,"content":{"@type":"messageText","text":{"text":"restricted","entities":[]}}}),
        )
        .unwrap();
        assert!(!exports_allowed(&message), "Unknown export permission must be denied");
        message.data["can_be_saved"] = json!(true);
        assert!(exports_allowed(&message));
        message.data["has_protected_content"] = json!(true);
        assert!(!exports_allowed(&message));
    }
}

pub fn exports_allowed(message: &MessageSummary) -> bool {
    message.data["can_be_saved"] == true && !protected(message)
}
pub fn copy_attachment(message: MessageSummary) {
    run(message.chat_id, "copy-attachment", move |scope| async move {
        let props = scope.call("getMessageProperties", json!({"chat_id":message.chat_id,"message_id":message.id})).await?;
        if props["can_be_saved"] != true || protected(&message) {
            return Err("Copying is not allowed for this message".into());
        }
        let path = message
            .files
            .iter()
            .find(|f| f.role == "original")
            .and_then(|f| f.path.as_ref())
            .ok_or("Download the attachment first")?;
        if let Some(display) = gdk::Display::default() {
            if let Ok(texture) = gdk::Texture::from_file(&gio::File::for_path(path)) {
                display.clipboard().set_texture(&texture);
            } else {
                let files = gdk::FileList::from_array(&[gio::File::for_path(path)]);
                display
                    .clipboard()
                    .set_content(Some(&gdk::ContentProvider::for_value(&files.to_value())))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    });
}
pub fn open_external_attachment(message: MessageSummary) {
    run(message.chat_id, "open-attachment", move |scope| async move {
        let props = scope.call("getMessageProperties", json!({"chat_id":message.chat_id,"message_id":message.id})).await?;
        if props["can_be_saved"] != true || protected(&message) {
            return Err("This protected attachment cannot be opened in an external application".into());
        }
        let path = message
            .files
            .iter()
            .find(|f| f.role == "original")
            .and_then(|f| f.path.as_ref())
            .ok_or("Download the attachment first")?;
        gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), gio::AppLaunchContext::NONE)
            .map_err(|e| e.to_string())?;
        Ok(())
    });
}
