//! Stars, gifts, subscriptions, invoice review, boosts and paid reactions.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::prelude::*;
use serde_json::{json,Value};
fn open_url(url: &str) {
    if url.starts_with("https://") {
        let _ = gtk::gio::AppInfo::launch_default_for_uri(url, gtk::gio::AppLaunchContext::NONE);
    }
}
pub fn open() {
    features::load("Premium, Stars and gifts", |s, d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        let owner = json!({
        "@type":"messageSenderUser","user_id":me["id"]}
        );
        let data = s
            .call(
                "getStarTransactions",
                json!({
                "owner_id":owner,"subscription_id":"","direction":null,"offset":"","limit":1}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(
            &b,
            &format!(
                "Balance: {} Stars{}",
                data["star_amount"]["star_count"],
                if me["is_premium"] == true { " · Premium active" } else { "" }
            ),
        );
        button(&b, "Star transactions", transactions);
        button(&b, "Star subscriptions", subscriptions);
        button(&b, "Send a gift…", gifts);
        button(&b, "Premium offers", premium);
        Ok(())
    });
}
pub fn invoice(chat:i64,id:i64){crate::payments::review(json!({"@type":"inputInvoiceMessage","chat_id":chat,"message_id":id}),None);}
pub fn invoice_input(input:serde_json::Value,done:Option<std::rc::Rc<dyn Fn(&str)>>){crate::payments::review(input,done);}
fn transactions() {
    features::load("Star transactions", |s, d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        let owner = json!({
        "@type":"messageSenderUser","user_id":me["id"]}
        );
        let list = nixe_ui::widgets::vbox(6);
        b.append(&list);
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let offset = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let weak = d.downgrade();
        more.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, owner, list, offset, weak, button) = (
                s.clone(),
                owner.clone(),
                list.clone(),
                offset.clone(),
                weak.clone(),
                button.clone(),
            );
            gtk::glib::spawn_future_local(async move {
                let result = s
                    .call(
                        "getStarTransactions",
                        json!({
                        "owner_id":owner,"subscription_id":"","direction":null,"offset":offset.borrow().clone(),"limit":50}
                        ),
                    )
                    .await;
                button.set_sensitive(true);
                if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                match result {
                    Err(e) => {
                        note(&list, &e);
                    }
                    Ok(data) => {
                        for t in data["transactions"].as_array().into_iter().flatten() {
                            note(&list, &features::summary(t));
                        }
                        let next = data["next_offset"].as_str().unwrap_or("").to_string();
                        button.set_visible(!next.is_empty() && next != *offset.borrow());
                        *offset.borrow_mut() = next;
                    }
                }
            });
        });
        more.emit_clicked();
        Ok(())
    });
}
fn subscriptions(){features::pages("Star subscriptions","getStarSubscriptions",json!({"only_expiring":false}),"subscriptions",|b,s,sub|{
    note(b,&features::summary(sub));let id=sub["id"].clone();let canceled=sub["is_canceled"]==true;let scope=s.clone();
    button(b,if canceled{"Resume subscription…"}else{"Cancel renewal…"},move||{if !scope.valid(){return}let id=id.clone();actions::operation(if canceled{"Resume subscription?"}else{"Cancel renewal?"},if canceled{"Renew using your Stars balance."}else{"The subscription remains active until its paid period ends."},vec![],if canceled{"Resume"}else{"Cancel renewal"},move|s,_|{let id=id.clone();async move{s.call("editStarSubscription",json!({"subscription_id":id,"is_canceled":!canceled})).await?;Ok(())}});});
});}
fn premium() {
    features::load("Telegram Premium", |s, d, b| async move {
        let state = s.call("getPremiumState", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, state["state"]["text"].as_str().unwrap_or("Telegram Premium"));
        for option in state["payment_options"].as_array().into_iter().flatten() {
            note(&b, &features::summary(&option["payment_option"]));
            let link = option["payment_option"]["payment_link"].clone();
            let scope = s.clone();
            button(&b, "Review offer", move || {
                if !scope.valid() {
                    return;
                }
                let link = link.clone();
                actions::run(0, "premium-offer", move |s| async move {
                    let result = s
                        .call(
                            "getInternalLink",
                            json!({
                            "type":link,"is_http":true}
                            ),
                        )
                        .await?;
                    open_url(result["url"].as_str().or(result["text"].as_str()).unwrap_or(""));
                    Ok(())
                });
            });
        }
        Ok(())
    });
}
fn gifts() {
    features::load("Send gift", |s, d, b| async move {
        let gifts = s.call("getAvailableGifts", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let recipient = entry(&b, "Recipient username", "", 64);
        for item in gifts["gifts"].as_array().into_iter().flatten() {
            let gift = &item["gift"];
            let id = gift["id"].clone();
            let price = gift["star_count"].clone();
            let title = format!("{} · {price} Stars", item["title"].as_str().unwrap_or("Gift"));
            let (scope, recipient) = (s.clone(), recipient.clone());
            button(&b, &title, move || {
                if !scope.valid() {
                    return;
                }
                let (name, id, price) = (
                    recipient.text().trim().trim_start_matches('@').to_string(),
                    id.clone(),
                    price.clone(),
                );
                if name.is_empty() {
                    window::show_status("Choose a recipient username.");
                    return;
                }
                actions::run(0, "gift-review", move |s| async move {
                    let chat = s
                        .call(
                            "searchPublicChat",
                            json!({
                            "username":name}
                            ),
                        )
                        .await?;
                    let owner = if let Some(user) = chat["type"]["user_id"].as_i64() {
                        json!({
                        "@type":"messageSenderUser","user_id":user}
                        )
                    } else if chat["type"]["is_channel"] == true {
                        json!({
                        "@type":"messageSenderChat","chat_id":chat["id"]}
                        )
                    } else {
                        return Err("Choose a user or channel.".into());
                    };
                    let who = chat["title"].as_str().unwrap_or("recipient");
                    let description = format!("Send this gift to {who} for {price} Stars? This spends Stars immediately.");
                    actions::operation(
                        "Send gift?",
                        &description,
                        vec![("Hide sender and message from other viewers", true, true)],
                        "Send gift",
                        move |s, checks| {
                            let (id, price, owner) = (id.clone(), price.clone(), owner.clone());
                            async move {
                                let gifts = s.call("getAvailableGifts", json!({})).await?;
                                let current = gifts["gifts"]
                                    .as_array()
                                    .and_then(|g| g.iter().find(|g| g["gift"]["id"] == id))
                                    .ok_or("This gift is no longer available.")?;
                                if current["gift"]["star_count"] != price {
                                    return Err("The gift price changed. Review it again.".into());
                                }
                                crate::groups::commit(
                                    &s,
                                    "sendGift",
                                    json!({
                                    "gift_id":id,"owner_id":owner,"text":{
                                    "@type":"formattedText","text":"","entities":[]}
                                    ,"is_private":checks[0],"pay_for_upgrade":false}
                                    ),
                                )
                                .await?;
                                Ok(())
                            }
                        },
                    );
                    Ok(())
                });
            });
        }
        Ok(())
    });
}
pub fn paid_reaction(chat: i64, id: i64) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nixe_ui::widgets::dialog("Paid reaction", 440);
    let fields = nixe_ui::widgets::vbox(8);
    b.append(&fields);
    note(
        &fields,
        "Each reaction spends one Telegram Star. Telegram validates eligibility and balance.",
    );
    let count = gtk::SpinButton::with_range(1., 2500., 1.);
    fields.append(&count);
    let anonymous = check(&fields, "Send anonymously", false);
    submit(&d, &b, &fields, s, "Review payment", move |s| {
        let (count, anonymous) = (count.value_as_int(), anonymous.is_active());
        async move {
            let me = s.call("getMe", json!({})).await?;
            let balance = s
                .call(
                    "getStarTransactions",
                    json!({
                    "owner_id":{
                    "@type":"messageSenderUser","user_id":me["id"]}
                    ,"subscription_id":"","direction":null,"offset":"","limit":1}
                    ),
                )
                .await?;
            if balance["star_amount"]["star_count"].as_i64().unwrap_or(0) < count as i64 {
                return Err("Not enough Telegram Stars.".into());
            }
            actions::operation(
                "Send paid reaction?",
                &format!("Spend {count} Telegram Stars on this message?"),
                vec![],
                "Pay Stars",
                move |s, _| async move {
                    s.call(
                        "removePendingPaidMessageReactions",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?;
                    s.call(
                        "addPendingPaidMessageReaction",
                        json!({
                        "chat_id":chat,"message_id":id,"star_count":count,"type":{
                        "@type":if anonymous{
                        "paidReactionTypeAnonymous"}
                        else{
                        "paidReactionTypeRegular"}
                        }
                        }
                        ),
                    )
                    .await?;
                    crate::groups::commit(
                        &s,
                        "commitPendingPaidMessageReactions",
                        json!({
                        "chat_id":chat,"message_id":id}
                        ),
                    )
                    .await?;
                    Ok(())
                },
            );
            Ok(())
        }
    });
    d.present();
}
pub fn boost(chat: i64) {
    features::load("Channel boosts", move |s, d, b| async move {
        let status = s
            .call(
                "getChatBoostStatus",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        let slots = s.call("getAvailableChatBoostSlots", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, &features::summary(&status));
        for slot in slots["slots"].as_array().into_iter().flatten() {
            let id = slot["slot_id"].clone();
            let scope = s.clone();
            button(
                &b,
                &format!("Apply slot {} (currently {})…", id, slot["currently_boosted_chat_id"]),
                move || {
                    if !scope.valid() {
                        return;
                    }
                    let id = id.clone();
                    actions::operation(
                        "Apply boost?",
                        "This moves the selected boost slot to this chat. Telegram may impose a cooldown before it can be moved again.",
                        vec![],
                        "Apply boost",
                        move |s, _| {
                            let id = id.clone();
                            async move {
                                s.call(
                                    "boostChat",
                                    json!({
                                    "chat_id":chat,"slot_ids":[id]}
                                    ),
                                )
                                .await?;
                                Ok(())
                            }
                        },
                    );
                },
            );
        }
        Ok(())
    });
}

pub fn subscription_invite(chat:i64,link:Value){features::load("Paid channel invitation",move|s,d,b|async move{let c=crate::groups::context(&s,chat).await?;c.require("can_invite_users")?;if !c.channel(){return Err("Subscription invitations require a channel.".into())}if !alive(&s,&d){return Ok(())}let fields=nixe_ui::widgets::vbox(6);b.append(&fields);let name=entry(&fields,"Administrator label",link["name"].as_str().unwrap_or(""),32);let count=entry(&fields,"Stars per 30 days",&link["subscription_pricing"]["star_count"].as_i64().unwrap_or(100).to_string(),8);let edit=link.is_object();count.set_editable(!edit);note(&fields,"Members joining through this invitation pay the displayed amount every 30 days until they cancel. Existing subscription prices cannot be edited; create a new invitation to change the price.");submit(&d,&b,&fields,s,"Save paid invitation",move|s|{let(name,count,link)=(name.text().to_string(),count.text().parse::<i64>(),link.clone());async move{let c=crate::groups::context(&s,chat).await?;c.require("can_invite_users")?;if !c.channel(){return Err("Channel eligibility changed.".into())}let count=count.map_err(|_|"Enter a whole number of Stars.")?;if !(1..=10000).contains(&count){return Err("Use a subscription price from 1 to 10000 Stars.".into())}let result=if edit{let fresh=s.call("getChatInviteLink",json!({"chat_id":chat,"invite_link":link["invite_link"]})).await?;if fresh["subscription_pricing"]!=link["subscription_pricing"]||fresh["name"]!=link["name"]{return Err("The invitation changed. Reopen the editor.".into())}s.call("editChatSubscriptionInviteLink",json!({"chat_id":chat,"invite_link":link["invite_link"],"name":name})).await?}else{crate::groups::commit(&s,"createChatSubscriptionInviteLink",json!({"chat_id":chat,"name":name,"subscription_pricing":{"@type":"starSubscriptionPricing","period":2592000,"star_count":count}})).await?};let(d,b)=nixe_ui::widgets::dialog("Paid invitation",440);let url=result["invite_link"].as_str().unwrap_or("").to_string();note(&b,&url);button(&b,"Copy invitation",move||{if let Some(display)=gtk::gdk::Display::default(){display.clipboard().set_text(&url);}});d.present();Ok(())}});Ok(())});}
pub fn receipt(chat:i64,id:i64){features::load("Payment receipt",move|s,d,b|async move{let data=s.call("getPaymentReceipt",json!({"chat_id":chat,"message_id":id})).await?;if alive(&s,&d){note(&b,&features::summary(&data));}Ok(())});}
