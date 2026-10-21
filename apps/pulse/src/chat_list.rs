//! Conversation list in the sidebar: filters, pinned chats, and conversation rows.

use gtk::gdk;
use gtk::gio;
use gtk::prelude::*;
use nixe_telegram_core::models::{ChatId, ChatListId, ChatSummary, MessageContent};
use nixe_ui::widgets as w;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChatFilter {
    All,
    Unread,
}

struct Inner {
    filter: ChatFilter,
    list: ChatListId,
    search_query: String,
    all_chats: Vec<ChatSummary>,
    active_chat_id: Option<ChatId>,
    buttons: HashMap<ChatId, gtk::Button>,
    on_select: Option<Rc<dyn Fn(ChatId)>>,
}

pub struct ChatList {
    pub root: gtk::Box,
    pinned_group: gtk::Box,
    pinned_box: gtk::Box,
    chats_group: gtk::Box,
    chats_box: gtk::Box,
    empty_note: gtk::Label,
    filter_all_btn: gtk::Button,
    filter_unread_btn: gtk::Button,
    archive_btn: gtk::Button,
    folder_box: gtk::Box,
    folder_buttons: RefCell<Vec<(i32, gtk::Button)>>,
    load_limit: Cell<i32>,
    compact: Cell<bool>,
    inner: Rc<RefCell<Inner>>,
}

impl ChatList {
    pub fn new(search: &gtk::SearchEntry) -> Rc<Self> {
        let root = w::vbox(0);
        root.add_css_class("chat-list-root");
        root.set_hexpand(false);

        // Quick Category / Folder navigation buttons
        let (all_btn, _) = nav_button("mail-inbox-symbolic", "All Chats", "All active conversations");
        let (unread_btn, _) = nav_button("mail-unread-symbolic", "Unread", "Conversations with unread messages");

        all_btn.add_css_class("active");
        root.append(&all_btn);
        root.append(&unread_btn);
        let (archive_btn, _) = nav_button("folder-symbolic", "Archive", "Archived conversations");
        root.append(&archive_btn);
        let folder_box = w::vbox(0);
        root.append(&folder_box);
        let (manage, _) = nav_button("folder-new-symbolic", "Manage folders", "Create, edit, reorder and share folders");
        manage.connect_clicked(|_| crate::folders::manage());
        root.append(&manage);
        let (find, _) = nav_button("edit-find-symbolic", "Search messages", "Search all Telegram messages");
        find.connect_clicked(|_| crate::message_tools::search(None));
        root.append(&find);
        let (saved, _) = nav_button("user-bookmarks-symbolic", "Saved Messages", "Browse Saved Messages by source and tag");
        saved.connect_clicked(|_| crate::message_tools::saved());
        root.append(&saved);

        let (create, _) = nav_button("list-add-symbolic", "New group or channel", "Create a private group, forum or channel");
        create.connect_clicked(|_| crate::groups::create());
        root.append(&create);

        let (tools, _) = nav_button("applications-system-symbolic", "Tools", "Account, downloads, stories, Business and more");
        tools.connect_clicked(|_| crate::features::hub());
        root.append(&tools);

        // Pinned Group & Container
        let pinned_group = nav_group("Pinned", false, None);
        let pinned_box = w::vbox(1);
        root.append(&pinned_group);
        root.append(&pinned_box);

        // Chats Group & Container
        let chats_group = nav_group("Chats", false, None);
        let chats_box = w::vbox(1);
        root.append(&chats_group);
        root.append(&chats_box);

        let empty_note = w::label("No matching conversations", "dim");
        empty_note.set_margin_top(16);
        empty_note.set_margin_bottom(16);
        empty_note.set_visible(false);
        root.append(&empty_note);

        let inner = Rc::new(RefCell::new(Inner {
            filter: ChatFilter::All,
            list: ChatListId::Main,
            search_query: String::new(),
            all_chats: Vec::new(),
            active_chat_id: None,
            buttons: HashMap::new(),
            on_select: None,
        }));

        let list_view = Rc::new(Self {
            root,
            pinned_group,
            pinned_box,
            chats_group,
            chats_box,
            empty_note,
            filter_all_btn: all_btn.clone(),
            filter_unread_btn: unread_btn.clone(),
            archive_btn: archive_btn.clone(),
            folder_box,
            folder_buttons: RefCell::new(vec![(-1, manage), (-2, find), (-3, saved), (-4, create)]),
            load_limit: Cell::new(50),
            compact: Cell::new(false),
            inner,
        });

        // Filter button handlers
        {
            let lv = list_view.clone();
            all_btn.connect_clicked(move |b| {
                for (_, button) in lv.folder_buttons.borrow().iter() {
                    button.remove_css_class("active");
                }
                lv.archive_btn.remove_css_class("active");
                lv.inner.borrow_mut().list = ChatListId::Main;
                lv.filter_all_btn.add_css_class("active");
                lv.filter_unread_btn.remove_css_class("active");
                lv.inner.borrow_mut().filter = ChatFilter::All;
                lv.rebuild_list();
                b.grab_focus();
            });
        }
        {
            let lv = list_view.clone();
            unread_btn.connect_clicked(move |b| {
                for (_, button) in lv.folder_buttons.borrow().iter() {
                    button.remove_css_class("active");
                }
                lv.archive_btn.remove_css_class("active");
                lv.inner.borrow_mut().list = ChatListId::Main;
                lv.filter_unread_btn.add_css_class("active");
                lv.filter_all_btn.remove_css_class("active");
                lv.inner.borrow_mut().filter = ChatFilter::Unread;
                lv.rebuild_list();
                b.grab_focus();
            });
        }

        {
            let lv = list_view.clone();
            archive_btn.connect_clicked(move |_| {
                for (_, button) in lv.folder_buttons.borrow().iter() {
                    button.remove_css_class("active");
                }
                lv.inner.borrow_mut().list = ChatListId::Archive;
                lv.inner.borrow_mut().filter = ChatFilter::All;
                lv.filter_all_btn.remove_css_class("active");
                lv.filter_unread_btn.remove_css_class("active");
                lv.archive_btn.add_css_class("active");
                lv.rebuild_list();
                crate::window::with_service(|s| s.load_chat_list(ChatListId::Archive, lv.load_limit.get()));
            });
        }
        let more = w::bar_button("view-more-symbolic", "Load more chats");
        more.add_css_class("flat");
        more.set_tooltip_text(Some("Load more conversations from this list"));
        let weak = Rc::downgrade(&list_view);
        more.connect_clicked(move |_| {
            if let Some(list) = weak.upgrade() {
                let limit = list.load_limit.get().saturating_add(50);
                list.load_limit.set(limit);
                crate::window::with_service(|s| s.load_chat_list(list.current_list(), limit));
            }
        });
        list_view.root.append(&more);

        // Live search filtering
        {
            let lv = list_view.clone();
            search.connect_search_changed(move |entry| {
                lv.inner.borrow_mut().search_query = entry.text().trim().to_lowercase();
                lv.rebuild_list();
            });
        }

        list_view
    }

    pub fn refresh_folder_labels(&self, infos: &[serde_json::Value]) {
        for (id, button) in self.folder_buttons.borrow().iter() {
            if let Some(info) = infos.iter().find(|v| v["id"].as_i64() == Some(*id as i64)) {
                if let Some(label) =
                    button.child().and_downcast::<gtk::Box>().and_then(|b| b.last_child()).and_downcast::<gtk::Label>()
                {
                    label.set_text(&crate::folders::title(info));
                }
            }
        }
    }
    pub fn set_folders(self: &Rc<Self>, infos: &[serde_json::Value]) {
        while let Some(child) = self.folder_box.first_child() {
            self.folder_box.remove(&child);
        }
        self.folder_buttons.borrow_mut().retain(|(id, _)| *id < 0);
        let active = self.current_list();
        if let ChatListId::Folder(id) = active {
            if !infos.iter().any(|i| i["id"].as_i64() == Some(id as i64)) {
                self.inner.borrow_mut().list = ChatListId::Main;
                self.filter_all_btn.add_css_class("active");
            }
        }
        for info in infos {
            let Some(id) = info["id"].as_i64().map(|id| id as i32) else { continue };
            let (button, _) = nav_button("folder-symbolic", &crate::folders::title(info), &crate::folders::name(info));
            if self.current_list() == ChatListId::Folder(id) {
                button.add_css_class("active");
            }
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(list) = weak.upgrade() {
                    list.inner.borrow_mut().list = ChatListId::Folder(id);
                    list.inner.borrow_mut().filter = ChatFilter::All;
                    for button in [&list.filter_all_btn, &list.filter_unread_btn, &list.archive_btn] {
                        button.remove_css_class("active");
                    }
                    for (other, button) in list.folder_buttons.borrow().iter() {
                        if *other == id {
                            button.add_css_class("active");
                        } else {
                            button.remove_css_class("active");
                        }
                    }
                    list.rebuild_list();
                    crate::window::with_service(|s| s.load_chat_list(ChatListId::Folder(id), list.load_limit.get()));
                }
            });
            self.folder_box.append(&button);
            self.folder_buttons.borrow_mut().push((id, button));
        }
        self.rebuild_list();
        self.set_compact(self.compact.get());
    }

    /// Sets the callback invoked when a conversation is selected.
    pub fn connect_chat_selected<F: Fn(ChatId) + 'static>(&self, callback: F) {
        self.inner.borrow_mut().on_select = Some(Rc::new(callback));
    }

    /// Retrieves a chat summary by ID if present.
    pub fn get_chat(&self, chat_id: ChatId) -> Option<ChatSummary> {
        self.inner.borrow().all_chats.iter().find(|c| c.id == chat_id).cloned()
    }

    /// Retrieves all cached chat summaries.
    pub fn reset(&self) {
        self.inner.borrow_mut().active_chat_id = None;
        self.inner.borrow_mut().all_chats.clear();
        self.load_limit.set(50);
        self.inner.borrow_mut().list = ChatListId::Main;
        self.inner.borrow_mut().filter = ChatFilter::All;
        while let Some(child) = self.folder_box.first_child() {
            self.folder_box.remove(&child);
        }
        self.folder_buttons.borrow_mut().retain(|(id, _)| *id < 0);
        self.rebuild_list();
    }

    pub fn get_all_chats(&self) -> Vec<ChatSummary> {
        self.inner.borrow().all_chats.clone()
    }

    /// Selects a conversation by ID.
    pub fn select_chat(&self, chat_id: ChatId) {
        Self::do_select_chat(&self.inner, chat_id);
    }

    fn do_select_chat(inner: &Rc<RefCell<Inner>>, chat_id: ChatId) {
        {
            let mut inr = inner.borrow_mut();
            inr.active_chat_id = Some(chat_id);
            for (&id, btn) in inr.buttons.iter() {
                if id == chat_id {
                    btn.add_css_class("active");
                } else {
                    btn.remove_css_class("active");
                }
            }
        }
        let cb = inner.borrow().on_select.clone();
        if let Some(cb) = cb {
            cb(chat_id);
        }
    }

    /// Populates the conversation list.
    pub fn current_list(&self) -> ChatListId {
        self.inner.borrow().list
    }
    pub fn clear_selection(&self) {
        self.inner.borrow_mut().active_chat_id = None;
        for button in self.inner.borrow().buttons.values() {
            button.remove_css_class("active");
        }
    }
    pub fn update_chats(&self, chats: &[ChatSummary]) {
        {
            let mut inner = self.inner.borrow_mut();
            for chat in chats {
                if let Some(old) = inner.all_chats.iter_mut().find(|c| c.id == chat.id) {
                    *old = chat.clone();
                } else {
                    inner.all_chats.push(chat.clone());
                }
            }
        }
        self.rebuild_list();
        if self.inner.borrow().active_chat_id.is_none() {
            let chosen = {
                let inner = self.inner.borrow();
                inner
                    .all_chats
                    .iter()
                    .filter(|c| c.list_position(inner.list).is_some())
                    .find(|c| Some(c.id) == crate::prefs::get().last_chat_id)
                    .map(|c| c.id)
            };
            if let Some(id) = chosen {
                self.select_chat(id);
            }
        }
    }
    pub fn upsert_chat(&self, chat: &ChatSummary) {
        let (list, query, filter, active) = {
            let inner = self.inner.borrow();
            (inner.list, inner.search_query.clone(), inner.filter, inner.active_chat_id)
        };
        {
            let mut inner = self.inner.borrow_mut();
            if let Some(old) = inner.all_chats.iter_mut().find(|c| c.id == chat.id) {
                *old = chat.clone();
            } else {
                inner.all_chats.push(chat.clone());
            }
            inner.all_chats.sort_by_key(|c| std::cmp::Reverse(c.list_position(list).map(|p| p.0).unwrap_or(0)));
        }
        let old = self.inner.borrow_mut().buttons.remove(&chat.id);
        let focused = old.as_ref().is_some_and(|b| b.has_focus());
        if let Some(old) = old {
            if let Some(parent) = old.parent().and_downcast::<gtk::Box>() {
                parent.remove(&old);
            }
        }
        let visible = |c: &ChatSummary| {
            c.list_position(list).is_some()
                && if !query.is_empty() {
                    c.title.to_lowercase().contains(&query)
                        || c.last_message.as_ref().is_some_and(|m| match &m.content {
                            MessageContent::Text(t) => t.text.to_lowercase().contains(&query),
                            _ => false,
                        })
                } else {
                    filter != ChatFilter::Unread || c.unread_count > 0 || c.is_marked_unread()
                }
        };
        if visible(chat) {
            let projected = chat.for_list(list).unwrap();
            let pinned = projected.is_pinned && query.is_empty();
            let container = if pinned { &self.pinned_box } else { &self.chats_box };
            let preceding = {
                let inner = self.inner.borrow();
                let index = inner.all_chats.iter().position(|c| c.id == chat.id).unwrap();
                inner.all_chats[..index]
                    .iter()
                    .rev()
                    .find(|c| visible(c) && (c.list_position(list).is_some_and(|p| p.1) && query.is_empty()) == pinned)
                    .and_then(|c| inner.buttons.get(&c.id).cloned())
            };
            let (button, _) = create_chat_item(&projected, active == Some(chat.id), self.compact.get());
            let weak = Rc::downgrade(&self.inner);
            let id = chat.id;
            button.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    Self::do_select_chat(&inner, id)
                }
            });
            bind_row_actions(&button, chat.id);
            container.insert_child_after(&button, preceding.as_ref());
            if focused {
                button.grab_focus();
            }
            self.inner.borrow_mut().buttons.insert(chat.id, button);
        }
        self.set_compact(self.compact.get());
    }

    pub fn remove_chat(&self, id: i64) {
        {
            let mut inner = self.inner.borrow_mut();
            inner.all_chats.retain(|c| c.id != id);
            if inner.active_chat_id == Some(id) {
                inner.active_chat_id = None;
            }
        }
        if let Some(button) = self.inner.borrow_mut().buttons.remove(&id) {
            if let Some(parent) = button.parent().and_downcast::<gtk::Box>() {
                parent.remove(&button);
            }
        }
        self.set_compact(self.compact.get());
    }

    /// Updates compact (icon-only) state across all chat list components.
    pub fn set_compact(&self, compact: bool) {
        self.compact.set(compact);
        let inr = self.inner.borrow();
        let query_empty = inr.search_query.is_empty();

        let has_pinned = self.pinned_box.first_child().is_some();
        let has_chats = self.chats_box.first_child().is_some();

        if compact {
            self.filter_all_btn.set_visible(true);
            self.filter_unread_btn.set_visible(true);
            self.pinned_group.set_visible(false);
            self.chats_group.set_visible(false);
            self.empty_note.set_visible(false);
        } else if query_empty {
            self.filter_all_btn.set_visible(true);
            self.filter_unread_btn.set_visible(true);
            self.pinned_group.set_visible(has_pinned);
            self.chats_group.set_visible(has_chats);
            self.empty_note.set_visible(!has_pinned && !has_chats);
        } else {
            self.filter_all_btn.set_visible(false);
            self.filter_unread_btn.set_visible(false);
            self.pinned_group.set_visible(false);
            self.chats_group.set_visible(has_chats);
            self.empty_note.set_visible(!has_chats);
        }

        // Update category filter buttons
        let folder_buttons = self.folder_buttons.borrow();
        for btn in [&self.filter_all_btn, &self.filter_unread_btn, &self.archive_btn]
            .into_iter()
            .chain(folder_buttons.iter().map(|(_, b)| b))
        {
            if let Some(content) = btn.child().and_downcast::<gtk::Box>() {
                content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
                content.set_spacing(if compact { 0 } else { 12 });
                if let Some(l) = content.last_child() {
                    l.set_visible(!compact);
                }
            }
        }

        for container in [&self.pinned_box, &self.chats_box] {
            let mut child = container.first_child();
            while let Some(btn_widget) = child {
                if let Some(btn) = btn_widget.downcast_ref::<gtk::Button>() {
                    if let Some(content) = btn.child().and_downcast::<gtk::Box>() {
                        content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
                        content.set_spacing(if compact { 0 } else { 10 });

                        // Avatar overlay unread pip
                        if let Some(first) = content.first_child() {
                            if let Some(ov) = first.downcast_ref::<gtk::Overlay>() {
                                if let Some(pip) = ov.last_child() {
                                    if pip.has_css_class("unread-pip") {
                                        let has_unread = btn.tooltip_text().map(|t| t.contains("unread")).unwrap_or(false);
                                        pip.set_visible(compact && has_unread);
                                    }
                                }
                            }
                            // Info box is the next sibling of avatar
                            if let Some(info) = first.next_sibling() {
                                info.set_visible(!compact);
                            }
                        }
                    }
                }
                child = btn_widget.next_sibling();
            }
        }
    }

    fn rebuild_list(&self) {
        // Clear containers
        while let Some(child) = self.pinned_box.first_child() {
            self.pinned_box.remove(&child);
        }
        while let Some(child) = self.chats_box.first_child() {
            self.chats_box.remove(&child);
        }
        self.inner.borrow_mut().buttons.clear();

        let (query, current_filter, chats, active_id) = {
            let inr = self.inner.borrow();
            (inr.search_query.clone(), inr.filter, inr.all_chats.clone(), inr.active_chat_id)
        };

        let list = self.current_list();
        let mut chats = chats.into_iter().filter_map(|c| c.for_list(list)).collect::<Vec<_>>();
        chats.sort_by(|a, b| b.order.cmp(&a.order));
        let compact = self.compact.get();
        let mut pinned_count = 0;
        let mut normal_count = 0;

        for chat in chats {
            // Apply search filter
            if !query.is_empty() {
                let matches_title = chat.title.to_lowercase().contains(&query);
                let matches_snippet = chat
                    .last_message
                    .as_ref()
                    .map(|m| match &m.content {
                        MessageContent::Text(t) => t.text.to_lowercase().contains(&query),
                        _ => false,
                    })
                    .unwrap_or(false);

                if !matches_title && !matches_snippet {
                    continue;
                }
            }

            // Apply category filter
            if query.is_empty() && current_filter == ChatFilter::Unread && chat.unread_count == 0 && !chat.is_marked_unread() {
                continue;
            }

            let is_active = active_id == Some(chat.id);
            let (btn, _) = create_chat_item(&chat, is_active, compact);

            let id = chat.id;
            let inner_clone = self.inner.clone();
            btn.connect_clicked(move |_| {
                Self::do_select_chat(&inner_clone, id);
            });

            bind_row_actions(&btn, chat.id);
            self.inner.borrow_mut().buttons.insert(chat.id, btn.clone());

            if chat.is_pinned && query.is_empty() {
                self.pinned_box.append(&btn);
                pinned_count += 1;
            } else {
                self.chats_box.append(&btn);
                normal_count += 1;
            }
        }

        // Manage filter buttons and group header visibility
        if compact {
            self.filter_all_btn.set_visible(true);
            self.filter_unread_btn.set_visible(true);
            self.pinned_group.set_visible(false);
            self.pinned_box.set_visible(true);
            self.chats_group.set_visible(false);
            self.chats_box.set_visible(true);
            self.empty_note.set_visible(false);
        } else if !query.is_empty() {
            self.filter_all_btn.set_visible(false);
            self.filter_unread_btn.set_visible(false);
            self.pinned_group.set_visible(false);
            self.pinned_box.set_visible(false);
            self.chats_group.set_visible(normal_count > 0);
            self.chats_box.set_visible(normal_count > 0);
            self.empty_note.set_visible(normal_count == 0);
        } else {
            self.filter_all_btn.set_visible(true);
            self.filter_unread_btn.set_visible(true);
            self.pinned_group.set_visible(pinned_count > 0);
            self.pinned_box.set_visible(pinned_count > 0);
            self.chats_group.set_visible(normal_count > 0);
            self.chats_box.set_visible(normal_count > 0);
            self.empty_note.set_visible(pinned_count == 0 && normal_count == 0);
        }
    }
}

pub fn nav_group(title: &str, first: bool, add: Option<(&str, Box<dyn Fn()>)>) -> gtk::Box {
    let row = w::hbox(4);
    row.add_css_class("nav-group");
    row.add_css_class("compact-hide");
    row.set_hexpand(false);
    if first {
        row.add_css_class("first");
    }
    let l = w::label(&title.to_uppercase(), "nav-group-label");
    l.set_hexpand(true);
    l.add_css_class("compact-hide");
    row.append(&l);
    if let Some((tip, f)) = add {
        let b = w::icon_button("list-add-symbolic", tip);
        b.add_css_class("nav-add");
        b.add_css_class("compact-hide");
        b.connect_clicked(move |_| f());
        row.append(&b);
    }
    row
}

pub fn nav_button(icon: &str, title: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let l = w::label(title, "nav-label");
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.add_css_class("compact-hide");
    content.append(&l);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    (button, l)
}

fn create_chat_item(chat: &ChatSummary, is_active: bool, compact: bool) -> (gtk::Button, gtk::Box) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    button.add_css_class("chat-item");
    if is_active {
        button.add_css_class("active");
    }

    let content = gtk::Box::new(gtk::Orientation::Horizontal, if compact { 0 } else { 10 });
    content.set_hexpand(true);
    content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });

    // 32x32 Avatar circle with optional compact unread pip
    let avatar_widget = build_avatar_widget(chat, 32);

    let avatar_overlay = gtk::Overlay::new();
    avatar_overlay.set_child(Some(&avatar_widget));
    avatar_overlay.set_halign(gtk::Align::Center);
    avatar_overlay.set_valign(gtk::Align::Center);

    let pip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    pip.add_css_class("unread-pip");
    pip.add_css_class("compact-show");
    pip.set_visible(compact && (chat.unread_count > 0 || chat.is_marked_unread()));
    pip.set_halign(gtk::Align::End);
    pip.set_valign(gtk::Align::Start);
    avatar_overlay.add_overlay(&pip);

    content.append(&avatar_overlay);

    // Info container with compact-hide
    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.add_css_class("compact-hide");
    info.set_visible(!compact);
    info.set_hexpand(true);
    info.set_valign(gtk::Align::Center);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let title = if chat.is_secret {
        let l = w::label(&format!("🔒 {}", chat.title), "chat-title");
        l.add_css_class("secret-chat");
        l
    } else {
        w::label(&chat.title, "chat-title")
    };
    title.set_hexpand(true);
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&title);

    let time_str = if let Some(ref m) = chat.last_message { format_time(m.date) } else { String::new() };
    let time_lbl = w::label(&time_str, "chat-time");
    top.append(&time_lbl);
    info.append(&top);

    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let snippet = chat
        .last_message
        .as_ref()
        .map(|m| match &m.content {
            MessageContent::Text(t) => t.text.clone(),
            MessageContent::Photo { .. } => "📷 Photo".into(),
            MessageContent::Video { .. } => "📹 Video".into(),
            MessageContent::Document { file_name, .. } => format!("📎 {file_name}"),
            MessageContent::VoiceNote { .. } => "🎤 Voice message".into(),
            MessageContent::Call { is_video, .. } => {
                if *is_video {
                    "📹 Video call".into()
                } else {
                    "📞 Call".into()
                }
            }
            _ => "Message".into(),
        })
        .unwrap_or_default();

    let clean_snippet = snippet.lines().next().unwrap_or("").trim();
    let snippet_lbl = w::label(clean_snippet, "chat-snippet");
    snippet_lbl.set_hexpand(true);
    snippet_lbl.set_xalign(0.0);
    snippet_lbl.set_single_line_mode(true);
    snippet_lbl.set_lines(1);
    snippet_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    bottom.append(&snippet_lbl);

    if chat.is_muted {
        top.append(&w::label("⌁", "dim"));
    }
    if chat.unread_count > 0 || chat.is_marked_unread() {
        let badge = w::label(&if chat.unread_count > 0 { chat.unread_count.to_string() } else { "●".into() }, "unread-badge");
        badge.set_halign(gtk::Align::End);
        bottom.append(&badge);
    }
    info.append(&bottom);
    let tags = crate::folders::tags(chat);
    if !tags.is_empty() {
        let label = w::label(&tags.join(" · "), "folder-tags");
        label.set_max_width_chars(22);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        info.append(&label);
    }

    content.append(&info);
    button.set_child(Some(&content));

    let tooltip = if chat.unread_count > 0 || chat.is_marked_unread() {
        format!("{} ({} unread)", chat.title, chat.unread_count)
    } else {
        chat.title.clone()
    };
    button.set_tooltip_text(Some(&tooltip));

    (button, content)
}

fn format_time(timestamp: i64) -> String {
    use chrono::{Local, TimeZone};
    if let Some(dt) = Local.timestamp_opt(timestamp, 0).single() {
        let now = Local::now();
        if dt.date_naive() == now.date_naive() { dt.format("%H:%M").to_string() } else { dt.format("%b %d").to_string() }
    } else {
        String::new()
    }
}

/// Constructs a circular profile picture widget (or centered fallback initials) for a chat.
pub fn build_avatar_widget(chat: &ChatSummary, size: i32) -> gtk::Widget {
    // 1. Try loading texture from chat.avatar_path
    if let Some(ref path) = chat.avatar_path {
        let p = std::path::Path::new(path);
        if p.exists() {
            let file = gio::File::for_path(p);
            if let Ok(texture) = gdk::Texture::from_file(&file) {
                return build_texture_avatar(&texture, size);
            }
        }
    }

    // 3. Fallback letter circle with exact optical centering
    build_initials_avatar(&chat.title, size)
}

fn build_texture_avatar(texture: &gdk::Texture, size: i32) -> gtk::Widget {
    let base = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    base.set_size_request(size, size);
    base.set_hexpand(false);
    base.set_vexpand(false);

    let pic = gtk::Picture::for_paintable(texture);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_can_shrink(true);
    pic.set_size_request(size, size);
    pic.set_hexpand(false);
    pic.set_vexpand(false);

    let frame = gtk::Overlay::new();
    frame.set_size_request(size, size);
    frame.set_hexpand(false);
    frame.set_vexpand(false);
    frame.set_halign(gtk::Align::Center);
    frame.set_valign(gtk::Align::Center);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.add_css_class("chat-avatar-mini");
    frame.set_child(Some(&base));
    frame.add_overlay(&pic);
    frame.upcast()
}

fn build_initials_avatar(title: &str, size: i32) -> gtk::Widget {
    let initials = title.split_whitespace().take(2).filter_map(|s| s.chars().next()).collect::<String>().to_uppercase();
    let text = if initials.is_empty() { "T".into() } else { initials };

    let frame = gtk::Overlay::new();
    frame.set_size_request(size, size);
    frame.set_hexpand(false);
    frame.set_vexpand(false);
    frame.set_halign(gtk::Align::Center);
    frame.set_valign(gtk::Align::Center);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.add_css_class("chat-avatar-mini");

    let lbl = w::label(&text, "chat-avatar-initials");
    lbl.set_xalign(0.5);
    lbl.set_yalign(0.5);
    lbl.set_justify(gtk::Justification::Center);
    lbl.set_halign(gtk::Align::Center);
    lbl.set_valign(gtk::Align::Center);
    lbl.set_size_request(size, size);
    frame.set_child(Some(&lbl));

    frame.upcast()
}

fn bind_row_actions(button: &gtk::Button, id: ChatId) {
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let weak = button.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        if let Some(button) = weak.upgrade() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            button.grab_focus();
            show_chat_menu(&button, id, Some((x, y)));
        }
    });
    button.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = button.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gdk::Key::Delete
            && !modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK
                    | gdk::ModifierType::SHIFT_MASK,
            )
        {
            crate::window::request_delete_chat(id);
            return gtk::glib::Propagation::Stop;
        }
        if key == gdk::Key::Menu || (key == gdk::Key::F10 && modifiers.contains(gdk::ModifierType::SHIFT_MASK)) {
            if let Some(button) = weak.upgrade() {
                show_chat_menu(&button, id, None);
            }
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    button.add_controller(keys);
}

fn show_chat_menu(button: &gtk::Button, id: ChatId, position: Option<(f64, f64)>) {
    crate::actions::show_chat_menu(button.upcast_ref(), id, position);
}

#[cfg(test)]
pub fn check_chat_removal(chat: &ChatSummary) {
    let list = ChatList::new(&gtk::SearchEntry::new());
    let mut chat = chat.clone();
    chat.order = 1;
    list.update_chats(&[chat.clone()]);
    list.select_chat(chat.id);
    assert_eq!(list.inner.borrow().active_chat_id, Some(chat.id));
    list.remove_chat(chat.id);
    assert!(list.inner.borrow().active_chat_id.is_none());
    assert!(list.get_all_chats().is_empty());
    assert!(list.empty_note.is_visible());
    chat.data["positions"] = serde_json::json!([{"list":{"@type":"chatListMain"},"order":"0","is_pinned":false},{"list":{"@type":"chatListArchive"},"order":"500","is_pinned":true}]);
    list.upsert_chat(&chat);
    assert!(list.get_chat(chat.id).is_some(), "Archive must remain in the account cache");
    assert!(list.inner.borrow().buttons.is_empty(), "Archived chat is absent from Main");
    list.inner.borrow_mut().list = ChatListId::Archive;
    list.rebuild_list();
    assert_eq!(list.inner.borrow().buttons.len(), 1);
    assert!(list.pinned_box.first_child().is_some());
    chat.data["positions"] = serde_json::json!([{"list":{"@type":"chatListMain"},"order":600,"is_pinned":false}]);
    chat.data["is_marked_as_unread"] = serde_json::json!(true);
    list.upsert_chat(&chat);
    assert!(list.inner.borrow().buttons.is_empty(), "Unarchive removes the row from Archive");
    list.inner.borrow_mut().list = ChatListId::Main;
    list.inner.borrow_mut().filter = ChatFilter::Unread;
    list.rebuild_list();
    assert_eq!(list.inner.borrow().buttons.len(), 1, "Manual unread is visible even with zero unread messages");
}
