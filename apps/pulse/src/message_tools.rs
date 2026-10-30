//! Search, scheduling, auto-delete, mentions and Saved Messages organization.
use crate::{
    actions::{self, Scope},
    window,
};
use chrono::TimeZone;
use gtk::{glib, prelude::*};
use nyx_telegram_core::{client::parse_message, models::*};
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};
#[derive(Clone)]
struct ScheduledView {
    chat: i64,
    scope: Scope,
    list: glib::WeakRef<gtk::Box>,
    revision: Rc<Cell<u64>>,
}
#[derive(Clone)]
struct SavedView {
    scope: Scope,
    list: glib::WeakRef<gtk::Box>,
}
thread_local! {static SCHEDULED:RefCell<Vec<ScheduledView>>=const{RefCell::new(Vec::new())};static TOPICS:RefCell<HashMap<i64,Value>>=RefCell::new(HashMap::new());static SAVED:RefCell<Vec<SavedView>>=const{RefCell::new(Vec::new())};}
pub fn reset() {
    SCHEDULED.with(|s| s.borrow_mut().clear());
    TOPICS.with(|s| s.borrow_mut().clear());
    SAVED.with(|s| s.borrow_mut().clear());
}
pub fn update(v: &Value) {
    if v["@type"] == "updateSavedMessagesTopic" {
        let topic = v["topic"].clone();
        if let Some(id) = topic["id"].as_i64() {
            TOPICS.with(|s| {
                s.borrow_mut().insert(id, topic);
            });
            refresh_saved();
        }
    }
}
fn scroll(body: &gtk::Box, height: i32) -> gtk::Box {
    let list = w::vbox(6);
    body.append(
        &gtk::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(180)
            .max_content_height(height)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .build(),
    );
    list
}
fn clear(body: &gtk::Box) {
    while let Some(child) = body.first_child() {
        body.remove(&child)
    }
}
fn label(body: &gtk::Box, text: &str, class: &str) {
    let l = w::label(text, class);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_max_width_chars(48);
    body.append(&l);
}
fn date_time(text: &str) -> Result<i64, String> {
    let date =
        chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M").map_err(|_| "Use YYYY-MM-DD HH:MM".to_owned())?;
    chrono::Local
        .from_local_datetime(&date)
        .single()
        .map(|d| d.timestamp())
        .ok_or("This local time is ambiguous or does not exist. Choose a different time.".into())
}
fn day(text: &str, end: bool) -> Result<i64, String> {
    if text.trim().is_empty() {
        return Ok(0);
    }
    let date =
        chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").map_err(|_| "Use YYYY-MM-DD for date filters".to_owned())?;
    let date = if end { date.succ_opt().ok_or("Date out of range")? } else { date };
    let time = chrono::Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .ok_or("This date has an ambiguous local midnight")?;
    Ok(time.timestamp() - if end { 1 } else { 0 })
}
const REPEATS: [i64; 9] = [0, 86400, 604800, 1209600, 2592000, 7862400, 15724800, 31536000, 0];
pub fn schedule_options(chat: i64, callback: impl Fn(Value) + 'static) {
    schedule_dialog(chat, true, callback);
}
fn schedule_dialog(chat: i64, allow_silent: bool, callback: impl Fn(Value) + 'static) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Schedule message", 440);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    label(&body, "Choose a local date and time. Repeating messages require Telegram Premium.", "dim");
    let when = gtk::Entry::new();
    when.set_text(&(chrono::Local::now() + chrono::Duration::hours(1)).format("%Y-%m-%d %H:%M").to_string());
    body.append(&w::row("Send at", "YYYY-MM-DD HH:MM", Some(when.upcast_ref())));
    let repeat = gtk::DropDown::from_strings(&[
        "Once",
        "Daily",
        "Weekly",
        "Every 2 weeks",
        "Every 30 days",
        "Every 91 days",
        "Every 182 days",
        "Yearly",
        "When recipient is online",
    ]);
    body.append(&w::row("Repeat / delivery", "", Some(repeat.upcast_ref())));
    let silent = gtk::CheckButton::with_label("Send silently");
    silent.set_visible(allow_silent);
    body.append(&silent);
    let error = w::label("", "error");
    error.set_wrap(true);
    body.append(&error);
    let submit = gtk::Button::with_label("Schedule");
    body.append(&submit);
    let weak = dialog.downgrade();
    let callback = Rc::new(callback);
    submit.connect_clicked(move |button| {
        let Some(dialog) = weak.upgrade() else { return };
        if !scope.valid() {
            dialog.close();
            return;
        }
        let index = repeat.selected() as usize;
        let state = if index == 8 {
            if !window::chat(chat).is_some_and(|c| c.chat_type == ChatType::Private && !c.is_secret) {
                error.set_text("Online delivery is available only in non-secret private chats");
                return;
            }
            json!({"@type":"messageSchedulingStateSendWhenOnline"})
        } else {
            let time = match date_time(&when.text()) {
                Ok(t) if t > chrono::Utc::now().timestamp() + 10 && t <= i32::MAX as i64 => t,
                Ok(_) => {
                    error.set_text("Choose a future date and time");
                    return;
                }
                Err(e) => {
                    error.set_text(&e);
                    return;
                }
            };
            json!({"@type":"messageSchedulingStateSendAtDate","send_date":time,"repeat_period":REPEATS[index]})
        };
        if window::chat(chat).is_some_and(|c| c.is_secret) {
            error.set_text("Telegram does not support scheduled messages in secret chats");
            return;
        }
        if index == 8 {
            let (scope, error, button, callback) = (scope.clone(), error.clone(), button.clone(), callback.clone());
            let silent = silent.is_active();
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                let result = async {
                    let chat = scope.call("getChat", json!({"chat_id":chat})).await?;
                    let user = scope.call("getUser", json!({"user_id":chat["type"]["user_id"]})).await?;
                    if !matches!(user["status"]["@type"].as_str(), Some("userStatusOnline" | "userStatusOffline")) {
                        return Err("The recipient's exact online status is hidden".into());
                    }
                    Ok::<(), String>(())
                }
                .await;
                match result {
                    Ok(()) => {
                        callback(json!({"@type":"messageSendOptions","disable_notification":silent,"scheduling_state":state}));
                        dialog.close()
                    }
                    Err(e) => {
                        if !scope.valid() {
                            dialog.close();
                            return;
                        }
                        error.set_text(&e);
                        button.set_sensitive(true);
                    }
                }
            });
        } else {
            callback(json!({"@type":"messageSendOptions","disable_notification":silent.is_active(),"scheduling_state":state}));
            dialog.close();
        }
    });
    dialog.present();
}
pub fn send_menu(parent: &gtk::Widget, chat: i64, callback: impl Fn(Value) + 'static) {
    let Some(scope) = Scope::current() else { return };
    let callback = Rc::new(callback);
    let (p, body) = actions::popover(parent, None);
    let c = callback.clone();
    let s = scope.clone();
    actions::menu_item(&p, &body, "Send silently", move || {
        if s.valid() {
            c(json!({"@type":"messageSendOptions","disable_notification":true}))
        }
    });
    let c = callback.clone();
    actions::menu_item(&p, &body, "Schedule message…", move || {
        if scope.valid() {
            let c = c.clone();
            schedule_options(chat, move |v| c(v));
        }
    });
    p.popup();
}
pub fn auto_delete(chat: i64) {
    actions::run(chat, "auto-delete-ui", move |scope| async move {
        let data = scope.call("getChat", json!({"chat_id":chat})).await?;
        let (dialog, body) = w::dialog("Auto-delete messages", 420);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        label(
            &body,
            "New messages will be deleted after the selected period. Existing messages are unaffected. Telegram checks your permission to change this setting.",
            "dim",
        );
        let secret = data["type"]["@type"] == "chatTypeSecret";
        let values = if secret { [0, 10, 30, 60] } else { [0, 86400, 604800, 2592000] };
        let choices = gtk::DropDown::from_strings(if secret {
            &["Off", "10 seconds", "30 seconds", "1 minute", "Custom"]
        } else {
            &["Off", "1 day", "1 week", "30 days", "Custom"]
        });
        let current = data["message_auto_delete_time"].as_i64().unwrap_or(0);
        choices.set_selected(values.iter().position(|s| *s == current).map(|i| i as u32).unwrap_or(4));
        body.append(&choices);
        let custom = gtk::SpinButton::with_range(1.0, if secret { 31536000.0 } else { 365.0 }, 1.0);
        custom.set_value((if secret { current } else { current / 86400 }).max(1) as f64);
        body.append(&w::row("Custom period", if secret { "Seconds after opening" } else { "Days" }, Some(custom.upcast_ref())));
        let error = w::label("", "error");
        error.set_wrap(true);
        body.append(&error);
        let save = gtk::Button::with_label("Apply");
        body.append(&save);
        let weak = dialog.downgrade();
        save.connect_clicked(move |button| {
            let Some(dialog) = weak.upgrade() else { return };
            let seconds = values
                .get(choices.selected() as usize)
                .copied()
                .unwrap_or(custom.value_as_int() as i64 * if secret { 1 } else { 86400 });
            let (scope, error, button) = (scope.clone(), error.clone(), button.clone());
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                match scope.call("setChatMessageAutoDeleteTime", json!({"chat_id":chat,"message_auto_delete_time":seconds})).await
                {
                    Ok(_) => {
                        window::with_service(|s| s.load_chat(chat));
                        dialog.close()
                    }
                    Err(e) => {
                        if !scope.valid() {
                            dialog.close();
                            return;
                        }
                        error.set_text(&e);
                        button.set_sensitive(true);
                    }
                }
            });
        });
        dialog.present();
        Ok(())
    });
}
pub fn choose_member(chat: i64, callback: impl Fn(Value) + 'static) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Choose a member", 440);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    let query = gtk::SearchEntry::new();
    query.set_placeholder_text(Some("Search names or usernames"));
    body.append(&query);
    let list = scroll(&body, 400);
    let version = Rc::new(Cell::new(0u64));
    let callback = Rc::new(callback);
    let weak = dialog.downgrade();
    let load: Rc<dyn Fn(String)> = Rc::new(move |query| {
        let n = version.get() + 1;
        version.set(n);
        let (version, scope, list, callback, weak) =
            (version.clone(), scope.clone(), list.clone(), callback.clone(), weak.clone());
        glib::spawn_future_local(async move {
            let result = if window::chat(chat).is_some_and(|c| c.chat_type == ChatType::Private) {
                let mut members = Vec::new();
                if let Ok(me) = scope.call("getMe", json!({})).await {
                    members.push(json!({"member_id":{"@type":"messageSenderUser","user_id":me["id"]}}));
                }
                if let Some(id) = window::chat(chat).and_then(|c| c.data["type"]["user_id"].as_i64()) {
                    members.push(json!({"member_id":{"@type":"messageSenderUser","user_id":id}}));
                }
                Ok(json!({"members":members}))
            } else {
                scope.call("searchChatMembers", json!({"chat_id":chat,"query":query,"limit":50,"filter":null})).await
            };
            if version.get() != n || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                return;
            }
            clear(&list);
            let data = match result {
                Ok(d) => d,
                Err(e) => {
                    label(&list, &e, "error");
                    return;
                }
            };
            for member in data["members"].as_array().into_iter().flatten() {
                let Some(user) = member["member_id"]["user_id"].as_i64() else { continue };
                let data = match scope.call("getUser", json!({"user_id":user})).await {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                if version.get() != n {
                    return;
                }
                let name = format!("{} {}", data["first_name"].as_str().unwrap_or(""), data["last_name"].as_str().unwrap_or(""));
                if !name.to_lowercase().contains(&query.to_lowercase())
                    && window::chat(chat).is_some_and(|c| c.chat_type == ChatType::Private)
                {
                    continue;
                }
                let button = gtk::Button::with_label(&name);
                let callback = callback.clone();
                let weak = weak.clone();
                let scope = scope.clone();
                button.connect_clicked(move |_| {
                    if scope.valid() {
                        callback(data.clone())
                    }
                    if let Some(d) = weak.upgrade() {
                        d.close()
                    }
                });
                list.append(&button);
            }
        });
    });
    let initial = load.clone();
    query.connect_search_changed(move |e| load(e.text().to_string()));
    dialog.present();
    initial(String::new());
}
pub fn mention(view: &Rc<crate::chat_view::ChatView>) {
    let Some(chat) = *view.current_chat_id.borrow() else { return };
    let weak = Rc::downgrade(view);
    let topic=view.topic_id();
    choose_member(chat, move |user| {
        if let Some(view) = weak.upgrade() {
            if *view.current_chat_id.borrow() != Some(chat) || view.topic_id()!=topic {
                return;
            }
            let name = format!("{} {}", user["first_name"].as_str().unwrap_or(""), user["last_name"].as_str().unwrap_or(""));
            if let Some(id) = user["id"].as_i64() {
                view.insert_mention(name.trim(), id);
            }
        }
    });
}

#[derive(Clone)]
enum SearchMode {
    Global,
    Chat(i64),
    Saved { topic: i64, tag: Value },
}
struct Browser {
    scope: Scope,
    window: glib::WeakRef<gtk::Window>,
    mode: SearchMode,
    topic: Value,
    query: gtk::SearchEntry,
    filter: gtk::DropDown,
    kind: gtk::DropDown,
    min: gtk::Entry,
    max: gtk::Entry,
    sender: RefCell<Value>,
    list: gtk::Box,
    status: gtk::Label,
    more: gtk::Button,
    cursor: Cell<i64>,
    offset: RefCell<String>,
    revision: Cell<u64>,
    loading: Cell<bool>,
    seen: RefCell<HashSet<(i64, i64)>>,
}
const FILTERS: [&str; 9] = [
    "",
    "searchMessagesFilterPhoto",
    "searchMessagesFilterVideo",
    "searchMessagesFilterDocument",
    "searchMessagesFilterUrl",
    "searchMessagesFilterAudio",
    "searchMessagesFilterVoiceNote",
    "searchMessagesFilterAnimation",
    "searchMessagesFilterPinned",
];
pub fn search(chat: Option<i64>) {
    browser(if let Some(chat) = chat { SearchMode::Chat(chat) } else { SearchMode::Global }, 0);
}
pub fn pinned(chat: i64) {
    browser(SearchMode::Chat(chat), 8);
}
fn browser(mode: SearchMode, filter: u32) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog(
        match mode {
            SearchMode::Global => "Search all messages",
            SearchMode::Chat(_) => "Search conversation",
            SearchMode::Saved { .. } => "Saved Messages",
        },
        560,
    );
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    let query = gtk::SearchEntry::new();
    query.set_placeholder_text(Some("Search messages"));
    body.append(&query);
    let filters = gtk::DropDown::from_strings(if matches!(mode, SearchMode::Global) {
        &["All media", "Photos", "Videos", "Files", "Links", "Music", "Voice messages", "Animations"]
    } else {
        &["All media", "Photos", "Videos", "Files", "Links", "Music", "Voice messages", "Animations", "Pinned"]
    });
    filters.set_selected(filter);
    body.append(&filters);
    let kind = gtk::DropDown::from_strings(&["All cloud chats", "Private chats", "Groups", "Channels", "Secret chats"]);
    kind.set_visible(matches!(mode, SearchMode::Global));
    body.append(&kind);
    let min = gtk::Entry::new();
    min.set_placeholder_text(Some("From YYYY-MM-DD"));
    let max = gtk::Entry::new();
    max.set_placeholder_text(Some("Through YYYY-MM-DD"));
    let dates = w::hbox(6);
    min.set_hexpand(true);
    max.set_hexpand(true);
    dates.append(&min);
    dates.append(&max);
    let apply = gtk::Button::with_label("Search");
    dates.append(&apply);
    body.append(&dates);
    let from = gtk::Button::with_label("From anyone");
    from.set_visible(matches!(mode, SearchMode::Chat(_)));
    body.append(&from);
    let list = scroll(&body, 360);
    let status = w::label("", "dim");
    status.set_wrap(true);
    body.append(&status);
    let more = gtk::Button::with_label("Load more results");
    more.set_visible(false);
    body.append(&more);
    let browser = Rc::new(Browser {
        scope,
        window: dialog.downgrade(),
        topic: if let SearchMode::Chat(chat)=&mode {window::composer_topic(*chat)}else{Value::Null},
        mode,
        query: query.clone(),
        filter: filters.clone(),
        kind: kind.clone(),
        min: min.clone(),
        max: max.clone(),
        sender: RefCell::new(Value::Null),
        list,
        status,
        more: more.clone(),
        cursor: Cell::new(0),
        offset: RefCell::new(String::new()),
        revision: Cell::new(0),
        loading: Cell::new(false),
        seen: RefCell::new(HashSet::new()),
    });
    let weak = Rc::downgrade(&browser);
    query.connect_search_changed(move |_| {
        if let Some(b) = weak.upgrade() {
            b.load(false)
        }
    });
    let weak = Rc::downgrade(&browser);
    filters.connect_selected_notify(move |_| {
        if let Some(b) = weak.upgrade() {
            b.load(false)
        }
    });
    let weak = Rc::downgrade(&browser);
    kind.connect_selected_notify(move |_| {
        if let Some(b) = weak.upgrade() {
            b.load(false)
        }
    });
    for field in [&min, &max] {
        let weak = Rc::downgrade(&browser);
        field.connect_activate(move |_| {
            if let Some(b) = weak.upgrade() {
                b.load(false)
            }
        });
    }
    let weak = Rc::downgrade(&browser);
    apply.connect_clicked(move |_| {
        if let Some(b) = weak.upgrade() {
            b.load(false)
        }
    });
    let weak = Rc::downgrade(&browser);
    more.connect_clicked(move |_| {
        if let Some(b) = weak.upgrade() {
            b.load(true)
        }
    });
    let weak = Rc::downgrade(&browser);
    from.connect_clicked(move |button| {
        let Some(b) = weak.upgrade() else { return };
        let SearchMode::Chat(chat) = b.mode else { return };
        if !b.sender.borrow().is_null() {
            *b.sender.borrow_mut() = Value::Null;
            button.set_label("From anyone");
            b.load(false);
            return;
        }
        let weak = Rc::downgrade(&b);
        let button = button.clone();
        choose_member(chat, move |user| {
            if let Some(b) = weak.upgrade() {
                *b.sender.borrow_mut() = json!({"@type":"messageSenderUser","user_id":user["id"]});
                button.set_label(&format!("From {} · click to clear", user["first_name"].as_str().unwrap_or("member")));
                b.load(false);
            }
        });
    });
    let owner = browser.clone();
    dialog.connect_close_request(move |_| {
        let _ = &owner;
        glib::Propagation::Proceed
    });
    dialog.present();
    browser.load(false);
}
impl Browser {
    fn load(self: &Rc<Self>, append: bool) {
        if append && self.loading.get() {
            return;
        }
        let n = self.revision.get() + 1;
        self.revision.set(n);
        self.loading.set(true);
        self.more.set_sensitive(false);
        let min = match day(&self.min.text(), false) {
            Ok(d) => d,
            Err(e) => {
                self.status.set_text(&e);
                self.loading.set(false);
                return;
            }
        };
        let max = match day(&self.max.text(), true) {
            Ok(d) => d,
            Err(e) => {
                self.status.set_text(&e);
                self.loading.set(false);
                return;
            }
        };
        if min > 0 && max > 0 && min > max {
            self.status.set_text("The starting date must be before the ending date");
            self.loading.set(false);
            return;
        }
        if !append {
            self.cursor.set(0);
            self.offset.borrow_mut().clear();
            self.seen.borrow_mut().clear();
            clear(&self.list);
            self.more.set_visible(false);
        }
        let query = self.query.text().to_string();
        let index = self.filter.selected() as usize;
        let filter = if index == 0 { Value::Null } else { json!({"@type":FILTERS[index]}) };
        if matches!(self.mode, SearchMode::Global) && query.trim().is_empty() && filter.is_null() {
            self.status.set_text("Enter a query or choose a media filter");
            self.loading.set(false);
            return;
        }
        if matches!(self.mode, SearchMode::Global) && self.kind.selected() == 4 && query.is_empty() {
            self.status.set_text("Enter text to search secret chats");
            self.loading.set(false);
            return;
        }
        self.status.set_text("Searching…");
        let this = self.clone();
        let cursor = self.cursor.get();
        let offset = self.offset.borrow().clone();
        let sender = self.sender.borrow().clone();
        let kind = self.kind.selected();
        glib::spawn_future_local(async move {
            let result=async{
                let secret=matches!(this.mode,SearchMode::Chat(chat)if window::chat(chat).is_some_and(|c|c.is_secret))||matches!(this.mode,SearchMode::Global)&&kind==4;
                if secret&&!query.is_empty(){return this.scope.call("searchSecretMessages",json!({"chat_id":if let SearchMode::Chat(c)=this.mode{c}else{0},"query":query,"offset":offset,"limit":50,"filter":filter})).await}
                match &this.mode{
                    SearchMode::Global=>this.scope.call("searchMessages",json!({"chat_list":null,"query":query,"offset":offset,"limit":50,"filter":filter,"chat_type_filter":match kind{1=>json!({"@type":"searchMessagesChatTypeFilterPrivate"}),2=>json!({"@type":"searchMessagesChatTypeFilterGroup"}),3=>json!({"@type":"searchMessagesChatTypeFilterChannel"}),_=>Value::Null},"min_date":min,"max_date":max})).await,
                    SearchMode::Chat(chat)=>{let from=if cursor==0&&max>0{let m=this.scope.call("getChatMessageByDate",json!({"chat_id":chat,"date":max})).await?;m["id"].as_i64().unwrap_or(0)}else{cursor};this.scope.call("searchChatMessages",json!({"chat_id":chat,"topic_id":this.topic,"query":query,"sender_id":sender,"from_message_id":from,"offset":0,"limit":50,"filter":filter})).await},
                    SearchMode::Saved{topic,tag}=>{if tag.is_null()&&query.is_empty()&&*topic!=0{this.scope.call("getSavedMessagesTopicHistory",json!({"saved_messages_topic_id":topic,"from_message_id":cursor,"offset":0,"limit":50})).await}else{this.scope.call("searchSavedMessages",json!({"saved_messages_topic_id":topic,"tag":tag,"query":query,"from_message_id":cursor,"offset":0,"limit":50})).await}},
                }
            }.await;
            if this.revision.get() != n || this.window.upgrade().is_none_or(|d| !d.is_visible()) {
                return;
            }
            this.loading.set(false);
            let data = match result {
                Ok(d) => d,
                Err(e) => {
                    this.status.set_text(&e);
                    this.more.set_sensitive(true);
                    return;
                }
            };
            let messages = data["messages"].as_array().into_iter().flatten().filter_map(parse_message).collect::<Vec<_>>();
            let last = messages.last().map(|m| m.id).unwrap_or(0);
            let below = messages.iter().any(|m| min > 0 && m.date < min);
            let next = data["next_from_message_id"].as_i64().unwrap_or_else(|| if !messages.is_empty() { last } else { 0 });
            let next_offset = data["next_offset"].as_str().unwrap_or("").to_owned();
            this.cursor.set(next);
            *this.offset.borrow_mut() = next_offset.clone();
            for m in messages.into_iter().filter(|m| {
                (min == 0 || m.date >= min)
                    && (max == 0 || m.date <= max)
                    && (sender.is_null() || sender["user_id"].as_i64() == Some(m.sender_id))
                    && (!matches!(this.mode, SearchMode::Saved { .. }) || matches_media(m, index))
            }) {
                if !this.seen.borrow_mut().insert((m.chat_id, m.id)) {
                    continue;
                }
                let title = if let Some(chat) = window::chat(m.chat_id) {
                    chat.title
                } else {
                    match this.scope.call("getChat", json!({"chat_id":m.chat_id})).await {
                        Ok(chat) => chat["title"].as_str().unwrap_or("Conversation").to_owned(),
                        Err(_) => "Conversation".into(),
                    }
                };
                if this.revision.get() != n || !this.scope.valid() {
                    return;
                }
                let date = chrono::DateTime::from_timestamp(m.date, 0)
                    .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let button = gtk::Button::new();
                let contents = w::vbox(3);
                label(&contents, &format!("{title} · {date}"), "dim");
                label(&contents, &crate::chat_view::preview(&m), "");
                button.set_child(Some(&contents));
                let scope = this.scope.clone();
                let weak = this.window.clone();
                button.connect_clicked(move |_| {
                    if scope.valid() {
                        window::jump_message(&m);
                    }
                    if let Some(d) = weak.upgrade() {
                        d.close();
                    }
                });
                this.list.append(&button);
            }
            this.status.set_text(if this.seen.borrow().is_empty() {
                "No matching messages on this page"
            } else {
                "Select a message to open it"
            });
            this.more
                .set_visible(!below && ((next != 0 && next != cursor) || (!next_offset.is_empty() && next_offset != offset)));
            this.more.set_sensitive(true);
        });
    }
}

pub fn scheduled(chat: i64) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Scheduled messages", 540);
    crate::dialog_dismissal::add(&dialog, &body, "Close");
    let list = scroll(&body, 500);
    let refresh = gtk::Button::with_label("Refresh");
    body.append(&refresh);
    let view = ScheduledView { chat, scope, list: list.downgrade(), revision: Rc::new(Cell::new(0)) };
    SCHEDULED.with(|s| s.borrow_mut().push(view.clone()));
    let reload = view.clone();
    refresh.connect_clicked(move |_| load_scheduled(reload.clone()));
    dialog.present();
    load_scheduled(view);
}
pub fn changed(chat: i64) {
    let views = SCHEDULED.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|v| v.scope.valid() && v.list.upgrade().is_some());
        s.iter().filter(|v| v.chat == chat).cloned().collect::<Vec<_>>()
    });
    for view in views {
        load_scheduled(view)
    }
}
fn load_scheduled(view: ScheduledView) {
    let n = view.revision.get() + 1;
    view.revision.set(n);
    glib::spawn_future_local(async move {
        let result = view.scope.call("getChatScheduledMessages", json!({"chat_id":view.chat})).await;
        let Some(list) = view.list.upgrade() else { return };
        if view.revision.get() != n {
            return;
        }
        clear(&list);
        let data = match result {
            Ok(d) => d,
            Err(e) => {
                label(&list, &e, "error");
                return;
            }
        };
        let messages = data["messages"].as_array().into_iter().flatten().filter_map(parse_message).collect::<Vec<_>>();
        if messages.is_empty() {
            label(&list, "No scheduled messages", "dim")
        }
        for message in messages {
            let row = w::vbox(4);
            let state = &message.data["scheduling_state"];
            let time = if state["@type"] == "messageSchedulingStateSendWhenOnline" {
                "When recipient is online".into()
            } else {
                chrono::DateTime::from_timestamp(state["send_date"].as_i64().unwrap_or(0), 0)
                    .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default()
            };
            let repeat = state["repeat_period"].as_i64().unwrap_or(0);
            label(&row, &if repeat > 0 { format!("{time} · repeats every {} days", repeat / 86400) } else { time }, "dim");
            label(&row, &crate::chat_view::preview(&message), "");
            let buttons = w::hbox(6);
            let send = gtk::Button::with_label("Send now");
            let scope = view.scope.clone();
            let (chat, id) = (message.chat_id, message.id);
            send.connect_clicked(move |_| {
                if scope.valid() {
                    actions::operation(
                        "Send scheduled message now?",
                        "This sends the message immediately to this conversation.",
                        vec![],
                        "Send now",
                        move |scope, _| async move {
                            scope
                                .call(
                                    "editMessageSchedulingState",
                                    json!({"chat_id":chat,"message_id":id,"scheduling_state":null}),
                                )
                                .await?;
                            Ok(())
                        },
                    )
                }
            });
            buttons.append(&send);
            let reschedule = gtk::Button::with_label("Reschedule");
            let scope = view.scope.clone();
            reschedule.connect_clicked(move |_| {
                if !scope.valid() {
                    return;
                }
                schedule_dialog(chat, false, move |options| {
                    actions::request(
                        chat,
                        "editMessageSchedulingState",
                        json!({"chat_id":chat,"message_id":id,"scheduling_state":options["scheduling_state"]}),
                    )
                });
            });
            buttons.append(&reschedule);
            let edit = gtk::Button::with_label("Edit");
            edit.set_visible(
                message.data["content"]["@type"] == "messageText" || message.data["content"].get("caption").is_some(),
            );
            let scope = view.scope.clone();
            let m = message.clone();
            edit.connect_clicked(move |_| {
                if scope.valid() {
                    actions::edit_message(m.clone())
                }
            });
            buttons.append(&edit);
            let delete = gtk::Button::with_label("Delete…");
            let scope = view.scope.clone();
            delete.connect_clicked(move |_| {
                if scope.valid() {
                    actions::delete_messages(vec![message.clone()], None)
                }
            });
            buttons.append(&delete);
            row.append(&buttons);
            list.append(&row);
        }
    });
}
pub fn saved() {
    actions::run(0, "saved-browser", move |scope| async move {
        let me = scope.call("getMe", json!({})).await?;
        let own = scope.call("createPrivateChat", json!({"user_id":me["id"],"force":false})).await?;
        let own = own["id"].as_i64().ok_or("Saved Messages unavailable")?;
        let (dialog, body) = w::dialog("Saved Messages", 540);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        let buttons = w::hbox(6);
        let open = gtk::Button::with_label("All saved messages");
        let s = scope.clone();
        let weak = dialog.downgrade();
        open.connect_clicked(move |_| {
            if s.valid() {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                window::open_chat(own)
            }
        });
        buttons.append(&open);
        let tags_button = gtk::Button::with_label("Tags");
        let s = scope.clone();
        tags_button.connect_clicked(move |_| {
            if s.valid() {
                saved_tags()
            }
        });
        buttons.append(&tags_button);
        body.append(&buttons);
        let list = scroll(&body, 460);
        let view = SavedView { scope: scope.clone(), list: list.downgrade() };
        SAVED.with(|s| s.borrow_mut().push(view.clone()));
        let more = gtk::Button::with_label("Load more sources");
        let s = scope.clone();
        more.connect_clicked(move |_| {
            let s = s.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = s.call("loadSavedMessagesTopics", json!({"limit":50})).await {
                    if !e.contains("404") {
                        s.error(&e)
                    }
                }
                refresh_saved();
            });
        });
        body.append(&more);
        dialog.present();
        render_saved(view);
        if let Err(e) = scope.call("loadSavedMessagesTopics", json!({"limit":50})).await {
            if !e.contains("404") {
                scope.error(&e)
            }
        }
        refresh_saved();
        Ok(())
    });
}
fn refresh_saved() {
    let views = SAVED.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|v| v.scope.valid() && v.list.upgrade().is_some());
        s.clone()
    });
    for view in views {
        render_saved(view)
    }
}
fn order(v: &Value) -> i64 {
    v["order"].as_str().and_then(|s| s.parse().ok()).or_else(|| v["order"].as_i64()).unwrap_or(0)
}
fn render_saved(view: SavedView) {
    let Some(list) = view.list.upgrade() else { return };
    clear(&list);
    let mut topics = TOPICS.with(|s| s.borrow().values().filter(|v| order(v) > 0).cloned().collect::<Vec<_>>());
    topics.sort_by_key(|v| std::cmp::Reverse(order(v)));
    if topics.is_empty() {
        label(&list, "No sources loaded yet", "dim")
    }
    for topic in topics {
        let id = topic["id"].as_i64().unwrap_or(0);
        let title = match topic["type"]["@type"].as_str() {
            Some("savedMessagesTopicTypeMyNotes") => "My notes".into(),
            Some("savedMessagesTopicTypeAuthorHidden") => "Hidden authors".into(),
            _ => window::chat(topic["type"]["chat_id"].as_i64().unwrap_or(0))
                .map(|c| c.title)
                .unwrap_or("Saved from a conversation".into()),
        };
        let row = w::hbox(6);
        let open = gtk::Button::with_label(&title);
        open.set_hexpand(true);
        let scope = view.scope.clone();
        let parent = view.list.clone();
        open.connect_clicked(move |_| {
            if scope.valid() {
                if let Some(d) = parent.upgrade().and_then(|l| l.root()).and_then(|r| r.downcast::<gtk::Window>().ok()) {
                    d.close();
                }
                browser(SearchMode::Saved { topic: id, tag: Value::Null }, 0)
            }
        });
        row.append(&open);
        if title == "Saved from a conversation" {
            let scope = view.scope.clone();
            let button = open.downgrade();
            let chat = topic["type"]["chat_id"].clone();
            glib::spawn_future_local(async move {
                if let Ok(chat) = scope.call("getChat", json!({"chat_id":chat})).await {
                    if let Some(button) = button.upgrade() {
                        button.set_label(chat["title"].as_str().unwrap_or("Saved conversation"));
                    }
                }
            });
        }
        let pinned = topic["is_pinned"] == true;
        let pin = gtk::Button::with_label(if pinned { "Unpin" } else { "Pin" });
        let scope = view.scope.clone();
        pin.connect_clicked(move |_| {
            if scope.valid() {
                actions::run(0, "saved-pin", move |scope| async move {
                    scope
                        .call("toggleSavedMessagesTopicIsPinned", json!({"saved_messages_topic_id":id,"is_pinned":!pinned}))
                        .await?;
                    Ok(())
                });
            }
        });
        row.append(&pin);
        list.append(&row);
    }
}
fn saved_tags() {
    actions::run(0, "saved-tags", move |scope| async move {
        let data = scope.call("getSavedMessagesTags", json!({"saved_messages_topic_id":0})).await?;
        let (dialog, body) = w::dialog("Saved Messages tags", 500);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        label(
            &body,
            "Add tags by reacting to messages in Saved Messages. Filtering and labels may require Telegram Premium.",
            "dim",
        );
        let list = scroll(&body, 420);
        if data["tags"].as_array().is_none_or(|v| v.is_empty()) {
            label(&list, "No tags yet", "dim")
        }
        for item in data["tags"].as_array().into_iter().flatten() {
            let row = w::hbox(6);
            let tag = item["tag"].clone();
            let text = format!(
                "{} {} · {}",
                actions::reaction_label(&tag),
                item["label"].as_str().unwrap_or(""),
                item["count"].as_i64().unwrap_or(0)
            );
            let open = gtk::Button::with_label(&text);
            open.set_hexpand(true);
            let open_scope = scope.clone();
            let t = tag.clone();
            let parent = dialog.downgrade();
            open.connect_clicked(move |_| {
                if open_scope.valid() {
                    if let Some(d) = parent.upgrade() {
                        d.close();
                    }
                    browser(SearchMode::Saved { topic: 0, tag: t.clone() }, 0)
                }
            });
            row.append(&open);
            let rename = gtk::Button::with_label("Rename");
            let scope = scope.clone();
            let old = item["label"].as_str().unwrap_or("").to_owned();
            rename.connect_clicked(move |_| {
                if !scope.valid() {
                    return;
                }
                let (dialog, body) = w::dialog("Tag label", 360);
                crate::dialog_dismissal::add(&dialog, &body, "Cancel");
                let entry = gtk::Entry::new();
                entry.set_max_length(12);
                entry.set_text(&old);
                body.append(&entry);
                let save = gtk::Button::with_label("Save");
                let error = w::label("", "error");
                body.append(&error);
                body.append(&save);
                let weak = dialog.downgrade();
                let (scope, tag) = (scope.clone(), tag.clone());
                save.connect_clicked(move |_| {
                    let (scope, tag, text, weak, error) =
                        (scope.clone(), tag.clone(), entry.text().to_string(), weak.clone(), error.clone());
                    glib::spawn_future_local(async move {
                        match scope.call("setSavedMessagesTagLabel", json!({"tag":tag,"label":text})).await {
                            Ok(_) => {
                                if let Some(d) = weak.upgrade() {
                                    d.close()
                                }
                            }
                            Err(e) => error.set_text(&e),
                        }
                    });
                });
                dialog.present();
            });
            row.append(&rename);
            list.append(&row);
        }
        dialog.present();
        Ok(())
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheduling_dates_and_repeat_values() {
        assert!(date_time("not a date").is_err());
        assert_eq!(day("", false).unwrap(), 0);
        assert!(day("2026-02-30", false).is_err());
        assert_eq!(REPEATS[1], 86400);
        assert_eq!(REPEATS[2], 604800);
    }
}

fn matches_media(message: &MessageSummary, index: usize) -> bool {
    match index {
        0 => true,
        1 => message.data["content"]["@type"] == "messagePhoto",
        2 => message.data["content"]["@type"] == "messageVideo",
        3 => message.data["content"]["@type"] == "messageDocument",
        4 => {
            let text = actions::message_text(message);
            text["entities"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|e| matches!(e["type"]["@type"].as_str(), Some("textEntityTypeUrl" | "textEntityTypeTextUrl")))
                || message.data["content"].get("web_page").is_some_and(|p| !p.is_null())
        }
        5 => message.data["content"]["@type"] == "messageAudio",
        6 => message.data["content"]["@type"] == "messageVoiceNote",
        7 => message.data["content"]["@type"] == "messageAnimation",
        8 => message.is_pinned,
        _ => false,
    }
}
