//! Focused, permission-checked group/channel settings; one mutation per confirmation.
use crate::{
    actions::{self, Scope},
    groups::{self, entry, note, scroll, submit},
    window,
};
use gtk::{gio, prelude::*};
use nyx_ui::widgets as w;
use serde_json::{Value, json};

async fn full(s: &Scope, c: &groups::Context) -> Result<Value, String> {
    s.call("getSupergroupFullInfo", json!({"supergroup_id":c.supergroup().ok_or("Upgrade this basic group first.")?})).await
}
fn info(c: &groups::Context) -> Result<(), String> {
    if c.right("can_change_info") || c.chat["permissions"]["can_change_info"] == true {
        Ok(())
    } else {
        Err("You cannot change this chat's information.".into())
    }
}
fn refresh(chat: i64) {
    window::with_service(|s| s.load_chat(chat));
}
fn action(body: &gtk::Box, title: &str, s: &Scope, work: impl Fn() + 'static) {
    let b = gtk::Button::with_label(title);
    let s = s.clone();
    b.connect_clicked(move |_| {
        if s.valid() {
            work()
        }
    });
    body.append(&b);
}
pub fn open(chat: i64) {
    actions::run(chat, "group-settings", move |s| async move {
        let c = groups::context(&s, chat).await?;
        let (dialog, body) = w::dialog("Group / channel settings", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Close");
        let list = scroll(&body);
        if info(&c).is_ok() {
            action(&list, "Change photo…", &s, move || photo(chat));
            action(&list, "Remove photo…", &s, move || remove_photo(chat));
        }
        if let Some(_) = c.supergroup() {
            let f = full(&s, &c).await?;
            if c.owner() {
                action(&list, "Public username…", &s, move || username(chat));
                action(&list, "Content protection…", &s, move || toggle(chat, Setting::Protection));
                if !c.channel() {
                    action(&list, "Enable / disable topics…", &s, move || toggle(chat, Setting::Forum));
                }
            }
            if info(&c).is_ok() {
                if c.channel() {
                    action(&list, "Discussion group…", &s, move || discussion(chat));
                    action(&list, "Post signatures…", &s, move || signatures(chat));
                } else {
                    action(&list, "History for new members…", &s, move || toggle(chat, Setting::History));
                }
            }
            if !c.channel() && c.right("can_restrict_members") {
                action(&list, "Slow mode…", &s, move || slow_mode(chat));
                action(&list, "Require joining to send…", &s, move || toggle(chat, Setting::JoinToSend));
                action(&list, "Approve new members…", &s, move || toggle(chat, Setting::JoinApproval));
            }
            if f["can_hide_members"] == true {
                action(&list, "Hide member list…", &s, move || toggle(chat, Setting::HiddenMembers));
            }
            if f["can_toggle_aggressive_anti_spam"] == true {
                action(&list, "Aggressive anti-spam…", &s, move || toggle(chat, Setting::AntiSpam));
            }
        } else if c.owner() {
            action(&list, "Content protection…", &s, move || toggle(chat, Setting::Protection));
            action(&list, "Upgrade to supergroup…", &s, move || upgrade(chat));
        }
        if list.first_child().is_none() {
            note(&list, "No settings are available with your current permissions.");
        }
        dialog.present();
        Ok(())
    });
}
fn username(chat: i64) {
    actions::run(chat, "group-username", move |s| async move {
        let c = groups::context(&s, chat).await?;
        if !c.owner() {
            return Err("Only the owner can change the public username.".into());
        }
        let original = c.group["usernames"]["editable_username"].as_str().unwrap_or("").to_string();
        let (dialog, body) = w::dialog("Public username", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        note(
            &fields,
            "Anyone can find a public chat by its username. Leave blank to remove the editable username; other active usernames may still keep the chat public.",
        );
        let value = entry(&fields, "Username", &original, 32);
        submit(&dialog, &body, &fields, s, "Save username", move |s| {
            let value = value.text().trim().trim_start_matches('@').to_string();
            let original = original.clone();
            async move {
                let c = groups::context(&s, chat).await?;
                if !c.owner() {
                    return Err("You are no longer the owner.".into());
                }
                if c.group["usernames"]["editable_username"].as_str().unwrap_or("") != original {
                    return Err("The username changed elsewhere. Reopen the editor.".into());
                }
                if !value.is_empty() && value != original {
                    let check = s.call("checkChatUsername", json!({"chat_id":chat,"username":value})).await?;
                    if check["@type"] != "checkChatUsernameResultOk" {
                        return Err(match check["@type"].as_str() {
                            Some("checkChatUsernameResultUsernameOccupied") => "That username is already taken.",
                            Some("checkChatUsernameResultUsernamePurchasable") => {
                                "That username requires a purchase through Telegram."
                            }
                            Some("checkChatUsernameResultPublicChatsTooMany") => {
                                "Your account has reached Telegram's public chat limit."
                            }
                            _ => "Telegram cannot assign this username. Check its format and your account's eligibility.",
                        }
                        .into());
                    }
                }
                s.call("setSupergroupUsername", json!({"supergroup_id":c.supergroup(),"username":value})).await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn photo(chat: i64) {
    let Some(scope) = Scope::current() else { return };
    let chooser = gtk::FileDialog::builder().title("Choose group / channel photo").build();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    filter.add_pixbuf_formats();
    filters.append(&filter);
    chooser.set_filters(Some(&filters));
    chooser.open(w::main_window().as_ref(),gio::Cancellable::NONE,move |result|{
        if !scope.valid(){return}let Ok(file)=result else{return};let Some(path)=file.path()else{return};
        let texture=match gtk::gdk::Texture::from_file(&file){Ok(t)=>t,Err(e)=>{scope.error(&format!("Unable to read image: {e}"));return}};
        let (dialog,body)=w::dialog("Change chat photo",420);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");let fields=w::vbox(8);body.append(&fields);
        let image=gtk::Picture::for_paintable(&texture);image.set_can_shrink(true);image.set_size_request(180,180);fields.append(&image);
        submit(&dialog,&body,&fields,scope.clone(),"Set photo",move |s|{let path=path.clone();async move{
            info(&groups::context(&s,chat).await?)?;
            s.call("setChatPhoto",json!({"chat_id":chat,"photo":{"@type":"inputChatPhotoStatic","photo":{"@type":"inputFileLocal","path":path.to_string_lossy()}}})).await?;refresh(chat);Ok(())
        }});dialog.present();
    });
}
fn remove_photo(chat: i64) {
    actions::operation(
        "Remove chat photo?",
        "Remove the group's or channel's current photo?",
        vec![],
        "Remove photo",
        move |s, _| async move {
            info(&groups::context(&s, chat).await?)?;
            s.call("setChatPhoto", json!({"chat_id":chat,"photo":null})).await?;
            refresh(chat);
            Ok(())
        },
    );
}
const DELAYS: [i32; 8] = [0, 5, 10, 30, 60, 300, 900, 3600];
fn slow_mode(chat: i64) {
    actions::run(chat, "slow-mode", move |s| async move {
        let c = groups::context(&s, chat).await?;
        c.require("can_restrict_members")?;
        let original = full(&s, &c).await?["slow_mode_delay"].as_i64().unwrap_or(0);
        let (dialog, body) = w::dialog("Slow mode", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        note(&fields, "Minimum time between messages from ordinary members.");
        let value = gtk::DropDown::from_strings(&[
            "Off",
            "5 seconds",
            "10 seconds",
            "30 seconds",
            "1 minute",
            "5 minutes",
            "15 minutes",
            "1 hour",
        ]);
        value.set_selected(DELAYS.iter().position(|v| *v as i64 == original).unwrap_or(0) as u32);
        fields.append(&value);
        submit(&dialog, &body, &fields, s, "Save slow mode", move |s| {
            let delay = DELAYS[value.selected() as usize];
            async move {
                let c = groups::context(&s, chat).await?;
                c.require("can_restrict_members")?;
                if full(&s, &c).await?["slow_mode_delay"].as_i64().unwrap_or(0) != original {
                    return Err("Slow mode changed elsewhere. Reopen the editor.".into());
                }
                s.call("setChatSlowModeDelay", json!({"chat_id":chat,"slow_mode_delay":delay})).await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
#[derive(Clone, Copy)]
enum Setting {
    Protection,
    Forum,
    History,
    JoinToSend,
    JoinApproval,
    HiddenMembers,
    AntiSpam,
}
impl Setting {
    fn spec(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Protection => (
                "Content protection",
                "Restrict saving, forwarding and copying",
                "toggleChatHasProtectedContent",
                "has_protected_content",
            ),
            Self::Forum => ("Forum topics", "Organize this group into topics", "toggleSupergroupIsForum", "is_forum"),
            Self::History => (
                "History for new members",
                "Allow new members to see old messages",
                "toggleSupergroupIsAllHistoryAvailable",
                "is_all_history_available",
            ),
            Self::JoinToSend => (
                "Joining required",
                "Require joining before sending messages",
                "toggleSupergroupJoinToSendMessages",
                "join_to_send_messages",
            ),
            Self::JoinApproval => (
                "Join approval",
                "Require administrator approval for new members",
                "toggleSupergroupJoinByRequest",
                "join_by_request",
            ),
            Self::HiddenMembers => (
                "Member list privacy",
                "Hide ordinary members from non-administrators",
                "toggleSupergroupHasHiddenMembers",
                "has_hidden_members",
            ),
            Self::AntiSpam => (
                "Anti-spam",
                "Enable aggressive anti-spam checks",
                "toggleSupergroupHasAggressiveAntiSpamEnabled",
                "has_aggressive_anti_spam_enabled",
            ),
        }
    }
    fn from_full(self) -> bool {
        matches!(self, Self::History | Self::HiddenMembers | Self::AntiSpam)
    }
    fn check(self, c: &groups::Context, f: &Value) -> Result<(), String> {
        match self {
            Self::Protection | Self::Forum => {
                if c.owner() {
                    Ok(())
                } else {
                    Err("Only the owner can change this setting.".into())
                }
            }
            Self::History => info(c),
            Self::JoinToSend | Self::JoinApproval => c.require("can_restrict_members"),
            Self::HiddenMembers => {
                if f["can_hide_members"] == true {
                    Ok(())
                } else {
                    Err("Telegram does not permit changing member visibility.".into())
                }
            }
            Self::AntiSpam => {
                if f["can_toggle_aggressive_anti_spam"] == true {
                    Ok(())
                } else {
                    Err("Telegram does not permit changing anti-spam here.".into())
                }
            }
        }
    }
}
fn toggle(chat: i64, setting: Setting) {
    actions::run(chat, "group-setting", move |s| async move {
        let c = groups::context(&s, chat).await?;
        let f = if c.supergroup().is_some() { full(&s, &c).await? } else { Value::Null };
        setting.check(&c, &f)?;
        let (title, description, method, key) = setting.spec();
        let original = if matches!(setting, Setting::Protection) {
            c.chat[key].clone()
        } else if setting.from_full() {
            f[key].clone()
        } else {
            c.group[key].clone()
        };
        let (dialog, body) = w::dialog(title, 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        let enabled = gtk::CheckButton::with_label(description);
        enabled.set_active(original == true);
        fields.append(&enabled);
        if matches!(setting, Setting::History) {
            note(
                &fields,
                "Public groups, forums and linked discussion groups always expose old history. Telegram controls eligibility.",
            );
        }
        if matches!(setting, Setting::Forum) {
            note(&fields, "Changing this reorganizes the group. Linked discussion groups cannot become forums.");
        }
        let links = gtk::CheckButton::with_label("Also apply to existing invite links");
        if matches!(setting, Setting::JoinApproval) {
            fields.append(&links);
            note(&fields, "The existing guard bot, if any, is retained.");
        }
        submit(&dialog, &body, &fields, s, "Apply", move |s| {
            let (value, links, original) = (enabled.is_active(), links.is_active(), original.clone());
            async move {
                let c = groups::context(&s, chat).await?;
                let f = if c.supergroup().is_some() { full(&s, &c).await? } else { Value::Null };
                setting.check(&c, &f)?;
                let current = if matches!(setting, Setting::Protection) {
                    &c.chat[key]
                } else if setting.from_full() {
                    &f[key]
                } else {
                    &c.group[key]
                };
                if *current != original {
                    return Err("This setting changed elsewhere. Reopen the dialog.".into());
                }
                let mut params = json!({"chat_id":chat,"supergroup_id":c.supergroup(),"has_forum_tabs":c.group["has_forum_tabs"],"guard_bot_user_id":f["guard_bot_user_id"].as_i64().unwrap_or(0),"apply_to_invite_links":links});
                params[key] = json!(value);
                s.call(method, params).await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn signatures(chat: i64) {
    actions::run(chat, "channel-signatures", move |s| async move {
        let c = groups::context(&s, chat).await?;
        info(&c)?;
        let original = (c.group["sign_messages"].clone(), c.group["show_message_sender"].clone());
        let (dialog, body) = w::dialog("Post signatures", 440);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        let sign = gtk::CheckButton::with_label("Sign posts");
        sign.set_active(original.0 == true);
        fields.append(&sign);
        let profile = gtk::CheckButton::with_label("Show author's profile");
        profile.set_active(original.1 == true);
        fields.append(&profile);
        submit(&dialog, &body, &fields, s, "Save signatures", move |s| {
            let (sign, profile, original) = (sign.is_active(), profile.is_active(), original.clone());
            async move {
                if profile && !sign {
                    return Err("Enable signatures before showing the author's profile.".into());
                }
                let c = groups::context(&s, chat).await?;
                info(&c)?;
                if (c.group["sign_messages"].clone(), c.group["show_message_sender"].clone()) != original {
                    return Err("Signature settings changed elsewhere. Reopen this dialog.".into());
                }
                s.call(
                    "toggleSupergroupSignMessages",
                    json!({"supergroup_id":c.supergroup(),"sign_messages":sign,"show_message_sender":profile}),
                )
                .await?;
                refresh(chat);
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn discussion(chat: i64) {
    actions::run(chat, "channel-discussion", move |s| async move {
        let c = groups::context(&s, chat).await?;
        info(&c)?;
        let original = full(&s, &c).await?["linked_chat_id"].as_i64().unwrap_or(0);
        let available = s.call("getSuitableDiscussionChats", json!({})).await?;
        let mut ids = vec![0];
        let mut labels = vec!["No discussion group".to_string()];
        let mut skipped = 0;
        for id in available["chat_ids"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
            let chat = s.call("getChat", json!({"chat_id":id})).await?;
            if chat["type"]["supergroup_id"].is_null() {
                skipped += 1;
                continue;
            }
            ids.push(id);
            labels.push(chat["title"].as_str().unwrap_or("Group").to_string());
        }
        if original != 0 && !ids.contains(&original) {
            let chat = s.call("getChat", json!({"chat_id":original})).await?;
            ids.push(original);
            labels.push(chat["title"].as_str().unwrap_or("Current discussion group").to_string());
        }
        let (dialog, body) = w::dialog("Discussion group", 460);
        crate::dialog_dismissal::add(&dialog, &body, "Cancel");
        let fields = w::vbox(8);
        body.append(&fields);
        note(
            &fields,
            "Channel posts will have a comment thread in the linked group. Linking requires old history to be visible to new group members.",
        );
        if skipped > 0 {
            note(&fields, "Some basic groups need to be upgraded first through their group settings.");
        }
        let choice = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        choice.set_selected(ids.iter().position(|id| *id == original).unwrap_or(0) as u32);
        fields.append(&choice);
        submit(&dialog, &body, &fields, s, "Set discussion group", move |s| {
            let id = ids[choice.selected() as usize];
            async move {
                let c = groups::context(&s, chat).await?;
                info(&c)?;
                if full(&s, &c).await?["linked_chat_id"].as_i64().unwrap_or(0) != original {
                    return Err("The discussion link changed elsewhere. Reopen this dialog.".into());
                }
                if id != 0 {
                    let group = groups::context(&s, id).await?;
                    if full(&s, &group).await?["is_all_history_available"] != true {
                        return Err(
                            "First enable 'History for new members' in the selected group's settings, then return here.".into()
                        );
                    }
                }
                s.call("setChatDiscussionGroup", json!({"chat_id":chat,"discussion_chat_id":id})).await?;
                refresh(chat);
                if id != 0 {
                    refresh(id);
                }
                Ok(())
            }
        });
        dialog.present();
        Ok(())
    });
}
fn upgrade(chat: i64) {
    actions::operation(
        "Upgrade basic group?",
        "Convert this basic group to a supergroup? This cannot be reversed. Members and history are retained by Telegram.",
        vec![],
        "Upgrade",
        move |s, _| async move {
            if !groups::context(&s, chat).await?.owner() {
                return Err("Only the owner can upgrade this group.".into());
            }
            let result = groups::commit(&s, "upgradeBasicGroupChatToSupergroupChat", json!({"chat_id":chat})).await?;
            if let Some(id) = result["id"].as_i64() {
                refresh(id);
                window::open_chat(id);
            }
            Ok(())
        },
    );
}
