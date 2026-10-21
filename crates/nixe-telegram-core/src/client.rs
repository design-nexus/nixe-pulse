//! Safe, asynchronous TDLib client wrapper.
//!
//! Spawns a dedicated polling thread to read events from TDLib and maps request
//! responses back to calling futures via `@extra` identifiers.

use crate::ffi::{RawClient, TdJsonLib};
use crate::models::{AuthState, FormattedText, MessageContent, MessageSummary, TelegramUpdate};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use thiserror::Error;
use tokio::sync::oneshot;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("TDLib FFI error: {0}")]
    Ffi(#[from] crate::ffi::FfiError),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Request timed out")]
    Timeout,
    #[error("TDLib Error [{code}]: {message}")]
    Tdlib { code: i32, message: String },
    #[error("Client shut down")]
    Closed,
}

type ResponseSender = oneshot::Sender<Result<Value, ClientError>>;

/// Thread-safe TDLib asynchronous client instance.
#[derive(Clone)]
pub struct TdClient {
    raw: Arc<RawClient>,
    next_request_id: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<u64, ResponseSender>>>,
    #[allow(dead_code)]
    update_tx: async_channel::Sender<TelegramUpdate>,
    update_rx: async_channel::Receiver<TelegramUpdate>,
    is_running: Arc<AtomicBool>,
}

impl TdClient {
    /// Initializes a new TDLib client instance and starts its background polling loop.
    pub fn new(lib: &TdJsonLib) -> Result<Self, ClientError> {
        let _ = lib.execute("{\"@type\":\"setLogVerbosityLevel\",\"new_verbosity_level\":1}");
        let raw = Arc::new(lib.create_client()?);
        let next_request_id = Arc::new(AtomicU64::new(1));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (update_tx, update_rx) = async_channel::unbounded();
        let is_running = Arc::new(AtomicBool::new(true));

        let client = Self {
            raw: raw.clone(),
            next_request_id,
            pending: pending.clone(),
            update_tx: update_tx.clone(),
            update_rx,
            is_running: is_running.clone(),
        };

        // Spawn background polling thread
        std::thread::Builder::new()
            .name("tdlib-receiver".into())
            .spawn(move || {
                tracing::debug!("TDLib polling loop started");
                while is_running.load(Ordering::Relaxed) {
                    match raw.receive(1.0) {
                        Ok(Some(json_str)) => {
                            Self::handle_incoming(&json_str, &pending, &update_tx);
                            if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
                                if v["authorization_state"]["@type"] == "authorizationStateClosed" {
                                    is_running.store(false, Ordering::Relaxed);
                                    update_tx.close();
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!("Error receiving from TDLib: {e}");
                        }
                    }
                }
                tracing::debug!("TDLib polling loop stopped");
            })
            .expect("Failed to spawn TDLib worker thread");

        Ok(client)
    }

    /// Subscribes to the live stream of Telegram updates.
    pub fn subscribe(&self) -> async_channel::Receiver<TelegramUpdate> {
        self.update_rx.clone()
    }

    /// Sends an asynchronous request to TDLib and awaits the response.
    pub async fn execute(&self, method: &str, mut params: Value) -> Result<Value, ClientError> {
        if method == "sendMessage" {
            if let Some(content) = params.get_mut("input_message_content") {
                normalize_input_media(content);
            }
        }
        if !self.is_running.load(Ordering::Relaxed) {
            return Err(ClientError::Closed);
        }

        let id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        // Inject @type and @extra
        if let Some(obj) = params.as_object_mut() {
            obj.insert("@type".into(), json!(method));
            obj.insert("@extra".into(), json!(id.to_string()));
        } else {
            params = json!({
                "@type": method,
                "@extra": id.to_string()
            });
        }

        self.pending.lock().insert(id, tx);
        let payload = serde_json::to_string(&params)?;
        self.raw.send(&payload);

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ClientError::Closed),
            Err(_) => {
                self.pending.lock().remove(&id);
                Err(ClientError::Timeout)
            }
        }
    }

    /// Shuts down the client instance and frees resources.
    pub fn close(&self) {
        self.raw.send("{\"@type\":\"close\"}");
        for _ in 0..if self.raw.is_mock() { 0 } else { 60 } {
            if !self.is_running.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        self.is_running.store(false, Ordering::Relaxed);
        self.update_tx.close();
        let mut map = self.pending.lock();
        for (_, tx) in map.drain() {
            let _ = tx.send(Err(ClientError::Closed));
        }
    }

    fn handle_incoming(
        raw_json: &str,
        pending: &Arc<Mutex<HashMap<u64, ResponseSender>>>,
        update_tx: &async_channel::Sender<TelegramUpdate>,
    ) {
        let Ok(val) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };

        // Check if this is a response to an asynchronous request
        if let Some(extra_str) = val.get("@extra").and_then(Value::as_str) {
            if let Ok(id) = extra_str.parse::<u64>() {
                if let Some(tx) = pending.lock().remove(&id) {
                    if val.get("@type").and_then(Value::as_str) == Some("error") {
                        let code = val.get("code").and_then(Value::as_i64).unwrap_or(0) as i32;
                        let message = val.get("message").and_then(Value::as_str).unwrap_or("Unknown TDLib error").to_string();
                        let _ = tx.send(Err(ClientError::Tdlib { code, message }));
                    } else {
                        let _ = tx.send(Ok(val));
                    }
                    return;
                }
            }
        }

        // Otherwise, process as an asynchronous update
        if let Some(update) = parse_update(&val) {
            let _ = update_tx.try_send(update);
        }
    }
}

/// Parses raw TDLib JSON updates into strongly typed events.
fn parse_update(val: &Value) -> Option<TelegramUpdate> {
    let type_name = val.get("@type")?.as_str()?;
    match type_name {
        "updateAuthorizationState" => {
            let state_val = val.get("authorization_state")?;
            let state_type = state_val.get("@type")?.as_str()?;
            let auth_state = match state_type {
                "authorizationStateWaitTdlibParameters" => AuthState::WaitTdlibParameters,
                "authorizationStateWaitPhoneNumber" => AuthState::WaitPhoneNumber,
                "authorizationStateWaitOtherDeviceConfirmation" => {
                    let link = state_val.get("link").and_then(Value::as_str).unwrap_or_default().to_string();
                    AuthState::WaitOtherDeviceConfirmation { link }
                }
                "authorizationStateWaitCode" => {
                    let is_registered = state_val.get("is_registered").and_then(Value::as_bool).unwrap_or(true);
                    AuthState::WaitCode { is_registered }
                }
                "authorizationStateWaitPassword" => {
                    let hint = state_val.get("password_hint").and_then(Value::as_str).map(str::to_string);
                    AuthState::WaitPassword { hint }
                }
                "authorizationStateWaitRegistration" => AuthState::WaitRegistration,
                "authorizationStateReady" => AuthState::Ready,
                "authorizationStateLoggingOut" => AuthState::LoggingOut,
                "authorizationStateClosing" => AuthState::Closing,
                "authorizationStateClosed" => AuthState::Closed,
                _ => return Some(TelegramUpdate::Raw(val.clone())),
            };
            Some(TelegramUpdate::AuthStateChanged(auth_state))
        }
        "updateNewMessage" => {
            let msg_val = val.get("message")?;
            let msg = parse_message(msg_val)?;
            Some(TelegramUpdate::NewMessage(msg))
        }
        "updateMessageContent" => {
            let chat_id = val.get("chat_id")?.as_i64()?;
            let message_id = val.get("message_id")?.as_i64()?;
            let content_val = val.get("new_content")?;
            let content = parse_message_content(content_val);
            Some(TelegramUpdate::MessageEdited { chat_id, message_id, content, edit_date: chrono::Utc::now().timestamp() })
        }
        "updateDeleteMessages" => {
            if val["is_permanent"].as_bool() == Some(false) {
                return Some(TelegramUpdate::Raw(val.clone()));
            }
            let chat_id = val.get("chat_id")?.as_i64()?;
            let ids = val.get("message_ids")?.as_array()?.iter().filter_map(Value::as_i64).collect();
            Some(TelegramUpdate::MessagesDeleted { chat_id, message_ids: ids })
        }
        "updateChatReadInbox" => {
            let chat_id = val.get("chat_id")?.as_i64()?;
            let last_read = val.get("last_read_inbox_message_id")?.as_i64()?;
            let unread = val.get("unread_count")?.as_i64()? as i32;
            Some(TelegramUpdate::ChatReadInbox { chat_id, last_read_inbox_message_id: last_read, unread_count: unread })
        }
        "updateUserAction" => {
            let chat_id = val.get("chat_id")?.as_i64()?;
            let user_id = val.get("user_id")?.as_i64()?;
            let action_type = val.get("action").and_then(|a| a.get("@type")).and_then(Value::as_str).unwrap_or("typing");
            Some(TelegramUpdate::UserTyping { chat_id, user_id, action: action_type.to_string() })
        }
        "updateUserStatus" => {
            let user_id = val.get("user_id")?.as_i64()?;
            let status = val.get("status")?.get("@type")?.as_str()?;
            let is_online = status == "userStatusOnline";
            Some(TelegramUpdate::UserStatusChanged { user_id, is_online })
        }
        "updateCall" => Some(TelegramUpdate::Raw(val.clone())),
        "updateFile" => Some(TelegramUpdate::Raw(val.clone())),
        _ => Some(TelegramUpdate::Raw(val.clone())),
    }
}

pub fn parse_message(val: &Value) -> Option<MessageSummary> {
    let id = val.get("id")?.as_i64()?;
    let chat_id = val.get("chat_id")?.as_i64()?;
    let date = val.get("date")?.as_i64()?;
    let is_outgoing = val.get("is_outgoing").and_then(Value::as_bool).unwrap_or(false);
    let edit_date = val.get("edit_date").and_then(Value::as_i64).unwrap_or(0);
    let reply_to_message_id = val.get("reply_to").and_then(|r| r.get("message_id")).and_then(Value::as_i64);

    let sender_id = val.get("sender_id").and_then(|s| s.get("user_id")).and_then(Value::as_i64).unwrap_or(0);

    let content_val = val.get("content")?;
    let content = parse_message_content(content_val);

    let sender_name = val
        .get("author_signature")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| if is_outgoing { "Me".into() } else { String::new() });

    Some(MessageSummary {
        id,
        chat_id,
        sender_id,
        sender_name,
        date,
        is_outgoing,
        content,
        reply_to_message_id,
        is_pinned: val["is_pinned"].as_bool().unwrap_or(false),
        edit_date,
        files: media_files(content_val),
        sending_state: val.get("sending_state").filter(|v| !v.is_null()).map(|v| v["@type"].as_str().unwrap_or("").to_string()),
        data: val.clone(),
    })
}

pub fn parse_message_content(val: &Value) -> MessageContent {
    let content_type = val.get("@type").and_then(Value::as_str).unwrap_or("unknown");

    match content_type {
        "messageText" => MessageContent::Text(parse_formatted_text(val.get("text").unwrap_or(&Value::Null))),
        "messagePhoto" => {
            let mut file_path = None;
            let mut thumbnail_path = None;
            let mut width = 0;
            let mut height = 0;

            if let Some(photo) = val.get("photo") {
                if let Some(sizes) = photo.get("sizes").and_then(Value::as_array) {
                    for s in sizes {
                        let w = s.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
                        let h = s.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
                        if w >= width {
                            width = w;
                            height = h;
                            if let Some(local) = s.get("photo").and_then(|p| p.get("local")) {
                                let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                                let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                                if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                                    file_path = Some(p.to_string());
                                }
                            }
                        }
                    }
                    if let Some(first) = sizes.first() {
                        if let Some(local) = first.get("photo").and_then(|p| p.get("local")) {
                            let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                            let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                            if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                                thumbnail_path = Some(p.to_string());
                            }
                        }
                    }
                }
            }

            MessageContent::Photo {
                caption: parse_formatted_text(val.get("caption").unwrap_or(&Value::Null)),
                file_path,
                thumbnail_path,
                width,
                height,
            }
        }
        "messageVideo" => {
            let vid = val.get("video");
            let duration = vid.and_then(|v| v.get("duration")).and_then(Value::as_i64).unwrap_or(0) as i32;
            let width = vid.and_then(|v| v.get("width")).and_then(Value::as_i64).unwrap_or(0) as i32;
            let height = vid.and_then(|v| v.get("height")).and_then(Value::as_i64).unwrap_or(0) as i32;

            let mut file_path = None;
            if let Some(local) = vid.and_then(|v| v.get("video")).and_then(|f| f.get("local")) {
                let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                    file_path = Some(p.to_string());
                }
            }

            MessageContent::Video {
                caption: parse_formatted_text(val.get("caption").unwrap_or(&Value::Null)),
                file_path,
                duration_seconds: duration,
                width,
                height,
            }
        }
        "messageVideoNote" if val["is_secret"] != true => {
            let video = &val["video_note"];
            let local = &video["video"]["local"];
            let file_path = local["path"]
                .as_str()
                .filter(|p| local["is_downloading_completed"] == true && !p.is_empty() && std::path::Path::new(p).exists())
                .map(str::to_owned);
            let length = video["length"].as_i64().unwrap_or(0) as i32;
            MessageContent::Video {
                caption: parse_formatted_text(&Value::Null),
                file_path,
                duration_seconds: video["duration"].as_i64().unwrap_or(0) as i32,
                width: length,
                height: length,
            }
        }
        "messageDocument" => {
            let doc = val.get("document");
            let file_name = doc.and_then(|d| d.get("file_name")).and_then(Value::as_str).unwrap_or("document").to_string();
            let mime_type =
                doc.and_then(|d| d.get("mime_type")).and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
            let file_size = doc.and_then(|d| d.get("document")).and_then(|f| f.get("size")).and_then(Value::as_i64).unwrap_or(0);
            let mut file_path = None;
            if let Some(local) = doc.and_then(|d| d.get("document")).and_then(|f| f.get("local")) {
                let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                    file_path = Some(p.to_string());
                }
            }

            MessageContent::Document { file_name, mime_type, file_size, file_path }
        }
        "messageVoiceNote" => {
            let voice = val.get("voice_note");
            let duration = voice.and_then(|v| v.get("duration")).and_then(Value::as_i64).unwrap_or(0) as i32;
            let mut file_path = None;
            if let Some(local) = voice.and_then(|v| v.get("voice")).and_then(|f| f.get("local")) {
                let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                    file_path = Some(p.to_string());
                }
            }

            MessageContent::VoiceNote { duration_seconds: duration, waveform: Vec::new(), file_path }
        }
        "messageCall" => {
            let duration = val.get("duration").and_then(Value::as_i64).unwrap_or(0) as i32;
            let is_video = val.get("is_video").and_then(Value::as_bool).unwrap_or(false);
            let reason = val
                .get("discard_reason")
                .and_then(|r| r.get("@type"))
                .and_then(Value::as_str)
                .unwrap_or("callDiscardReasonEmpty")
                .to_string();
            MessageContent::Call { duration_seconds: duration, is_video, discard_reason: reason }
        }
        "messageSticker" => {
            let sticker = val.get("sticker");
            let emoji = sticker.and_then(|s| s.get("emoji")).and_then(Value::as_str).unwrap_or("🐱").to_string();
            let set_name = sticker.and_then(|s| s.get("set_name")).and_then(Value::as_str).unwrap_or("").to_string();
            let width = sticker.and_then(|s| s.get("width")).and_then(Value::as_i64).unwrap_or(256) as i32;
            let height = sticker.and_then(|s| s.get("height")).and_then(Value::as_i64).unwrap_or(256) as i32;
            let mut file_path = None;
            if let Some(local) = sticker.and_then(|s| s.get("sticker")).and_then(|f| f.get("local")) {
                let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                    file_path = Some(p.to_string());
                }
            }

            MessageContent::Sticker { emoji, set_name, file_path, width, height }
        }
        "messageAnimation" => {
            let anim = val.get("animation");
            let duration = anim.and_then(|a| a.get("duration")).and_then(Value::as_i64).unwrap_or(0) as i32;
            let width = anim.and_then(|a| a.get("width")).and_then(Value::as_i64).unwrap_or(256) as i32;
            let height = anim.and_then(|a| a.get("height")).and_then(Value::as_i64).unwrap_or(256) as i32;
            let mut file_path = None;
            if let Some(local) = anim.and_then(|a| a.get("animation")).and_then(|f| f.get("local")) {
                let p = local.get("path").and_then(Value::as_str).unwrap_or("");
                let comp = local.get("is_downloading_completed").and_then(Value::as_bool).unwrap_or(false);
                if comp && !p.is_empty() && std::path::Path::new(p).exists() {
                    file_path = Some(p.to_string());
                }
            }
            MessageContent::Video {
                caption: parse_formatted_text(val.get("caption").unwrap_or(&Value::Null)),
                file_path,
                duration_seconds: duration,
                width,
                height,
            }
        }
        other => MessageContent::Unsupported(other.to_string()),
    }
}

/// Parse TDLib's formattedText without interpreting incoming text as Markdown.
pub fn parse_formatted_text(val: &Value) -> FormattedText {
    use crate::models::{TextEntity, TextEntityKind as K};
    let text = val["text"].as_str().unwrap_or_default().to_owned();
    let entities = val["entities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let t = &e["type"];
            let kind = match t["@type"].as_str()? {
                "textEntityTypeBold" => K::Bold,
                "textEntityTypeItalic" => K::Italic,
                "textEntityTypeUnderline" => K::Underline,
                "textEntityTypeStrikethrough" => K::Strikethrough,
                "textEntityTypeCode" => K::Code,
                "textEntityTypePre" | "textEntityTypePreCode" => K::Pre(t["language"].as_str().map(str::to_owned)),
                "textEntityTypeSpoiler" => K::Spoiler,
                "textEntityTypeTextUrl" => K::TextUrl(t["url"].as_str()?.to_owned()),
                "textEntityTypeUrl" => {
                    let utf16: Vec<u16> = text.encode_utf16().collect();
                    let start = e["offset"].as_u64()? as usize;
                    let end = start.checked_add(e["length"].as_u64()? as usize)?;
                    K::TextUrl(String::from_utf16(utf16.get(start..end)?).ok()?)
                }
                "textEntityTypeCustomEmoji" => K::CustomEmoji(t["custom_emoji_id"].to_string()),
                _ => return None,
            };
            Some(TextEntity { offset: e["offset"].as_u64()? as usize, length: e["length"].as_u64()? as usize, kind })
        })
        .collect();
    FormattedText { text, entities }
}

pub fn media_files(content: &Value) -> Vec<crate::models::MediaFile> {
    fn add(files: &mut Vec<crate::models::MediaFile>, f: &Value, role: &str) {
        let Some(id) = f["id"].as_i64() else { return };
        if files.iter().any(|v| v.id == id as i32) {
            return;
        }
        let completed = f["local"]["is_downloading_completed"].as_bool().unwrap_or(false);
        files.push(crate::models::MediaFile {
            id: id as i32,
            role: role.into(),
            path: if completed { f["local"]["path"].as_str().filter(|s| !s.is_empty()).map(str::to_owned) } else { None },
            size: f["size"].as_i64().or_else(|| f["expected_size"].as_i64()).unwrap_or(0),
            downloaded: f["local"]["downloaded_size"].as_i64().unwrap_or(0),
            active: f["local"]["is_downloading_active"].as_bool().unwrap_or(false),
        });
    }
    let mut files = Vec::new();
    if let Some(sizes) = content["link_preview"]["type"]["photo"]["sizes"].as_array() {
        if let Some(size) = sizes
            .iter()
            .filter(|s| s["width"].as_i64().unwrap_or(0) <= 640)
            .max_by_key(|s| s["width"].as_i64().unwrap_or(0))
            .or_else(|| sizes.first())
        {
            add(&mut files, &size["photo"], "link-preview");
        }
    }
    add(&mut files, &content["link_preview"]["type"]["video"]["thumbnail"]["file"], "link-preview");
    if let Some(sizes) = content["photo"]["sizes"].as_array() {
        if let Some(s) = sizes.first() {
            add(&mut files, &s["photo"], "thumbnail");
        }
        if let Some(s) = sizes.iter().max_by_key(|s| s["width"].as_i64().unwrap_or(0)) {
            add(&mut files, &s["photo"], "original");
        }
        if files.len() == 1 {
            files[0].role = "original".into();
        }
    }
    for (kind, field) in [
        ("video", "video"),
        ("animation", "animation"),
        ("sticker", "sticker"),
        ("document", "document"),
        ("voice_note", "voice"),
        ("audio", "audio"),
        ("video_note", "video"),
    ] {
        add(&mut files, &content[kind]["thumbnail"]["file"], "thumbnail");
        add(&mut files, &content[kind][field], "original");
    }
    files
}

/// TDLib 1.8.67 separates media metadata from InputFile in typed input objects.
/// Accept legacy callers at this boundary while preserving already typed inputs.
pub fn normalize_input_media(content: &mut Value) {
    let (field, wrapper) = match content["@type"].as_str() {
        Some("inputMessagePhoto") => ("photo", "inputPhoto"),
        Some("inputMessageVideo") => ("video", "inputVideo"),
        Some("inputMessageDocument") => ("document", "inputDocument"),
        Some("inputMessageAnimation") => ("animation", "inputAnimation"),
        Some("inputMessageSticker") => ("sticker", "inputSticker"),
        Some("inputMessageVoiceNote") => ("voice_note", "inputVoiceNote"),
        Some("inputMessageVideoNote") => ("video_note", "inputVideoNote"),
        Some("inputMessageAudio") => ("audio", "inputAudio"),
        _ => return,
    };
    if !content[field]["@type"].as_str().is_some_and(|t| t.starts_with("inputFile")) {
        return;
    }
    let file = content[field].take();
    let mut media = json!({"@type":wrapper,field:file});
    if let Some(object) = content.as_object_mut() {
        for key in [
            "thumbnail",
            "duration",
            "width",
            "height",
            "length",
            "waveform",
            "added_sticker_file_ids",
            "disable_content_type_detection",
            "supports_streaming",
            "album_cover_thumbnail",
            "title",
            "performer",
            "cover",
            "start_timestamp",
        ] {
            if let Some(value) = object.remove(key) {
                media[key] = value;
            }
        }
    }
    content[field] = media;
}
#[cfg(test)]
mod upload_schema_regression {
    use super::*;
    #[test]
    fn media_metadata_belongs_to_typed_inputs() {
        let mut voice = json!({"@type":"inputMessageVoiceNote","voice_note":{"@type":"inputFileLocal","path":"/tmp/voice.ogg"},"duration":7,"waveform":"","caption":{"text":"caption","entities":[]}});
        normalize_input_media(&mut voice);
        assert_eq!(voice["voice_note"]["@type"], "inputVoiceNote");
        assert_eq!(voice["voice_note"]["voice_note"]["path"], "/tmp/voice.ogg");
        assert_eq!(voice["voice_note"]["duration"], 7);
        assert!(voice.get("duration").is_none());
        let once = voice.clone();
        normalize_input_media(&mut voice);
        assert_eq!(voice, once);
        let mut photo = json!({"@type":"inputMessagePhoto","photo":{"@type":"inputFileId","id":17}});
        normalize_input_media(&mut photo);
        assert_eq!(photo["photo"]["@type"], "inputPhoto");
        assert_eq!(photo["photo"]["photo"]["id"], 17);
    }
}
