//! Rich message cards, checklists, translation and suggested-post workflows.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::{glib, prelude::*};
use nixe_telegram_core::models::MessageSummary;
use serde_json::json;
pub fn render(body: &gtk::Box, m: &MessageSummary) -> bool {
    let c = &m.data["content"];
    match c["@type"].as_str().unwrap_or("") {
        "messageChecklist" => {
            note(body, c["list"]["title"]["text"].as_str().unwrap_or("Checklist"));
            for task in c["list"]["tasks"].as_array().into_iter().flatten() {
                let done = task["completion_date"].as_i64().unwrap_or(0) > 0;
                let b = gtk::CheckButton::with_label(task["text"]["text"].as_str().unwrap_or("Task"));
                b.set_active(done);
                b.set_sensitive(c["list"]["can_mark_tasks_as_done"] == true);
                let (chat, id, task) = (m.chat_id, m.id, task["id"].clone());
                let scope = Scope::current();
                b.connect_toggled(move |b| {
                    if !scope.as_ref().is_some_and(Scope::valid) {
                        return;
                    }
                    let task = task.clone();
                    let target = b.is_active();
                    actions::run(chat, "checklist-toggle", move |s| async move {
                        if s.call(
                            "getMessageProperties",
                            json!({
                            "chat_id":chat,"message_id":id}
                            ),
                        )
                        .await?["can_mark_tasks_as_done"]
                            != true
                        {
                            return Err("Telegram does not permit marking these tasks.".into());
                        }
                        s.call(
                            "markChecklistTasksAsDone",
                            json!({
                            "chat_id":chat,"message_id":id,"marked_as_done_task_ids":if target{
                            vec![task.clone()]}
                            else{
                            vec![]}
                            ,"marked_as_not_done_task_ids":if target{
                            vec![]}
                            else{
                            vec![task]}
                            }
                            ),
                        )
                        .await?;
                        Ok(())
                    });
                });
                body.append(&b);
            }
            if c["list"]["can_add_tasks"] == true {
                let (chat, id) = (m.chat_id, m.id);
                button(body, "Add tasks…", move || add_tasks(chat, id));
            }
            true
        }
        "messageStory" => {
            let (chat, id) = (
                c["story_poster_chat_id"].as_i64().unwrap_or(0),
                c["story_id"].as_i64().unwrap_or(0),
            );
            button(body, "View story", move || crate::stories::view(chat, id));
            true
        }
        "messageInvoice" => {
            note(body, c["product_info"]["title"].as_str().unwrap_or("Invoice"));
            note(body, c["product_info"]["description"].as_str().unwrap_or(""));
            let (chat, id) = (m.chat_id, m.id);
            button(body, "Review invoice…", move || crate::commerce::invoice(chat, id));
            true
        }
        "messagePaidMedia" => {
            note(body, c["caption"]["text"].as_str().unwrap_or("Paid media"));
            for media in c["media"].as_array().into_iter().flatten() {
                match media["@type"].as_str() {
                    Some("paidMediaPhoto") => crate::poll_media::render(
                        body,
                        m,
                        &json!({
                        "@type":"pollMediaPhoto","photo":media["photo"]}
                        ),
                    ),
                    Some("paidMediaVideo") => crate::poll_media::render(
                        body,
                        m,
                        &json!({
                        "@type":"pollMediaVideo","video":media["video"]}
                        ),
                    ),
                    _ => {
                        note(body, &format!("Locked media · {} Stars", c["star_count"]));
                    }
                }
            }
            let (chat, id) = (m.chat_id, m.id);
            button(body, "Review media purchase…", move || crate::commerce::invoice(chat, id));
            true
        }
        "messageGame" => {
            note(body, c["game"]["title"].as_str().unwrap_or("Game"));
            note(body, c["game"]["description"].as_str().unwrap_or(""));
            crate::poll_media::render(
                body,
                m,
                &json!({
                "@type":"pollMediaPhoto","photo":c["game"]["photo"]}
                ),
            );
            true
        }
        "messageAnimatedEmoji" => {
            note(body, c["emoji"].as_str().unwrap_or("Emoji"));
            if !c["animated_emoji"]["sticker"].is_null() {
                crate::poll_media::render(
                    body,
                    m,
                    &json!({
                    "@type":"pollMediaSticker","sticker":c["animated_emoji"]["sticker"]}
                    ),
                );
            }
            true
        }
        "messageDice" => {
            note(
                body,
                &format!(
                    "{} {}",
                    c["emoji"].as_str().unwrap_or("🎲"),
                    c["value"]
                        .as_i64()
                        .filter(|v| *v != 0)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "Rolling…".into())
                ),
            );
            true
        }
        "messageExpiredPhoto" | "messageExpiredVideo" | "messageExpiredVideoNote" | "messageExpiredVoiceNote" => {
            note(body, "This self-destructing media has expired.");
            true
        }
        "messageUnsupported" => {
            note(body, "This message requires a newer Telegram library.");
            true
        }
        kind if kind.starts_with("messageChat")
            || kind.starts_with("messageForumTopic")
            || kind.starts_with("messageGiveaway")
            || kind.starts_with("messageGift")
            || kind.starts_with("messagePayment")
            || kind.starts_with("messageSuggest")
            || kind.starts_with("messageVideoChat")
            || kind.starts_with("messageContactRegistered")
            || kind.starts_with("messagePinMessage")
            || kind.starts_with("messageScreenshotTaken")
            || kind.starts_with("messageCustomServiceAction")
            || kind.starts_with("messageProximityAlertTriggered") =>
        {
            note(body, &features::pretty(kind.trim_start_matches("message")));
            let details = features::summary(c);
            if !details.is_empty() {
                note(body, &details);
            }
            true
        }
        _ => false,
    }
}
fn add_tasks(chat: i64, id: i64) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nixe_ui::widgets::dialog("Add checklist tasks", 440);
    let fields = nixe_ui::widgets::vbox(8);
    b.append(&fields);
    let text = gtk::TextView::new();
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_size_request(-1, 120);
    note(&fields, "One task per line");
    fields.append(&text);
    submit(&d, &b, &fields, s, "Add tasks", move |s| {
        let tasks = text
            .buffer()
            .text(&text.buffer().start_iter(), &text.buffer().end_iter(), false)
            .to_string();
        async move {
            if s.call(
                "getMessageProperties",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?["can_add_tasks"]
                != true
            {
                return Err("Telegram does not permit adding tasks.".into());
            }
            let m = s
                .call(
                    "getMessage",
                    json!({
                    "chat_id":chat,"message_id":id}
                    ),
                )
                .await?;
            let mut next = m["content"]["list"]["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["id"].as_i64())
                .max()
                .unwrap_or(0)
                + 1;
            let tasks = tasks
                .lines()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| {
                    let id = next;
                    next += 1;
                    json!({
                    "@type":"inputChecklistTask","id":id,"text":{
                    "@type":"formattedText","text":t,"entities":[]}
                    }
                    )
                })
                .collect::<Vec<_>>();
            if tasks.is_empty() {
                return Err("Enter at least one task.".into());
            }
            crate::groups::commit(
                &s,
                "addChecklistTasks",
                json!({
                "chat_id":chat,"message_id":id,"tasks":tasks}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
pub fn compose_checklist(chat: i64) {
    let Some(s) = Scope::current() else { return };
    let (topic, reply) = (window::composer_topic(chat), window::composer_reply(chat));
    let (d, b) = nixe_ui::widgets::dialog("Send checklist", 440);
    let fields = nixe_ui::widgets::vbox(8);
    b.append(&fields);
    let title = entry(&fields, "Title", "", 255);
    let text = gtk::TextView::new();
    text.set_size_request(-1, 120);
    note(&fields, "One task per line");
    fields.append(&text);
    let add = check(&fields, "Others can add tasks", true);
    let mark = check(&fields, "Others can complete tasks", true);
    note(&fields, "Telegram Premium is required to create checklists.");
    submit(&d, &b, &fields, s, "Send checklist", move |s| {
        let (title, text, add, mark, topic, reply) = (
            title.text().to_string(),
            text.buffer()
                .text(&text.buffer().start_iter(), &text.buffer().end_iter(), false)
                .to_string(),
            add.is_active(),
            mark.is_active(),
            topic.clone(),
            reply.clone(),
        );
        async move {
            let tasks = text
                .lines()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .enumerate()
                .map(|(i, t)| {
                    json!({
                    "@type":"inputChecklistTask","id":i+1,"text":{
                    "@type":"formattedText","text":t,"entities":[]}
                    }
                    )
                })
                .collect::<Vec<_>>();
            if title.trim().is_empty() || tasks.is_empty() {
                return Err("Enter a title and at least one task.".into());
            }
            if s.call("getMe", json!({})).await?["is_premium"] != true {
                return Err("Telegram Premium is required for checklists.".into());
            }
            let m = crate::groups::commit(
                &s,
                "sendMessage",
                json!({
                "chat_id":chat,"topic_id":topic,"reply_to":reply,"options":{
                "@type":"messageSendOptions","paid_message_star_count":0}
                ,"input_message_content":{
                "@type":"inputMessageChecklist","checklist":{
                "@type":"inputChecklist","title":{
                "@type":"formattedText","text":title,"entities":[]}
                ,"tasks":tasks,"others_can_add_tasks":add,"others_can_mark_tasks_as_done":mark}
                }
                }
                ),
            )
            .await?;
            if let Some(m) = nixe_telegram_core::client::parse_message(&m) {
                window::display_message(&m);
            }
            Ok(())
        }
    });
    d.present();
}
pub fn translate(chat: i64, id: i64) {
    let Some(s) = Scope::current() else { return };
    if window::chat(chat).is_some_and(|c| c.is_secret) {
        s.error("Secret-chat messages cannot be sent to Telegram's translation service.");
        return;
    }
    let (d, b) = nixe_ui::widgets::dialog("Translate message", 480);
    let language = entry(&b, "Target language code (e.g. en, es, de)", "en", 16);
    let result = note(&b, "");
    let weak = d.downgrade();
    let go = gtk::Button::with_label("Translate");
    b.append(&go);
    go.connect_clicked(move |button| {
        if !s.valid() {
            return;
        }
        let (s, weak, result, lang, button) = (
            s.clone(),
            weak.clone(),
            result.clone(),
            language.text().trim().to_string(),
            button.clone(),
        );
        button.set_sensitive(false);
        glib::spawn_future_local(async move {
            let response = s
                .call(
                    "translateMessageText",
                    json!({
                    "chat_id":chat,"message_id":id,"to_language_code":lang,"tone":"neutral"}
                    ),
                )
                .await;
            button.set_sensitive(true);
            if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                return;
            }
            result.set_text(
                &response
                    .map(|v| v["text"].as_str().unwrap_or("").to_string())
                    .unwrap_or_else(|e| e),
            );
        });
    });
    d.present();
}
pub fn suggested(chat: i64, id: i64) {
    features::load("Suggested post", move |s, d, b| async move {
        let m = s
            .call(
                "getMessage",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        let props = s
            .call(
                "getMessageProperties",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let info = m["suggested_post_info"].clone();
        note(&b, &features::summary(&info));
        if props["can_be_approved"] == true {
            let scope = s.clone();
            button(&b, "Approve suggestion…", move || {
                if !scope.valid() {
                    return;
                }
                let info = info.clone();
                let description = format!(
                    "Approve this post under these terms?\n{}\nApproval can spend the indicated Stars or TON amount.",
                    features::summary(&info)
                );
                actions::operation("Approve suggested post?", &description, vec![], "Approve", move |s, _| {
                    let info = info.clone();
                    async move {
                        let props = s
                            .call(
                                "getMessageProperties",
                                json!({
                                "chat_id":chat,"message_id":id}
                                ),
                            )
                            .await?;
                        let m = s
                            .call(
                                "getMessage",
                                json!({
                                "chat_id":chat,"message_id":id}
                                ),
                            )
                            .await?;
                        if props["can_be_approved"] != true || m["suggested_post_info"] != info {
                            return Err("The suggestion or your permission changed. Review it again.".into());
                        }
                        crate::groups::commit(
                            &s,
                            "approveSuggestedPost",
                            json!({
                            "chat_id":chat,"message_id":id,"send_date":0}
                            ),
                        )
                        .await?;
                        Ok(())
                    }
                });
            });
        }
        if props["can_be_declined"] == true {
            let fields = nixe_ui::widgets::vbox(8);
            b.append(&fields);
            let comment = entry(&fields, "Decline reason (optional)", "", 128);
            submit(&d, &b, &fields, s, "Decline suggestion", move |s| {
                let comment = comment.text().to_string();
                async move {
                    if s.call(
                        "getMessageProperties",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?["can_be_declined"]
                        != true
                    {
                        return Err("This suggestion can no longer be declined.".into());
                    }
                    s.call(
                        "declineSuggestedPost",
                        json!({
                        "chat_id":chat,"message_id":id,"comment":comment}
                        ),
                    )
                    .await?;
                    Ok(())
                }
            });
        }
        Ok(())
    });
}

#[cfg(test)]
pub(crate) fn check_widgets() {
    crate::desktop_extras::check_shortcuts();
    let body = nixe_ui::widgets::vbox(6);
    let mut m=nixe_telegram_core::client::parse_message(&json!({"id":77,"chat_id":7,"date":1,"content":{"@type":"messageChecklist","list":{"title":{"text":"Shopping"},"tasks":[{"id":1,"text":{"text":"Milk"},"completion_date":0}],"can_mark_tasks_as_done":false}}})).unwrap();
    assert!(render(&body, &m));
    assert_eq!(
        body.first_child()
            .unwrap()
            .next_sibling()
            .unwrap()
            .downcast::<gtk::CheckButton>()
            .unwrap()
            .is_sensitive(),
        false
    );
    m.data["reply_markup"] = json!({"@type":"replyMarkupInlineKeyboard","rows":[[{"text":"Copy","type":{"@type":"inlineKeyboardButtonTypeCopyText","text":"value"}}],[{"text":"Unavailable","type":{"@type":"inlineKeyboardButtonTypeDisabled"}}]]});
    let keyboard = nixe_ui::widgets::vbox(4);
    crate::bots::render(&keyboard, &m);
    assert!(keyboard.first_child().is_some());
    assert!(keyboard.last_child().unwrap().last_child().unwrap().downcast::<gtk::FlowBox>().is_ok());
    m.data["content"] = json!({"@type":"messageExpiredVideo"});
    assert!(render(&body, &m));
}
