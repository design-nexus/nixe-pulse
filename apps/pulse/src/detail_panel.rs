//! Conversation information and shared media fetched from Telegram.
use gtk::{glib, prelude::*};
use nyx_telegram_core::{client::parse_message, models::*};
use nyx_ui::widgets as w;
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub struct DetailPanel {
    pub root: gtk::Box,
    body: gtk::Box,
    revision: Cell<u64>,
    media_ids: RefCell<std::collections::HashSet<i64>>,
    chat: RefCell<Option<ChatSummary>>,
    on_secret: RefCell<Option<Box<dyn Fn(i64)>>>,
    on_media: RefCell<Option<Box<dyn Fn(&MessageSummary)>>>,
}
impl DetailPanel {
    pub fn new() -> Rc<Self> {
        let root = w::vbox(0);
        root.add_css_class("detail-panel");
        root.set_size_request(280, -1);
        root.set_visible(false);
        let header = w::hbox(8);
        header.add_css_class("detail-header");
        header.set_margin_start(16);
        header.set_margin_end(16);
        header.set_margin_top(8);
        header.set_margin_bottom(8);
        let heading = w::label("INFO", "section-title");
        heading.set_hexpand(true);
        heading.set_xalign(0.0);
        header.append(&heading);
        let close = w::bar_button("window-close-symbolic", "Close details");
        header.append(&close);
        root.append(&header);
        let body = w::vbox(12);
        body.set_margin_start(16);
        body.set_margin_end(16);
        body.set_margin_top(12);
        body.set_margin_bottom(16);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&body)
            .build();
        root.append(&scroll);
        let p = Rc::new(Self {
            root,
            body,
            revision: Cell::new(0),
            media_ids: Default::default(),
            chat: Default::default(),
            on_secret: Default::default(),
            on_media: Default::default(),
        });
        let weak = Rc::downgrade(&p);
        close.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.root.set_visible(false);
            }
        });
        p
    }
    pub fn set_chat(self: &Rc<Self>, chat: &ChatSummary) {
        self.refresh(chat);
    }
    pub fn clear(&self) {
        self.revision.set(self.revision.get() + 1);
        self.chat.borrow_mut().take();
        self.media_ids.borrow_mut().clear();
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        self.root.set_visible(false);
    }
    fn valid(&self, revision: u64, generation: &str) -> bool {
        self.revision.get() == revision && crate::window::service_is_current(generation)
    }
    pub fn refresh(self: &Rc<Self>, chat: &ChatSummary) {
        let unchanged = self.chat.borrow().as_ref().is_some_and(|old| {
            old.id == chat.id
                && old.title == chat.title
                && old.avatar_path == chat.avatar_path
                && old.is_muted == chat.is_muted
                && old.is_secret == chat.is_secret
                && old.data["type"] == chat.data["type"]
        });
        *self.chat.borrow_mut() = Some(chat.clone());
        if unchanged {
            return;
        }
        let revision = self.revision.get() + 1;
        self.revision.set(revision);
        self.media_ids.borrow_mut().clear();
        while let Some(c) = self.body.first_child() {
            self.body.remove(&c);
        }
        self.body.append(&crate::chat_list::build_avatar_widget(chat, 64));
        let title = w::label(&chat.title, "chat-header-title");
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_max_width_chars(24);
        self.body.append(&title);
        self.body.append(&w::label(
            if chat.is_secret {
                "Secret chat"
            } else {
                match chat.chat_type {
                    ChatType::Private => "Telegram contact",
                    ChatType::Channel => "Channel",
                    _ => "Group",
                }
            },
            "dim",
        ));
        let id = chat.id;
        if chat.data["pulse_is_forum"] == true {
            let b = gtk::Button::with_label("Topics…");
            b.connect_clicked(move |_| crate::topics::browse(id));
            self.body.append(&b);
        }
        let user = chat.data["type"]["user_id"].as_i64();
        if let Some(user) = user {
            if !chat.is_secret {
                let b = gtk::Button::with_label("Start secret chat");
                let weak = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        if let Some(cb) = p.on_secret.borrow().as_ref() {
                            cb(user);
                        }
                    }
                });
                self.body.append(&b);
            }
        }
        let (row, _) = w::switch_row("Notifications", "Notify for this conversation", !chat.is_muted, move |on| {
            crate::actions::run(id, "details-notifications", move |scope| async move {
                let chat = scope.call("getChat", json!({"chat_id":id})).await?;
                let mut settings = chat["notification_settings"].clone();
                settings["use_default_mute_for"] = json!(false);
                settings["mute_for"] = json!(if on { 0 } else { i32::MAX });
                scope.call("setChatNotificationSettings", json!({"chat_id":id,"notification_settings":settings})).await?;
                Ok(())
            });
        });
        self.body.append(&row);
        let pinned = gtk::Button::with_label("Pinned messages");
        pinned.connect_clicked(move |_| crate::message_tools::pinned(id));
        self.body.append(&pinned);
        let load = gtk::Button::with_label("Load shared media");
        let weak = Rc::downgrade(self);
        load.connect_clicked(move |button| {
            if let Some(p) = weak.upgrade() {
                p.load_media(id, 0, button.clone());
            }
        });
        self.body.append(&load);
        if matches!(chat.chat_type, ChatType::BasicGroup | ChatType::Supergroup | ChatType::Channel) {
            let members = gtk::Button::with_label("Members and administrators…");
            members.connect_clicked(move |_| crate::groups::members(id));
            self.body.append(&members);
            let manage = gtk::Button::with_label("Manage group / channel…");
            manage.connect_clicked(move |_| crate::groups::manage(id));
            self.body.append(&manage);
        }
        let Some(service) = crate::window::service() else { return };
        let (method, args) = if let Some(user) = user {
            ("getUserFullInfo", json!({"user_id":user}))
        } else if let Some(group) = chat.data["type"]["supergroup_id"].as_i64() {
            ("getSupergroupFullInfo", json!({"supergroup_id":group}))
        } else if let Some(group) = chat.data["type"]["basic_group_id"].as_i64() {
            ("getBasicGroupFullInfo", json!({"basic_group_id":group}))
        } else {
            return;
        };
        let generation = service.generation.clone();
        let rx = service.request(method, args);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(Ok(data)) = rx.recv().await else { return };
            let Some(panel) = weak.upgrade() else { return };
            if !panel.valid(revision, &generation) {
                return;
            }
            let bio = data["bio"]["text"].as_str().or(data["description"].as_str()).unwrap_or("");
            if !bio.is_empty() {
                let label = w::label(bio, "dim");
                label.set_wrap(true);
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.set_max_width_chars(24);
                panel.body.append(&label);
            }
            if let Some(count) = data["member_count"].as_i64() {
                panel.body.append(&w::label(&format!("{count} members"), "dim"));
            }
            if let Some(link) = data["invite_link"]["invite_link"].as_str() {
                panel.body.append(&gtk::LinkButton::with_label(link, "Invite link"));
            }
        });
    }
    fn load_media(self: &Rc<Self>, id: i64, from: i64, button: gtk::Button) {
        if !button.is_sensitive() {
            return;
        }
        let Some(s) = crate::window::service() else { return };
        button.set_sensitive(false);
        let generation = s.generation.clone();
        let revision = self.revision.get();
        let rx=s.request("searchChatMessages",json!({"chat_id":id,"query":"","from_message_id":from,"offset":0,"limit":30,"filter":{"@type":"searchMessagesFilterPhotoAndVideo"}}));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            let Some(p) = weak.upgrade() else { return };
            if !p.valid(revision, &generation) {
                return;
            }
            let data = match result {
                Ok(Ok(data)) => data,
                other => {
                    button.set_sensitive(true);
                    crate::window::show_status(&match other {
                        Ok(Err(e)) => e,
                        _ => "Telegram service stopped".into(),
                    });
                    return;
                }
            };
            p.body.remove(&button);
            let mut last = 0;
            let mut added = 0;
            for m in data["messages"].as_array().into_iter().flatten().filter_map(parse_message) {
                last = m.id;
                if !p.media_ids.borrow_mut().insert(m.id) {
                    continue;
                }
                added += 1;
                let b = gtk::Button::with_label(&crate::chat_view::preview(&m).chars().take(24).collect::<String>());
                let weak = Rc::downgrade(&p);
                let generation = generation.clone();
                b.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        if !p.valid(revision, &generation) {
                            return;
                        }
                        if let Some(cb) = p.on_media.borrow().as_ref() {
                            cb(&m);
                        }
                    }
                });
                p.body.append(&b);
            }
            if last != 0 && last != from && added > 0 {
                let b = gtk::Button::with_label("More media");
                let weak = Rc::downgrade(&p);
                b.connect_clicked(move |b| {
                    if let Some(p) = weak.upgrade() {
                        p.load_media(id, last, b.clone());
                    }
                });
                p.body.append(&b);
            } else {
                p.body.append(&w::label(
                    if from == 0 && added == 0 { "No shared photos or videos" } else { "All media loaded" },
                    "dim",
                ));
            }
        });
    }
    pub fn toggle_visible(self: &Rc<Self>) {
        self.root.set_visible(!self.root.is_visible());
        if self.root.is_visible() {
            let chat = self.chat.borrow().clone();
            if let Some(c) = chat {
                self.refresh(&c);
            }
        }
    }
    pub fn connect_start_secret_chat<F: Fn(i64) + 'static>(&self, f: F) {
        *self.on_secret.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_media<F: Fn(&MessageSummary) + 'static>(&self, f: F) {
        *self.on_media.borrow_mut() = Some(Box::new(f));
    }
}

#[cfg(test)]
pub fn check_layout_and_refresh(chat: &ChatSummary) {
    let panel = DetailPanel::new();
    let header = panel.root.first_child().unwrap();
    assert!(header.has_css_class("detail-header"));
    assert!(header.first_child().unwrap().hexpands());
    assert_eq!(header.margin_start(), 16);
    assert_eq!(header.margin_end(), 16);
    assert!(panel.body.margin_bottom() > 0);
    panel.set_chat(chat);
    let marker = w::label("Loaded media", "dim");
    panel.body.append(&marker);
    let mut update = chat.clone();
    update.unread_count += 1;
    panel.refresh(&update);
    assert!(marker.parent().is_some(), "Unread updates must preserve loaded details");
    update.id += 1;
    panel.set_chat(&update);
    assert!(marker.parent().is_none(), "Changing chats must clear prior details");
    panel.clear();
    assert!(panel.body.first_child().is_none());
}
