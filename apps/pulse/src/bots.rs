//! Telegram bot commands, reply/inline keyboards, callbacks, and inline result delivery.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button},
    groups::{entry, note, submit},
    window,
};
use gtk::{gio, glib, prelude::*};
use nyx_telegram_core::{client::parse_message, models::MessageSummary};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
fn launch(url: &str) {
    if url.starts_with("tg://") || url.starts_with("https://t.me/") {
        window::resolve_link(url);
    } else if url.starts_with("https://") || url.starts_with("http://") {
        let _ = gio::AppInfo::launch_default_for_uri(url, gio::AppLaunchContext::NONE);
    } else {
        window::show_status("Unsupported link scheme.");
    }
}
async fn send(s: &Scope, chat: i64, topic: Value, reply: Value, content: Value) -> Result<(), String> {
    let data = crate::groups::commit(
        s,
        "sendMessage",
        json!({
        "chat_id":chat,"topic_id":topic,"reply_to":reply,"options":{
        "@type":"messageSendOptions","paid_message_star_count":0}
        ,"input_message_content":content}
        ),
    )
    .await?;
    if let Some(m) = parse_message(&data) {
        window::display_message(&m);
    }
    Ok(())
}
fn send_text(chat: i64, topic: Value, text: String) {
    actions::run(chat, "bot-command", move |s| async move {
        send(
            &s,
            chat,
            topic,
            Value::Null,
            json!({
            "@type":"inputMessageText","text":{
            "@type":"formattedText","text":text,"entities":[]}
            ,"clear_draft":false}
            ),
        )
        .await
    });
}
pub fn menu(chat: i64) {
    let topic = window::composer_topic(chat);
    features::load("Bot commands and keyboard", move |s, d, b| async move {
        let c = s
            .call(
                "getChat",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        let mut groups = Vec::new();
        if let Some(user) = c["type"]["user_id"].as_i64() {
            let info = s
                .call(
                    "getUserFullInfo",
                    json!({
                    "user_id":user}
                    ),
                )
                .await?;
            if !info["bot_info"].is_null() {
                groups.push(json!({
                "bot_user_id":user,"commands":info["bot_info"]["commands"]}
                ));
                let scope = s.clone();
                button(&b, "Start bot", move || {
                    if scope.valid() {
                        actions::run(chat, "bot-start", move |s| async move {
                            crate::groups::commit(
                                &s,
                                "sendBotStartMessage",
                                json!({
                                "bot_user_id":user,"chat_id":chat,"parameter":""}
                                ),
                            )
                            .await?;
                            Ok(())
                        });
                    }
                });
            }
        } else {
            let (method, key, id) = if c["type"]["@type"] == "chatTypeBasicGroup" {
                ("getBasicGroupFullInfo", "basic_group_id", c["type"]["basic_group_id"].clone())
            } else {
                ("getSupergroupFullInfo", "supergroup_id", c["type"]["supergroup_id"].clone())
            };
            let mut args = json!({});
            args[key] = id;
            let info = s.call(method, args).await?;
            groups = info["bot_commands"].as_array().cloned().unwrap_or_default();
        }
        if !alive(&s, &d) {
            return Ok(());
        }
        let mut count = 0;
        for group in groups {
            let id = group["bot_user_id"].as_i64().unwrap_or(0);
            let user = s
                .call(
                    "getUser",
                    json!({
                    "user_id":id}
                    ),
                )
                .await
                .unwrap_or(Value::Null);
            let username = user["usernames"]["active_usernames"][0].as_str().unwrap_or("");
            for command in group["commands"].as_array().into_iter().flatten() {
                let name = command["command"].as_str().unwrap_or("");
                let text = if username.is_empty() { format!("/{name}") } else { format!("/{name}@{username}") };
                let scope = s.clone();
                let topic = topic.clone();
                button(&b, &format!("{text} — {}", command["description"].as_str().unwrap_or("")), move || {
                    if scope.valid() {
                        send_text(chat, topic.clone(), text.clone());
                    }
                });
                count += 1;
            }
        }
        if count == 0 {
            note(&b, "No bot commands were provided for this conversation.");
        }
        if let Some(id) = c["reply_markup_message_id"].as_i64().filter(|i| *i != 0) {
            let data = s
                .call(
                    "getMessage",
                    json!({
                    "chat_id":chat,"message_id":id}
                    ),
                )
                .await?;
            if alive(&s, &d) {
                if let Some(m) = parse_message(&data) {
                    render(&b, &m);
                }
            }
        }
        let scope = s.clone();
        button(&b, "Search an inline bot…", move || {
            if scope.valid() {
                inline(chat);
            }
        });
        Ok(())
    });
}
pub fn render(body: &gtk::Box, m: &MessageSummary) {
    let markup = &m.data["reply_markup"];
    if !matches!(markup["@type"].as_str(), Some("replyMarkupInlineKeyboard" | "replyMarkupShowKeyboard")) {
        return;
    }
    let scope = Scope::current();
    let keyboard = markup["@type"] == "replyMarkupShowKeyboard";
    if keyboard
        && window::chat(m.chat_id).is_some_and(|c| c.data["reply_markup_message_id"].as_i64().is_some_and(|id| id != m.id))
    {
        return;
    }
    let container = nyx_ui::widgets::vbox(3);
    body.append(&container);
    let body = &container;
    if keyboard {
        KEYBOARDS.with(|v| {
            let mut v = v.borrow_mut();
            v.retain(|_, rows| {
                rows.retain(|r| r.upgrade().is_some());
                !rows.is_empty()
            });
            v.entry((m.chat_id, m.id)).or_default().push(container.downgrade());
        });
    }
    for (r, row) in markup["rows"].as_array().into_iter().flatten().enumerate() {
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_min_children_per_line(1);
        flow.set_max_children_per_line(3);
        flow.set_column_spacing(3);
        flow.set_row_spacing(3);
        for (c, key) in row.as_array().into_iter().flatten().enumerate() {
            let title = key["text"].as_str().unwrap_or("Bot action");
            let b = gtk::Button::with_label(title);
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_wrap(true);
                l.set_max_width_chars(22);
            }
            b.set_sensitive(scope.is_some() && key["type"]["@type"] != "inlineKeyboardButtonTypeDisabled");
            let (s, m, key) = (scope.clone(), m.clone(), key.clone());
            b.connect_clicked(move |_| {
                if !s.as_ref().is_some_and(Scope::valid) {
                    return;
                }
                pressed(m.clone(), key.clone(), r, c, keyboard);
            });
            flow.insert(&b, -1);
        }
        body.append(&flow);
    }
}
fn pressed(m: MessageSummary, key: Value, row: usize, col: usize, keyboard: bool) {
    let (chat, id) = (m.chat_id, m.id);
    actions::run(chat, "bot-button", move |s| async move {
        let fresh = s
            .call(
                "getMessage",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        if fresh["reply_markup"]["rows"][row][col] != key {
            return Err("The bot keyboard changed. Use its latest buttons.".into());
        }
        if keyboard {
            let c = s
                .call(
                    "getChat",
                    json!({
                    "chat_id":chat}
                    ),
                )
                .await?;
            if c["reply_markup_message_id"].as_i64() != Some(id) {
                return Err("This keyboard is no longer active.".into());
            }
        }
        let topic = fresh["topic_id"].clone();
        let typ = &key["type"];
        match typ["@type"].as_str().unwrap_or(""){
 "inlineKeyboardButtonTypeUrl"=>launch(typ["url"].as_str().unwrap_or("")),
 "inlineKeyboardButtonTypeCopyText"=>{
if let Some(d)=gtk::gdk::Display::default(){
d.clipboard().set_text(typ["text"].as_str().unwrap_or(""));
}
}
,
 "inlineKeyboardButtonTypeUser"=>{
let chat=s.call("createPrivateChat",json!({
"user_id":typ["user_id"],"force":false}
)).await?;
if let Some(id)=chat["id"].as_i64(){
window::open_chat(id);
}
}
,
 "inlineKeyboardButtonTypeCallback"=>callback(&s,chat,id,json!({
"@type":"callbackQueryPayloadData","data":typ["data"]}
)).await?,
 "inlineKeyboardButtonTypeCallbackWithPassword"=>{
password_callback(s.clone(),chat,id,typ["data"].clone());
}
,
 "inlineKeyboardButtonTypeCallbackGame"=>callback(&s,chat,id,json!({
"@type":"callbackQueryPayloadGame","game_short_name":fresh["content"]["game"]["short_name"].as_str().unwrap_or("")}
)).await?,
 "inlineKeyboardButtonTypeSwitchInline"=>switch_inline(&s,chat,&fresh,typ).await?,
 "inlineKeyboardButtonTypeLoginUrl"=>{
login(s.clone(),chat,id,typ["id"].clone());
}
,
 "inlineKeyboardButtonTypeWebApp"|"keyboardButtonTypeWebApp"=>{
web_app(s.clone(),chat,fresh["sender_id"]["user_id"].as_i64().unwrap_or(0),typ["url"].as_str().unwrap_or("").to_string(),key["text"].as_str().unwrap_or("").to_string());
}
,
 "inlineKeyboardButtonTypeBuy"=>crate::commerce::invoice(chat,id),
 "keyboardButtonTypeText"=>{
send(&s,chat,topic,Value::Null,json!({
"@type":"inputMessageText","text":{
"@type":"formattedText","text":key["text"],"entities":[]}
,"clear_draft":false}
)).await?;
consume_keyboard(chat,id);
}
,
 "keyboardButtonTypeRequestPhoneNumber"=>{
let user=fresh["sender_id"]["user_id"].as_i64().unwrap_or(0);
actions::operation("Share your phone number?","This bot will receive the phone number on your Telegram account.",vec![],"Share number",move|s,_|async move{
s.call("sharePhoneNumber",json!({
"user_id":user}
)).await?;
consume_keyboard(chat,id);
Ok(())}
);
}
,
 "keyboardButtonTypeRequestLocation"=>crate::rich_messages::compose_location(chat),
 "keyboardButtonTypeRequestPoll"=>crate::rich_messages::compose_poll(chat),
 "keyboardButtonTypeRequestManagedBot"=>managed_bot(&s,&fresh,typ),
 "keyboardButtonTypeRequestUsers"|"keyboardButtonTypeRequestChat"=>share_request(s.clone(),chat,id,key.clone()),
 _=>return Err("This bot button needs a Telegram capability that Pulse does not yet provide.".into())
 }
        Ok(())
    });
}
async fn callback(s: &Scope, chat: i64, id: i64, payload: Value) -> Result<(), String> {
    let result = s
        .call(
            "getCallbackQueryAnswer",
            json!({
            "chat_id":chat,"message_id":id,"payload":payload}
            ),
        )
        .await?;
    let text = result["text"].as_str().unwrap_or("");
    if result["show_alert"] == true {
        let (d, b) = nyx_ui::widgets::dialog("Bot response", 420);
        note(&b, text);
        d.present();
    } else if !text.is_empty() {
        window::show_status(text)
    }
    if let Some(url) = result["url"].as_str().filter(|u| !u.is_empty()) {
        let url = url.to_owned();
        let (d, b) = nyx_ui::widgets::dialog("Bot link", 420);
        note(&b, &url);
        button(&b, "Open link", move || launch(&url));
        d.present();
    }
    Ok(())
}
fn password_callback(scope: Scope, chat: i64, id: i64, data: Value) {
    let (d, b) = nyx_ui::widgets::dialog("Confirm bot action", 420);
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    note(&fields, "This bot action requires your Telegram two-step verification password.");
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    fields.append(&password);
    submit(&d, &b, &fields, scope, "Confirm", move |s| {
        let (data, password) = (data.clone(), password.text().to_string());
        async move {
            callback(
                &s,
                chat,
                id,
                json!({
                "@type":"callbackQueryPayloadDataWithPassword","data":data,"password":password}
                ),
            )
            .await
        }
    });
    d.present();
}
fn login(scope: Scope, chat: i64, id: i64, button_id: Value) {
    features::load("Telegram website login", move |s, d, b| async move {
        if !scope.valid() {
            return Ok(());
        }
        let info = s
            .call(
                "getLoginUrlInfo",
                json!({
                "chat_id":chat,"message_id":id,"button_id":button_id}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, &features::summary(&info));
        let write = features::check(&b, "Allow the bot to send me messages", false);
        let scope = s.clone();
        button(&b, "Continue to website", move || {
            if !scope.valid() {
                return;
            }
            let button_id = button_id.clone();
            let write = write.is_active();
            actions::run(chat, "bot-login", move |s| async move {
                let result = s
                    .call(
                        "getLoginUrl",
                        json!({
                        "chat_id":chat,"message_id":id,"button_id":button_id,"allow_write_access":write}
                        ),
                    )
                    .await?;
                launch(result["url"].as_str().unwrap_or(""));
                Ok(())
            });
        });
        Ok(())
    });
}
fn web_app(scope: Scope, chat: i64, bot: i64, url: String, text: String) {
    crate::webapps::open(scope, chat, bot, url, text);
}
fn share_request(scope: Scope, chat: i64, id: i64, key: Value) {
    let (d, b) = nyx_ui::widgets::dialog("Share with bot", 460);
    let fields = crate::groups::scroll(&b);
    let users = key["type"]["@type"] == "keyboardButtonTypeRequestUsers";
    let selected: Rc<RefCell<Vec<i64>>> = Default::default();
    let list = nyx_ui::widgets::vbox(4);
    fields.append(&list);
    let maximum = if users { key["type"]["max_quantity"].as_u64().unwrap_or(1).clamp(1, 100) as usize } else { 1 };
    let disclosures = if users {
        [("request_name", "name"), ("request_username", "username"), ("request_photo", "profile photo")].as_slice()
    } else {
        [("request_title", "title"), ("request_username", "username"), ("request_photo", "photo")].as_slice()
    };
    let details = disclosures
        .iter()
        .filter(|(field, _)| key["type"][*field] == true)
        .map(|(_, label)| *label)
        .collect::<Vec<_>>()
        .join(", ");
    note(
        &fields,
        &format!(
            "This bot receives the selected {} identities{}. Share up to {maximum}.",
            if users { "people" } else { "chat" },
            if details.is_empty() { String::new() } else { format!(" and their {details}") }
        ),
    );
    if !users {
        if let Some(rights) = key["type"]["bot_administrator_rights"].as_object() {
            let rights = rights
                .iter()
                .filter(|(_, v)| **v == true)
                .map(|(k, _)| features::pretty(k.strip_prefix("can_").unwrap_or(k)))
                .collect::<Vec<_>>();
            if !rights.is_empty() {
                note(
                    &fields,
                    &format!(
                        "The bot requests administrator access: {}. Sharing requires granting the requested access.",
                        rights.join(", ")
                    ),
                );
            }
        }
    }
    let (s, state, rows, key_) = (scope.clone(), selected.clone(), list.clone(), key.clone());
    button(&fields, if users { "Choose person…" } else { "Choose chat…" }, move || {
        if !s.valid() {
            return;
        }
        if users {
            let (state, rows, s, key) = (state.clone(), rows.clone(), s.clone(), key_.clone());
            crate::groups::choose_user("Share person", move |user| {
                if !s.valid() {
                    return;
                }
                let t = &key["type"];
                if t["restrict_user_is_bot"] == true && (user["type"]["@type"] == "userTypeBot") != (t["user_is_bot"] == true) {
                    s.error("This person does not match the bot's request.");
                    return;
                }
                if t["restrict_user_is_premium"] == true && (user["is_premium"] == true) != (t["user_is_premium"] == true) {
                    s.error("This person does not match the requested Premium status.");
                    return;
                }
                if let Some(id) = user["id"].as_i64() {
                    sharing_row(
                        &rows,
                        &state,
                        id,
                        format!("{} {}", user["first_name"].as_str().unwrap_or(""), user["last_name"].as_str().unwrap_or("")),
                        maximum,
                    );
                }
            });
        } else {
            let (pick, body) = nyx_ui::widgets::dialog("Share chat", 440);
            let chats = crate::groups::scroll(&body);
            for c in window::list_chats(10000) {
                let channel = c.chat_type == nyx_telegram_core::models::ChatType::Channel;
                if channel != (key_["type"]["chat_is_channel"] == true)
                    || c.chat_type == nyx_telegram_core::models::ChatType::Private
                {
                    continue;
                }
                let (s, rows, state, weak, name) = (s.clone(), rows.clone(), state.clone(), pick.downgrade(), c.title.clone());
                let cid = c.id;
                button(&chats, &c.title, move || {
                    if s.valid() {
                        sharing_row(&rows, &state, cid, name.clone(), 1);
                        if let Some(d) = weak.upgrade() {
                            d.close();
                        }
                    }
                });
            }
            pick.present();
        }
    });
    submit(&d, &b, &fields, scope, "Share selected identities", move |s| {
        let (ids, key) = (selected.borrow().clone(), key.clone());
        async move {
            if ids.is_empty() || ids.len() > maximum {
                return Err("Choose identities within the requested limit.".into());
            }
            let message = s.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
            let still_present = message["reply_markup"]["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|row| row.as_array().into_iter().flatten())
                .any(|button| button == &key);
            if !still_present {
                return Err("The bot's sharing request changed. Open its latest keyboard.".into());
            }
            let method = if users { "shareUsersWithBot" } else { "shareChatWithBot" };
            let mut args = json!({"source":{"@type":"keyboardButtonSourceMessage","chat_id":chat,"message_id":id},"button_id":key["type"]["id"],"only_check":true});
            if users {
                args["shared_user_ids"] = json!(ids)
            } else {
                args["shared_chat_id"] = json!(ids[0]);
            }
            s.call(method, args.clone()).await?;
            args["only_check"] = json!(false);
            crate::groups::commit(&s, method, args).await?;
            consume_keyboard(chat, id);
            Ok(())
        }
    });
    d.present();
}
fn sharing_row(rows: &gtk::Box, state: &Rc<RefCell<Vec<i64>>>, id: i64, name: String, max: usize) {
    if state.borrow().contains(&id) {
        return;
    }
    if state.borrow().len() >= max {
        window::show_status("Remove a selection before adding another.");
        return;
    }
    state.borrow_mut().push(id);
    let row = nyx_ui::widgets::hbox(6);
    note(&row, &name);
    let remove = gtk::Button::with_label("Remove");
    row.append(&remove);
    let (state, weak, list) = (state.clone(), row.downgrade(), rows.downgrade());
    remove.connect_clicked(move |_| {
        state.borrow_mut().retain(|n| *n != id);
        if let (Some(rows), Some(row)) = (list.upgrade(), weak.upgrade()) {
            rows.remove(&row);
        }
    });
    rows.append(&row);
}
pub fn inline(chat: i64) {
    inline_prefilled(chat, "", "");
}
pub fn inline_prefilled(chat: i64, initial_bot: &str, initial_query: &str) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Inline bot search", 520);
    let fields = crate::groups::scroll(&b);
    let username = entry(&fields, "Bot username", initial_bot, 64);
    let query = entry(&fields, "Query", initial_query, 512);
    let search = gtk::Button::with_label("Search");
    let next = gtk::Button::with_label("More results");
    fields.append(&search);
    fields.append(&next);
    let list = nyx_ui::widgets::vbox(6);
    fields.append(&list);
    let offset: Rc<RefCell<String>> = Default::default();
    let baseline: Rc<RefCell<Option<(String, String)>>> = Default::default();
    let busy = Rc::new(Cell::new(false));
    let weak = d.downgrade();
    let topic = window::composer_topic(chat);
    let reply = window::composer_reply(chat);
    let refresh = Rc::new(move |reset: bool| {
        if !s.valid() || busy.replace(true) {
            return;
        }
        let (name, q) = (username.text().trim().trim_start_matches('@').to_string(), query.text().to_string());
        if !reset && baseline.borrow().as_ref() != Some(&(name.clone(), q.clone())) {
            busy.set(false);
            window::show_status("Search again after changing the bot or query.");
            return;
        }
        if reset {
            offset.borrow_mut().clear();
            *baseline.borrow_mut() = Some((name.clone(), q.clone()));
            while let Some(c) = list.first_child() {
                list.remove(&c)
            }
        }
        let (s, weak, list, offset, busy, topic, reply) =
            (s.clone(), weak.clone(), list.clone(), offset.clone(), busy.clone(), topic.clone(), reply.clone());
        glib::spawn_future_local(async move {
            let result = async {
                let c = s
                    .call(
                        "searchPublicChat",
                        json!({
                        "username":name}
                        ),
                    )
                    .await?;
                let user = c["type"]["user_id"].as_i64().ok_or("Choose a bot username.")?;
                let u = s
                    .call(
                        "getUser",
                        json!({
                        "user_id":user}
                        ),
                    )
                    .await?;
                if u["type"]["@type"] != "userTypeBot" || u["type"]["is_inline"] != true {
                    return Err("This bot does not support inline queries.".into());
                }
                s.call(
                    "getInlineQueryResults",
                    json!({
                    "bot_user_id":user,"chat_id":chat,"user_location":null,"query":q,"offset":offset.borrow().clone()}
                    ),
                )
                .await
            }
            .await;
            busy.set(false);
            if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                return;
            }
            match result {
                Err(e) => {
                    note(&list, &e);
                }
                Ok(data) => {
                    *offset.borrow_mut() = data["next_offset"].as_str().unwrap_or("").to_string();
                    for result in data["results"].as_array().into_iter().flatten() {
                        let title = result["title"]
                            .as_str()
                            .or(result["description"].as_str())
                            .unwrap_or_else(|| result["@type"].as_str().unwrap_or("Result"));
                        for (key, kind) in [
                            ("photo", "Photo"),
                            ("video", "Video"),
                            ("animation", "Animation"),
                            ("audio", "Audio"),
                            ("document", "Document"),
                            ("sticker", "Sticker"),
                        ] {
                            if result[key].is_object() {
                                let raw = json!({"id":0,"chat_id":chat,"date":0,"can_be_saved":false,"has_protected_content":true,"content":{"@type":"messageText","text":{"text":title,"entities":[]}}});
                                if let Some(message) = parse_message(&raw) {
                                    let mut media = json!({"@type":format!("pollMedia{kind}")});
                                    media[key] = result[key].clone();
                                    crate::poll_media::render(&list, &message, &media);
                                }
                            }
                        }
                        let b = gtk::Button::with_label(title);
                        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                            l.set_wrap(true);
                            l.set_max_width_chars(44);
                        }
                        let (scope, query_id, result_id, topic, reply, weak) = (
                            s.clone(),
                            data["inline_query_id"].clone(),
                            result["id"].clone(),
                            topic.clone(),
                            reply.clone(),
                            weak.clone(),
                        );
                        b.connect_clicked(move |b| {
                            if !scope.valid() {
                                return;
                            }
                            b.set_sensitive(false);
                            let (s, b, query_id, result_id, topic, reply, weak) = (
                                scope.clone(),
                                b.clone(),
                                query_id.clone(),
                                result_id.clone(),
                                topic.clone(),
                                reply.clone(),
                                weak.clone(),
                            );
                            glib::spawn_future_local(async move {
                                let result=crate::groups::commit(&s,"sendInlineQueryResultMessage",json!({
"chat_id":chat,"topic_id":topic,"reply_to":reply,"query_id":query_id,"result_id":result_id,"hide_via_bot":false,"options":{
"@type":"messageSendOptions","paid_message_star_count":0}
}
)).await;
                                if !s.valid() {
                                    return;
                                }
                                match result {
                                    Ok(data) => {
                                        if let Some(m) = parse_message(&data) {
                                            window::display_message(&m);
                                        }
                                        if let Some(d) = weak.upgrade() {
                                            d.close();
                                        }
                                    }
                                    Err(e) => {
                                        b.set_sensitive(!e.contains("status is unknown"));
                                        window::show_status(&e);
                                    }
                                }
                            });
                        });
                        list.append(&b);
                    }
                }
            }
        });
    });
    let r = refresh.clone();
    search.connect_clicked(move |_| r(true));
    next.connect_clicked(move |_| refresh(false));
    d.present();
}

async fn switch_inline(s: &Scope, chat: i64, message: &Value, typ: &Value) -> Result<(), String> {
    let id = message["via_bot_user_id"]
        .as_i64()
        .filter(|id| *id != 0)
        .or(message["sender_id"]["user_id"].as_i64())
        .ok_or("No inline bot identity.")?;
    let user = s
        .call(
            "getUser",
            json!({
            "user_id":id}
            ),
        )
        .await?;
    let name = user["usernames"]["active_usernames"][0].as_str().ok_or("The bot has no active username.")?.to_string();
    let query = typ["query"].as_str().unwrap_or("").to_string();
    match typ["target_chat"]["@type"].as_str() {
        Some("targetChatCurrent") => inline_prefilled(chat, &name, &query),
        Some("targetChatInternalLink") => {
            let link = s
                .call(
                    "getInternalLink",
                    json!({
                    "type":typ["target_chat"]["link"],"is_http":false}
                    ),
                )
                .await?;
            window::resolve_link(link["link"].as_str().unwrap_or(""));
        }
        _ => {
            let (d, b) = nyx_ui::widgets::dialog("Choose inline destination", 440);
            let rows = crate::groups::scroll(&b);
            let types = &typ["target_chat"]["types"];
            for c in window::list_chats(10000) {
                use nyx_telegram_core::models::ChatType;
                let allowed = match c.chat_type {
                    ChatType::Private => types["allow_user_chats"] == true || types["allow_bot_chats"] == true,
                    ChatType::BasicGroup | ChatType::Supergroup => types["allow_group_chats"] == true,
                    ChatType::Channel => types["allow_channel_chats"] == true,
                };
                if !allowed {
                    continue;
                }
                let (s, name, query, weak, types) = (s.clone(), name.clone(), query.clone(), d.downgrade(), types.clone());
                button(&rows, &c.title.clone(), move || {
                    let (s, name, query, weak, types, c) =
                        (s.clone(), name.clone(), query.clone(), weak.clone(), types.clone(), c.clone());
                    glib::spawn_future_local(async move {
                        if !s.valid() {
                            return;
                        }
                        if let Some(d) = weak.upgrade() {
                            d.close();
                        }
                        if c.chat_type == ChatType::Private {
                            let Some(id) = c.data["type"]["user_id"].as_i64() else { return };
                            match s.call("getUser", json!({"user_id":id})).await {
                                Ok(user) => {
                                    let bot = user["type"]["@type"] == "userTypeBot";
                                    if types[if bot { "allow_bot_chats" } else { "allow_user_chats" }] != true {
                                        s.error("The bot does not allow this destination type.");
                                        return;
                                    }
                                }
                                Err(e) => {
                                    s.error(&e);
                                    return;
                                }
                            }
                        }
                        window::open_chat(c.id);
                        inline_prefilled(c.id, &name, &query);
                    });
                });
            }
            d.present();
        }
    }
    Ok(())
}
fn managed_bot(s: &Scope, message: &Value, typ: &Value) {
    let manager = message["sender_id"]["user_id"].as_i64().unwrap_or(0);
    let name = typ["suggested_name"].as_str().unwrap_or("").to_string();
    let username = typ["suggested_username"].as_str().unwrap_or("").to_string();
    let scope = s.clone();
    features::load("Create managed bot", move |_s, d, b| async move {
        note(&b, "This creates a bot managed by the requesting bot. Review its identity before creating it.");
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let name = entry(&fields, "Bot name", &name, 64);
        let username = entry(&fields, "Bot username", &username, 64);
        submit(&d, &b, &fields, scope, "Create managed bot", move |s| {
            let (name, username) = (name.text().to_string(), username.text().trim().trim_start_matches('@').to_string());
            async move {
                if name.trim().is_empty() || username.is_empty() {
                    return Err("Enter a name and username.".into());
                }
                crate::groups::commit(
                    &s,
                    "createBot",
                    json!({
                    "manager_bot_user_id":manager,"name":name,"username":username,"via_link":false}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}

thread_local! {
static KEYBOARDS:RefCell<std::collections::BTreeMap<(i64,i64),Vec<glib::WeakRef<gtk::Box>>>>=const{
RefCell::new(std::collections::BTreeMap::new())}
;
}
pub fn consume_keyboard(chat: i64, id: i64) {
    actions::run(chat, "one-time-keyboard", move |s| async move {
        let m = s
            .call(
                "getMessage",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        if m["reply_markup"]["@type"] != "replyMarkupShowKeyboard" || m["reply_markup"]["one_time"] != true {
            return Ok(());
        }
        let c = s
            .call(
                "getChat",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        if c["reply_markup_message_id"].as_i64() != Some(id) {
            return Ok(());
        }
        s.call(
            "deleteChatReplyMarkup",
            json!({
            "chat_id":chat,"message_id":id}
            ),
        )
        .await?;
        KEYBOARDS.with(|v| {
            if let Some(rows) = v.borrow_mut().remove(&(chat, id)) {
                for row in rows {
                    if let Some(row) = row.upgrade() {
                        row.set_visible(false);
                    }
                }
            }
        });
        Ok(())
    });
}
