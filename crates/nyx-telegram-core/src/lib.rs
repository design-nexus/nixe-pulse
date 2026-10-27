//! `nyx-telegram-core`: Telegram MTProto protocol handling, TDLib FFI,
//! multi-account management, authentication, and chat synchronization for Nyx Pulse.

pub mod account;
pub mod auth;
pub mod client;
pub mod ffi;
pub mod models;
pub mod sync;

pub use account::{AccountInfo, AccountManager};
pub use auth::{AuthManager, TdlibParameters};
pub use client::{ClientError, TdClient};
pub use ffi::{FfiError, TdJsonLib};
pub use models::*;
pub use sync::SyncEngine;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatted_text_escapes_markup() {
        let text = FormattedText::plain("Hello <world> & \"friends\"");
        assert_eq!(text.to_pango_markup(), "Hello &lt;world&gt; &amp; &quot;friends&quot;");
    }

    #[test]
    fn formatted_text_markdown_parsing() {
        let text = FormattedText::plain("This is **bold**, *italic*, and `code`!");
        assert_eq!(text.to_pango_markup(), "This is **bold**, *italic*, and `code`!");
    }

    #[test]
    fn formatted_text_entities_parsing() {
        let text = FormattedText {
            text: "Hello Rustaceans".to_string(),
            entities: vec![TextEntity { offset: 6, length: 10, kind: TextEntityKind::Bold }],
        };
        assert_eq!(text.to_pango_markup(), "Hello <b>Rustaceans</b>");
    }

    #[test]
    fn user_profile_helpers() {
        let profile = UserProfile {
            id: 12345,
            first_name: "John".into(),
            last_name: "Doe".into(),
            username: Some("johndoe".into()),
            phone_number: Some("+1234567890".into()),
            is_online: true,
            avatar_path: None,
        };

        assert_eq!(profile.full_name(), "John Doe");
        assert_eq!(profile.initials(), "JD");
    }

    #[tokio::test]
    async fn mock_client_flow() {
        let lib = TdJsonLib::mock();
        let client = TdClient::new(&lib).expect("Failed to create TdClient");

        // Test sending request to mock client
        let res = client.execute("getMe", serde_json::json!({})).await.expect("Mock request should succeed");

        assert_eq!(res.get("@type").and_then(serde_json::Value::as_str), Some("ok"));
        client.close();
    }

    #[test]
    fn tdlib_library_loads() {
        let lib = TdJsonLib::load().expect("Failed to initialize TdJsonLib");
        let client = lib.create_client().expect("Failed to create raw client");
        drop(client);
    }
}
