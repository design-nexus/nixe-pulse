//! User preferences and settings for Nixe Pulse.

pub use nixe_theme::ThemeMode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub text_scale: f64,
    pub high_contrast: bool,
    pub shortcuts: std::collections::BTreeMap<String,String>,
    pub mode: ThemeMode,
    pub theme: String,
    pub glow: bool,
    pub reduce_motion: bool,
    pub sidebar_collapsed: bool,
    pub right_panel_open: bool,
    pub last_chat_id: Option<i64>,
    pub notifications_enabled: bool,
    pub sound_enabled: bool,
    pub active_account_id: String,
    pub api_id: Option<i32>,
    pub api_hash: Option<String>,
    pub microphone: String,
    pub speaker: String,
    pub camera: String,
    pub drafts: std::collections::BTreeMap<String, Draft>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            text_scale: 1.0,
            high_contrast: false,
            shortcuts: Default::default(),
            mode: ThemeMode::Omarchy,
            theme: "tokyo-night".into(),
            glow: true,
            reduce_motion: false,
            sidebar_collapsed: false,
            right_panel_open: false,
            last_chat_id: None,
            notifications_enabled: true,
            sound_enabled: true,
            active_account_id: "default".into(),
            api_id: None,
            api_hash: None,
            microphone: String::new(),
            speaker: String::new(),
            camera: String::new(),
            drafts: Default::default(),
        }
    }
}

nixe_core::prefs_store!(Prefs);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub text: String,
    pub reply: Option<i64>,
    pub formatted: serde_json::Value,
    pub remote: serde_json::Value,
    pub quote: serde_json::Value,
    pub updated_at: i64,
    pub pending: bool,
}
impl Draft {
    pub fn formatted_text(&self) -> serde_json::Value {
        if self.formatted.is_object() {
            self.formatted.clone()
        } else {
            serde_json::json!({"@type":"formattedText","text":self.text,"entities":[]})
        }
    }
    pub fn empty(&self) -> bool {
        self.text.is_empty() && self.reply.is_none()
    }
    pub fn to_td(&self) -> serde_json::Value {
        use serde_json::{Value, json};
        if self.empty() {
            return Value::Null;
        }
        let mut value = if self.remote.is_object() { self.remote.clone() } else { json!({"@type":"draftMessage"}) };
        let reply = self
            .reply
            .map(|id| {
                let mut reply = if self.remote["reply_to"]["message_id"].as_i64() == Some(id) {
                    self.remote["reply_to"].clone()
                } else {
                    json!({"@type":"inputMessageReplyToMessage"})
                };
                reply["message_id"] = json!(id);
                reply["quote"] = self.quote.clone();
                reply
            })
            .unwrap_or(Value::Null);
        value["@type"] = json!("draftMessage");
        value["date"] = json!(self.updated_at);
        value["reply_to"] = reply;
        if !value["content"].is_object() {
            value["content"] = json!({"@type":"draftMessageContentText"});
        }
        value["content"]["@type"] = json!("draftMessageContentText");
        value["content"]["text"] = self.formatted_text();
        value
    }
    pub fn from_td(value: &serde_json::Value) -> Self {
        if unsupported_draft(value) {
            return Self::default();
        }
        let formatted = value["content"]["text"].clone();
        Self {
            text: formatted["text"].as_str().unwrap_or("").into(),
            reply: value["reply_to"]["message_id"].as_i64(),
            formatted,
            remote: value.clone(),
            quote: value["reply_to"]["quote"].clone(),
            updated_at: value["date"].as_i64().unwrap_or(0),
            pending: false,
        }
    }
    pub fn same_content(&self, other: &Self) -> bool {
        let (mut a, mut b) = (self.to_td(), other.to_td());
        if a.is_object() {
            a["date"] = serde_json::json!(0);
        }
        if b.is_object() {
            b["date"] = serde_json::json!(0);
        }
        a == b
    }
}

#[cfg(test)]
mod draft_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn legacy_and_formatted_drafts_preserve_reply_quotes() {
        let legacy: Draft = serde_json::from_value(json!({"text":"old draft","reply":17})).unwrap();
        assert_eq!(legacy.to_td()["content"]["@type"], "draftMessageContentText");
        assert_eq!(legacy.to_td()["content"]["text"]["text"], "old draft");
        let mut draft = legacy;
        draft.quote =
            json!({"@type":"inputTextQuote","text":{"@type":"formattedText","text":"quote","entities":[]},"position":3});
        draft.formatted = json!({"@type":"formattedText","text":"old draft","entities":[{"@type":"textEntity","offset":0,"length":3,"type":{"@type":"textEntityTypeBold"}}]});
        draft.updated_at = 100;
        draft.remote = json!({"@type":"draftMessage","effect_id":"42","content":{"@type":"draftMessageContentText","link_preview_options":{"@type":"linkPreviewOptions","is_disabled":true}}});
        let restored = Draft::from_td(&draft.to_td());
        assert_eq!(restored.to_td()["effect_id"], "42");
        assert_eq!(restored.to_td()["content"]["link_preview_options"]["is_disabled"], true);
        assert!(draft.same_content(&restored));
        assert_eq!(restored.reply, Some(17));
        assert!(Draft::default().to_td().is_null());
        let unsupported = json!({"@type":"draftMessage","reply_to":{"message_id":9},"content":{"@type":"draftMessageContentVoiceNote","file_path":"recording.ogg"}});
        assert!(unsupported_draft(&unsupported));
        assert!(Draft::from_td(&unsupported).empty());
        assert!(unsupported_draft(&json!({"content":{"@type":"draftMessageContentText"},"suggested_post_info":{"@type":"suggestedPostInfo"}})));
    }
}

pub fn unsupported_draft(value: &serde_json::Value) -> bool {
    !value.is_null()
        && (value["content"]["@type"].as_str() != Some("draftMessageContentText")
            || !value["suggested_post_info"].is_null())
}
