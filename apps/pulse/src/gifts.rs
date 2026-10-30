//! Gift inventory, collections, resale, transfers, upgrades and auction review.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
};
use gtk::prelude::*;
use serde_json::{Value, json};
fn empty_text() -> Value {
    json!({
    "@type":"formattedText","text":"","entities":[]}
    )
}
pub fn open() {
    features::load("Gifts and collectibles", |s, _d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        let id = me["id"].as_i64().ok_or("No current user.")?;
        button(&b, "My gifts", move || inventory(id));
        button(&b, "Gift collections", move || collections(id));
        button(&b, "Resale marketplace…", market);
        button(&b, "Gift auction…", auction);
        button(&b, "Gift Premium with Stars…", premium);
        Ok(())
    });
}
fn inventory(user: i64) {
    features::pages(
        "My gifts",
        "getReceivedGifts",
        json!({
        "business_connection_id":"","owner_id":{
        "@type":"messageSenderUser","user_id":user}
        ,"collection_id":0,"exclude_unsaved":false,"exclude_saved":false,"exclude_unlimited":false,"exclude_upgradable":false,"exclude_non_upgradable":false,"exclude_upgraded":false,"exclude_without_colors":false,"exclude_hosted":false,"sort_by_price":false,"limit":50}
        ),
        "gifts",
        |b, s, gift| {
            let title = gift["gift"]["gift"]["title"].as_str().or(gift["gift"]["gift"]["name"].as_str()).unwrap_or("Gift");
            note(b, &format!("{title} · {}", gift["received_gift_id"].as_str().unwrap_or("")));
            let id = gift["received_gift_id"].as_str().unwrap_or("").to_string();
            let scope = s.clone();
            button(b, "Gift actions…", move || {
                if scope.valid() {
                    detail(id.clone());
                }
            });
        },
    );
}
fn detail(id: String) {
    features::load("Gift details", move |s, d, b| async move {
        let gift = s
            .call(
                "getReceivedGift",
                json!({
                "received_gift_id":id}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, &features::summary(&gift));
        let saved = gift["is_saved"] == true;
        let (scope, id_) = (s.clone(), id.clone());
        button(&b, if saved { "Hide from profile" } else { "Show on profile" }, move || {
            if scope.valid() {
                actions::request(
                    0,
                    "toggleGiftIsSaved",
                    json!({
                    "received_gift_id":id_,"is_saved":!saved}
                    ),
                );
            }
        });
        let id_ = id.clone();
        button(&b, "Copy gift identifier", move || {
            if let Some(d) = gtk::gdk::Display::default() {
                d.clipboard().set_text(&id_);
            }
        });
        if gift["sell_star_count"].as_i64().unwrap_or(0) > 0 {
            let (scope, id, price) = (s.clone(), id.clone(), gift["sell_star_count"].clone());
            button(&b, "Convert to Stars…", move || {
                if !scope.valid() {
                    return;
                }
                let (id, price) = (id.clone(), price.clone());
                let message = format!("Permanently exchange this gift for {price} Stars? The gift will be lost.");
                actions::operation("Convert gift?", &message, vec![], "Convert gift", move |s, _| {
                    let (id, price) = (id.clone(), price.clone());
                    async move {
                        let fresh = s
                            .call(
                                "getReceivedGift",
                                json!({
                                "received_gift_id":id}
                                ),
                            )
                            .await?;
                        if fresh["sell_star_count"] != price {
                            return Err("Gift conversion value changed. Review it again.".into());
                        }
                        crate::groups::commit(
                            &s,
                            "sellGift",
                            json!({
                            "business_connection_id":"","received_gift_id":id}
                            ),
                        )
                        .await?;
                        Ok(())
                    }
                });
            });
        }
        if gift["can_be_upgraded"] == true {
            let (scope, id, cost) = (
                s.clone(),
                id.clone(),
                if gift["prepaid_upgrade_star_count"].as_i64().unwrap_or(0) > 0 {
                    0
                } else {
                    gift["gift"]["gift"]["upgrade_star_count"].as_i64().unwrap_or(0)
                },
            );
            button(&b, "Upgrade gift…", move || {
                if !scope.valid() {
                    return;
                }
                let id = id.clone();
                actions::operation(
                    "Upgrade gift?",
                    &format!("Spend {cost} Stars to upgrade this gift? The resulting collectible is selected by Telegram."),
                    vec![("Keep original sender and gift message", true, true)],
                    "Upgrade",
                    move |s, checks| {
                        let id = id.clone();
                        async move {
                            let fresh = s
                                .call(
                                    "getReceivedGift",
                                    json!({
                                    "received_gift_id":id}
                                    ),
                                )
                                .await?;
                            let actual = if fresh["prepaid_upgrade_star_count"].as_i64().unwrap_or(0) > 0 {
                                0
                            } else {
                                fresh["gift"]["gift"]["upgrade_star_count"].as_i64().unwrap_or(-1)
                            };
                            if fresh["can_be_upgraded"] != true || actual != cost {
                                return Err("Gift upgrade availability or price changed.".into());
                            }
                            crate::groups::commit(&s,"upgradeGift",json!({
"business_connection_id":"","received_gift_id":id,"keep_original_details":checks[0],"star_count":cost}
)).await?;
                            Ok(())
                        }
                    },
                );
            });
        }
        if gift["can_be_transferred"] == true {
            let (id, cost) = (id.clone(), gift["transfer_star_count"].as_i64().unwrap_or(0));
            button(&b, "Transfer to another owner…", move || transfer(id.clone(), cost));
        }
        if gift["gift"]["@type"] == "sentGiftUpgraded" {
            let id = id.clone();
            button(&b, "Set / remove resale listing…", move || resale(id.clone()));
        }
        Ok(())
    });
}
fn transfer(id: String, cost: i64) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Transfer gift", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let username = entry(&fields, "New owner username", "", 64);
    note(&fields, &format!("Transfer costs {cost} Stars. The gift leaves your account."));
    submit(&d, &b, &fields, s, "Review transfer", move |s| {
        let (name, id) = (username.text().trim().trim_start_matches('@').to_string(), id.clone());
        async move {
            let owner = resolve(&s, &name).await?;
            let message = format!("Give this gift to @{name} and spend {cost} Stars? This cannot be undone.");
            actions::operation("Confirm gift transfer", &message, vec![], "Transfer gift", move |s, _| {
                let (id, owner) = (id.clone(), owner.clone());
                async move {
                    let fresh = s
                        .call(
                            "getReceivedGift",
                            json!({
                            "received_gift_id":id}
                            ),
                        )
                        .await?;
                    if fresh["can_be_transferred"] != true || fresh["transfer_star_count"].as_i64() != Some(cost) {
                        return Err("Gift transfer eligibility or cost changed.".into());
                    }
                    crate::groups::commit(
                        &s,
                        "transferGift",
                        json!({
                        "business_connection_id":"","received_gift_id":id,"new_owner_id":owner,"star_count":cost}
                        ),
                    )
                    .await?;
                    Ok(())
                }
            });
            Ok(())
        }
    });
    d.present();
}
async fn resolve(s: &Scope, name: &str) -> Result<Value, String> {
    let c = s
        .call(
            "searchPublicChat",
            json!({
            "username":name}
            ),
        )
        .await?;
    if let Some(id) = c["type"]["user_id"].as_i64() {
        Ok(json!({
        "@type":"messageSenderUser","user_id":id}
        ))
    } else if c["type"]["is_channel"] == true {
        Ok(json!({
        "@type":"messageSenderChat","chat_id":c["id"]}
        ))
    } else {
        Err("Choose a user or channel username.".into())
    }
}
fn resale(id: String) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Gift resale listing", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let enabled = check(&fields, "List for resale", true);
    let currency = gtk::DropDown::from_strings(&["Stars", "Hundredths of TON"]);
    fields.append(&currency);
    let price = entry(&fields, "Asking price", "", 20);
    note(
        &fields,
        "Telegram deducts its marketplace fee from the sale proceeds. Listing does not transfer the gift until someone purchases it.",
    );
    submit(&d, &b, &fields, s, "Save listing", move |s| {
        let (id, enabled, kind, price) = (id.clone(), enabled.is_active(), currency.selected(), price.text().parse::<i64>());
        async move {
            let price = if !enabled {
                Value::Null
            } else {
                let price = price.map_err(|_| "Enter a whole-number price.")?;
                if price <= 0 {
                    return Err("Price must be positive.".into());
                }
                if kind == 0 {
                    json!({
                    "@type":"giftResalePriceStar","star_count":price}
                    )
                } else {
                    json!({
                    "@type":"giftResalePriceGram","gram_cent_count":price}
                    )
                }
            };
            s.call(
                "setGiftResalePrice",
                json!({
                "received_gift_id":id,"price":price}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
fn collections(user: i64) {
    features::load("Gift collections", move |s, d, b| async move {
        let owner = json!({
        "@type":"messageSenderUser","user_id":user}
        );
        let data = s
            .call(
                "getGiftCollections",
                json!({
                "owner_id":owner}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        for collection in data["collections"].as_array().into_iter().flatten() {
            let (id, name) =
                (collection["id"].as_i64().unwrap_or(0), collection["name"].as_str().unwrap_or("Collection").to_string());
            let scope = s.clone();
            button(&b, &name.clone(), move || {
                if scope.valid() {
                    collection_edit(user, id, name.clone());
                }
            });
        }
        button(&b, "Create collection…", move || collection_edit(user, 0, String::new()));
        Ok(())
    });
}
fn collection_edit(user: i64, id: i64, name: String) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Gift collection", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let name = entry(&fields, "Name", &name, 64);
    let ids = entry(&fields, "Gift identifiers to add (comma separated)", "", 4096);
    let remove = check(&fields, "Remove these gifts instead of adding", false);
    let owner = json!({
    "@type":"messageSenderUser","user_id":user}
    );
    if id != 0 {
        let owner = owner.clone();
        button(&fields, "Delete collection…", move || {
            let owner = owner.clone();
            actions::operation(
                "Delete collection?",
                "Only the collection is removed; you keep the gifts.",
                vec![],
                "Delete",
                move |s, _| {
                    let owner = owner.clone();
                    async move {
                        s.call(
                            "deleteGiftCollection",
                            json!({
                            "owner_id":owner,"collection_id":id}
                            ),
                        )
                        .await?;
                        Ok(())
                    }
                },
            );
        });
    }
    submit(&d, &b, &fields, s, "Save collection", move |s| {
        let (name, ids, remove, owner) = (
            name.text().trim().to_string(),
            ids.text().split(',').map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned).collect::<Vec<_>>(),
            remove.is_active(),
            owner.clone(),
        );
        async move {
            if name.is_empty() {
                return Err("Enter a name.".into());
            }
            if id == 0 {
                crate::groups::commit(
                    &s,
                    "createGiftCollection",
                    json!({
                    "owner_id":owner,"name":name,"received_gift_ids":ids}
                    ),
                )
                .await?;
            } else {
                s.call(
                    "setGiftCollectionName",
                    json!({
                    "owner_id":owner,"collection_id":id,"name":name}
                    ),
                )
                .await?;
                if !ids.is_empty() {
                    s.call(
                        if remove { "removeGiftCollectionGifts" } else { "addGiftCollectionGifts" },
                        json!({
                        "owner_id":owner,"collection_id":id,"received_gift_ids":ids}
                        ),
                    )
                    .await?;
                }
            }
            Ok(())
        }
    });
    d.present();
}
fn market() {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Gift marketplace", 440);
    crate::dialog_dismissal::add(&d, &b, "Close");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let gift = entry(&fields, "Regular gift ID", "", 24);
    submit(&d, &b, &fields, s, "Browse resale offers", move |s| {
        let gift = gift.text().parse::<i64>();
        async move {
            let gift = gift.map_err(|_| "Enter a regular gift ID.")?;
            if !s.valid() {
                return Err("Account changed.".into());
            }
            features::pages(
                "Resale offers",
                "searchGiftsForResale",
                json!({
                "gift_id":gift,"order":{
                "@type":"giftForResaleOrderPrice"}
                ,"for_crafting":false,"for_stars":true,"attributes":[],"limit":50}
                ),
                "gifts",
                |b, s, row| {
                    let gift = &row["gift"];
                    let (name, price) = (
                        gift["name"].as_str().unwrap_or("").to_string(),
                        gift["resale_parameters"]["star_count"].as_i64().unwrap_or(0),
                    );
                    note(b, &format!("{name} · {price} Stars"));
                    let scope = s.clone();
                    button(b, "Buy for myself…", move || {
                        if !scope.valid() {
                            return;
                        }
                        let name = name.clone();
                        actions::operation(
                            "Buy resale gift?",
                            &format!("Spend {price} Stars to buy {name} for yourself?"),
                            vec![],
                            "Buy gift",
                            move |s, _| {
                                let name = name.clone();
                                async move {
                                    let current = s
                                        .call(
                                            "getUpgradedGift",
                                            json!({
                                            "name":name}
                                            ),
                                        )
                                        .await?;
                                    if current["resale_parameters"]["star_count"].as_i64() != Some(price) {
                                        return Err("The listing price changed. Review it again.".into());
                                    }
                                    let me = s.call("getMe", json!({})).await?;
                                    let result = crate::groups::commit(
                                        &s,
                                        "sendResoldGift",
                                        json!({
                                        "gift_name":name,"owner_id":{
                                        "@type":"messageSenderUser","user_id":me["id"]}
                                        ,"price":{
                                        "@type":"giftResalePriceStar","star_count":price}
                                        ,"text":empty_text(),"is_private":true}
                                        ),
                                    )
                                    .await?;
                                    if result["@type"] != "giftResaleResultOk" {
                                        return Err("Gift availability or price changed. Review the listing again.".into());
                                    }
                                    Ok(())
                                }
                            },
                        );
                    });
                },
            );
            Ok(())
        }
    });
    d.present();
}
fn auction() {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Gift auction", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let id = entry(&fields, "Auction ID", "", 128);
    submit(&d, &b, &fields, s, "View auction", move |s| {
        let id = id.text().trim().to_string();
        async move {
            let auction = s
                .call(
                    "getGiftAuctionState",
                    json!({
                    "auction_id":id}
                    ),
                )
                .await?;
            let gift = auction["gift"]["id"].clone();
            let (d, b) = nyx_ui::widgets::dialog("Auction details", 480);
            crate::dialog_dismissal::add(&d, &b, "Close");
            note(&b, &features::summary(&auction));
            let fields = nyx_ui::widgets::vbox(8);
            b.append(&fields);
            let bid = entry(&fields, "Bid in Stars", "", 20);
            submit(&d, &b, &fields, s, "Review bid", move |s| {
                let (gift, bid) = (gift.clone(), bid.text().parse::<i64>());
                async move {
                    let bid = bid.map_err(|_| "Enter a positive whole-number bid.")?;
                    if bid <= 0 {
                        return Err("Bid must be positive.".into());
                    }
                    let me = s.call("getMe", json!({})).await?;
                    let user = me["id"].clone();
                    actions::operation(
                        "Place gift auction bid?",
                        &format!("Commit {bid} Stars to this auction? Telegram's auction and refund rules apply."),
                        vec![],
                        "Place bid",
                        move |s, _| {
                            let (gift, user) = (gift.clone(), user.clone());
                            async move {
                                crate::groups::commit(
                                    &s,
                                    "placeGiftAuctionBid",
                                    json!({
                                    "gift_id":gift,"star_count":bid,"user_id":user,"text":empty_text(),"is_private":true}
                                    ),
                                )
                                .await?;
                                Ok(())
                            }
                        },
                    );
                    Ok(())
                }
            });
            d.present();
            Ok(())
        }
    });
    d.present();
}
fn premium() {
    features::load("Gift Premium", |s, d, b| async move {
        let options = s.call("getPremiumGiftPaymentOptions", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let recipient = entry(&b, "Recipient username", "", 64);
        for option in options["options"].as_array().into_iter().flatten() {
            let (price, months) = (option["star_count"].as_i64().unwrap_or(0), option["month_count"].as_i64().unwrap_or(0));
            if price <= 0 {
                continue;
            }
            let (scope, recipient) = (s.clone(), recipient.clone());
            button(&b, &format!("{months} months · {price} Stars…"), move || {
                if !scope.valid() {
                    return;
                }
                let name = recipient.text().trim().trim_start_matches('@').to_string();
                actions::run(0, "premium-gift-review", move |s| async move {
                    let owner = resolve(&s, &name).await?;
                    let user = owner["user_id"].as_i64().ok_or("Choose a user.")?;
                    actions::operation(
                        "Gift Telegram Premium?",
                        &format!("Spend {price} Stars to give @{name} {months} months of Premium?"),
                        vec![],
                        "Send gift",
                        move |s, _| async move {
                            let fresh = s.call("getPremiumGiftPaymentOptions", json!({})).await?;
                            if !fresh["options"].as_array().is_some_and(|a| {
                                a.iter()
                                    .any(|o| o["star_count"].as_i64() == Some(price) && o["month_count"].as_i64() == Some(months))
                            }) {
                                return Err("Premium gift price changed. Review the options again.".into());
                            }
                            crate::groups::commit(
                                &s,
                                "giftPremiumWithStars",
                                json!({
                                "user_id":user,"star_count":price,"month_count":months,"text":empty_text()}
                                ),
                            )
                            .await?;
                            Ok(())
                        },
                    );
                    Ok(())
                });
            });
        }
        Ok(())
    });
}
