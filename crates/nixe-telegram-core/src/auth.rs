//! Telegram authentication manager and state machine.

use crate::client::{ClientError, TdClient};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

/// Initial configuration parameters for starting a TDLib session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TdlibParameters {
    pub database_directory: PathBuf,
    pub files_directory: PathBuf,
    pub api_id: i32,
    pub api_hash: String,
    pub system_language_code: String,
    pub device_model: String,
    pub application_version: String,
    pub use_message_database: bool,
    pub use_secret_chats: bool,
    pub database_encryption_key: Option<String>,
}

impl Default for TdlibParameters {
    fn default() -> Self {
        Self {
            database_directory: PathBuf::from("tdlib_data"),
            files_directory: PathBuf::from("tdlib_files"),
            api_id: 611335,
            api_hash: "d524b414d21f4d37f08684c1df41ac9c".into(),
            system_language_code: "en".into(),
            device_model: "Nixe Desktop (Omarchy Linux)".into(),
            application_version: "0.1.0".into(),
            use_message_database: true,
            use_secret_chats: true,
            database_encryption_key: None,
        }
    }
}

/// Manages interactive authentication with TDLib.
pub struct AuthManager {
    client: TdClient,
}

impl AuthManager {
    pub fn new(client: TdClient) -> Self {
        Self { client }
    }

    /// Submits TDLib setup parameters when in `WaitTdlibParameters` state.
    pub async fn set_parameters(&self, params: TdlibParameters) -> Result<(), ClientError> {
        let db_dir = params.database_directory.to_string_lossy().to_string();
        let files_dir = params.files_directory.to_string_lossy().to_string();
        let enc_key = params.database_encryption_key.unwrap_or_default();

        let request = json!({
            "database_directory": db_dir,
            "files_directory": files_dir,
            "database_encryption_key": enc_key,
            "use_file_database": true,
            "use_chat_info_database": true,
            "use_message_database": params.use_message_database,
            "use_secret_chats": params.use_secret_chats,
            "api_id": params.api_id,
            "api_hash": params.api_hash,
            "system_language_code": params.system_language_code,
            "device_model": params.device_model,
            "system_version": "Linux",
            "application_version": params.application_version,
            "enable_storage_optimizer": true,
            "ignore_file_names": false
        });

        self.client.execute("setTdlibParameters", request).await?;
        Ok(())
    }

    /// Submits a phone number when in `WaitPhoneNumber` state.
    pub async fn set_phone_number(&self, phone_number: &str) -> Result<(), ClientError> {
        let clean_phone = phone_number.trim().replace([' ', '-', '(', ')'], "");
        let request = json!({
            "phone_number": clean_phone,
            "settings": {
                "@type": "phoneNumberAuthenticationSettings",
                "allow_sms": true,
                "allow_flash_calls": false,
                "allow_missed_calls": false
            }
        });
        self.client.execute("setAuthenticationPhoneNumber", request).await?;
        Ok(())
    }

    /// Requests a QR code for barcode authentication.
    pub async fn request_qr_code(&self) -> Result<String, ClientError> {
        let resp = self
            .client
            .execute(
                "requestQrCodeAuthentication",
                json!({
                    "other_user_ids": []
                }),
            )
            .await?;
        let link = resp.get("link").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        Ok(link)
    }

    /// Submits a verification code received via Telegram or SMS.
    pub async fn check_code(&self, code: &str) -> Result<(), ClientError> {
        let clean_code = code.trim().to_string();
        let request = json!({ "code": clean_code });
        self.client.execute("checkAuthenticationCode", request).await?;
        Ok(())
    }

    /// Submits the 2-step verification cloud password.
    pub async fn check_password(&self, password: &str) -> Result<(), ClientError> {
        let request = json!({ "password": password });
        self.client.execute("checkAuthenticationPassword", request).await?;
        Ok(())
    }

    /// Signs out the current account and revokes the active session on Telegram servers.
    pub async fn log_out(&self) -> Result<(), ClientError> {
        self.client.execute("logOut", json!({})).await?;
        Ok(())
    }
}
