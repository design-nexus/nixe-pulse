//! Account-local Telegram cache. Raw message data retains typed file roles on refresh.
use crate::client::{ClientError, TdClient, parse_message};
use crate::models::*;
use parking_lot::RwLock;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone)]
pub struct SyncEngine {
    client: TdClient,
    chats: Arc<RwLock<HashMap<ChatId, ChatSummary>>>,
    messages: Arc<RwLock<HashMap<(ChatId, MessageId), MessageSummary>>>,
    downloads: Arc<RwLock<HashSet<i32>>>,
    transfer_slots: Arc<tokio::sync::Semaphore>,
}
impl SyncEngine {
    pub fn new(client: TdClient) -> Self {
        Self {
            client,
            chats: Default::default(),
            messages: Default::default(),
            downloads: Default::default(),
            transfer_slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
    pub async fn load_chats(&self, limit: i32) -> Result<Vec<ChatSummary>, ClientError> {
        let mut chats = self.load_chat_list(ChatListId::Main, limit).await?;
        chats.extend(self.load_chat_list(ChatListId::Archive, limit).await?);
        chats.sort_by_key(|c| c.id);
        chats.dedup_by_key(|c| c.id);
        Ok(chats)
    }
    pub async fn load_chat_list(&self, list: ChatListId, limit: i32) -> Result<Vec<ChatSummary>, ClientError> {
        let req = json!({"chat_list":list.to_td(),"limit":limit});
        if let Err(e) = self.client.execute("loadChats", req.clone()).await {
            if !matches!(e, ClientError::Tdlib { code: 404, .. }) {
                return Err(e);
            }
        }
        let val = self.client.execute("getChats", req).await?;
        let mut chats = Vec::new();
        for id in val["chat_ids"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
            chats.push(self.get_chat(id).await?);
        }
        chats.sort_by(|a, b| b.order.cmp(&a.order));
        Ok(chats)
    }
    pub async fn get_chat(&self, id: ChatId) -> Result<ChatSummary, ClientError> {
        let mut v = self.client.execute("getChat", json!({"chat_id":id})).await?;
        if let Some(group) = v["type"]["supergroup_id"].as_i64() {
            if let Ok(g) = self.client.execute("getSupergroup", json!({"supergroup_id":group})).await {
                v["pulse_is_forum"] = g["is_forum"].clone();
                let status = &g["status"];
                let can_send = match status["@type"].as_str() {
                    Some("chatMemberStatusCreator") => true,
                    Some("chatMemberStatusAdministrator") => {
                        if v["type"]["is_channel"] == true {
                            status["rights"]["can_post_messages"] == true
                        } else {
                            true
                        }
                    }
                    Some("chatMemberStatusRestricted") => status["permissions"]["can_send_basic_messages"] == true,
                    Some("chatMemberStatusLeft" | "chatMemberStatusBanned") => false,
                    _ => v["permissions"]["can_send_basic_messages"] == true,
                };
                v["pulse_can_send"] = json!(can_send);
            }
        }

        let sender = v["message_sender_id"].clone();
        if let Some(user) = sender["user_id"].as_i64() {
            if let Ok(user)=self.client.execute("getUser",json!({"user_id":user})).await {
                v["pulse_sender_name"]=json!(format!("{} {}",user["first_name"].as_str().unwrap_or(""),user["last_name"].as_str().unwrap_or("")).trim());
            }
        } else if let Some(chat)=sender["chat_id"].as_i64() {
            if let Ok(chat)=self.client.execute("getChat",json!({"chat_id":chat})).await {v["pulse_sender_name"]=chat["title"].clone();}
        }
        if let Some(secret) = v["type"]["secret_chat_id"].as_i64() {
            if let Ok(data) = self.client.execute("getSecretChat", json!({"secret_chat_id":secret})).await {
                v["pulse_secret_state"] = data["state"].clone();
            }
        }
        if let Some(user) = v["type"]["user_id"].as_i64() {
            if let Ok(data) = self.client.execute("getUser", json!({"user_id":user})).await {
                v["pulse_user_online"] = json!(data["status"]["@type"] == "userStatusOnline");
            }
        }
        if v["notification_settings"]["use_default_mute_for"] == true {
            let scope = match v["type"]["@type"].as_str() {
                Some("chatTypePrivate" | "chatTypeSecret") => "notificationSettingsScopePrivateChats",
                Some("chatTypeSupergroup") if v["type"]["is_channel"] == true => "notificationSettingsScopeChannelChats",
                _ => "notificationSettingsScopeGroupChats",
            };
            if let Ok(settings) = self.client.execute("getScopeNotificationSettings", json!({"scope":{"@type":scope}})).await {
                v["pulse_effective_mute_for"] = settings["mute_for"].clone();
            }
        }
        let position = v["positions"].as_array().into_iter().flatten().find(|p| p["list"]["@type"] == "chatListMain");
        let order =
            position.and_then(|p| p["order"].as_str().and_then(|s| s.parse().ok()).or_else(|| p["order"].as_u64())).unwrap_or(0);
        let mut avatar = v["photo"]["small"]["local"]["path"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        if !v["photo"]["small"]["local"]["is_downloading_completed"].as_bool().unwrap_or(false) {
            avatar = None;
            if let Some(fid) = v["photo"]["small"]["id"].as_i64() {
                self.download(fid as i32).await;
            }
        }
        let kind = match v["type"]["@type"].as_str() {
            Some("chatTypePrivate" | "chatTypeSecret") => ChatType::Private,
            Some("chatTypeBasicGroup") => ChatType::BasicGroup,
            Some("chatTypeSupergroup") if !v["type"]["is_channel"].as_bool().unwrap_or(false) => ChatType::Supergroup,
            _ => ChatType::Channel,
        };
        let chat = ChatSummary {
            id,
            title: v["title"].as_str().unwrap_or("Conversation").into(),
            chat_type: kind,
            unread_count: v["unread_count"].as_i64().unwrap_or(0) as i32,
            is_pinned: position.and_then(|p| p["is_pinned"].as_bool()).unwrap_or(false),
            is_muted: v["pulse_effective_mute_for"]
                .as_i64()
                .or_else(|| v["notification_settings"]["mute_for"].as_i64())
                .unwrap_or(0)
                > 0,
            last_message: v.get("last_message").and_then(parse_message),
            avatar_path: avatar,
            order,
            is_secret: v["type"]["@type"] == "chatTypeSecret",
            data: v,
        };
        self.chats.write().insert(id, chat.clone());
        Ok(chat)
    }
    pub async fn download(&self, id: i32) {
        if !self.downloads.write().insert(id) {
            return;
        }
        let client = self.client.clone();
        let downloads = self.downloads.clone();
        let slots = self.transfer_slots.clone();
        tokio::spawn(async move {
            let Ok(_permit) = slots.acquire_owned().await else { return };
            let _ =
                client.execute("downloadFile", json!({"file_id":id,"priority":16,"offset":0,"limit":0,"synchronous":true})).await;
            downloads.write().remove(&id);
        });
    }
    pub async fn register(&self, msg: &MessageSummary) {
        self.messages.write().insert((msg.chat_id, msg.id), msg.clone());
        for f in &msg.files {
            if f.path.is_none()
                && (f.role == "thumbnail"
                    || f.role == "link-preview"
                    || (f.size <= 5 * 1024 * 1024
                        && matches!(msg.content, MessageContent::Photo { .. } | MessageContent::Sticker { .. })))
            {
                self.download(f.id).await;
            }
        }
    }
    pub async fn ingest(&self, update: TelegramUpdate) -> Vec<TelegramUpdate> {
        match update {
            TelegramUpdate::NewMessage(mut msg) => {
                self.resolve_sender(&mut msg).await;
                self.register(&msg).await;
                vec![TelegramUpdate::NewMessage(msg)]
            }
            TelegramUpdate::Raw(v) => {
                let typ = v["@type"].as_str().unwrap_or("");
                match typ {
                    "updateFile" => {
                        let file = &v["file"];
                        let id = file["id"].as_i64().unwrap_or(0) as i32;
                        if !file["local"]["is_downloading_active"].as_bool().unwrap_or(false) {
                            self.downloads.write().remove(&id);
                        }
                        let mut events = Vec::new();
                        let mut store = self.messages.write();
                        for msg in store.values_mut().filter(|m| m.files.iter().any(|f| f.id == id)) {
                            replace_file(&mut msg.data, id, file);
                            if let Some(mut new) = parse_message(&msg.data) {
                                new.sender_name = msg.sender_name.clone();
                                *msg = new.clone();
                                events.push(TelegramUpdate::MessageReplaced { old_id: msg.id, message: new });
                            }
                        }
                        drop(store);
                        for chat in self.chats.write().values_mut() {
                            if chat.data["photo"]["small"]["id"].as_i64() == Some(id as i64)
                                && file["local"]["is_downloading_completed"] == true
                            {
                                let path = file["local"]["path"].as_str().unwrap_or("").to_owned();
                                chat.avatar_path = Some(path.clone());
                                events.push(TelegramUpdate::ChatPhotoUpdated { chat_id: chat.id, path });
                            }
                        }
                        events
                    }
                    "updateMessageSendSucceeded" | "updateMessageSendFailed" => {
                        let Some(mut message) = parse_message(&v["message"]) else { return vec![] };
                        if typ == "updateMessageSendFailed" {
                            message.sending_state =
                                Some(format!("Failed: {}", v["error"]["message"].as_str().unwrap_or("Could not send; retry")));
                        }
                        let old_id = v["old_message_id"].as_i64().unwrap_or(message.id);
                        self.messages.write().remove(&(message.chat_id, old_id));
                        self.resolve_sender(&mut message).await;
                        self.register(&message).await;
                        vec![TelegramUpdate::MessageReplaced { old_id, message }]
                    }
                    "updateSecretChat" => {
                        let id = v["secret_chat"]["id"].as_i64().unwrap_or(0);
                        let chats = self
                            .chats
                            .read()
                            .values()
                            .filter(|c| c.data["type"]["secret_chat_id"].as_i64() == Some(id))
                            .map(|c| c.id)
                            .collect::<Vec<_>>();
                        let mut events = Vec::new();
                        for chat in chats {
                            if let Ok(c) = self.get_chat(chat).await {
                                events.push(TelegramUpdate::ChatUpdated(c));
                            }
                        }
                        events
                    }
                    "updateMessageInteractionInfo" => {
                        let key = (v["chat_id"].as_i64().unwrap_or(0), v["message_id"].as_i64().unwrap_or(0));
                        if let Some(message) = self.messages.write().get_mut(&key) {
                            message.data["interaction_info"] = v["interaction_info"].clone();
                            vec![TelegramUpdate::MessageReplaced { old_id: key.1, message: message.clone() }]
                        } else {
                            vec![]
                        }
                    }
                    "updateMessageEdited" | "updateMessageIsPinned" => {
                        let chat = v["chat_id"].as_i64().unwrap_or(0);
                        let id = v["message_id"].as_i64().unwrap_or(0);
                        if let Ok(data) = self.client.execute("getMessage", json!({"chat_id":chat,"message_id":id})).await {
                            if let Some(mut message) = parse_message(&data) {
                                self.resolve_sender(&mut message).await;
                                self.register(&message).await;
                                vec![TelegramUpdate::MessageReplaced { old_id: id, message }]
                            } else {
                                vec![]
                            }
                        } else {
                            vec![]
                        }
                    }
                    "updateNewChat" => {
                        let id = v["chat"]["id"].as_i64().unwrap_or(0);
                        self.get_chat(id).await.ok().map(TelegramUpdate::ChatUpdated).into_iter().collect()
                    }
                    "updateChatDraftMessage" if v["topic_id"].is_object() => vec![TelegramUpdate::Raw(v)],
                    "updateChatPosition"
                    | "updateChatLastMessage"
                    | "updateChatTitle"
                    | "updateChatNotificationSettings"
                    | "updateChatMessageSender"
                    | "updateChatPermissions"
                    | "updateChatIsMarkedAsUnread"
                    | "updateChatBlockList"
                    | "updateChatAvailableReactions"
                    | "updateChatDraftMessage"
                    | "updateChatPhoto"
                    | "updateChatReadOutbox" => self
                        .get_chat(v["chat_id"].as_i64().unwrap_or(0))
                        .await
                        .ok()
                        .map(TelegramUpdate::ChatUpdated)
                        .into_iter()
                        .collect(),
                    _ => vec![TelegramUpdate::Raw(v)],
                }
            }
            TelegramUpdate::MessageEdited { chat_id, message_id, .. } => {
                match self.client.execute("getMessage", json!({"chat_id":chat_id,"message_id":message_id})).await {
                    Ok(v) => {
                        if let Some(mut message) = parse_message(&v) {
                            self.resolve_sender(&mut message).await;
                            self.register(&message).await;
                            vec![TelegramUpdate::MessageReplaced { old_id: message_id, message }]
                        } else {
                            vec![]
                        }
                    }
                    Err(_) => vec![],
                }
            }
            TelegramUpdate::MessagesDeleted { chat_id, message_ids } => {
                for id in &message_ids {
                    self.messages.write().remove(&(chat_id, *id));
                }
                vec![TelegramUpdate::MessagesDeleted { chat_id, message_ids }]
            }
            other => vec![other],
        }
    }
    async fn resolve_sender(&self, msg: &mut MessageSummary) {
        if msg.sender_name.is_empty() && msg.sender_id != 0 {
            if let Ok(u) = self.client.execute("getUser", json!({"user_id":msg.sender_id})).await {
                msg.sender_name = format!("{} {}", u["first_name"].as_str().unwrap_or(""), u["last_name"].as_str().unwrap_or(""))
                    .trim()
                    .to_owned();
            }
        }
    }
    pub async fn get_chat_history(
        &self,
        chat_id: ChatId,
        from_message_id: MessageId,
        limit: i32,
    ) -> Result<Vec<MessageSummary>, ClientError> {
        self.get_topic_history(chat_id, Value::Null, from_message_id, limit).await
    }
    pub async fn get_topic_history(&self, chat_id: ChatId, topic: Value, from_message_id: MessageId, limit: i32) -> Result<Vec<MessageSummary>, ClientError> {
        let method = match topic["@type"].as_str() {
            Some("messageTopicForum") => "getForumTopicHistory",
            Some("messageTopicThread") => "getMessageThreadHistory",
            _ => "getChatHistory",
        };
        let v = self.client.execute(method, json!({"chat_id":chat_id,"forum_topic_id":topic["forum_topic_id"].as_i64().unwrap_or(0),"message_id":topic["message_thread_id"].as_i64().unwrap_or(0),"from_message_id":from_message_id,"offset":0,"limit":limit,"only_local":false})).await?;
        let mut messages = Vec::new();
        for v in v["messages"].as_array().into_iter().flatten() {
            if let Some(mut msg) = parse_message(v) {
                self.resolve_sender(&mut msg).await;
                self.register(&msg).await;
                messages.push(msg);
            }
        }
        messages.reverse();
        let store = self.messages.read();
        Ok(messages.into_iter().map(|m| store.get(&(m.chat_id, m.id)).cloned().unwrap_or(m)).collect())
    }
    pub async fn send_text_message(
        &self,
        chat_id: ChatId,
        text: &str,
        reply: Option<MessageId>,
    ) -> Result<MessageSummary, ClientError> {
        self.send_content(
            chat_id,
            json!({"@type":"inputMessageText","text":{"@type":"formattedText","text":text,"entities":[]},"clear_draft":true}),
            reply,
        )
        .await
    }
    pub async fn send_content(
        &self,
        chat_id: ChatId,
        content: Value,
        reply: Option<MessageId>,
    ) -> Result<MessageSummary, ClientError> {
        let mut req = json!({"chat_id":chat_id,"input_message_content":content});
        if let Some(id) = reply {
            req["reply_to"] = json!({"@type":"inputMessageReplyToMessage","message_id":id});
        }
        let val = self.client.execute("sendMessage", req).await?;
        let msg = parse_message(&val).ok_or(ClientError::Closed)?;
        self.register(&msg).await;
        Ok(msg)
    }
    pub async fn send_typing(&self, chat_id: ChatId) -> Result<(), ClientError> {
        self.client.execute("sendChatAction", json!({"chat_id":chat_id,"action":{"@type":"chatActionTyping"}})).await?;
        Ok(())
    }
    pub async fn view_messages(&self, chat_id: ChatId, ids: &[MessageId]) -> Result<(), ClientError> { self.view_topic_messages(chat_id,ids,&Value::Null).await }
    pub async fn view_topic_messages(&self, chat_id: ChatId, ids: &[MessageId], topic: &Value) -> Result<(), ClientError> {
        if ids.is_empty() {
            return Ok(());
        }
        let source=match topic["@type"].as_str(){Some("messageTopicForum")=>"messageSourceForumTopicHistory",Some("messageTopicThread")=>"messageSourceMessageThreadHistory",_=>"messageSourceChatHistory"};
        self.client.execute("viewMessages", json!({"chat_id":chat_id,"message_ids":ids,"source":{"@type":source},"force_read":true})).await?;
        Ok(())
    }
}
fn replace_file(v: &mut Value, id: i32, file: &Value) {
    if v["id"].as_i64() == Some(id as i64) && v.get("local").is_some() {
        *v = file.clone();
        return;
    }
    match v {
        Value::Object(m) => {
            for value in m.values_mut() {
                replace_file(value, id, file)
            }
        }
        Value::Array(a) => {
            for value in a {
                replace_file(value, id, file)
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    fn message(id: i64) -> MessageSummary {
        parse_message(&json!({"id":id,"chat_id":7,"date":1,"content":{"@type":"messagePhoto","photo":{"sizes":[{"width":40,"height":30,"photo":{"id":11,"size":100,"local":{"path":"","is_downloading_completed":false}}},{"width":800,"height":600,"photo":{"id":12,"size":1000,"local":{"path":"","is_downloading_completed":false}}}]},"caption":{"text":"caption","entities":[]}}})).unwrap()
    }
    #[tokio::test]
    async fn thumbnail_updates_fan_out_without_replacing_originals_or_resurrecting_deletes() {
        let c = TdClient::new(&crate::TdJsonLib::mock()).unwrap();
        let sync = SyncEngine::new(c.clone());
        sync.messages.write().insert((7, 1), message(1));
        sync.messages.write().insert((7, 2), message(2));
        let events=sync.ingest(TelegramUpdate::Raw(json!({"@type":"updateFile","file":{"id":11,"size":100,"local":{"path":"/tmp/preview.webp","is_downloading_completed":true,"is_downloading_active":false}}}))).await;
        assert_eq!(events.len(), 2);
        for msg in sync.messages.read().values() {
            assert_eq!(
                msg.files.iter().find(|f| f.role == "thumbnail" || f.role == "link-preview").unwrap().path.as_deref(),
                Some("/tmp/preview.webp")
            );
            assert!(msg.files.iter().find(|f| f.role == "original").unwrap().path.is_none());
        }
        sync.ingest(TelegramUpdate::MessagesDeleted { chat_id: 7, message_ids: vec![1, 2] }).await;
        assert!(sync.ingest(TelegramUpdate::Raw(json!({"@type":"updateFile","file":{"id":11,"local":{"path":"/tmp/preview.webp","is_downloading_completed":true}}}))).await.is_empty());
        c.close();
    }
    #[test]
    fn telegram_entities_preserve_utf16_offsets_and_plain_text() {
        let text = crate::client::parse_formatted_text(
            &json!({"text":"😀 bold & **literal**","entities":[{"offset":3,"length":4,"type":{"@type":"textEntityTypeBold"}}]}),
        );
        assert_eq!(text.to_pango_markup(), "😀 <b>bold</b> &amp; **literal**");
        let overlap = FormattedText {
            text: "abcd".into(),
            entities: vec![
                TextEntity { offset: 0, length: 3, kind: TextEntityKind::Bold },
                TextEntity { offset: 1, length: 3, kind: TextEntityKind::Italic },
            ],
        };
        assert_eq!(overlap.to_pango_markup(), "<b>a</b><b><i>bc</i></b><i>d</i>");
    }
}
