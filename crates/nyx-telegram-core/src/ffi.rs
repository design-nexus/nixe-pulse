//! Low-level FFI bindings to TDLib's C JSON interface (`libtdjson.so`).
//!
//! Provides dynamic symbol loading with environment and system path discovery,
//! and fallback simulation support for test environments.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FfiError {
    #[error("Could not load libtdjson library: {0}")]
    LibraryLoad(#[from] libloading::Error),
    #[error("Null string encountered in TDLib response")]
    NullResponse,
    #[error("String conversion error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("TDLib client initialization failed")]
    ClientInitFailed,
}

type TdCreateFn = unsafe extern "C" fn() -> *mut c_void;
type TdSendFn = unsafe extern "C" fn(*mut c_void, *const c_char);
type TdReceiveFn = unsafe extern "C" fn(*mut c_void, c_double) -> *const c_char;
type TdExecuteFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *const c_char;
type TdDestroyFn = unsafe extern "C" fn(*mut c_void);

/// Loaded function pointers for a `libtdjson.so` instance.
struct TdJsonSymbols {
    _lib: &'static libloading::Library,
    create: TdCreateFn,
    send: TdSendFn,
    receive: TdReceiveFn,
    execute: TdExecuteFn,
    destroy: TdDestroyFn,
}

/// Dynamic loader and manager for TDLib's C JSON interface.
#[derive(Clone)]
pub struct TdJsonLib {
    symbols: Option<Arc<TdJsonSymbols>>,
}

impl TdJsonLib {
    /// Attempts to locate and load `libtdjson.so` from standard locations:
    /// 1. Path specified in `TDLIB_PATH` environment variable.
    /// 2. `/usr/lib/libtdjson.so`
    /// 3. `/usr/local/lib/libtdjson.so`
    /// 4. Default dynamic linker resolution (`libtdjson.so`)
    pub fn load() -> Result<Self, FfiError> {
        let candidates = candidate_paths();
        for path in &candidates {
            if path.exists() {
                if let Ok(lib) = Self::load_from_path(path) {
                    tracing::info!("Loaded TDLib from: {}", path.display());
                    return Ok(lib);
                }
            }
        }

        // Try standard system linker name
        if let Ok(lib) = Self::load_from_name("libtdjson.so") {
            tracing::info!("Loaded TDLib via dynamic linker (libtdjson.so)");
            return Ok(lib);
        }

        // Fallback: mock client for testing and development if libtdjson.so is not present
        tracing::warn!("libtdjson.so not found on system; falling back to mock TDLib driver");
        Err(FfiError::ClientInitFailed)
    }

    /// Creates an explicit mock driver instance for headless unit tests.
    pub fn mock() -> Self {
        Self { symbols: None }
    }

    /// Loads `libtdjson` from a specific file path.
    pub fn load_from_path(path: &Path) -> Result<Self, FfiError> {
        unsafe {
            let lib = Box::leak(Box::new(libloading::Library::new(path)?));
            let create = *lib.get(b"td_json_client_create\0")?;
            let send = *lib.get(b"td_json_client_send\0")?;
            let receive = *lib.get(b"td_json_client_receive\0")?;
            let execute = *lib.get(b"td_json_client_execute\0")?;
            let destroy = *lib.get(b"td_json_client_destroy\0")?;

            Ok(Self { symbols: Some(Arc::new(TdJsonSymbols { _lib: lib, create, send, receive, execute, destroy })) })
        }
    }

    /// Loads `libtdjson` using library name.
    pub fn load_from_name(name: &str) -> Result<Self, FfiError> {
        unsafe {
            let lib = Box::leak(Box::new(libloading::Library::new(name)?));
            let create = *lib.get(b"td_json_client_create\0")?;
            let send = *lib.get(b"td_json_client_send\0")?;
            let receive = *lib.get(b"td_json_client_receive\0")?;
            let execute = *lib.get(b"td_json_client_execute\0")?;
            let destroy = *lib.get(b"td_json_client_destroy\0")?;

            Ok(Self { symbols: Some(Arc::new(TdJsonSymbols { _lib: lib, create, send, receive, execute, destroy })) })
        }
    }

    /// Creates a new TDLib client instance handle.
    pub fn create_client(&self) -> Result<RawClient, FfiError> {
        if let Some(ref sym) = self.symbols {
            let handle = unsafe { (sym.create)() };
            if handle.is_null() {
                return Err(FfiError::ClientInitFailed);
            }
            Ok(RawClient { handle, symbols: Some(sym.clone()), mock_queue: None })
        } else {
            // Mock driver: queue initial WaitTdlibParameters state
            let queue = Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::new()));
            queue.lock().push_back(
                serde_json::json!({
                    "@type": "updateAuthorizationState",
                    "authorization_state": {
                        "@type": "authorizationStateWaitTdlibParameters"
                    }
                })
                .to_string(),
            );

            Ok(RawClient { handle: std::ptr::null_mut(), symbols: None, mock_queue: Some(queue) })
        }
    }

    /// Executes a synchronous TDLib request that does not depend on client state.
    pub fn execute(&self, request: &str) -> Result<Option<String>, FfiError> {
        if let Some(ref sym) = self.symbols {
            let c_req = CString::new(request).unwrap_or_default();
            unsafe {
                let res_ptr = (sym.execute)(std::ptr::null_mut(), c_req.as_ptr());
                if res_ptr.is_null() {
                    Ok(None)
                } else {
                    let c_str = CStr::from_ptr(res_ptr);
                    Ok(Some(c_str.to_str()?.to_string()))
                }
            }
        } else {
            Ok(None)
        }
    }
}

/// Managed handle to a single TDLib native client instance.
pub struct RawClient {
    handle: *mut c_void,
    symbols: Option<Arc<TdJsonSymbols>>,
    pub(crate) mock_queue: Option<Arc<parking_lot::Mutex<std::collections::VecDeque<String>>>>,
}

unsafe impl Send for RawClient {}
unsafe impl Sync for RawClient {}

impl RawClient {
    pub fn is_mock(&self) -> bool {
        self.symbols.is_none()
    }

    /// Sends an asynchronous JSON request to TDLib.
    pub fn send(&self, request: &str) {
        if let Some(ref sym) = self.symbols {
            if !self.handle.is_null() {
                let c_req = CString::new(request).unwrap_or_default();
                unsafe {
                    (sym.send)(self.handle, c_req.as_ptr());
                }
            }
        } else if let Some(ref queue) = self.mock_queue {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(request) {
                let extra = val.get("@extra").cloned();
                let method = val.get("@type").and_then(|v| v.as_str()).unwrap_or("");
                let mut q = queue.lock();

                match method {
                    "setTdlibParameters" => {
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateAuthorizationState",
                                "authorization_state": {
                                    "@type": "authorizationStateWaitPhoneNumber"
                                }
                            })
                            .to_string(),
                        );
                    }
                    "setAuthenticationPhoneNumber" => {
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateAuthorizationState",
                                "authorization_state": {
                                    "@type": "authorizationStateWaitCode",
                                    "is_registered": true
                                }
                            })
                            .to_string(),
                        );
                    }
                    "requestQrCodeAuthentication" => {
                        let token = "tg://login?token=nyx_pulse_login_token_demo_987654321";
                        if let Some(ref ex) = extra {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "ok",
                                    "@extra": ex,
                                    "link": token
                                })
                                .to_string(),
                            );
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateAuthorizationState",
                                "authorization_state": {
                                    "@type": "authorizationStateWaitOtherDeviceConfirmation",
                                    "link": token
                                }
                            })
                            .to_string(),
                        );
                    }
                    "checkAuthenticationCode" => {
                        let code = val.get("code").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        if code == "2222" {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "updateAuthorizationState",
                                    "authorization_state": {
                                        "@type": "authorizationStateWaitPassword",
                                        "password_hint": "Enter your 2FA cloud password"
                                    }
                                })
                                .to_string(),
                            );
                        } else {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "updateAuthorizationState",
                                    "authorization_state": {
                                        "@type": "authorizationStateReady"
                                    }
                                })
                                .to_string(),
                            );
                        }
                    }
                    "checkAuthenticationPassword" => {
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateAuthorizationState",
                                "authorization_state": {
                                    "@type": "authorizationStateReady"
                                }
                            })
                            .to_string(),
                        );
                    }
                    "logOut" => {
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateAuthorizationState",
                                "authorization_state": {
                                    "@type": "authorizationStateWaitPhoneNumber"
                                }
                            })
                            .to_string(),
                        );
                    }
                    "loadChats" => {
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                    }
                    "getChats" => {
                        if let Some(ref ex) = extra {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "chats",
                                    "@extra": ex,
                                    "chat_ids": [101, 102, 103, 104],
                                    "total_count": 4
                                })
                                .to_string(),
                            );
                        }
                    }
                    "getChat" => {
                        let chat_id = val.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(101);
                        let (title, chat_type, unread, pinned, snippet) = match chat_id {
                            101 => ("Saved Messages", "chatTypePrivate", 0, true, "Nyx Pulse configuration notes"),
                            102 => ("Telegram Team", "chatTypeChannel", 1, false, "Welcome to Telegram on Linux."),
                            103 => (
                                "Omarchy Community",
                                "chatTypeSupergroup",
                                3,
                                false,
                                "New Hyprland dynamic blur profile is awesome!",
                            ),
                            104 => ("Durov", "chatTypeSecret", 0, false, "🔒 End-to-end encrypted secret chat active"),
                            _ => ("Nyx Developers", "chatTypeBasicGroup", 0, false, "Pulse GTK4 integration verified."),
                        };
                        let now = chrono::Utc::now().timestamp();
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({
                                "@type": "chat",
                                "@extra": ex,
                                "id": chat_id,
                                "title": title,
                                "type": { "@type": chat_type },
                                "unread_count": unread,
                                "is_pinned": pinned,
                                "last_message": {
                                    "id": 1000 + chat_id,
                                    "chat_id": chat_id,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": if chat_id == 101 { 1 } else { 777000 } },
                                    "date": now - 300,
                                    "is_outgoing": chat_id == 101,
                                    "can_be_edited": chat_id == 101,
                                    "can_be_deleted_for_all_users": chat_id == 101,
                                    "content": {
                                        "@type": "messageText",
                                        "text": {
                                            "@type": "formattedText",
                                            "text": snippet,
                                            "entities": []
                                        }
                                    }
                                }
                            }).to_string());
                        }
                    }
                    "getChatHistory" => {
                        let chat_id = val.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(101);
                        let now = chrono::Utc::now().timestamp();
                        let messages = match chat_id {
                            101 => vec![
                                serde_json::json!({
                                    "id": 1,
                                    "chat_id": 101,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                                    "date": now - 86400,
                                    "is_outgoing": true,
                                    "can_be_edited": true,
                                    "can_be_deleted_for_all_users": true,
                                    "content": {
                                        "@type": "messageText",
                                        "text": {
                                            "@type": "formattedText",
                                            "text": "Pulse workspace notes: **GTK4** + TDLib native client on **Omarchy Linux**.",
                                            "entities": []
                                        }
                                    }
                                }),
                                serde_json::json!({
                                    "id": 2,
                                    "chat_id": 101,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                                    "date": now - 3600,
                                    "is_outgoing": true,
                                    "can_be_edited": true,
                                    "can_be_deleted_for_all_users": true,
                                    "content": {
                                        "@type": "messageText",
                                        "text": {
                                            "@type": "formattedText",
                                            "text": "Tested message composer with `Enter` to send, `Shift+Enter` for newline.",
                                            "entities": []
                                        }
                                    }
                                }),
                                serde_json::json!({
                                    "id": 3,
                                    "chat_id": 101,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                                    "date": now - 300,
                                    "is_outgoing": true,
                                    "can_be_edited": true,
                                    "can_be_deleted_for_all_users": true,
                                    "content": {
                                        "@type": "messageText",
                                        "text": {
                                            "@type": "formattedText",
                                            "text": "Nyx Pulse configuration notes",
                                            "entities": []
                                        }
                                    }
                                }),
                                serde_json::json!({
                                    "id": 4,
                                    "chat_id": 101,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                                    "date": now - 180,
                                    "is_outgoing": true,
                                    "can_be_edited": true,
                                    "can_be_deleted_for_all_users": true,
                                    "content": {
                                        "@type": "messagePhoto",
                                        "photo": {
                                            "sizes": [
                                                {
                                                    "type": "m",
                                                    "width": 640,
                                                    "height": 480,
                                                    "photo": {
                                                        "id": 501,
                                                        "local": {
                                                            "path": "/home/ken/.local/share/nyx-pulse/avatars/durov.jpg",
                                                            "is_downloading_completed": true
                                                        }
                                                    }
                                                }
                                            ]
                                        },
                                        "caption": {
                                            "@type": "formattedText",
                                            "text": "Pulse UI architecture diagram and asset preview"
                                        }
                                    }
                                }),
                                serde_json::json!({
                                    "id": 5,
                                    "chat_id": 101,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                                    "date": now - 60,
                                    "is_outgoing": true,
                                    "can_be_edited": true,
                                    "can_be_deleted_for_all_users": true,
                                    "content": {
                                        "@type": "messageDocument",
                                        "document": {
                                            "file_name": "release-notes-v0.1.0.pdf",
                                            "mime_type": "application/pdf",
                                            "document": {
                                                "id": 502,
                                                "size": 2457600,
                                                "local": {
                                                    "path": "",
                                                    "is_downloading_completed": false
                                                }
                                            }
                                        }
                                    }
                                }),
                                serde_json::json!({
                                    "id": 6,
                                    "chat_id": 101,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                                    "date": now - 10,
                                    "is_outgoing": true,
                                    "can_be_edited": true,
                                    "can_be_deleted_for_all_users": true,
                                    "content": {
                                        "@type": "messageVoiceNote",
                                        "voice_note": {
                                            "duration": 42,
                                            "voice": {
                                                "id": 503,
                                                "local": {
                                                    "path": "",
                                                    "is_downloading_completed": false
                                                }
                                            }
                                        }
                                    }
                                }),
                            ],
                            102 => vec![serde_json::json!({
                                "id": 10,
                                "chat_id": 102,
                                "sender_id": { "@type": "messageSenderUser", "user_id": 777000 },
                                "date": now - 7200,
                                "is_outgoing": false,
                                "can_be_edited": false,
                                "can_be_deleted_for_all_users": false,
                                "content": {
                                    "@type": "messageText",
                                    "text": {
                                        "@type": "formattedText",
                                        "text": "Welcome to Telegram on Linux. Your account is active and verified.",
                                        "entities": []
                                    }
                                }
                            })],
                            103 => vec![
                                serde_json::json!({
                                    "id": 20,
                                    "chat_id": 103,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 501 },
                                    "date": now - 1800,
                                    "is_outgoing": false,
                                    "can_be_edited": false,
                                    "can_be_deleted_for_all_users": false,
                                    "content": {
                                        "@type": "messageText",
                                        "text": {
                                            "@type": "formattedText",
                                            "text": "Hey everyone! How is the new Wayland pipewire screen sharing working?",
                                            "entities": []
                                        }
                                    }
                                }),
                                serde_json::json!({
                                    "id": 21,
                                    "chat_id": 103,
                                    "sender_id": { "@type": "messageSenderUser", "user_id": 502 },
                                    "date": now - 600,
                                    "is_outgoing": false,
                                    "can_be_edited": false,
                                    "can_be_deleted_for_all_users": false,
                                    "reply_to": { "message_id": 20 },
                                    "content": {
                                        "@type": "messageText",
                                        "text": {
                                            "@type": "formattedText",
                                            "text": "Working smoothly with xdg-desktop-portal-hyprland!",
                                            "entities": []
                                        }
                                    }
                                }),
                            ],
                            _ => vec![serde_json::json!({
                                "id": 30,
                                "chat_id": chat_id,
                                "sender_id": { "@type": "messageSenderUser", "user_id": 601 },
                                "date": now - 1200,
                                "is_outgoing": false,
                                "can_be_edited": false,
                                "can_be_deleted_for_all_users": false,
                                "content": {
                                    "@type": "messageText",
                                    "text": {
                                        "@type": "formattedText",
                                        "text": "Pulse GTK4 integration verified.",
                                        "entities": []
                                    }
                                }
                            })],
                        };
                        if let Some(ref ex) = extra {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "messages",
                                    "@extra": ex,
                                    "total_count": messages.len(),
                                    "messages": messages
                                })
                                .to_string(),
                            );
                        }
                    }
                    "sendMessage" => {
                        let chat_id = val.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(101);
                        let text = val
                            .get("input_message_content")
                            .and_then(|c| c.get("text"))
                            .and_then(|t| t.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();
                        let reply_id = val.get("reply_to").and_then(|r| r.get("message_id")).and_then(|m| m.as_i64());
                        let now = chrono::Utc::now().timestamp();
                        let msg_id = now * 1000 + 42;
                        let msg_obj = serde_json::json!({
                            "@type": "message",
                            "id": msg_id,
                            "chat_id": chat_id,
                            "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                            "date": now,
                            "is_outgoing": true,
                            "can_be_edited": true,
                            "can_be_deleted_for_all_users": true,
                            "reply_to": reply_id.map(|rid| serde_json::json!({ "message_id": rid })),
                            "content": {
                                "@type": "messageText",
                                "text": {
                                    "@type": "formattedText",
                                    "text": text,
                                    "entities": []
                                }
                            }
                        });

                        if let Some(ref ex) = extra {
                            let mut resp = msg_obj.clone();
                            resp["@extra"] = ex.clone();
                            q.push_back(resp.to_string());
                        }

                        q.push_back(
                            serde_json::json!({
                                "@type": "updateNewMessage",
                                "message": msg_obj
                            })
                            .to_string(),
                        );
                    }
                    "sendChatAction" => {
                        let chat_id = val.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        if chat_id > 0 {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "updateUserAction",
                                    "chat_id": chat_id,
                                    "user_id": 1,
                                    "action": { "@type": "chatActionTyping" }
                                })
                                .to_string(),
                            );
                        }
                    }
                    "viewMessages" => {
                        let chat_id = val.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        if chat_id > 0 {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "updateChatReadInbox",
                                    "chat_id": chat_id,
                                    "last_read_inbox_message_id": 999999,
                                    "unread_count": 0
                                })
                                .to_string(),
                            );
                        }
                    }
                    "createCall" => {
                        let user_id = val.get("user_id").and_then(|v| v.as_i64()).unwrap_or(101);
                        let is_video = val.get("is_video").and_then(|v| v.as_bool()).unwrap_or(false);
                        if let Some(ref ex) = extra {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "callId",
                                    "@extra": ex,
                                    "id": 7001
                                })
                                .to_string(),
                            );
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateCall",
                                "call": {
                                    "id": 7001,
                                    "user_id": user_id,
                                    "is_outgoing": true,
                                    "is_video": is_video,
                                    "state": {
                                        "@type": "callStateReady",
                                        "emojis": ["🍎", "🚀", "🌟", "🐱"]
                                    }
                                }
                            })
                            .to_string(),
                        );
                    }
                    "acceptCall" => {
                        let call_id = val.get("call_id").and_then(|v| v.as_i64()).unwrap_or(7001) as i32;
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateCall",
                                "call": {
                                    "id": call_id,
                                    "user_id": 101,
                                    "is_outgoing": false,
                                    "is_video": false,
                                    "state": {
                                        "@type": "callStateReady",
                                        "emojis": ["🍎", "🚀", "🌟", "🐱"]
                                    }
                                }
                            })
                            .to_string(),
                        );
                    }
                    "discardCall" => {
                        let call_id = val.get("call_id").and_then(|v| v.as_i64()).unwrap_or(7001) as i32;
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateCall",
                                "call": {
                                    "id": call_id,
                                    "user_id": 101,
                                    "is_outgoing": true,
                                    "is_video": false,
                                    "state": {
                                        "@type": "callStateDiscarded"
                                    }
                                }
                            })
                            .to_string(),
                        );
                    }
                    "createNewSecretChat" => {
                        let user_id = val.get("user_id").and_then(|v| v.as_i64()).unwrap_or(104);
                        if let Some(ref ex) = extra {
                            q.push_back(
                                serde_json::json!({
                                    "@type": "chat",
                                    "@extra": ex,
                                    "id": user_id,
                                    "title": "Durov (Secret)",
                                    "type": { "@type": "chatTypeSecret" },
                                    "unread_count": 0,
                                    "is_pinned": false
                                })
                                .to_string(),
                            );
                        }
                    }
                    "sendSticker" => {
                        let chat_id = val.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(101);
                        let emoji = val.get("emoji").and_then(|e| e.as_str()).unwrap_or("🐱");
                        let now = chrono::Utc::now().timestamp();
                        let msg_id = now * 1000 + 43;
                        let msg_obj = serde_json::json!({
                            "@type": "message",
                            "id": msg_id,
                            "chat_id": chat_id,
                            "sender_id": { "@type": "messageSenderUser", "user_id": 1 },
                            "date": now,
                            "is_outgoing": true,
                            "can_be_edited": false,
                            "can_be_deleted_for_all_users": true,
                            "content": {
                                "@type": "messageSticker",
                                "sticker": {
                                    "emoji": emoji,
                                    "set_name": "OmarchyStickers",
                                    "width": 256,
                                    "height": 256
                                }
                            }
                        });
                        if let Some(ref ex) = extra {
                            let mut resp = msg_obj.clone();
                            resp["@extra"] = ex.clone();
                            q.push_back(resp.to_string());
                        }
                        q.push_back(
                            serde_json::json!({
                                "@type": "updateNewMessage",
                                "message": msg_obj
                            })
                            .to_string(),
                        );
                    }
                    _ => {
                        if let Some(ref ex) = extra {
                            q.push_back(serde_json::json!({ "@type": "ok", "@extra": ex }).to_string());
                        }
                    }
                }
            }
        }
    }

    /// Receives a JSON event or response from TDLib with a timeout in seconds.
    pub fn receive(&self, timeout: f64) -> Result<Option<String>, FfiError> {
        if let Some(ref sym) = self.symbols {
            if !self.handle.is_null() {
                unsafe {
                    let res_ptr = (sym.receive)(self.handle, timeout as c_double);
                    if res_ptr.is_null() {
                        return Ok(None);
                    }
                    let c_str = CStr::from_ptr(res_ptr);
                    let s = c_str.to_str()?.to_string();
                    return Ok(Some(s));
                }
            }
            Ok(None)
        } else if let Some(ref queue) = self.mock_queue {
            if let Some(resp) = queue.lock().pop_front() {
                return Ok(Some(resp));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(None)
        } else {
            Ok(None)
        }
    }
}

impl Drop for RawClient {
    fn drop(&mut self) {
        if let Some(ref sym) = self.symbols {
            if !self.handle.is_null() {
                unsafe {
                    (sym.destroy)(self.handle);
                }
                self.handle = std::ptr::null_mut();
            }
        }
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(env_path) = std::env::var_os("TDLIB_PATH") {
        paths.push(PathBuf::from(env_path));
    }
    paths.push(PathBuf::from("/usr/lib/libtdjson.so"));
    paths.push(PathBuf::from("/usr/local/lib/libtdjson.so"));
    paths.push(PathBuf::from("/usr/lib64/libtdjson.so"));
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/share/nyx/lib/libtdjson.so"));
    }
    paths
}
