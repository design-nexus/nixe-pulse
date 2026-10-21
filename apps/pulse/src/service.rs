//! One isolated TDLib client and update stream per account.
use async_channel::{Receiver, Sender};
use nixe_telegram_core::{
    TdJsonLib,
    account::AccountManager,
    auth::{AuthManager, TdlibParameters},
    client::TdClient,
    models::*,
    sync::SyncEngine,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::runtime::Runtime;

#[derive(Clone, Debug)]
pub enum ServiceEvent {
    Account(String, String, Box<ServiceEvent>),
    Update(TelegramUpdate),
    ChatsLoaded(Vec<ChatSummary>),
    DraftSynced { chat_id: i64, topic: Value, draft: Value },
    SecretChatCreated(ChatSummary),
    ContextReady { chat: ChatSummary, topic: Value, title: String, draft: Value, navigation: u64, jump: Option<i64> },
    TopicHistoryLoaded { chat_id: i64, topic: Value, revision: u64, from: i64, result: Result<Vec<MessageSummary>, String> },
    AuthError(String),
    Error(String),
}
#[derive(Clone)]
pub struct TelegramService {
    runtime: Arc<Runtime>,
    client: TdClient,
    sync: Arc<SyncEngine>,
    tx: Sender<ServiceEvent>,
    draft_versions: Arc<std::sync::Mutex<std::collections::HashMap<(i64, String), u64>>>,
    pub account: String,
    pub generation: String,
}
impl TelegramService {
    pub fn start(tx: Sender<ServiceEvent>) -> Result<Self, nixe_telegram_core::client::ClientError> {
        Self::start_account(tx, crate::prefs::get().active_account_id)
    }
    pub fn start_account(out: Sender<ServiceEvent>, account: String) -> Result<Self, nixe_telegram_core::client::ClientError> {
        let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?);
        let client = TdClient::new(&TdJsonLib::load()?)?;
        let auth = Arc::new(AuthManager::new(client.clone()));
        let sync = Arc::new(SyncEngine::new(client.clone()));
        let (tx, rx) = async_channel::unbounded();
        let generation = uuid::Uuid::new_v4().to_string();
        let service = Self {
            runtime: runtime.clone(),
            client: client.clone(),
            sync: sync.clone(),
            tx: tx.clone(),
            draft_versions: Default::default(),
            account: account.clone(),
            generation: generation.clone(),
        };
        let tag = account.clone();
        runtime.spawn(async move {
            while let Ok(event) = rx.recv().await {
                if out.send(ServiceEvent::Account(tag.clone(), generation.clone(), Box::new(event))).await.is_err() {
                    break;
                }
            }
        });
        runtime.spawn(async move {
            let rx = client.subscribe();
            while let Ok(update) = rx.recv().await {
                if let TelegramUpdate::AuthStateChanged(state) = &update {
                    if matches!(state, AuthState::WaitTdlibParameters) {
                        let mgr = AccountManager::new(nixe_core::paths::data_dir(), nixe_core::paths::cache_dir());
                        let prefs = crate::prefs::get();
                        let database = mgr.account_data_dir(&account);
                        let key = match crate::secrets::key(&account, !database.join("td.binlog").exists()).await {
                            Ok(key) => key,
                            Err(e) => {
                                let _ = tx.send(ServiceEvent::AuthError(e)).await;
                                continue;
                            }
                        };
                        let params = TdlibParameters {
                            database_encryption_key: key,
                            database_directory: database,
                            files_directory: mgr.account_files_dir(&account),
                            api_id: prefs.api_id.unwrap_or(611335),
                            api_hash: prefs.api_hash.unwrap_or_else(|| "d524b414d21f4d37f08684c1df41ac9c".into()),
                            ..Default::default()
                        };
                        if let Err(e) = auth.set_parameters(params.clone()).await {
                            if crate::secrets::pending(&account) {
                                let mut fallback = params;
                                fallback.database_encryption_key = None;
                                if let Err(e) = auth.set_parameters(fallback).await {
                                    let _ = tx.send(ServiceEvent::AuthError(e.to_string())).await;
                                }
                            } else {
                                let _ = tx.send(ServiceEvent::AuthError(e.to_string())).await;
                            }
                        }
                    }
                    if matches!(state, AuthState::Ready) {
                        if let Err(e) = crate::secrets::encrypt(&client, &account).await {
                            let _ = tx.send(ServiceEvent::Error(format!("Local database encryption: {e}"))).await;
                        }
                        for list in [ChatListId::Main, ChatListId::Archive] {
                            let sync = sync.clone();
                            let tx = tx.clone();
                            tokio::spawn(async move {
                                let event = match sync.load_chat_list(list, 50).await {
                                    Ok(chats) => ServiceEvent::ChatsLoaded(chats),
                                    Err(e) => ServiceEvent::Error(e.to_string()),
                                };
                                let _ = tx.send(event).await;
                            });
                        }
                    }
                }
                for mapped in sync.ingest(update).await {
                    let _ = tx.send(ServiceEvent::Update(mapped)).await;
                }
            }
        });
        Ok(service)
    }
    pub fn cancel_topic_draft(&self, chat: i64, topic: &Value) {
        let mut versions = self.draft_versions.lock().unwrap();
        let version = versions.entry((chat, topic.to_string())).or_default();
        *version = version.wrapping_add(1);
    }
    pub fn set_topic_draft(&self, chat: i64, topic: Value, draft: Value) {
        let key = (chat, topic.to_string());
        let version = {
            let mut versions = self.draft_versions.lock().unwrap();
            let version = versions.entry(key.clone()).or_default();
            *version = version.wrapping_add(1);
            *version
        };
        let versions = self.draft_versions.clone();
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            if versions.lock().unwrap().get(&key) != Some(&version) {
                return;
            }
            match client.execute("setChatDraftMessage", json!({"chat_id":chat,"topic_id":topic,"draft_message":draft})).await {
                Ok(_) => {
                    let latest = versions.lock().unwrap().get(&key) == Some(&version);
                    if latest {
                        let _ = tx.send(ServiceEvent::DraftSynced { chat_id: chat, topic, draft }).await;
                    }
                }
                Err(e) => {
                    let latest = versions.lock().unwrap().get(&key) == Some(&version);
                    if latest {
                        let _ = tx.send(ServiceEvent::Error(format!("Draft kept locally; synchronization failed: {e}"))).await;
                    }
                }
            }
        });
    }
    pub fn delete_chat(&self, id: ChatId, leave: bool) -> Receiver<Result<Value, String>> {
        let (tx, rx) = async_channel::bounded(1);
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let result = delete_conversation(id, leave, move |method, params| {
                let client = client.clone();
                async move { client.execute(method, params).await.map_err(|e| e.to_string()) }
            })
            .await;
            let _ = tx.send(result).await;
        });
        rx
    }

    pub fn request(&self, method: &str, params: Value) -> Receiver<Result<Value, String>> {
        let (tx, rx) = async_channel::bounded(1);
        let c = self.client.clone();
        let method = method.to_owned();
        self.runtime.spawn(async move {
            let result = c.execute(&method, params).await.map_err(|e| e.to_string());
            let _ = tx.send(result).await;
        });
        rx
    }
    fn auth_request(&self, method: &str, params: Value) {
        let rx = self.request(method, params);
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            if let Ok(Err(e)) = rx.recv().await {
                let _ = tx.send(ServiceEvent::AuthError(e)).await;
            }
        });
    }
    pub fn submit_phone(&self, s: String) {
        self.auth_request("setAuthenticationPhoneNumber", json!({"phone_number":s.trim()}));
    }
    pub fn submit_code(&self, s: String) {
        self.auth_request("checkAuthenticationCode", json!({"code":s.trim()}));
    }
    pub fn submit_password(&self, s: String) {
        self.auth_request("checkAuthenticationPassword", json!({"password":s}));
    }
    pub fn request_qr(&self) {
        self.auth_request("requestQrCodeAuthentication", json!({"other_user_ids":[]}));
    }
    pub fn log_out(&self) {
        self.auth_request("logOut", json!({}));
    }
    pub fn close(&self) {
        self.draft_versions.lock().unwrap().clear();
        self.client.close();
    }
    pub fn load_chats(&self, limit: i32) {
        let sync = self.sync.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let e = match sync.load_chats(limit).await {
                Ok(c) => ServiceEvent::ChatsLoaded(c),
                Err(e) => ServiceEvent::Error(e.to_string()),
            };
            let _ = tx.send(e).await;
        });
    }
    pub fn load_chat_list(&self, list: ChatListId, limit: i32) {
        let sync = self.sync.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let event = match sync.load_chat_list(list, limit).await {
                Ok(chats) => ServiceEvent::ChatsLoaded(chats),
                Err(e) => ServiceEvent::Error(e.to_string()),
            };
            let _ = tx.send(event).await;
        });
    }
    pub fn load_chat(&self, id: i64) {
        let sync = self.sync.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            match sync.get_chat(id).await {
                Ok(chat) => {
                    let _ = tx.send(ServiceEvent::Update(TelegramUpdate::ChatUpdated(chat))).await;
                }
                Err(e) => {
                    let _ = tx.send(ServiceEvent::Error(e.to_string())).await;
                }
            }
        });
    }
    pub fn resolve_context(&self, chat:i64, topic:Value, title:String, draft:Value, navigation:u64, jump:Option<i64>) {
        let sync=self.sync.clone();let tx=self.tx.clone();
        self.runtime.spawn(async move {
            let event=match sync.get_chat(chat).await {Ok(chat)=>ServiceEvent::ContextReady{chat,topic,title,draft,navigation,jump},Err(e)=>ServiceEvent::Error(e.to_string())};
            let _=tx.send(event).await;
        });
    }
    pub fn history_request(&self,chat:i64,topic:Value,from:i64)->async_channel::Receiver<Result<Vec<MessageSummary>,String>> {
        let (tx,rx)=async_channel::bounded(1);let sync=self.sync.clone();
        self.runtime.spawn(async move {let result=sync.get_topic_history(chat,topic,from,50).await.map_err(|e|e.to_string());let _=tx.send(result).await;});rx
    }
    pub fn load_topic_history(&self, chat: i64, topic: Value, revision: u64, from: i64) {
        let sync = self.sync.clone(); let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let result = sync.get_topic_history(chat, topic.clone(), from, 50).await.map_err(|e| e.to_string());
            let _ = tx.send(ServiceEvent::TopicHistoryLoaded { chat_id:chat, topic, revision, from, result }).await;
        });
    }
    pub fn send_text_message(&self, id: ChatId, text: String, reply: Option<MessageId>) {
        self.send_content(
            id,
            json!({"@type":"inputMessageText","text":{"@type":"formattedText","text":text,"entities":[]},"clear_draft":true}),
            reply,
        );
    }
    pub fn send_content(&self, id: ChatId, content: Value, reply: Option<MessageId>) {
        let sync = self.sync.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let e = match sync.send_content(id, content, reply).await {
                Ok(m) => ServiceEvent::Update(TelegramUpdate::NewMessage(m)),
                Err(e) => ServiceEvent::Error(format!("Message not sent: {e}")),
            };
            let _ = tx.send(e).await;
        });
    }
    pub fn download(&self, id: i32) {
        let s = self.sync.clone();
        self.runtime.spawn(async move {
            s.download(id).await;
        });
    }
    pub fn mark_chat_read(&self, id: ChatId, topic: Value, ids: Vec<MessageId>) {
        let s = self.sync.clone();
        self.runtime.spawn(async move {
            let _ = s.view_topic_messages(id, &ids, &topic).await;
        });
    }
    pub fn create_secret_chat(&self, user: i64) {
        let s = self.sync.clone();
        let c = self.client.clone();
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            match c.execute("createNewSecretChat", json!({"user_id":user})).await {
                Ok(v) => {
                    if let Some(id) = v["id"].as_i64() {
                        if let Ok(chat) = s.get_chat(id).await {
                            let _ = tx.send(ServiceEvent::SecretChatCreated(chat)).await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(ServiceEvent::Error(e.to_string())).await;
                }
            }
        });
    }
    pub fn start_call(&self, user: i64, video: bool) -> bool {
        crate::calls::start(self.clone(), user, video)
    }
    pub fn accept_call(&self, id: i32) {
        crate::calls::accept(self.clone(), id);
    }
    pub fn discard_call(&self, id: i32, duration: i32, video: bool) {
        self.request(
            "discardCall",
            json!({"call_id":id,"duration":duration,"is_video":video,"is_disconnected":false,"connection_id":0}),
        );
    }
}

// Keep destructive operations in one sequence shared by mouse and keyboard.
async fn delete_conversation<F, Fut>(id: ChatId, leave: bool, mut request: F) -> Result<Value, String>
where
    F: FnMut(&'static str, Value) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let chat = request("getChat", json!({"chat_id":id})).await?;
    let is_group = matches!(chat["type"]["@type"].as_str(), Some("chatTypeBasicGroup" | "chatTypeSupergroup"));
    if leave != is_group {
        return Err("The chat type changed. Please try again.".into());
    }
    if leave {
        request("leaveChat", json!({"chat_id":id})).await?;
    } else if chat["can_be_deleted_only_for_self"] != true {
        return Err("Telegram does not allow deleting this chat only for you.".into());
    }
    request("deleteChatHistory", json!({"chat_id":id,"remove_from_chat_list":true,"revoke":false}))
        .await
        .map_err(|e| if leave { format!("You left the chat, but removing its history failed: {e}") } else { e })
}

#[cfg(test)]
mod deletion_regression {
    use super::*;
    #[tokio::test]
    async fn deletion_is_local_and_stops_on_failure() {
        for (kind, allowed, leave, fail, expected) in [
            ("chatTypePrivate", true, false, "", vec!["getChat", "deleteChatHistory"]),
            ("chatTypeSecret", true, false, "", vec!["getChat", "deleteChatHistory"]),
            ("chatTypePrivate", false, false, "", vec!["getChat"]),
            ("chatTypePrivate", true, true, "", vec!["getChat"]),
            ("chatTypeSupergroup", false, true, "", vec!["getChat", "leaveChat", "deleteChatHistory"]),
            ("chatTypeBasicGroup", false, true, "leaveChat", vec!["getChat", "leaveChat"]),
            ("chatTypeSupergroup", false, true, "deleteChatHistory", vec!["getChat", "leaveChat", "deleteChatHistory"]),
        ] {
            let calls = std::cell::RefCell::new(Vec::new());
            let result = delete_conversation(7, leave, |method, params| {
                if method == "deleteChatHistory" {
                    assert_eq!(params["revoke"], false);
                    assert_eq!(params["remove_from_chat_list"], true);
                }
                calls.borrow_mut().push(method);
                std::future::ready(if method == fail {
                    Err("fixture failure".into())
                } else if method == "getChat" {
                    Ok(json!({"type":{"@type":kind},"can_be_deleted_only_for_self":allowed}))
                } else {
                    Ok(json!({"@type":"ok"}))
                })
            })
            .await;
            assert_eq!(*calls.borrow(), expected);
            let should_succeed =
                fail.is_empty() && (leave == matches!(kind, "chatTypeBasicGroup" | "chatTypeSupergroup")) && (leave || allowed);
            assert_eq!(result.is_ok(), should_succeed);
            if fail == "deleteChatHistory" {
                assert!(result.unwrap_err().contains("You left the chat"));
            }
        }
    }
}
