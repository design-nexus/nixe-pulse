//! Story browsing, media, publication, privacy and interactions.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::{glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
thread_local! {
static ACTIVE:RefCell<BTreeMap<i64,Value>>=const{
RefCell::new(BTreeMap::new())}
;
}
pub fn reset() {
    ACTIVE.with(|v| v.borrow_mut().clear());
}
pub fn update(v: &Value) {
    if v["@type"] == "updateChatActiveStories" {
        let a = &v["active_stories"];
        if let Some(id) = a["chat_id"].as_i64() {
            ACTIVE.with(|v| {
                v.borrow_mut().insert(id, a.clone());
            });
        }
    }
}
pub fn browse() {
    features::load("Stories", |s, d, b| async move {
        let _ = s
            .call(
                "loadActiveStories",
                json!({
                "story_list":{
                "@type":"storyListMain"}
                }
                ),
            )
            .await;
        let me = s.call("getMe", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let id = me["id"].as_i64().unwrap_or(0);
        button(&b, "My story albums", move || crate::story_extras::albums(id));
        button(&b, "Stealth mode…", crate::story_extras::stealth);
        button(&b, "My active stories", move || chat_stories(id, false));
        button(&b, "My story archive", move || chat_stories(id, true));
        button(&b, "Post a story…", move || post(id));
        let list = nyx_ui::widgets::vbox(6);
        b.append(&list);
        let refresh = gtk::Button::with_label("Refresh active stories");
        b.append(&refresh);
        let scope = s.clone();
        refresh.connect_clicked(move |_| {
            if !scope.valid() {
                return;
            }
            while let Some(c) = list.first_child() {
                list.remove(&c)
            }
            let mut values = ACTIVE.with(|v| v.borrow().values().cloned().collect::<Vec<_>>());
            values.sort_by_key(|v| std::cmp::Reverse(v["order"].as_i64().unwrap_or(0)));
            for a in values {
                let Some(id) = a["chat_id"].as_i64() else { continue };
                if a["stories"].as_array().is_none_or(|a| a.is_empty()) {
                    continue;
                }
                let title = window::chat(id).map(|c| c.title).unwrap_or_else(|| format!("Chat {id}"));
                button(&list, &title, move || chat_stories(id, false));
            }
        });
        refresh.emit_clicked();
        note(&b, "Refresh loads received story updates. Open any chat's Stories action to fetch that chat directly.");
        Ok(())
    });
}
pub fn chat_stories(chat: i64, archive: bool) {
    features::load(if archive { "Story archive" } else { "Stories" }, move |s, d, b| async move {
        let list = nyx_ui::widgets::vbox(6);
        b.append(&list);
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let from = Rc::new(Cell::new(0));
        let weak = d.downgrade();
        more.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, weak, list, from, button) = (s.clone(), weak.clone(), list.clone(), from.clone(), button.clone());
            glib::spawn_future_local(async move {
                let result = if archive {
                    s.call(
                        "getChatArchivedStories",
                        json!({
                        "chat_id":chat,"from_story_id":from.get(),"limit":50}
                        ),
                    )
                    .await
                } else {
                    s.call(
                        "getChatActiveStories",
                        json!({
                        "chat_id":chat}
                        ),
                    )
                    .await
                };
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
                        for story in &rows {
                            let id = story["story_id"].as_i64().or(story["id"].as_i64()).unwrap_or(0);
                            let scope = s.clone();
                            let caption = story["caption"]["text"].as_str().unwrap_or("");
                            button_story(&list, &format!("Story {id} {caption}"), scope, chat, id);
                        }
                        let next = rows.iter().filter_map(|v| v["id"].as_i64()).min().unwrap_or(0);
                        button.set_visible(archive && !rows.is_empty() && next != from.get());
                        from.set(next);
                        if rows.is_empty() {
                            note(&list, "No stories available.");
                        }
                    }
                }
            });
        });
        more.emit_clicked();
        Ok(())
    });
}
fn button_story(b: &gtk::Box, title: &str, s: Scope, chat: i64, id: i64) {
    button(b, title, move || {
        if s.valid() {
            view(chat, id)
        }
    });
}
fn media(story: &Value) -> Option<Value> {
    let c = &story["content"];
    match c["@type"].as_str()? {
        "storyContentPhoto" => Some(json!({
        "@type":"pollMediaPhoto","photo":c["photo"]}
        )),
        "storyContentVideo" => Some(json!({
        "@type":"pollMediaVideo","video":c["video"]}
        )),
        _ => None,
    }
}
pub fn view(chat: i64, id: i64) {
    features::load("Story", move |s, d, b| async move {
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
        s.call(
            "openStory",
            json!({
            "story_poster_chat_id":chat,"story_id":id}
            ),
        )
        .await?;
        let scope = s.clone();
        d.connect_close_request(move |_| {
            if scope.valid() {
                let scope = scope.clone();
                glib::spawn_future_local(async move {
                    let _ = scope
                        .call(
                            "closeStory",
                            json!({
                            "story_poster_chat_id":chat,"story_id":id}
                            ),
                        )
                        .await;
                });
            }
            glib::Propagation::Proceed
        });
        note(&b, story["caption"]["text"].as_str().unwrap_or(""));
        if let Some(media) = media(&story) {
            let raw = json!({
            "id":id,"chat_id":chat,"date":story["date"],"can_be_saved":story["can_be_forwarded"]==true,"has_protected_content":story["can_be_forwarded"]!=true,"content":{
            "@type":"messageText","text":{
            "text":"Story","entities":[]}
            }
            }
            );
            if let Some(message) = nyx_telegram_core::client::parse_message(&raw) {
                crate::poll_media::render(&b, &message, &media);
            }
        } else {
            if story["content"]["@type"] == "storyContentLive" {
                let call = story["content"]["group_call_id"].as_i64().unwrap_or(0);
                button(&b, "Watch live stream", move || crate::group_calls::join_live(chat, call));
            } else {
                note(&b, "Story media is unavailable.");
            }
        }
        let scope = s.clone();
        button(&b, "Story tools…", move || crate::story_extras::tools(chat, id));
        button(&b, "React…", move || {
            if scope.valid() {
                react(chat, id);
            }
        });
        if story["can_be_replied"] == true {
            let scope = s.clone();
            button(&b, "Reply…", move || {
                if scope.valid() {
                    reply(chat, id);
                }
            });
        }
        if story["can_be_edited"] == true {
            let scope = s.clone();
            let caption = story["caption"]["text"].as_str().unwrap_or("").to_string();
            button(&b, "Edit caption…", move || {
                if scope.valid() {
                    caption_edit(chat, id, caption.clone());
                }
            });
        }
        if story["can_set_privacy_settings"] == true {
            let scope = s.clone();
            let original = story["privacy_settings"].clone();
            button(&b, "Privacy…", move || {
                if scope.valid() {
                    privacy_edit(chat, id, original.clone());
                }
            });
        }
        if story["can_toggle_is_posted_to_chat_page"] == true {
            let posted = story["is_posted_to_chat_page"] == true;
            let scope = s.clone();
            button(&b, if posted { "Remove from profile" } else { "Show on profile" }, move || {
                if !scope.valid() {
                    return;
                }
                actions::run(chat, "story-profile", move |s| async move {
                    let current = fresh(&s, chat, id, "can_toggle_is_posted_to_chat_page").await?;
                    if current["is_posted_to_chat_page"] != posted {
                        return Err("Story profile visibility changed. Reopen it.".into());
                    }
                    s.call(
                        "toggleStoryIsPostedToChatPage",
                        json!({
                        "story_poster_chat_id":chat,"story_id":id,"is_posted_to_chat_page":!posted}
                        ),
                    )
                    .await?;
                    Ok(())
                });
            });
        }
        if story["can_be_deleted"] == true {
            let scope = s.clone();
            button(&b, "Delete story…", move || {
                if !scope.valid() {
                    return;
                }
                actions::operation(
                    "Delete story?",
                    "This permanently removes the story.",
                    vec![],
                    "Delete",
                    move |s, _| async move {
                        fresh(&s, chat, id, "can_be_deleted").await?;
                        s.call(
                            "deleteStory",
                            json!({
                            "story_poster_chat_id":chat,"story_id":id}
                            ),
                        )
                        .await?;
                        Ok(())
                    },
                );
            });
        }
        Ok(())
    });
}
async fn fresh(s: &Scope, chat: i64, id: i64, right: &str) -> Result<Value, String> {
    let story = s
        .call(
            "getStory",
            json!({
            "story_poster_chat_id":chat,"story_id":id,"only_local":false}
            ),
        )
        .await?;
    if story[right] != true {
        return Err("Telegram no longer permits this story action.".into());
    }
    Ok(story)
}
fn privacy_fields(b: &gtk::Box, original: &Value) -> (gtk::DropDown, gtk::Entry) {
    let kind = gtk::DropDown::from_strings(&["Everyone", "Contacts", "Close friends", "Selected users"]);
    kind.set_selected(match original["@type"].as_str() {
        Some("storyPrivacySettingsContacts") => 1,
        Some("storyPrivacySettingsCloseFriends") => 2,
        Some("storyPrivacySettingsSelectedUsers") => 3,
        _ => 0,
    });
    b.append(&kind);
    let ids = entry(
        b,
        "Excluded users, or selected users (IDs)",
        &features::id_text(if original["@type"] == "storyPrivacySettingsSelectedUsers" {
            &original["user_ids"]
        } else {
            &original["except_user_ids"]
        }),
        4096,
    );
    (kind, ids)
}
fn privacy(kind: u32, ids: Vec<i64>) -> Value {
    match kind {
        1 => json!({
        "@type":"storyPrivacySettingsContacts","except_user_ids":ids}
        ),
        2 => json!({
        "@type":"storyPrivacySettingsCloseFriends"}
        ),
        3 => json!({
        "@type":"storyPrivacySettingsSelectedUsers","user_ids":ids}
        ),
        _ => json!({
        "@type":"storyPrivacySettingsEveryone","except_user_ids":ids}
        ),
    }
}
fn privacy_edit(chat: i64, id: i64, original: Value) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Story privacy", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let (kind, ids) = privacy_fields(&fields, &original);
    submit(&d, &b, &fields, s, "Save privacy", move |s| {
        let (kind, ids, original) = (kind.selected(), features::ids(&ids.text()), original.clone());
        async move {
            let story = fresh(&s, chat, id, "can_set_privacy_settings").await?;
            if story["privacy_settings"] != original {
                return Err("Story privacy changed. Reopen the editor.".into());
            }
            s.call(
                "setStoryPrivacySettings",
                json!({
                "story_id":id,"privacy_settings":privacy(kind,ids?)}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
fn caption_edit(chat: i64, id: i64, caption: String) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Story caption", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let text = entry(&fields, "Caption", &caption, 4096);
    submit(&d, &b, &fields, s, "Save caption", move |s| {
        let text = text.text().to_string();
        async move {
            fresh(&s, chat, id, "can_be_edited").await?;
            s.call(
                "editStory",
                json!({
                "story_poster_chat_id":chat,"story_id":id,"content":null,"areas":null,"caption":{
                "@type":"formattedText","text":text,"entities":[]}
                }
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
fn react(chat: i64, id: i64) {
    features::load("Story reaction", move |s, d, b| async move {
        let data = s
            .call(
                "getStoryAvailableReactions",
                json!({
                "row_size":8}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let mut reactions = Vec::new();
        for group in ["top_reactions", "recent_reactions", "popular_reactions"] {
            for r in data[group].as_array().into_iter().flatten() {
                let reaction = r["type"].clone();
                if !reaction.is_null() && !reactions.contains(&reaction) {
                    reactions.push(reaction);
                }
            }
        }
        reactions.insert(0, Value::Null);
        for reaction in reactions {
            let title = reaction["emoji"].as_str().map(str::to_owned).unwrap_or_else(|| {
                if reaction.is_null() {
                    "Remove reaction".into()
                } else {
                    format!("Custom emoji {}", reaction["custom_emoji_id"])
                }
            });
            let scope = s.clone();
            button(&b, &title, move || {
                if !scope.valid() {
                    return;
                }
                let reaction = reaction.clone();
                actions::run(chat, "story-react", move |s| async move {
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
        Ok(())
    });
}
fn reply(chat: i64, id: i64) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Reply to story", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let text = entry(&fields, "Reply", "", 4096);
    submit(&d, &b, &fields, s, "Send reply", move |s| {
        let text = text.text().to_string();
        async move {
            if text.trim().is_empty() {
                return Err("Enter a reply.".into());
            }
            fresh(&s, chat, id, "can_be_replied").await?;
            crate::groups::commit(
                &s,
                "sendMessage",
                json!({
                "chat_id":chat,"topic_id":null,"reply_to":{
                "@type":"inputMessageReplyToStory","story_poster_chat_id":chat,"story_id":id}
                ,"options":{
                "@type":"messageSendOptions","paid_message_star_count":0}
                ,"input_message_content":{
                "@type":"inputMessageText","text":{
                "@type":"formattedText","text":text,"entities":[]}
                ,"clear_draft":false}
                }
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
pub fn post(chat: i64) {
    features::load("Post story", move |s, d, b| async move {
        let allowed = s
            .call(
                "canPostStory",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        if allowed["@type"] != "canPostStoryResultOk" {
            return Err(format!(
                "Story posting unavailable: {}",
                features::pretty(allowed["@type"].as_str().unwrap_or("unknown reason"))
            ));
        }
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let media = crate::poll_media::story_picker(&fields, &d, &s, "Photo or H.264 MP4 video");
        let caption = entry(&fields, "Caption", "", 4096);
        note(&fields, "Telegram enforces story duration and Premium eligibility. Video must meet Telegram story limits.");
        let (kind, ids) = privacy_fields(
            &fields,
            &json!({
            "@type":"storyPrivacySettingsContacts"}
            ),
        );
        let lifetime =
            gtk::DropDown::from_strings(&["24 hours", "6 hours (Premium)", "12 hours (Premium)", "48 hours (Premium)"]);
        fields.append(&lifetime);
        let areas = crate::story_extras::area_picker(&fields, &d, &s);
        let albums = entry(&fields, "Album IDs (optional)", "", 512);
        let cover = entry(&fields, "Video cover timestamp (seconds)", "0", 16);
        let profile = check(&fields, "Keep on profile after expiration", false);
        let protect = check(&fields, "Protect from forwarding and saving", false);
        submit(&d, &b, &fields, s, "Post story", move |s| {
            let (media, caption, kind, ids, profile, protect) = (
                media.borrow().as_ref().map(|a| a.input()).transpose(),
                caption.text().to_string(),
                kind.selected(),
                features::ids(&ids.text()),
                profile.is_active(),
                protect.is_active(),
            );
            let lifetime = [86400, 21600, 43200, 172800][lifetime.selected() as usize];
            let areas = areas.borrow().clone();
            let albums = features::ids(&albums.text());
            let cover = cover.text().parse::<f64>();
            async move {
                let cover = cover.map_err(|_| "Invalid cover timestamp.")?;
                if !cover.is_finite() || cover < 0. {
                    return Err("Cover timestamp must be nonnegative.".into());
                }
                let media = media?.ok_or("Choose story media.")?;
                if media["@type"] == "inputPollMediaVideo"
                    && !media["video"]["duration"].as_i64().is_some_and(|n| n > 0 && n <= 60)
                {
                    return Err("Story videos must have a usable duration of at most 60 seconds.".into());
                }
                let content = if media["@type"] == "inputPollMediaPhoto" {
                    json!({
                    "@type":"inputStoryContentPhoto","photo":media["photo"]["photo"],"added_sticker_file_ids":[]}
                    )
                } else if media["@type"] == "inputPollMediaVideo" {
                    json!({
                    "@type":"inputStoryContentVideo","video":media["video"]["video"],"added_sticker_file_ids":[],"duration":media["video"]["duration"],"cover_frame_timestamp":cover,"is_animation":false}
                    )
                } else {
                    return Err("Choose a photo or MP4 video.".into());
                };
                if s.call(
                    "canPostStory",
                    json!({
                    "chat_id":chat}
                    ),
                )
                .await?["@type"]
                    != "canPostStoryResultOk"
                {
                    return Err("Story posting eligibility changed.".into());
                }
                crate::groups::commit(&s,"postStory",json!({
"chat_id":chat,"content":content,"areas":{
"@type":"inputStoryAreas","areas":areas}
,"caption":{
"@type":"formattedText","text":caption,"entities":[]}
,"privacy_settings":privacy(kind,ids?),"album_ids":albums?,"active_period":lifetime,"from_story_full_id":null,"is_posted_to_chat_page":profile,"protect_content":protect}
)).await?;
                Ok(())
            }
        });
        Ok(())
    });
}
