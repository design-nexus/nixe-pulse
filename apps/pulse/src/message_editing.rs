//! Fresh-permission media replacement, suggested offers and live-location alerts.
use crate::{
    features::{self, alive},
    groups::{entry, note, submit},
    window,
};
use gtk::prelude::*;
use serde_json::{Value, json};
pub fn content(media: Value, caption: Value) -> Result<Value, String> {
    let kind = media["@type"].as_str().unwrap_or("").strip_prefix("inputPollMedia").ok_or("Choose media.")?.to_string();
    if !matches!(kind.as_str(), "Photo" | "Video" | "Animation" | "Audio" | "Document") {
        return Err("Choose a photo, video, animation, audio file or document.".into());
    }
    let key = kind.to_ascii_lowercase();
    let mut input = media[&key].clone();
    input["@type"] = json!(format!("inputMessage{kind}"));
    input["caption"] = caption;
    input["clear_draft"] = json!(false);
    input["has_spoiler"] = json!(false);
    Ok(input)
}
pub fn replace(chat: i64, id: i64) {
    features::load("Replace message media", move |s, d, b| async move {
        let original = s
            .call(
                "getMessage",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        let props = s
            .call(
                "getMessageProperties",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        if props["can_edit_media"] != true {
            return Err("This message's media cannot be replaced.".into());
        }
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let media = crate::poll_media::picker(&fields, &d, &s, "Replacement file", false);
        let caption = entry(&fields, "Caption", original["content"]["caption"]["text"].as_str().unwrap_or(""), 4096);
        let spoiler = features::check(&fields, "Spoiler", original["content"]["has_spoiler"] == true);
        submit(&d, &b, &fields, s, "Replace media", move |s| {
            let (media, caption, spoiler, original) = (
                media.borrow().as_ref().map(|a| a.input()).transpose(),
                caption.text().to_string(),
                spoiler.is_active(),
                original.clone(),
            );
            async move {
                let props = s
                    .call(
                        "getMessageProperties",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?;
                let current = s
                    .call(
                        "getMessage",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?;
                if props["can_edit_media"] != true || current["content"] != original["content"] {
                    return Err("The message changed or editing is no longer allowed. Reopen the editor.".into());
                }
                let caption = if caption == original["content"]["caption"]["text"].as_str().unwrap_or("") {
                    original["content"]["caption"].clone()
                } else {
                    json!({
                    "@type":"formattedText","text":caption,"entities":[]}
                    )
                };
                let mut input = content(media?.ok_or("Choose replacement media.")?, caption)?;
                input["has_spoiler"] = json!(spoiler);
                let m = crate::groups::commit(
                    &s,
                    "editMessageMedia",
                    json!({
                    "chat_id":chat,"message_id":id,"reply_markup":original["reply_markup"],"input_message_content":input}
                    ),
                )
                .await?;
                if let Some(m) = nyx_telegram_core::client::parse_message(&m) {
                    window::display_message(&m);
                }
                Ok(())
            }
        });
        Ok(())
    });
}
pub fn offer(chat: i64, id: i64) {
    features::load("Offer for suggested post", move |s, d, b| async move {
        if s.call(
            "getMessageProperties",
            json!({
            "chat_id":chat,"message_id":id}
            ),
        )
        .await?["can_add_offer"]
            != true
        {
            return Err("Offers are unavailable for this message.".into());
        }
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let currency = gtk::DropDown::from_strings(&["Stars", "TON (hundredths)"]);
        fields.append(&currency);
        let price = entry(&fields, "Offer amount", "0", 18);
        let date = entry(&fields, "Publish at Unix timestamp (0 = any time)", "0", 12);
        note(&fields, "This sends an offer under the chosen terms. Telegram will ask the other party to accept it.");
        submit(&d, &b, &fields, s, "Send offer", move |s| {
            let (currency, price, date) = (currency.selected(), price.text().parse::<i64>(), date.text().parse::<i64>());
            async move {
                let price = price.map_err(|_| "Invalid amount.")?;
                let date = date.map_err(|_| "Invalid publication time.")?;
                if price < 0 || date < 0 {
                    return Err("Amount and time must be nonnegative.".into());
                }
                if s.call(
                    "getMessageProperties",
                    json!({
                    "chat_id":chat,"message_id":id}
                    ),
                )
                .await?["can_add_offer"]
                    != true
                {
                    return Err("Offer eligibility changed.".into());
                }
                let price = if currency == 0 {
                    json!({
                    "@type":"suggestedPostPriceStar","star_count":price}
                    )
                } else {
                    json!({
                    "@type":"suggestedPostPriceGram","gram_cent_count":price}
                    )
                };
                crate::groups::commit(
                    &s,
                    "addOffer",
                    json!({
                    "chat_id":chat,"message_id":id,"options":{
                    "@type":"messageSendOptions","suggested_post_info":{
                    "@type":"inputSuggestedPostInfo","price":price,"send_date":date}
                    }
                    }
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
pub fn proximity(chat: i64, id: i64) {
    features::load("Live-location proximity alert", move |s, d, b| async move {
        let m = s
            .call(
                "getMessage",
                json!({
                "chat_id":chat,"message_id":id}
                ),
            )
            .await?;
        if m["is_outgoing"] != true || m["content"]["@type"] != "messageLiveLocation" {
            return Err("Choose your active live-location message.".into());
        }
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(6);
        b.append(&fields);
        let radius = entry(
            &fields,
            "Alert distance in meters (0 = off)",
            &m["content"]["location"]["proximity_alert_radius"].as_i64().unwrap_or(0).to_string(),
            8,
        );
        submit(&d, &b, &fields, s, "Save alert", move |s| {
            let radius = radius.text().parse::<i32>();
            async move {
                let radius = radius.map_err(|_| "Invalid distance.")?;
                if !(0..=100000).contains(&radius) {
                    return Err("Use a distance from 0 to 100000 meters.".into());
                }
                let m = s
                    .call(
                        "getMessage",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?;
                if m["is_outgoing"] != true || m["content"]["expires_in"].as_i64().unwrap_or(0) <= 0 {
                    return Err("This live-location share has ended.".into());
                }
                let mut live = m["content"]["location"].clone();
                live["proximity_alert_radius"] = json!(radius);
                s.call(
                    "editMessageLiveLocation",
                    json!({
                    "chat_id":chat,"message_id":id,"reply_markup":m["reply_markup"],"location":live}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
