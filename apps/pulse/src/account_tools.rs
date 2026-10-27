//! Profile, contacts, privacy, session and notification management.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::{glib, prelude::*};
use serde_json::{Value, json};

pub fn open() {
    features::load("Account and privacy", |s, d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(
            &b,
            &format!(
                "{} {}",
                me["first_name"].as_str().unwrap_or(""),
                me["last_name"].as_str().unwrap_or("")
            ),
        );
        button(&b, "Edit profile", profile);
        button(&b, "Contacts", contacts);
        button(&b, "Sessions and connected websites", sessions);
        button(&b, "Privacy and exceptions", privacy);
        button(&b, "Two-step verification and recovery email", crate::auth_tools::security);
        button(&b, "Notification scopes and exceptions", notifications);
        button(&b, "Blocked users", blocked);
        button(&b, "Account inactivity and session lifetime", lifetime);
        Ok(())
    });
}
pub fn profile() {
    features::load("Edit profile", |s, d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        let info = s
            .call(
                "getUserFullInfo",
                json!({
                "user_id":me["id"]}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let first = entry(&fields, "First name", me["first_name"].as_str().unwrap_or(""), 64);
        let last = entry(&fields, "Last name", me["last_name"].as_str().unwrap_or(""), 64);
        let bio = entry(&fields, "Bio", info["bio"]["text"].as_str().unwrap_or(""), 512);
        let username = entry(
            &fields,
            "Username",
            me["usernames"]["editable_username"].as_str().unwrap_or(""),
            32,
        );
        let s_ = s.clone();
        button(&fields, "Choose profile photo…", move || photo(s_.clone()));
        submit(&d, &b, &fields, s, "Save profile", move |s| {
            let (first, last, bio, username) = (
                first.text().trim().to_owned(),
                last.text().trim().to_owned(),
                bio.text().to_string(),
                username.text().trim().trim_start_matches('@').to_owned(),
            );
            let (original_me, original_info) = (me.clone(), info.clone());
            async move {
                if first.is_empty() {
                    return Err("Enter your first name.".into());
                }
                let fresh = s.call("getMe", json!({})).await?;
                let current = s.call("getUserFullInfo", json!({"user_id":fresh["id"]})).await?;
                if ["first_name", "last_name", "usernames"]
                    .iter()
                    .any(|key| fresh[*key] != original_me[*key])
                    || current["bio"] != original_info["bio"]
                {
                    return Err("Your profile changed on another device. Reopen the editor.".into());
                }
                s.call(
                    "setName",
                    json!({
                    "first_name":first,"last_name":last}
                    ),
                )
                .await?;
                s.call(
                    "setBio",
                    json!({
                    "bio":bio}
                    ),
                )
                .await?;
                s.call(
                    "setUsername",
                    json!({
                    "username":username}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
fn photo(scope: Scope) {
    let (d, b) = nyx_ui::widgets::dialog("Profile photo", 440);
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let selected = crate::poll_media::picker(&fields, &d, &scope, "Choose a photo", true);
    submit(&d, &b, &fields, scope, "Set profile photo", move |s| {
        let selected = selected.borrow().as_ref().map(|a| a.input()).transpose();
        async move {
            let photo = selected?.ok_or("Choose a photo.")?;
            if photo["@type"] != "inputPollMediaPhoto" {
                return Err("Choose a still photo for your profile.".into());
            }
            crate::groups::commit(
                &s,
                "setProfilePhoto",
                json!({
                "photo":{
                "@type":"inputChatPhotoStatic","photo":photo["photo"]["photo"]}
                ,"is_public":false}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
pub fn contacts() {
    features::load("Contacts", |s, d, b| async move {
        let query = entry(&b, "Find contact", "", 128);
        let list = nyx_ui::widgets::vbox(6);
        b.append(&list);
        let search = gtk::Button::with_label("Search");
        b.append(&search);
        let weak = d.downgrade();
        let s_ = s.clone();
        search.connect_clicked(move |button| {
            if !s_.valid() {
                return;
            }
            let (s, list, weak, button, query) = (
                s_.clone(),
                list.clone(),
                weak.clone(),
                button.clone(),
                query.text().to_string(),
            );
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                let result = s
                    .call(
                        "searchContacts",
                        json!({
                        "query":query,"limit":100}
                        ),
                    )
                    .await;
                button.set_sensitive(true);
                if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                while let Some(c) = list.first_child() {
                    list.remove(&c)
                }
                match result {
                    Err(e) => {
                        note(&list, &e);
                    }
                    Ok(data) => {
                        for id in data["user_ids"].as_array().into_iter().flatten() {
                            let Ok(user) = s
                                .call(
                                    "getUser",
                                    json!({
                                    "user_id":id}
                                    ),
                                )
                                .await
                            else {
                                continue;
                            };
                            if !s.valid() {
                                return;
                            }
                            contact_row(&list, &s, &user);
                        }
                        note(&list, "Showing up to 100 contacts. Narrow the search for more.");
                    }
                }
            });
        });
        search.emit_clicked();
        button(&b, "Add contact…", add_contact);
        Ok(())
    });
}
fn contact_row(b: &gtk::Box, s: &Scope, u: &Value) {
    let id = u["id"].as_i64().unwrap_or(0);
    let row = nyx_ui::widgets::vbox(3);
    note(
        &row,
        &format!(
            "{} {} · +{}",
            u["first_name"].as_str().unwrap_or(""),
            u["last_name"].as_str().unwrap_or(""),
            u["phone_number"].as_str().unwrap_or("")
        ),
    );
    let s_ = s.clone();
    button(&row, "Open conversation", move || {
        if !s_.valid() {
            return;
        }
        actions::run(0, "contact-open", move |s| async move {
            let chat = s
                .call(
                    "createPrivateChat",
                    json!({
                    "user_id":id,"force":false}
                    ),
                )
                .await?;
            if let Some(id) = chat["id"].as_i64() {
                window::open_chat(id);
            }
            Ok(())
        });
    });
    let s_ = s.clone();
    button(&row, "Remove contact…", move || {
        if !s_.valid() {
            return;
        }
        actions::operation(
            "Remove contact?",
            "This removes the contact from Telegram. It does not delete the conversation.",
            vec![],
            "Remove",
            move |s, _| async move {
                s.call(
                    "removeContacts",
                    json!({
                    "user_ids":[id]}
                    ),
                )
                .await?;
                Ok(())
            },
        );
    });
    b.append(&row);
}
fn add_contact() {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Add contact", 440);
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let first = entry(&fields, "First name", "", 64);
    let last = entry(&fields, "Last name", "", 64);
    let phone = entry(&fields, "Phone including country code", "", 32);
    submit(&d, &b, &fields, s, "Add contact", move |s| {
        let (first, last, phone) = (
            first.text().trim().to_string(),
            last.text().to_string(),
            phone.text().trim().to_string(),
        );
        async move {
            if first.is_empty() || phone.chars().filter(char::is_ascii_digit).count() < 5 {
                return Err("Enter a first name and phone number with country code.".into());
            }
            crate::groups::commit(
                &s,
                "importContacts",
                json!({
                "contacts":[{
                "@type":"importedContact","phone_number":phone,"first_name":first,"last_name":last}
                ]}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
pub fn sessions() {
    features::load("Sessions and websites", |s, d, b| async move {
        let data = s.call("getActiveSessions", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        for session in data["sessions"].as_array().into_iter().flatten() {
            note(
                &b,
                &format!(
                    "{} · {}{}\n{} · {}",
                    session["application_name"].as_str().unwrap_or("Session"),
                    session["device_model"].as_str().unwrap_or(""),
                    if session["is_current"] == true {
                        " · this device"
                    } else {
                        ""
                    },
                    session["location"].as_str().unwrap_or(""),
                    session["ip_address"].as_str().unwrap_or("")
                ),
            );
            if session["is_current"] != true {
                let id = session["id"].clone();
                let scope = s.clone();
                button(&b, "Revoke session…", move || {
                    if !scope.valid() {
                        return;
                    }
                    let id = id.clone();
                    actions::operation(
                        "Revoke session?",
                        "This signs the other device out of Telegram.",
                        vec![],
                        "Revoke",
                        move |s, _| {
                            let id = id.clone();
                            async move {
                                s.call(
                                    "terminateSession",
                                    json!({
                                    "session_id":id}
                                    ),
                                )
                                .await?;
                                Ok(())
                            }
                        },
                    );
                });
            }
        }
        let scope = s.clone();
        button(&b, "Revoke all other sessions…", move || {
            if scope.valid() {
                actions::operation(
                    "Revoke all other sessions?",
                    "Every other device will be signed out.",
                    vec![],
                    "Revoke sessions",
                    |s, _| async move {
                        s.call("terminateAllOtherSessions", json!({})).await?;
                        Ok(())
                    },
                );
            }
        });
        let websites = s.call("getConnectedWebsites", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, "Connected websites");
        for site in websites["websites"].as_array().into_iter().flatten() {
            let id = site["id"].clone();
            let scope = s.clone();
            let title = format!("Disconnect {}…", site["domain_name"].as_str().unwrap_or("website"));
            button(&b, &title, move || {
                if !scope.valid() {
                    return;
                }
                let id = id.clone();
                actions::operation(
                    "Disconnect website?",
                    "The website will lose its Telegram authorization.",
                    vec![],
                    "Disconnect",
                    move |s, _| {
                        let id = id.clone();
                        async move {
                            s.call(
                                "disconnectWebsite",
                                json!({
                                "website_id":id}
                                ),
                            )
                            .await?;
                            Ok(())
                        }
                    },
                );
            });
        }
        Ok(())
    });
}
const PRIVACY: &[(&str, &str)] = &[
    ("Last seen and online", "ShowStatus"),
    ("Profile photo", "ShowProfilePhoto"),
    ("Forwarded-message attribution", "ShowLinkInForwardedMessages"),
    ("Phone number", "ShowPhoneNumber"),
    ("Bio", "ShowBio"),
    ("Birthday", "ShowBirthdate"),
    ("Group invitations", "AllowChatInvites"),
    ("Calls", "AllowCalls"),
    ("Peer-to-peer calls", "AllowPeerToPeerCalls"),
    ("Find by phone", "AllowFindingByPhoneNumber"),
    ("Voice and video notes", "AllowPrivateVoiceAndVideoNoteMessages"),
    ("Unpaid messages", "AllowUnpaidMessages"),
];
pub fn privacy() {
    features::load("Privacy and exceptions", |s, _d, b| async move {
        for (title, typ) in PRIVACY {
            let typ = format!("userPrivacySetting{typ}");
            let scope = s.clone();
            let title = title.to_string();
            let name = title.clone();
            button(&b, &title, move || {
                if scope.valid() {
                    privacy_edit(&name, &typ);
                }
            });
        }
        Ok(())
    });
}
pub fn privacy_edit(title: &str, setting: &str) {crate::privacy_tools::edit(title,setting);}
pub fn notifications() {
    features::load("Notification settings", |s, _d, b| async move {
        for (label, typ) in [
            ("Private chats", "notificationSettingsScopePrivateChats"),
            ("Groups", "notificationSettingsScopeGroupChats"),
            ("Channels", "notificationSettingsScopeChannelChats"),
        ] {
            let scope = s.clone();
            button(&b, label, move || {
                if scope.valid() {
                    notification_scope(typ);
                }
            });
        }
        Ok(())
    });
}
fn notification_scope(typ: &str) {
    let typ = typ.to_string();
    features::load("Notification scope", move |s, d, b| async move {
        let key = json!({
        "@type":typ}
        );
        let original = s
            .call(
                "getScopeNotificationSettings",
                json!({
                "scope":key}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let preview = check(&fields, "Show message previews", original["show_preview"] == true);
        let mentions = check(
            &fields,
            "Disable mention notifications",
            original["disable_mention_notifications"] == true,
        );
        let pins = check(
            &fields,
            "Disable pinned-message notifications",
            original["disable_pinned_message_notifications"] == true,
        );
        let mute = check(&fields, "Mute this category", original["mute_for"].as_i64().unwrap_or(0) > 0);
        let exceptions = s
            .call(
                "getChatNotificationSettingsExceptions",
                json!({
                "scope":key,"compare_sound":true}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&fields, "Exceptions (open a conversation to adjust its notifications)");
        for id in exceptions["chat_ids"].as_array().into_iter().flatten() {
            if let Some(id) = id.as_i64() {
                let title = window::chat(id).map(|c| c.title).unwrap_or_else(|| format!("Chat {id}"));
                let scope = s.clone();
                button(&fields, &title, move || {
                    if scope.valid() {
                        window::open_chat(id);
                    }
                });
            }
        }
        submit(&d, &b, &fields, s, "Save notifications", move |s| {
            let (mut settings, key) = (original.clone(), key.clone());
            settings["show_preview"] = json!(preview.is_active());
            settings["disable_mention_notifications"] = json!(mentions.is_active());
            settings["disable_pinned_message_notifications"] = json!(pins.is_active());
            settings["mute_for"] = json!(if mute.is_active() { i32::MAX } else { 0 });
            async move {
                s.call(
                    "setScopeNotificationSettings",
                    json!({
                    "scope":key,"notification_settings":settings}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
pub fn blocked() {
    features::load("Blocked users", |s, d, b| async move {
        let list = nyx_ui::widgets::vbox(4);
        b.append(&list);
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let offset = std::rc::Rc::new(std::cell::Cell::new(0));
        let weak = d.downgrade();
        more.connect_clicked(move |button| {
            if !s.valid() {
                return;
            }
            button.set_sensitive(false);
            let (s, list, offset, weak, button) = (s.clone(), list.clone(), offset.clone(), weak.clone(), button.clone());
            glib::spawn_future_local(async move {
                let result = s
                    .call(
                        "getBlockedMessageSenders",
                        json!({
                        "block_list":{
                        "@type":"blockListMain"}
                        ,"offset":offset.get(),"limit":50}
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
                        let rows = data["senders"].as_array().cloned().unwrap_or_default();
                        offset.set(offset.get() + rows.len() as i32);
                        for sender in rows {
                            let title = if let Some(id) = sender["user_id"].as_i64() {
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
                                format!("Chat {}", sender["chat_id"])
                            };
                            if !s.valid() {
                                return;
                            }
                            let scope = s.clone();
                            features::button(&list, &format!("Unblock {title}"), move || {
                                if scope.valid() {
                                    let sender = sender.clone();
                                    actions::run(0, "unblock", move |s| async move {
                                        s.call(
                                            "setMessageSenderBlockList",
                                            json!({
                                            "sender_id":sender,"block_list":null}
                                            ),
                                        )
                                        .await?;
                                        Ok(())
                                    });
                                }
                            });
                        }
                        button.set_visible(!data["senders"].as_array().is_none_or(|a| a.len() < 50));
                    }
                }
            });
        });
        more.emit_clicked();
        Ok(())
    });
}
fn lifetime() {
    features::load("Account inactivity", |s, d, b| async move {
        let ttl = s.call("getAccountTtl", json!({})).await?;
        let sessions = s.call("getActiveSessions", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        note(&fields, "Telegram deletes an inactive account after this many days.");
        let account = gtk::SpinButton::with_range(30., 730., 1.);
        account.set_value(ttl["days"].as_f64().unwrap_or(365.));
        fields.append(&account);
        note(&fields, "Revoke sessions inactive for this many days");
        let session = gtk::SpinButton::with_range(1., 366., 1.);
        session.set_value(sessions["inactive_session_ttl_days"].as_f64().unwrap_or(180.));
        fields.append(&session);
        submit(&d, &b, &fields, s, "Save inactivity settings", move |s| {
            let (a, b) = (account.value_as_int(), session.value_as_int());
            async move {
                s.call(
                    "setAccountTtl",
                    json!({
                    "ttl":{
                    "@type":"accountTtl","days":a}
                    }
                    ),
                )
                .await?;
                s.call(
                    "setInactiveSessionTtl",
                    json!({
                    "inactive_session_ttl_days":b}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
