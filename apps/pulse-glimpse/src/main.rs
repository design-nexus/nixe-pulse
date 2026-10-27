//! Search the active Pulse account over its existing read-only control interface.
use anyhow::{Result, bail};
use gio::prelude::*;
use glimpse_plugin_sdk::{Action, Hit, Request, serve};
use serde_json::{Value, json};
fn main() {
    if let Err(e) = serve(&["search", "preview"], answer) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
fn chats(query: &str) -> Result<Vec<Value>> {
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    let response = connection.call_sync(
        Some("io.github.design_nexus.nyx.Pulse"),
        "/io/github/design_nexus/nyx/Pulse",
        "io.github.design_nexus.nyx.Control",
        "Call",
        Some(&("pulse.chats.search", json!({"query":query}).to_string()).to_variant()),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        2000,
        gio::Cancellable::NONE,
    )?;
    let (text,) = response.get::<(String,)>().ok_or_else(|| anyhow::anyhow!("Invalid Pulse reply"))?;
    let value: Value = serde_json::from_str(&text)?;
    Ok(value["ok"]["results"].as_array().cloned().unwrap_or_default())
}
fn answer(request: &Request) -> Result<Value> {
    match request.op.as_str() {
        "search" => {
            let query = request.args["query"].as_str().unwrap_or("");
            let limit = request.args["limit"].as_u64().unwrap_or(10).min(20) as usize;
            let hits = chats(query)
                .unwrap_or_default()
                .into_iter()
                .take(limit)
                .filter_map(|c| {
                    Some(Hit {
                        id: c["id"].as_i64()?.to_string(),
                        title: c["title"].as_str()?.into(),
                        subtitle: format!("Telegram · {} unread", c["unread_count"].as_i64().unwrap_or(0)),
                        action: Action::Url { url: format!("nyx-pulse://chat/{}", c["id"]) },
                    })
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_value(hits)?)
        }
        "preview" => {
            let id = request.args["result_id"].as_str().unwrap_or("");
            let chat = chats("")?.into_iter().find(|c| c["id"].as_i64().map(|v| v.to_string()).as_deref() == Some(id));
            let Some(c) = chat else { bail!("Conversation is not loaded in Pulse") };
            Ok(
                json!({"text":format!("{}\n{} unread messages",c["title"].as_str().unwrap_or("Conversation"),c["unread_count"].as_i64().unwrap_or(0))}),
            )
        }
        _ => bail!("unsupported operation"),
    }
}
