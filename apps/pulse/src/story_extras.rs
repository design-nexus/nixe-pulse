//! Story albums, interactions, interactive links, media replacement and stealth mode.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::prelude::*;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
pub fn tools(chat: i64, id: i64) {
    features::load("Story tools", move |s, d, b| async move {
        let story = s
            .call(
                "getStory",
                json!({
                "story_poster_chat_id":chat,"story_id":id,"only_local":false}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        if story["can_get_interactions"] == true {
            button(&b, "Viewers, reactions and forwards", move || interactions(chat, id));
        }
        if story["can_be_edited"] == true {
            button(&b, "Replace media…", move || edit_media(chat, id));
            button(&b, "Edit interactive areas…", move || edit_areas(chat, id));
        }
        button(&b, "Story albums", move || albums(chat));
        for area in story["areas"].as_array().into_iter().flatten() {
            let t = &area["type"];
            match t["@type"].as_str() {
                Some("storyAreaTypeLink") => {
                    if let Some(url) = t["url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
                        b.append(&gtk::LinkButton::with_label(url, url));
                    }
                }
                Some("storyAreaTypeMessage") => {
                    let (chat, id) = (t["chat_id"].as_i64().unwrap_or(0), t["message_id"].as_i64().unwrap_or(0));
                    button(&b, "Open linked message", move || {
                        actions::run(chat, "story-message", move |s| async move {
                            let m = s
                                .call(
                                    "getMessage",
                                    json!({
                                    "chat_id":chat,"message_id":id}
                                    ),
                                )
                                .await?;
                            if let Some(m) = nixe_telegram_core::client::parse_message(&m) {
                                window::jump_message(&m);
                            }
                            Ok(())
                        })
                    });
                }
                Some("storyAreaTypeLocation") => {
                    let location = &t["location"];
                    let (lat, lon) = (location["latitude"].as_f64().unwrap_or(0.), location["longitude"].as_f64().unwrap_or(0.));
                    b.append(&gtk::LinkButton::with_label(
                        &format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}"),
                        "Open linked location",
                    ));
                }
                Some("storyAreaTypeSuggestedReaction") => {
                    let reaction = t["reaction_type"].clone();
                    button(&b, &format!("React {}", reaction["emoji"].as_str().unwrap_or("custom emoji")), move || {
                        let reaction = reaction.clone();
                        actions::run(chat, "story-area-react", move |s| async move {
                            s.call(
                                "setStoryReaction",
                                json!({
                                "story_poster_chat_id":chat,"story_id":id,"reaction_type":reaction,"update_recent_reactions":true}
                                ),
                            )
                            .await?;
                            Ok(())
                        });
                    });
                }
                _ => {
                    note(&b, &features::summary(t));
                }
            }
        }
        Ok(())
    });
}
pub fn stealth() {
    actions::operation(
        "Activate story stealth mode?",
        "Telegram Premium eligibility and cooldowns apply. This hides eligible recent and upcoming story views during Telegram's stealth window.",
        vec![],
        "Activate",
        |s, _| async move {
            s.call("activateStoryStealthMode", json!({})).await?;
            Ok(())
        },
    );
}
fn interactions(chat: i64, id: i64) {
    features::load("Story interactions", move |s, d, b| async move {
        let story = s
            .call(
                "getStory",
                json!({
                "story_poster_chat_id":chat,"story_id":id,"only_local":false}
                ),
            )
            .await?;
        if story["can_get_interactions"] != true {
            return Err("Telegram does not permit viewing these interactions.".into());
        }
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let list = nixe_ui::widgets::vbox(6);
        b.append(&list);
        let cursor: Rc<RefCell<String>> = Default::default();
        let weak = d.downgrade();
        more.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, button, list, cursor, weak) = (s.clone(), button.clone(), list.clone(), cursor.clone(), weak.clone());
            gtk::glib::spawn_future_local(async move {
                let offset = cursor.borrow().clone();
                let result=s.call("getChatStoryInteractions",json!({
"story_poster_chat_id":chat,"story_id":id,"reaction_type":null,"prefer_forwards":false,"offset":offset,"limit":50}
)).await;
                button.set_sensitive(true);
                if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                match result {
                    Err(e) => {
                        note(&list, &e);
                    }
                    Ok(data) => {
                        for row in data["interactions"].as_array().into_iter().flatten() {
                            let actor = &row["actor_id"];
                            let who = if let Some(id) = actor["user_id"].as_i64() {
                                s.call(
                                    "getUser",
                                    json!({
                                    "user_id":id}
                                    ),
                                )
                                .await
                                .map(|u| {
                                    format!(
                                        "{} {}",
                                        u["first_name"].as_str().unwrap_or(""),
                                        u["last_name"].as_str().unwrap_or("")
                                    )
                                })
                                .unwrap_or_else(|_| format!("User {id}"))
                            } else {
                                window::chat(actor["chat_id"].as_i64().unwrap_or(0))
                                    .map(|c| c.title)
                                    .unwrap_or_else(|| "Channel".into())
                            };
                            if !s.valid() {
                                return;
                            }
                            note(&list, &format!("{who} · {}", features::summary(&row["type"])));
                        }
                        let next = data["next_offset"].as_str().unwrap_or("").to_string();
                        button.set_visible(!next.is_empty() && next != offset);
                        *cursor.borrow_mut() = next;
                    }
                }
            });
        });
        more.emit_clicked();
        Ok(())
    });
}
fn edit_media(chat: i64, id: i64) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nixe_ui::widgets::dialog("Edit story media", 480);
    let fields = nixe_ui::widgets::vbox(8);
    b.append(&fields);
    let selected = crate::poll_media::story_picker(&fields, &d, &s, "Replacement photo or video");
    let caption = entry(&fields, "New caption (empty keeps existing)", "", 4096);
    submit(&d, &b, &fields, s, "Save story", move |s| {
        let (selected, caption) = (selected.borrow().as_ref().map(|a| a.input()).transpose(), caption.text().to_string());
        async move {
            let story = s
                .call(
                    "getStory",
                    json!({
                    "story_poster_chat_id":chat,"story_id":id,"only_local":false}
                    ),
                )
                .await?;
            if story["can_be_edited"] != true {
                return Err("This story can no longer be edited.".into());
            }
            let content = selected?.map(content).transpose()?;
            s.call(
                "editStory",
                json!({
                "story_poster_chat_id":chat,"story_id":id,"content":content,"areas":null,"caption":if caption.is_empty(){
                Value::Null}
                else{
                json!({
                "@type":"formattedText","text":caption,"entities":[]}
                )}
                }
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
pub fn content(media: Value) -> Result<Value, String> {
    match media["@type"].as_str() {
        Some("inputPollMediaPhoto") => Ok(json!({
        "@type":"inputStoryContentPhoto","photo":media["photo"]["photo"],"added_sticker_file_ids":[]}
        )),
        Some("inputPollMediaVideo") => {
            let duration = media["video"]["duration"]
                .as_i64()
                .filter(|n| *n > 0 && *n <= 60)
                .ok_or("Story videos must be at most 60 seconds.")?;
            Ok(json!({
            "@type":"inputStoryContentVideo","video":media["video"]["video"],"added_sticker_file_ids":[],"duration":duration,"cover_frame_timestamp":0.,"is_animation":false}
            ))
        }
        _ => Err("Choose a photo or H.264 MP4 video.".into()),
    }
}
pub fn albums(chat: i64) {
    features::load("Story albums", move |s, d, b| async move {
        let data = s
            .call(
                "getChatStoryAlbums",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let order = data["albums"].as_array().into_iter().flatten().filter_map(|a| a["id"].as_i64()).collect::<Vec<_>>();
        for (index, album) in data["albums"].as_array().into_iter().flatten().enumerate() {
            let id = album["id"].as_i64().unwrap_or(0);
            let title = album["name"].as_str().unwrap_or("Album").to_string();
            let scope = s.clone();
            let name = title.clone();
            button(&b, &title, move || {
                if scope.valid() {
                    album_edit(chat, id, name.clone());
                }
            });
            let scope = s.clone();
            button(&b, "Browse stories", move || {
                if scope.valid() {
                    album_stories(chat, id);
                }
            });
            for (label, delta) in [("Move up", -1isize), ("Move down", 1)] {
                let (scope, order) = (s.clone(), order.clone());
                button(&b, label, move || {
                    if !scope.valid() {
                        return;
                    }
                    let target = index as isize + delta;
                    if target < 0 || target as usize >= order.len() {
                        return;
                    }
                    let mut changed = order.clone();
                    changed.swap(index, target as usize);
                    let original = order.clone();
                    actions::run(chat, "story-album-order", move |s| async move {
                        let current = s
                            .call(
                                "getChatStoryAlbums",
                                json!({
                                "chat_id":chat}
                                ),
                            )
                            .await?;
                        let ids = current["albums"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|v| v["id"].as_i64())
                            .collect::<Vec<_>>();
                        if ids != original {
                            return Err("Album order changed. Reopen the album browser.".into());
                        }
                        s.call(
                            "reorderStoryAlbums",
                            json!({
                            "chat_id":chat,"story_album_ids":changed}
                            ),
                        )
                        .await?;
                        Ok(())
                    });
                });
            }
        }
        button(&b, "Create album…", move || album_edit(chat, 0, String::new()));
        Ok(())
    });
}
fn album_edit(chat: i64, id: i64, name: String) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nixe_ui::widgets::dialog("Story album", 440);
    let fields = nixe_ui::widgets::vbox(8);
    b.append(&fields);
    let name = entry(&fields, "Name", &name, 64);
    let ids = entry(&fields, "Story numbers to add (comma separated)", "", 1024);
    if id != 0 {
        button(&fields, "Delete album…", move || {
            actions::operation(
                "Delete album?",
                "The album is removed. Its stories remain in your profile or archive.",
                vec![],
                "Delete album",
                move |s, _| async move {
                    s.call(
                        "deleteStoryAlbum",
                        json!({
                        "chat_id":chat,"story_album_id":id}
                        ),
                    )
                    .await?;
                    Ok(())
                },
            )
        });
    }
    submit(&d, &b, &fields, s, "Save album", move |s| {
        let (name, ids) = (name.text().trim().to_string(), features::ids(&ids.text()));
        async move {
            if name.is_empty() {
                return Err("Enter an album name.".into());
            }
            let ids = ids?;
            if id == 0 {
                crate::groups::commit(
                    &s,
                    "createStoryAlbum",
                    json!({
                    "story_poster_chat_id":chat,"name":name,"story_ids":ids}
                    ),
                )
                .await?;
            } else {
                s.call(
                    "setStoryAlbumName",
                    json!({
                    "chat_id":chat,"story_album_id":id,"name":name}
                    ),
                )
                .await?;
                if !ids.is_empty() {
                    s.call(
                        "addStoryAlbumStories",
                        json!({
                        "chat_id":chat,"story_album_id":id,"story_ids":ids}
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
fn album_stories(chat: i64, id: i64) {
    features::load("Album stories", move |s, d, b| async move {
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let list = nixe_ui::widgets::vbox(6);
        b.append(&list);
        let offset = Rc::new(Cell::new(0));
        let weak = d.downgrade();
        more.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, button, list, offset, weak) = (s.clone(), button.clone(), list.clone(), offset.clone(), weak.clone());
            gtk::glib::spawn_future_local(async move {
                let result = s
                    .call(
                        "getStoryAlbumStories",
                        json!({
                        "chat_id":chat,"story_album_id":id,"offset":offset.get(),"limit":50}
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
                        let rows = data["stories"].as_array().cloned().unwrap_or_default();
                        offset.set(offset.get() + rows.len() as i32);
                        for story in &rows {
                            let id = story["id"].as_i64().unwrap_or(0);
                            let scope = s.clone();
                            button_fn(&list, &format!("Story {id}"), move || {
                                if scope.valid() {
                                    crate::stories::view(chat, id);
                                }
                            });
                        }
                        button.set_visible(!rows.is_empty() && (offset.get() as i64) < data["total_count"].as_i64().unwrap_or(0));
                    }
                }
            });
        });
        more.emit_clicked();
        Ok(())
    });
}
fn button_fn(b: &gtk::Box, title: &str, f: impl Fn() + 'static) {
    button(b, title, f);
}

pub fn area_picker(body: &gtk::Box, parent: &gtk::Window, scope: &Scope) -> Rc<RefCell<Vec<Value>>> {
    area_picker_initial(body, parent, scope, Vec::new())
}
fn area_picker_initial(body: &gtk::Box, parent: &gtk::Window, scope: &Scope, initial: Vec<Value>) -> Rc<RefCell<Vec<Value>>> {
    let selected: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(initial.clone()));
    let rows = nixe_ui::widgets::vbox(4);
    body.append(&rows);
    for area in initial {
        area_row(&rows, &selected, area);
    }
    let (s, weak, state) = (scope.clone(), parent.downgrade(), selected.clone());
    button(body, "Add interactive area…", move || {
        if !s.valid() {
            return;
        }
        let Some(parent) = weak.upgrade() else { return };
        let (d, b) = nixe_ui::widgets::dialog("Interactive story area", 480);
        d.set_transient_for(Some(&parent));
        let fields = crate::groups::scroll(&b);
        let kind = gtk::DropDown::from_strings(&["Link", "Location", "Reaction", "Message", "Weather", "Collectible gift"]);
        fields.append(&kind);
        let a = entry(&fields, "URL / latitude / emoji / chat ID / temperature / gift name", "", 2048);
        let c = entry(&fields, "Longitude / message ID / weather emoji (where applicable)", "", 128);
        let x = entry(&fields, "Horizontal center (%)", "50", 6);
        let y = entry(&fields, "Vertical center (%)", "75", 6);
        let width = entry(&fields, "Width (%)", "50", 6);
        let height = entry(&fields, "Height (%)", "12", 6);
        let rotation = entry(&fields, "Rotation (degrees)", "0", 8);
        let radius = entry(&fields, "Corner radius (%)", "10", 6);
        let dark = check(&fields, "Dark reaction background", false);
        let flip = check(&fields, "Flip reaction bubble", false);
        let (state, rows) = (state.clone(), rows.clone());
        submit(&d, &b, &fields, s.clone(), "Add area", move |_| {
            let (kind, a, c, x, y, width, height, rotation, radius, dark, flip, state, rows) = (
                kind.selected(),
                a.text().to_string(),
                c.text().to_string(),
                x.text().to_string(),
                y.text().to_string(),
                width.text().to_string(),
                height.text().to_string(),
                rotation.text().to_string(),
                radius.text().to_string(),
                dark.is_active(),
                flip.is_active(),
                state.clone(),
                rows.clone(),
            );
            async move {
                let parse = |s: &str, min: f64, max: f64| -> Result<f64, String> {
                    let v = s.parse::<f64>().map_err(|_| "Enter numeric area coordinates.")?;
                    if !v.is_finite() || v < min || v > max {
                        return Err("Area coordinates are out of range.".into());
                    }
                    Ok(v)
                };
                let position = json!({
                "@type":"storyAreaPosition","x_percentage":parse(&x,0.,100.)?,"y_percentage":parse(&y,0.,100.)?,"width_percentage":parse(&width,0.1,100.)?,"height_percentage":parse(&height,0.1,100.)?,"rotation_angle":parse(&rotation,-360.,360.)?,"corner_radius_percentage":parse(&radius,0.,100.)?}
                );
                let typ = match kind {
                    0 => {
                        if !a.starts_with("https://") {
                            return Err("Enter an HTTPS link.".into());
                        }
                        json!({
                        "@type":"inputStoryAreaTypeLink","url":a}
                        )
                    }
                    1 => json!({
                    "@type":"inputStoryAreaTypeLocation","location":{
                    "@type":"location","latitude":parse(&a,-90.,90.)?,"longitude":parse(&c,-180.,180.)?,"horizontal_accuracy":0.}
                    ,"address":null}
                    ),
                    2 => {
                        if a.trim().is_empty() {
                            return Err("Enter a reaction emoji.".into());
                        }
                        json!({
                        "@type":"inputStoryAreaTypeSuggestedReaction","reaction_type":{
                        "@type":"reactionTypeEmoji","emoji":a}
                        ,"is_dark":dark,"is_flipped":flip}
                        )
                    }
                    3 => json!({
                    "@type":"inputStoryAreaTypeMessage","chat_id":a.parse::<i64>().map_err(|_|"Invalid chat ID.")?,"message_id":c.parse::<i64>().map_err(|_|"Invalid message ID.")?}
                    ),
                    4 => json!({
                    "@type":"inputStoryAreaTypeWeather","temperature":parse(&a,-150.,150.)?,"emoji":c,"background_color":0x202938}
                    ),
                    _ => {
                        if a.trim().is_empty() {
                            return Err("Enter a collectible gift name.".into());
                        }
                        json!({
                        "@type":"inputStoryAreaTypeUpgradedGift","gift_name":a}
                        )
                    }
                };
                let area = json!({
                "@type":"inputStoryArea","position":position,"type":typ}
                );
                state.borrow_mut().push(area.clone());
                area_row(&rows, &state, area);
                Ok(())
            }
        });
        d.present();
    });
    selected
}

fn area_row(rows: &gtk::Box, state: &Rc<RefCell<Vec<Value>>>, area: Value) {
    let row = nixe_ui::widgets::hbox(6);
    let title = area["type"]["@type"].as_str().unwrap_or("").strip_prefix("inputStoryAreaType").unwrap_or("Area");
    note(&row, &format!("{} at {}%, {}%", title, area["position"]["x_percentage"], area["position"]["y_percentage"]));
    let remove = gtk::Button::with_label("Remove");
    row.append(&remove);
    let (state, weak, list) = (state.clone(), row.downgrade(), rows.downgrade());
    remove.connect_clicked(move |_| {
        let index = state.borrow().iter().position(|v| v == &area);
        if let Some(index) = index {
            state.borrow_mut().remove(index);
        }
        if let (Some(rows), Some(row)) = (list.upgrade(), weak.upgrade()) {
            rows.remove(&row);
        }
    });
    rows.append(&row);
}
fn input_area(area: &Value) -> Result<Value, String> {
    let mut result = area.clone();
    result["@type"] = json!("inputStoryArea");
    let t = &area["type"];
    let name = t["@type"].as_str().unwrap_or("");
    result["type"] = if name == "storyAreaTypeVenue" {
        json!({"@type":"inputStoryAreaTypePreviousVenue","venue_provider":t["venue"]["provider"],"venue_id":t["venue"]["id"]})
    } else {
        let suffix = name.strip_prefix("storyAreaType").ok_or("This story contains an unsupported interactive area.")?;
        if !matches!(suffix, "Location" | "SuggestedReaction" | "Message" | "Link" | "Weather" | "UpgradedGift") {
            return Err("This story contains an unsupported interactive area. Its existing areas were preserved.".into());
        }
        let mut input = t.clone();
        input["@type"] = json!(format!("inputStoryAreaType{suffix}"));
        if let Some(object) = input.as_object_mut() {
            object.remove("total_count");
        }
        input
    };
    Ok(result)
}
fn edit_areas(chat: i64, id: i64) {
    features::load("Edit story interactive areas", move |s, d, b| async move {
        let story = s.call("getStory", json!({"story_poster_chat_id":chat,"story_id":id,"only_local":false})).await?;
        if story["can_be_edited"] != true {
            return Err("This story cannot be edited.".into());
        }
        if !alive(&s, &d) {
            return Ok(());
        }
        let initial = story["areas"].as_array().into_iter().flatten().map(input_area).collect::<Result<Vec<_>, _>>()?;
        let fields = nixe_ui::widgets::vbox(6);
        b.append(&fields);
        let areas = area_picker_initial(&fields, &d, &s, initial);
        submit(&d, &b, &fields, s, "Save interactive areas", move |s| {
            let (areas, original) = (areas.borrow().clone(), story["areas"].clone());
            async move {
                let current = s.call("getStory", json!({"story_poster_chat_id":chat,"story_id":id,"only_local":false})).await?;
                if current["can_be_edited"] != true || current["areas"] != original {
                    return Err("Story areas changed. Reopen the editor.".into());
                }
                crate::groups::commit(&s,"editStory",json!({"story_poster_chat_id":chat,"story_id":id,"content":null,"areas":{"@type":"inputStoryAreas","areas":areas},"caption":null})).await?;
                Ok(())
            }
        });
        Ok(())
    });
}
