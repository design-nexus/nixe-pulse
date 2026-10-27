//! D-Bus Control interface (`nyx_core::control`) for Nyx Assistant.
//!
//! Provides external access to list chats, search messages, open conversations,
//! send text, and initiate voice/video calls.

use nyx_core::control::text;
use serde_json::{Value, json};

pub fn tools() -> Value {
    json!([
        {
            "name": "pulse.chats.list",
            "description": "List recent Telegram conversations with their title, unread count, and chat ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Maximum number of chats to return (default 10)" }
                }
            }
        },
        {
            "name": "pulse.chats.search",
            "description": "Search loaded Telegram conversations by title or latest message preview.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search term" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "pulse.chats.open",
            "description": "Bring Pulse to the foreground and select a Telegram chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": { "type": "integer", "description": "Telegram chat ID" }
                },
                "required": ["chat_id"]
            }
        },
        {
            "name": "pulse.messages.send",
            "description": "Send a Telegram message to a specific conversation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": { "type": "integer", "description": "Telegram chat ID" },
                    "text": { "type": "string", "description": "Message content to send" }
                },
                "required": ["chat_id", "text"]
            }
        },
        {
            "name": "pulse.calls.start",
            "description": "Initiate a Telegram voice or video call to a contact.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": { "type": "integer", "description": "Telegram user or chat ID" },
                    "is_video": { "type": "boolean", "description": "Whether to initiate a video call" }
                },
                "required": ["chat_id"]
            }
        }
    ])
}

pub fn handle(name: &str, args: Value) -> Result<Value, String> {
    match name {
        "pulse.chats.list" => {
            let limit = args["limit"].as_i64().unwrap_or(10).clamp(1, 50) as usize;
            let chats = crate::window::list_chats(limit);
            let chat_json = chats
                .into_iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "title": c.title,
                        "unread_count": c.unread_count,
                        "is_pinned": c.is_pinned,
                        "is_secret": c.is_secret,
                        "type": format!("{:?}", c.chat_type)
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "chats": chat_json,
                "count": chat_json.len()
            }))
        }
        "pulse.chats.search" => {
            let q = text(&args, "query")?;
            let chats = crate::window::search_chats(&q);
            let results = chats
                .into_iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "title": c.title,
                        "unread_count": c.unread_count,
                        "is_secret": c.is_secret
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "query": q,
                "results": results
            }))
        }
        "pulse.chats.open" => {
            let chat_id = args["chat_id"].as_i64().ok_or("chat_id is required")?;
            crate::window::open_chat(chat_id);
            Ok(json!({
                "opened": true,
                "chat_id": chat_id
            }))
        }
        "pulse.messages.send" => {
            let chat_id = args["chat_id"].as_i64().ok_or("chat_id is required")?;
            let msg = text(&args, "text")?;
            let sent = crate::window::send_message_from_assistant(chat_id, &msg);
            Ok(json!({
                "sent": sent,
                "chat_id": chat_id,
                "text": msg
            }))
        }
        "pulse.calls.start" => {
            let chat_id = args["chat_id"].as_i64().ok_or("chat_id is required")?;
            let is_video = args["is_video"].as_bool().unwrap_or(false);
            let initiated = crate::window::start_call_from_assistant(chat_id, is_video);
            Ok(json!({
                "call_initiated": false,
                "queued": initiated,
                "chat_id": chat_id,
                "is_video": is_video
            }))
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_schema_definitions() {
        let t = tools();
        let arr = t.as_array().expect("tools must be array");
        assert_eq!(arr.len(), 5);

        let names: Vec<&str> = arr.iter().filter_map(|x| x["name"].as_str()).collect();
        assert!(names.contains(&"pulse.chats.list"));
        assert!(names.contains(&"pulse.chats.search"));
        assert!(names.contains(&"pulse.chats.open"));
        assert!(names.contains(&"pulse.messages.send"));
        assert!(names.contains(&"pulse.calls.start"));
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let res = handle("pulse.unknown.tool", json!({}));
        assert!(res.is_err());
    }
}

/// Return acceptance and the actual TDLib message ID, never a fabricated delivery receipt.
pub fn handle_async(name: &str, args: Value, reply: nyx_core::control::Reply) {
    if name != "pulse.messages.send" {
        reply(handle(name, args));
        return;
    }
    let Some(chat) = args["chat_id"].as_i64() else {
        reply(Err("chat_id is required".into()));
        return;
    };
    let Ok(text) = text(&args, "text") else {
        reply(Err("text is required".into()));
        return;
    };
    if text.trim().is_empty() {
        reply(Err("Message is empty".into()));
        return;
    }
    let Some(s) = crate::window::service() else {
        reply(Err("Telegram is unavailable".into()));
        return;
    };
    let rx=s.request("sendMessage",json!({"chat_id":chat,"input_message_content":{"@type":"inputMessageText","text":{"@type":"formattedText","text":text,"entities":[]},"clear_draft":false}}));
    gtk::glib::spawn_future_local(async move {
        reply(match rx.recv().await {
            Ok(Ok(message)) => Ok(
                json!({"accepted":true,"queued":message["sending_state"].is_object(),"sent":message["sending_state"].is_null(),"message_id":message["id"],"chat_id":chat}),
            ),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Service stopped".into()),
        });
    });
}
