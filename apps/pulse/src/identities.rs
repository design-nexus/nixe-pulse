//! Telegram's chat-level sender selection, with a visible composer identity.
use crate::{
    actions::{self, Scope},
    groups::{note, scroll, submit},
    window,
};
use gtk::prelude::*;
use nyx_ui::widgets as w;
use serde_json::{Value, json};

pub async fn name(scope: &Scope, sender: &Value) -> Result<String, String> {
    if let Some(id) = sender["user_id"].as_i64() {
        let user = scope.call("getUser", json!({"user_id":id})).await?;
        let name = format!("{} {}", user["first_name"].as_str().unwrap_or(""), user["last_name"].as_str().unwrap_or(""))
            .trim()
            .to_string();
        Ok(if name.is_empty() { "Deleted account".into() } else { name })
    } else if let Some(id) = sender["chat_id"].as_i64() {
        Ok(scope.call("getChat", json!({"chat_id":id})).await?["title"].as_str().unwrap_or("Chat").into())
    } else {
        Ok("Default identity".into())
    }
}
fn available(data: &Value, sender: &Value, premium: bool) -> bool {
    data["senders"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| row["sender"] == *sender && (row["needs_premium"] != true || premium))
}
pub fn choose(chat: i64) {
    actions::run(chat, "sender-picker", move |scope| async move {
        let data = scope.call("getChatAvailableMessageSenders", json!({"chat_id":chat})).await?;
        let me = scope.call("getMe", json!({})).await?;
        let current = scope.call("getChat", json!({"chat_id":chat})).await?["message_sender_id"].clone();
        let (dialog, body) = w::dialog("Send as", 440);
        note(&body, "Choose the identity Telegram will use for new messages in this chat, including its topics.");
        let fields = scroll(&body);
        let group = gtk::CheckButton::new();
        let mut choices = Vec::new();
        for row in data["senders"].as_array().into_iter().flatten() {
            let sender = row["sender"].clone();
            let title = name(&scope, &sender).await?;
            let locked = row["needs_premium"] == true && me["is_premium"] != true;
            let c = gtk::CheckButton::with_label(&format!("{title}{}", if locked { " · Premium required" } else { "" }));
            c.set_group(Some(&group));
            c.set_sensitive(!locked);
            c.set_active(sender == current || (current.is_null() && sender["user_id"] == me["id"]));
            if let Some(label) = c.child().and_downcast::<gtk::Label>() {
                label.set_wrap(true);
                label.set_max_width_chars(36);
            }
            fields.append(&c);
            choices.push((sender, c));
        }
        if choices.is_empty() {
            note(&fields, "Telegram has no alternate identities for this chat.");
            dialog.present();
            return Ok(());
        }
        submit(&dialog, &body, &fields, scope, "Use identity", move |s| {
            let selected = choices.iter().find(|(_, c)| c.is_active() && c.is_sensitive()).map(|(sender, _)| sender.clone());
            let original = current.clone();
            async move {
                let selected = selected.ok_or("Choose an available identity.")?;
                let current = s.call("getChat", json!({"chat_id":chat})).await?;
                if current["message_sender_id"] != original {
                    return Err("The sending identity changed elsewhere. Reopen this dialog.".into());
                }
                let available_senders = s.call("getChatAvailableMessageSenders", json!({"chat_id":chat})).await?;
                let premium = s.call("getMe", json!({})).await?["is_premium"] == true;
                if !available(&available_senders, &selected, premium) {
                    return Err("This identity is no longer available to your account.".into());
                }
                s.call("setChatMessageSender", json!({"chat_id":chat,"message_sender_id":selected})).await?;
                window::with_service(|s| s.load_chat(chat));
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage3c_availability_checks_identity_and_premium() {
        let person = json!({"@type":"messageSenderUser","user_id":7});
        let channel = json!({"@type":"messageSenderChat","chat_id":-9});
        let data = json!({"senders":[{"sender":person,"needs_premium":false},{"sender":channel,"needs_premium":true}]});
        assert!(available(&data, &person, false));
        assert!(!available(&data, &channel, false));
        assert!(available(&data, &channel, true));
        assert!(!available(&data, &Value::Null, true));
    }
}
