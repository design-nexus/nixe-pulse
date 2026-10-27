//! Business bot rights and complete weekly opening intervals.
use crate::{
    actions,
    features::{self, alive, button, check},
    groups::{entry, note, submit},
};
use gtk::prelude::*;
use serde_json::{Value, json};
const RIGHTS: &[(&str, &str)] = &[
    ("can_reply", "Reply to messages"),
    ("can_read_messages", "Read selected chats"),
    ("can_delete_sent_messages", "Delete its sent messages"),
    ("can_delete_all_messages", "Delete all messages"),
    ("can_edit_name", "Edit profile name"),
    ("can_edit_bio", "Edit bio"),
    ("can_edit_profile_photo", "Edit profile photo"),
    ("can_edit_username", "Edit username"),
    ("can_view_gifts_and_stars", "View gifts and Stars"),
    ("can_sell_gifts", "Sell gifts"),
    ("can_change_gift_settings", "Change gift settings"),
    ("can_transfer_and_upgrade_gifts", "Transfer and upgrade gifts"),
    ("can_transfer_stars", "Transfer Stars"),
    ("can_manage_stories", "Manage stories"),
];
pub fn connected_bot() {
    features::load("Business bot and permissions", |s, d, b| async move {
        let current = s.call("getBusinessConnectedBot", json!({})).await.unwrap_or(Value::Null);
        let original = current["bot"].clone();
        let user = if let Some(id) = original["bot_user_id"].as_i64() {
            s.call(
                "getUser",
                json!({
                "user_id":id}
                ),
            )
            .await
            .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        note(
            &fields,
            "Connecting a Business bot grants the rights selected below for the selected chats. Enable only the access you intend to grant.",
        );
        let username = entry(&fields, "Bot username", user["usernames"]["active_usernames"][0].as_str().unwrap_or(""), 64);
        let contacts = check(&fields, "Contacts", original["recipients"]["select_contacts"] == true);
        let noncontacts = check(&fields, "Non-contacts", original["recipients"]["select_non_contacts"] == true);
        let existing = check(&fields, "Existing chats", original["recipients"]["select_existing_chats"] == true);
        let new = check(&fields, "New chats", original["recipients"]["select_new_chats"] == true);
        let excluded = check(
            &fields,
            "Exclude the selected chats instead of including them",
            original["recipients"]["exclude_selected"] == true,
        );
        let chats = entry(&fields, "Selected chat IDs", &features::id_text(&original["recipients"]["chat_ids"]), 4096);
        let except =
            entry(&fields, "Always-excluded chat IDs", &features::id_text(&original["recipients"]["excluded_chat_ids"]), 4096);
        let rights =
            RIGHTS.iter().map(|(key, title)| (*key, check(&fields, title, original["rights"][*key] == true))).collect::<Vec<_>>();
        if let Some(id) = original["bot_user_id"].as_i64() {
            let scope = s.clone();
            button(&fields, "Disconnect this bot…", move || {
                if !scope.valid() {
                    return;
                }
                actions::operation(
                    "Disconnect business bot?",
                    "The bot will lose Business access to your account.",
                    vec![],
                    "Disconnect",
                    move |s, _| async move {
                        s.call(
                            "deleteBusinessConnectedBot",
                            json!({
                            "bot_user_id":id}
                            ),
                        )
                        .await?;
                        Ok(())
                    },
                );
            });
        }
        submit(&d, &b, &fields, s, "Save bot access", move |s| {
            let (name, recipients, rights, original) = (
                username.text().trim().trim_start_matches('@').to_string(),
                (
                    features::ids(&chats.text()),
                    features::ids(&except.text()),
                    contacts.is_active(),
                    noncontacts.is_active(),
                    existing.is_active(),
                    new.is_active(),
                    excluded.is_active(),
                ),
                rights.iter().map(|(key, c)| ((*key).to_string(), c.is_active())).collect::<Vec<_>>(),
                original.clone(),
            );
            async move {
                let chat = s
                    .call(
                        "searchPublicChat",
                        json!({
                        "username":name}
                        ),
                    )
                    .await?;
                let id = chat["type"]["user_id"].as_i64().ok_or("Choose a bot username.")?;
                let user = s
                    .call(
                        "getUser",
                        json!({
                        "user_id":id}
                        ),
                    )
                    .await?;
                if user["type"]["@type"] != "userTypeBot" {
                    return Err("Choose a Telegram bot.".into());
                }
                let current = s.call("getBusinessConnectedBot", json!({})).await.unwrap_or(Value::Null);
                if current["bot"] != original {
                    return Err("Business bot configuration changed. Reopen the editor.".into());
                }
                let mut native = json!({
                "@type":"businessBotRights"}
                );
                for (key, value) in rights {
                    native[key] = json!(value);
                }
                let (chats, except, contacts, noncontacts, existing, new, excluded) = recipients;
                let recipients = json!({
                "@type":"businessRecipients","chat_ids":chats?,"excluded_chat_ids":except?,"select_existing_chats":existing,"select_new_chats":new,"select_contacts":contacts,"select_non_contacts":noncontacts,"exclude_selected":excluded}
                );
                crate::groups::commit(
                    &s,
                    "setBusinessConnectedBot",
                    json!({
                    "bot":{
                    "@type":"businessConnectedBot","bot_user_id":id,"recipients":recipients,"rights":native}
                    }
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
pub fn hours() {
    features::load("Weekly opening hours", |s, d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        let info = s
            .call(
                "getUserFullInfo",
                json!({
                "user_id":me["id"]}
                ),
            )
            .await?;
        let original = info["business_info"]["opening_hours"].clone();
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let enabled = check(&fields, "Show opening hours", !original.is_null());
        let timezone = entry(&fields, "Time zone", original["time_zone_id"].as_str().unwrap_or("UTC"), 128);
        note(
            &fields,
            "Enter intervals for each day, e.g. 09:00-12:00, 13:00-17:00. Leave blank for closed. An end earlier than the start continues into the next day.",
        );
        let mut days = Vec::new();
        for (i, title) in ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].iter().enumerate() {
            let intervals = original["opening_hours"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["start_minute"].as_i64().unwrap_or(-1) / 1440 == i as i64)
                .map(|v| {
                    let a = v["start_minute"].as_i64().unwrap_or(0) % 1440;
                    let b = v["end_minute"].as_i64().unwrap_or(0) % 1440;
                    format!("{:02}:{:02}-{:02}:{:02}", a / 60, a % 60, b / 60, b % 60)
                })
                .collect::<Vec<_>>()
                .join(", ");
            days.push(entry(&fields, title, &intervals, 1024));
        }
        submit(&d, &b, &fields, s, "Save weekly hours", move |s| {
            let (enabled, timezone, days, original) = (
                enabled.is_active(),
                timezone.text().trim().to_string(),
                days.iter().map(|e| e.text().to_string()).collect::<Vec<_>>(),
                original.clone(),
            );
            async move {
                let mut intervals = Vec::new();
                for (day, text) in days.iter().enumerate() {
                    intervals.extend(parse_intervals(day as i32, text)?);
                }
                intervals.sort_by_key(|v| v["start_minute"].as_i64().unwrap_or(0));
                for pair in intervals.windows(2) {
                    if pair[0]["end_minute"].as_i64().unwrap_or(0) > pair[1]["start_minute"].as_i64().unwrap_or(0) {
                        return Err("Opening intervals overlap.".into());
                    }
                }
                let me = s.call("getMe", json!({})).await?;
                let current = s
                    .call(
                        "getUserFullInfo",
                        json!({
                        "user_id":me["id"]}
                        ),
                    )
                    .await?;
                if current["business_info"]["opening_hours"] != original {
                    return Err("Opening hours changed. Reopen the editor.".into());
                }
                s.call(
                    "setBusinessOpeningHours",
                    json!({
                    "opening_hours":if enabled{
                    json!({
                    "@type":"businessOpeningHours","time_zone_id":timezone,"opening_hours":intervals}
                    )}
                    else{
                    Value::Null}
                    }
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
fn time(text: &str) -> Result<i32, String> {
    if text.trim() == "24:00" {
        return Ok(1440);
    }
    let (h, m) = text.trim().split_once(':').ok_or("Use HH:MM times.")?;
    let (h, m) = (h.parse::<i32>().map_err(|_| "Invalid hour.")?, m.parse::<i32>().map_err(|_| "Invalid minute.")?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err("Invalid time.".into());
    }
    Ok(h * 60 + m)
}
fn parse_intervals(day: i32, text: &str) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for span in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (a, b) = span.split_once('-').ok_or("Use start-end intervals separated by commas.")?;
        let (start, mut end) = (time(a)?, time(b)?);
        if start == 1440 {
            return Err("Opening time must be before 24:00.".into());
        }
        if end <= start {
            end += 1440;
        }
        result.push(json!({
        "@type":"businessOpeningHoursInterval","start_minute":day*1440+start,"end_minute":day*1440+end}
        ));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remainder_weekly_hours() {
        assert_eq!(parse_intervals(0, "09:00-12:00,13:00-17:00").unwrap().len(), 2);
        assert_eq!(parse_intervals(1, "22:00-02:00").unwrap()[0]["end_minute"], 3000);
        assert!(parse_intervals(0, "30:00-31:00").is_err());
    }
}
