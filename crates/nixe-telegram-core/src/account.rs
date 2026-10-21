//! Multi-account manager and directory isolation for Nixe Pulse.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Metadata for a configured Telegram account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: String,
    pub display_name: String,
    pub phone_number: String,
    pub username: Option<String>,
    pub avatar_path: Option<String>,
    pub is_active: bool,
}

/// Manages multiple local Telegram account directories and configurations.
pub struct AccountManager {
    base_data_dir: PathBuf,
    base_cache_dir: PathBuf,
}

impl AccountManager {
    /// Initializes the account manager at the specified application data and cache directories.
    pub fn new(base_data_dir: PathBuf, base_cache_dir: PathBuf) -> Self {
        let accounts_dir = base_data_dir.join("accounts");
        let _ = fs::create_dir_all(&accounts_dir);
        let _ = fs::create_dir_all(&base_cache_dir);
        Self { base_data_dir, base_cache_dir }
    }

    /// Returns the base directory holding all account data.
    pub fn accounts_dir(&self) -> PathBuf {
        self.base_data_dir.join("accounts")
    }

    /// Returns the dedicated data directory for a specific account.
    pub fn account_data_dir(&self, account_id: &str) -> PathBuf {
        let dir = self.accounts_dir().join(account_id).join("data");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// Returns the dedicated files and cache directory for a specific account.
    pub fn account_files_dir(&self, account_id: &str) -> PathBuf {
        let dir = self.base_cache_dir.join("accounts").join(account_id).join("files");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// Lists all known account IDs found in the accounts directory.
    pub fn list_accounts(&self) -> Vec<String> {
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(self.accounts_dir()) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        list.push(name);
                    }
                }
            }
        }
        if list.is_empty() {
            // Default first account
            list.push("default".into());
        }
        list
    }

    /// Generates a new unique account identifier.
    pub fn create_new_account_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
