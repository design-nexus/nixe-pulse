//! Per-account encryption keys in Secret Service; only recovery markers live on disk.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
};
fn command(verb: &str, account: &str) -> Command {
    let mut c = Command::new("secret-tool");
    c.arg(verb);
    if verb == "store" {
        c.arg("--label=Pulse database encryption key");
    }
    c.args(["application", "nixe-pulse", "account", account, "kind", "database-key"]);
    c
}
fn lookup(account: &str) -> Option<String> {
    let r = command("lookup", account).output().ok()?;
    if !r.status.success() {
        return None;
    }
    String::from_utf8(r.stdout).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}
fn marker(account: &str) -> PathBuf {
    nixe_core::paths::data_dir().join("accounts").join(account).join("key-migration-pending")
}
fn ensure(account: &str) -> Result<String, String> {
    if let Some(key) = lookup(account) {
        return Ok(key);
    }
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).map_err(|e| e.to_string())?;
    let key = STANDARD.encode(bytes);
    std::fs::write(marker(account), b"pending").map_err(|e| e.to_string())?;
    let mut child = command("store", account)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    child.stdin.take().ok_or("Missing keyring input")?.write_all(key.as_bytes()).map_err(|e| e.to_string())?;
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err("Secret Service could not save the database key".into());
    }
    Ok(key)
}
pub async fn key(account: &str, new_database: bool) -> Result<Option<String>, String> {
    let a = account.to_owned();
    tokio::task::spawn_blocking(move || if new_database { ensure(&a).map(Some) } else { Ok(lookup(&a)) })
        .await
        .map_err(|e| e.to_string())?
}
pub fn pending(account: &str) -> bool {
    marker(account).exists()
}
pub async fn encrypt(client: &nixe_telegram_core::TdClient, account: &str) -> Result<(), String> {
    let a = account.to_owned();
    let key = tokio::task::spawn_blocking(move || ensure(&a)).await.map_err(|e| e.to_string())??;
    client.execute("setDatabaseEncryptionKey", serde_json::json!({"new_encryption_key":key})).await.map_err(|e| e.to_string())?;
    let path = marker(account);
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn store_proxy(account: &str, id: i64, proxy: &serde_json::Value) -> Result<(), String> {
    let mut child = Command::new("secret-tool")
        .args([
            "store",
            "--label=Pulse proxy",
            "application",
            "nixe-pulse",
            "account",
            account,
            "kind",
            "proxy",
            "proxy-id",
            &id.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    child.stdin.take().ok_or("Missing keyring input")?.write_all(proxy.to_string().as_bytes()).map_err(|e| e.to_string())?;
    if child.wait().map_err(|e| e.to_string())?.success() {
        Ok(())
    } else {
        Err("Could not save proxy credentials in Secret Service".into())
    }
}
