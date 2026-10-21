//! Independent conversation windows sharing service updates and suite widgets.
use crate::{actions::Scope, chat_view::ChatView, service::ServiceEvent, window};
use gtk::{glib, prelude::*};
use nixe_telegram_core::models::*;
use serde_json::json;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
struct Conversation {
    view: Rc<ChatView>,
    details: Rc<crate::detail_panel::DetailPanel>,
    window: gtk::Window,
    scope: Scope,
}
thread_local! {
static WINDOWS:RefCell<BTreeMap<i64,Rc<Conversation>>>=const{
RefCell::new(BTreeMap::new())}
;
}
pub fn reset() {
    let windows = WINDOWS.with(|v| std::mem::take(&mut *v.borrow_mut()));
    for c in windows.into_values() {
        c.window.close();
    }
}
pub fn is_open(chat: i64) -> bool {
    WINDOWS.with(|v| v.borrow().contains_key(&chat))
}
pub fn focused(chat: i64) -> Option<Rc<ChatView>> {
    let active = gio_app()?.active_window()?;
    let mut parent = Some(active);
    for _ in 0..8 {
        let w = parent?;
        if let Some(v) = WINDOWS.with(|v| v.borrow().get(&chat).filter(|c| c.window == w).map(|c| c.view.clone())) {
            return Some(v);
        }
        parent = w.transient_for();
    }
    None
}
fn gio_app() -> Option<gtk::Application> {
    gtk::gio::Application::default()?.downcast().ok()
}
pub fn open(chat: i64) {
    if let Some(c) = WINDOWS.with(|v| v.borrow().get(&chat).cloned()) {
        c.window.present();
        return;
    }
    let Some(scope) = Scope::current() else { return };
    let Some(data) = window::chat(chat) else { return };
    let Some(app) = gio_app() else { return };
    let view = ChatView::new();
    view.detached.set(true);
    let w = gtk::ApplicationWindow::builder()
        .application(&app)
        .title(format!("{} — Pulse", data.title))
        .default_width(800)
        .default_height(700)
        .build();
    w.add_css_class("pulse-window");
    let details = crate::detail_panel::DetailPanel::new();
    details.set_chat(&data);
    details.root.set_visible(false);
    let layout = gtk::Paned::new(gtk::Orientation::Horizontal);
    layout.set_start_child(Some(&view.root));
    layout.set_end_child(Some(&details.root));
    layout.set_shrink_start_child(false);
    layout.set_shrink_end_child(false);
    w.set_child(Some(&layout));
    details.connect_media(|m| window::show_media(m));
    details.connect_start_secret_chat(|user| window::with_service(|s| s.create_secret_chat(user)));
    let conversation =
        Rc::new(Conversation { details: details.clone(), view: view.clone(), window: w.clone().upcast(), scope: scope.clone() });
    WINDOWS.with(|v| v.borrow_mut().insert(chat, conversation.clone()));
    view.set_chat(&data);
    view.load_history(0);
    let scope_ = scope.clone();
    view.connect_open_media(move |m| {
        if scope_.valid() {
            window::show_media(m);
        }
    });
    let scope_ = scope.clone();
    view.connect_toggle_details(move || {
        if scope_.valid() {
            details.toggle_visible();
        }
    });
    view.connect_start_call(move |chat, video| {
        window::start_call_from_assistant(chat, video);
    });
    view.connect_typing(move |chat| {
        window::with_service(|s| {
            s.request(
                "sendChatAction",
                json!({
                "chat_id":chat,"action":{
                "@type":"chatActionTyping"}
                }
                ),
            );
        });
    });
    let weak = w.downgrade();
    w.connect_close_request(move |_| {
        WINDOWS.with(|v| v.borrow_mut().remove(&chat));
        if window::active_chat_id() != Some(chat) {
            window::with_service(|s| {
                s.request(
                    "closeChat",
                    json!({
                    "chat_id":chat}
                    ),
                );
            });
        }
        glib::Propagation::Proceed
    });
    let view_ = view.clone();
    let scope_ = scope.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
        let Some(w) = weak.upgrade() else { return glib::ControlFlow::Break };
        if !scope_.valid() {
            w.close();
            return glib::ControlFlow::Break;
        }
        if w.is_active() {
            let ids = view_.visible_ids();
            if !ids.is_empty() {
                window::with_service(|s| {
                    s.request(
                        "viewMessages",
                        json!({
                        "chat_id":chat,"message_ids":ids,"source":{
                        "@type":"messageSourceChatHistory"}
                        ,"force_read":true}
                        ),
                    );
                });
            }
        }
        glib::ControlFlow::Continue
    });
    window::with_service(|s| {
        s.request(
            "openChat",
            json!({
            "chat_id":chat}
            ),
        );
    });
    w.present();
}
pub fn event(event: &ServiceEvent) {
    let windows = WINDOWS.with(|v| v.borrow().values().cloned().collect::<Vec<_>>());
    for c in windows {
        if !c.scope.valid() {
            continue;
        }
        let chat = *c.view.current_chat_id.borrow();
        match event {
            ServiceEvent::Update(TelegramUpdate::NewMessage(m)) => {
                c.view.bot_force_reply(m);
                c.view.append_message(m)
            }
            ServiceEvent::Update(TelegramUpdate::MessageReplaced { old_id, message }) => c.view.replace_message(*old_id, message),
            ServiceEvent::Update(TelegramUpdate::MessagesDeleted { chat_id, message_ids }) if chat == Some(*chat_id) => {
                c.view.delete_messages(message_ids)
            }
            ServiceEvent::Update(TelegramUpdate::ChatUpdated(data)) if chat == Some(data.id) => {
                c.view.update_chat(data);
                if c.details.root.is_visible() {
                    c.details.refresh(data);
                }
            }
            ServiceEvent::DraftSynced { chat_id, topic, draft } => c.view.remote_topic_draft(*chat_id, topic, draft),
            _ => {}
        }
    }
}
