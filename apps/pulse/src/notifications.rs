//! Complete chat notification exceptions while retaining unedited fields.
use crate::{
    features::{self, alive, check},
    groups::{entry, note, submit},
};
use gtk::prelude::*;
use serde_json::{Value, json};
pub fn chat(chat: i64) {
    features::load("Conversation notifications", move |s, d, b| async move {
        let data = s
            .call(
                "getChat",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        let sounds = s.call("getSavedNotificationSounds", json!({})).await.unwrap_or(Value::Null);
        if !alive(&s, &d) {
            return Ok(());
        }
        let original = data["notification_settings"].clone();
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let inherit_mute = check(&fields, "Use category mute setting", original["use_default_mute_for"] == true);
        let minutes =
            entry(&fields, "Mute minutes (0 = unmuted)", &(original["mute_for"].as_i64().unwrap_or(0) / 60).to_string(), 9);
        let forever = check(&fields, "Mute indefinitely", original["mute_for"].as_i64().unwrap_or(0) >= 2147483600);
        let mut controls = Vec::new();
        for (key, title, inherit) in [
            ("show_preview", "Message previews", "use_default_show_preview"),
            ("mute_stories", "Mute stories", "use_default_mute_stories"),
            ("show_story_poster", "Show story poster", "use_default_show_story_poster"),
            (
                "disable_pinned_message_notifications",
                "Disable pinned-message alerts",
                "use_default_disable_pinned_message_notifications",
            ),
            ("disable_mention_notifications", "Disable mention alerts", "use_default_disable_mention_notifications"),
        ] {
            let default = check(&fields, &format!("Use category setting: {title}"), original[inherit] == true);
            let value = check(&fields, title, original[key] == true);
            value.set_sensitive(!default.is_active());
            let v = value.clone();
            default.connect_toggled(move |c| v.set_sensitive(!c.is_active()));
            controls.push((key, inherit, default, value));
        }
        let mut sound_fields = Vec::new();
        for (key, inherit, title) in
            [("sound_id", "use_default_sound", "Message sound"), ("story_sound_id", "use_default_story_sound", "Story sound")]
        {
            let default = check(&fields, &format!("Use category {title}"), original[inherit] == true);
            let mut options = vec![("Default".to_string(), 0i64), ("Silent".to_string(), -1i64)];
            for sound in sounds["notification_sounds"].as_array().into_iter().flatten() {
                if let Some(id) = sound["id"].as_i64() {
                    options.push((sound["title"].as_str().unwrap_or("Saved sound").to_string(), id));
                }
            }
            let current = original[key].as_i64().unwrap_or(0);
            if !options.iter().any(|(_, id)| *id == current) {
                options.push(("Current custom sound".into(), current));
            }
            let names = options.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
            note(&fields, title);
            let selected = gtk::DropDown::from_strings(&names);
            selected.set_selected(options.iter().position(|(_, id)| *id == current).unwrap_or(0) as u32);
            selected.set_sensitive(!default.is_active());
            let v = selected.clone();
            default.connect_toggled(move |c| v.set_sensitive(!c.is_active()));
            fields.append(&selected);
            sound_fields.push((key, inherit, default, selected, options));
        }
        let reset = check(&fields, "Reset all exceptions to category defaults", false);
        submit(&d, &b, &fields, s, "Save notifications", move |s| {
            let forever = forever.is_active();
            let (mut settings, minutes, mute, reset) =
                (original.clone(), minutes.text().parse::<i64>(), inherit_mute.is_active(), reset.is_active());
            let changed = controls
                .iter()
                .map(|(key, inherit, default, value)| (*key, *inherit, default.is_active(), value.is_active()))
                .collect::<Vec<_>>();
            let sounds = sound_fields
                .iter()
                .map(|(key, inherit, default, selected, options)| {
                    (
                        *key,
                        *inherit,
                        default.is_active(),
                        options.get(selected.selected() as usize).map(|(_, id)| *id).unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            async move {
                let minutes = minutes.map_err(|_| "Enter a nonnegative mute duration.")?;
                if minutes < 0 {
                    return Err("Mute duration cannot be negative.".into());
                }
                settings["use_default_mute_for"] = json!(mute || reset);
                settings["mute_for"] =
                    json!(if forever { i32::MAX as i64 } else { minutes.saturating_mul(60).min(i32::MAX as i64) });
                for (key, inherit, default, value) in changed {
                    settings[key] = json!(value);
                    settings[inherit] = json!(default || reset);
                }
                for (key, inherit, default, value) in sounds {
                    settings[key] = json!(value);
                    settings[inherit] = json!(default || reset);
                }
                s.call(
                    "setChatNotificationSettings",
                    json!({
                    "chat_id":chat,"notification_settings":settings}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
