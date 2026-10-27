//! Strongly typed representations of Telegram and TDLib entities for Nyx Pulse.

use serde::{Deserialize, Serialize};

pub type ChatId = i64;
pub type MessageId = i64;
pub type UserId = i64;

/// High-level authorization states emitted by TDLib.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "details")]
pub enum AuthState {
    WaitTdlibParameters,
    WaitPhoneNumber,
    WaitOtherDeviceConfirmation { link: String },
    WaitCode { is_registered: bool },
    WaitPassword { hint: Option<String> },
    WaitRegistration,
    Ready,
    LoggingOut,
    Closing,
    Closed,
}

/// Category or type of a Telegram chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatType {
    Private,
    BasicGroup,
    Supergroup,
    Channel,
}

/// Telegram maintains independent order and pinning in every list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChatListId {
    #[default]
    Main,
    Archive,
    Folder(i32),
}
impl ChatListId {
    pub fn to_td(self) -> serde_json::Value {
        match self {
            Self::Main => serde_json::json!({"@type":"chatListMain"}),
            Self::Archive => serde_json::json!({"@type":"chatListArchive"}),
            Self::Folder(id) => serde_json::json!({"@type":"chatListFolder","chat_folder_id":id}),
        }
    }
    pub fn matches(self, value: &serde_json::Value) -> bool {
        self.to_td() == *value
    }
}

/// Summary information for a chat in the conversation list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatSummary {
    pub id: ChatId,
    pub title: String,
    pub chat_type: ChatType,
    pub unread_count: i32,
    pub last_message: Option<MessageSummary>,
    pub is_pinned: bool,
    pub is_muted: bool,
    pub avatar_path: Option<String>,
    pub order: u64,
    #[serde(default)]
    pub is_secret: bool,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl ChatSummary {
    pub fn is_marked_unread(&self) -> bool {
        self.data["is_marked_as_unread"] == true
    }
    pub fn list_position(&self, list: ChatListId) -> Option<(u64, bool)> {
        if let Some(positions) = self.data["positions"].as_array() {
            positions.iter().find(|p| list.matches(&p["list"])).and_then(|p| {
                let order = p["order"].as_str().and_then(|s| s.parse().ok()).or_else(|| p["order"].as_u64()).unwrap_or(0);
                (order > 0).then_some((order, p["is_pinned"] == true))
            })
        } else {
            (list == ChatListId::Main && self.order > 0).then_some((self.order, self.is_pinned))
        }
    }
    pub fn for_list(&self, list: ChatListId) -> Option<Self> {
        let (order, pinned) = self.list_position(list)?;
        let mut chat = self.clone();
        chat.order = order;
        chat.is_pinned = pinned;
        Some(chat)
    }
}

/// A single message entity in chat history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: MessageId,
    pub chat_id: ChatId,
    pub sender_id: UserId,
    pub sender_name: String,
    pub date: i64,
    pub is_outgoing: bool,
    pub content: MessageContent,
    pub reply_to_message_id: Option<MessageId>,
    pub is_pinned: bool,
    pub edit_date: i64,
    #[serde(default)]
    pub files: Vec<MediaFile>,
    #[serde(default)]
    pub sending_state: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
}

/// Authoritative, current message actions returned by getMessageProperties.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MessagePermissions {
    pub can_be_replied: bool,
    pub can_be_forwarded: bool,
    pub can_be_copied: bool,
    pub can_be_copied_to_secret_chat: bool,
    pub can_be_edited: bool,
    pub can_be_deleted_only_for_self: bool,
    pub can_be_deleted_for_all_users: bool,
    pub can_be_pinned: bool,
    pub can_be_saved: bool,
    pub can_get_link: bool,
    pub can_report_chat: bool,
}

/// A TDLib file reference. Preview and original files never share a slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaFile {
    pub id: i32,
    pub role: String,
    pub path: Option<String>,
    pub size: i64,
    pub downloaded: i64,
    pub active: bool,
}

/// Payload and media type of a message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MessageContent {
    Text(FormattedText),
    Photo { caption: FormattedText, file_path: Option<String>, thumbnail_path: Option<String>, width: i32, height: i32 },
    Video { caption: FormattedText, file_path: Option<String>, duration_seconds: i32, width: i32, height: i32 },
    Document { file_name: String, mime_type: String, file_size: i64, file_path: Option<String> },
    VoiceNote { duration_seconds: i32, waveform: Vec<u8>, file_path: Option<String> },
    Call { duration_seconds: i32, is_video: bool, discard_reason: String },
    Sticker { emoji: String, set_name: String, file_path: Option<String>, width: i32, height: i32 },
    Unsupported(String),
}

/// Rich text with formatting entity spans.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormattedText {
    pub text: String,
    pub entities: Vec<TextEntity>,
}

impl FormattedText {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), entities: Vec::new() }
    }

    /// Converts text and entities into GTK Pango Markup string.
    pub fn to_pango_markup(&self) -> String {
        if !self.entities.is_empty() {
            return self.entities_to_pango_markup();
        }
        glib_markup_escape(&self.text)
    }

    fn entities_to_pango_markup(&self) -> String {
        // Reopen spans at each boundary so equal and overlapping ranges always nest.
        let mut boundaries = vec![0, self.text.encode_utf16().count()];
        for entity in &self.entities {
            boundaries.push(entity.offset.min(boundaries[1]));
            boundaries.push(entity.offset.saturating_add(entity.length).min(boundaries[1]));
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut out = String::new();
        let chars: Vec<u16> = self.text.encode_utf16().collect();
        for pair in boundaries.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let Ok(part) = String::from_utf16(&chars[start..end]) else { continue };
            let mut closes = Vec::new();
            for entity in &self.entities {
                if entity.offset > start || entity.offset.saturating_add(entity.length) < end {
                    continue;
                }
                let (open, close) = match &entity.kind {
                    TextEntityKind::Bold => ("<b>".into(), "</b>"),
                    TextEntityKind::Italic => ("<i>".into(), "</i>"),
                    TextEntityKind::Underline => ("<u>".into(), "</u>"),
                    TextEntityKind::Strikethrough => ("<s>".into(), "</s>"),
                    TextEntityKind::Code | TextEntityKind::Pre(_) => ("<tt>".into(), "</tt>"),
                    TextEntityKind::TextUrl(url) => (format!("<a href=\"{}\">", glib_markup_escape(url)), "</a>"),
                    _ => continue,
                };
                out.push_str(&open);
                closes.push(close);
            }
            out.push_str(&glib_markup_escape(&part));
            for close in closes.into_iter().rev() {
                out.push_str(close);
            }
        }
        out
    }
}

/// Parses simple inline markdown into GTK Pango markup with XML escaping.
pub fn parse_markdown_to_pango(text: &str) -> String {
    let escaped = glib_markup_escape(text);

    // Code blocks ```...```
    let mut result = escaped;
    while let Some(start) = result.find("```") {
        if let Some(end) = result[start + 3..].find("```") {
            let actual_end = start + 3 + end;
            let inner = &result[start + 3..actual_end];
            let replacement = format!("<tt>{inner}</tt>");
            result = format!("{}{}{}", &result[..start], replacement, &result[actual_end + 3..]);
        } else {
            break;
        }
    }

    // Inline code `...`
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let actual_end = start + 1 + end;
            let inner = &result[start + 1..actual_end];
            let replacement = format!("<tt>{inner}</tt>");
            result = format!("{}{}{}", &result[..start], replacement, &result[actual_end + 1..]);
        } else {
            break;
        }
    }

    // Bold **...**
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let actual_end = start + 2 + end;
            let inner = &result[start + 2..actual_end];
            let replacement = format!("<b>{inner}</b>");
            result = format!("{}{}{}", &result[..start], replacement, &result[actual_end + 2..]);
        } else {
            break;
        }
    }

    // Italic *...*
    while let Some(start) = result.find('*') {
        if let Some(end) = result[start + 1..].find('*') {
            let actual_end = start + 1 + end;
            let inner = &result[start + 1..actual_end];
            let replacement = format!("<i>{inner}</i>");
            result = format!("{}{}{}", &result[..start], replacement, &result[actual_end + 1..]);
        } else {
            break;
        }
    }

    // Strikethrough ~~...~~
    while let Some(start) = result.find("~~") {
        if let Some(end) = result[start + 2..].find("~~") {
            let actual_end = start + 2 + end;
            let inner = &result[start + 2..actual_end];
            let replacement = format!("<s>{inner}</s>");
            result = format!("{}{}{}", &result[..start], replacement, &result[actual_end + 2..]);
        } else {
            break;
        }
    }

    result
}

/// Individual text entity formatting span.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEntity {
    pub offset: usize,
    pub length: usize,
    pub kind: TextEntityKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextEntityKind {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Pre(Option<String>),
    Spoiler,
    TextUrl(String),
    Mention(String),
    CustomEmoji(String),
}

/// Contact or user profile information.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: UserId,
    pub first_name: String,
    pub last_name: String,
    pub username: Option<String>,
    pub phone_number: Option<String>,
    pub is_online: bool,
    pub avatar_path: Option<String>,
}

impl UserProfile {
    pub fn full_name(&self) -> String {
        let f = self.first_name.trim();
        let l = self.last_name.trim();
        if f.is_empty() {
            l.to_string()
        } else if l.is_empty() {
            f.to_string()
        } else {
            format!("{f} {l}")
        }
    }

    pub fn initials(&self) -> String {
        let mut chars = String::new();
        if let Some(c) = self.first_name.chars().next() {
            chars.push(c);
        }
        if let Some(c) = self.last_name.chars().next() {
            chars.push(c);
        }
        if chars.is_empty() { "T".into() } else { chars.to_uppercase() }
    }
}

/// Real-time asynchronous update events delivered from TDLib.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TelegramUpdate {
    Raw(serde_json::Value),
    MessageReplaced { old_id: MessageId, message: MessageSummary },
    AuthStateChanged(AuthState),
    ChatUpdated(ChatSummary),
    ChatPositionUpdated { chat_id: ChatId, order: u64 },
    ChatReadInbox { chat_id: ChatId, last_read_inbox_message_id: MessageId, unread_count: i32 },
    NewMessage(MessageSummary),
    MessageEdited { chat_id: ChatId, message_id: MessageId, content: MessageContent, edit_date: i64 },
    MessagesDeleted { chat_id: ChatId, message_ids: Vec<MessageId> },
    UserTyping { chat_id: ChatId, user_id: UserId, action: String },
    UserStatusChanged { user_id: UserId, is_online: bool },
    CallUpdated { call_id: i32, user_id: UserId, is_outgoing: bool, is_video: bool, state: String, emojis: Vec<String> },
    ChatPhotoUpdated { chat_id: ChatId, path: String },
    FileUpdated { file_id: i32, path: String },
    MessageMediaUpdated { chat_id: ChatId, message_id: MessageId, file_path: String },
}

fn glib_markup_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}
