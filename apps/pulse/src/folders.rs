//! Account-scoped Telegram folder navigation, membership, editing and sharing.
use crate::{
    actions::{self, Scope},
    window,
};
use gtk::{gdk, glib, prelude::*};
use nixe_telegram_core::models::*;
use nixe_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
};
#[derive(Default)]
struct State {
    infos: Vec<Value>,
    main_position: i32,
    tags: bool,
    unread: HashMap<i32, i64>,
    checked: HashMap<i32, std::time::Instant>,
}
thread_local! {static STATE:RefCell<State>=RefCell::new(State::default());}
thread_local! { static MANAGERS: RefCell<Vec<(Scope,glib::WeakRef<gtk::Box>)>> = const { RefCell::new(Vec::new()) }; }
pub fn reset() {
    MANAGERS.with(|m| m.borrow_mut().clear());
    STATE.with(|s| *s.borrow_mut() = State::default());
}
pub fn infos() -> Vec<Value> {
    STATE.with(|s| s.borrow().infos.clone())
}
pub fn name(info: &Value) -> String {
    info["name"]["text"]["text"].as_str().unwrap_or("Folder").into()
}
pub fn update(value: &Value) {
    if value["@type"] == "updateChatFolders" {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.infos = value["chat_folders"].as_array().cloned().unwrap_or_default();
            s.main_position = value["main_chat_list_position"].as_i64().unwrap_or(0) as i32;
            s.tags = value["are_tags_enabled"] == true;
        });
    } else if value["@type"] == "updateUnreadChatCount" {
        if let Some(id) = value["chat_list"]["chat_folder_id"].as_i64() {
            STATE.with(|s| {
                s.borrow_mut().unread.insert(id as i32, value["unread_count"].as_i64().unwrap_or(0));
            });
        } else {
            return;
        }
    } else {
        return;
    }
    if value["@type"] == "updateUnreadChatCount" {
        window::refresh_folder_labels();
    } else {
        window::refresh_folders();
    }
    let managers = MANAGERS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|(s, w)| s.valid() && w.upgrade().is_some());
        m.clone()
    });
    for (scope, weak) in managers {
        if let Some(list) = weak.upgrade() {
            manager_rows(&list, &scope);
        }
    }
}
pub fn title(info: &Value) -> String {
    let count = STATE.with(|s| s.borrow().unread.get(&(info["id"].as_i64().unwrap_or(0) as i32)).copied().unwrap_or(0));
    if count > 0 { format!("{} · {count}", name(info)) } else { name(info) }
}
pub fn tags(chat: &ChatSummary) -> Vec<String> {
    STATE.with(|s| {
        let s = s.borrow();
        if !s.tags {
            return Vec::new();
        }
        s.infos
            .iter()
            .filter(|i| chat.list_position(ChatListId::Folder(i["id"].as_i64().unwrap_or(0) as i32)).is_some())
            .map(name)
            .collect()
    })
}
fn scroll(body: &gtk::Box) -> gtk::Box {
    let list = w::vbox(6);
    body.append(
        &gtk::ScrolledWindow::builder()
            .max_content_height(460)
            .min_content_height(160)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .build(),
    );
    list
}
fn button(body: &gtk::Box, title: &str, scope: Scope, action: impl Fn() + 'static) {
    let b = gtk::Button::with_label(title);
    b.connect_clicked(move |_| {
        if scope.valid() {
            action()
        }
    });
    body.append(&b);
}
pub fn manage() {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Chat folders", 520);
    let top = w::hbox(8);
    button(&top, "New folder", scope.clone(), || edit(None));
    body.append(&top);
    let tags = gtk::CheckButton::with_label("Show folder tags on chats (Premium)");
    tags.set_active(STATE.with(|s| s.borrow().tags));
    let s = scope.clone();
    tags.connect_toggled(move |b| {
        if !s.valid() || !b.is_sensitive() {
            return;
        }
        let enabled = b.is_active();
        b.set_sensitive(false);
        let (scope, b) = (s.clone(), b.clone());
        glib::spawn_future_local(async move {
            if let Err(e) = scope.call("toggleChatFolderTags", json!({"are_tags_enabled":enabled})).await {
                scope.error(&e);
                b.set_active(!enabled);
            }
            b.set_sensitive(true);
        });
    });
    body.append(&tags);
    let list = scroll(&body);
    MANAGERS.with(|m| m.borrow_mut().push((scope.clone(), list.downgrade())));
    manager_rows(&list, &scope);
    let close = gtk::Button::with_label("Close");
    let weak = dialog.downgrade();
    close.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close()
        }
    });
    body.append(&close);
    dialog.present();
}
fn manager_rows(list: &gtk::Box, scope: &Scope) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for info in infos() {
        let id = info["id"].as_i64().unwrap_or(0) as i32;
        let row = w::vbox(4);
        let label = w::label(&title(&info), "section-title");
        label.set_xalign(0.0);
        row.append(&label);
        let controls = w::hbox(4);
        button(&controls, "Edit", scope.clone(), move || edit(Some(id)));
        button(&controls, "Share", scope.clone(), move || sharing(id));
        if info["is_shareable"] == true {
            button(&controls, "Updates…", scope.clone(), move || shared_updates(id));
        }
        button(&controls, "↑", scope.clone(), move || reorder(id, -1));
        button(&controls, "↓", scope.clone(), move || reorder(id, 1));
        button(&controls, "Delete…", scope.clone(), move || {
            actions::operation(
                "Delete folder?",
                "Only the folder is removed. Conversations and group memberships are kept.",
                vec![],
                "Delete folder",
                move |scope, _| async move {
                    scope.call("deleteChatFolder", json!({"chat_folder_id":id,"leave_chat_ids":[]})).await?;
                    Ok(())
                },
            )
        });
        row.append(&controls);
        list.append(&row);
    }
    if infos().is_empty() {
        list.append(&w::label("No custom folders yet", "dim"));
    }
}
fn reorder(id: i32, direction: i32) {
    actions::run(0, "folder-order", move |scope| async move {
        let (mut ids, position) = STATE.with(|s| {
            let s = s.borrow();
            (s.infos.iter().filter_map(|i| i["id"].as_i64()).map(|i| i as i32).collect::<Vec<_>>(), s.main_position)
        });
        if let Some(index) = ids.iter().position(|i| *i == id) {
            let target = index as i32 + direction;
            if target < 0 || target >= ids.len() as i32 {
                return Ok(());
            }
            ids.swap(index, target as usize);
            scope.call("reorderChatFolders", json!({"chat_folder_ids":ids,"main_chat_list_position":position})).await?;
        }
        Ok(())
    });
}
fn ids(folder: &Value, key: &str) -> Vec<i64> {
    folder[key].as_array().into_iter().flatten().filter_map(Value::as_i64).collect()
}
pub fn apply_membership(folder: &mut Value, chat: i64, include: bool) {
    let mut included = ids(folder, "included_chat_ids");
    let mut excluded = ids(folder, "excluded_chat_ids");
    let mut pinned = ids(folder, "pinned_chat_ids");
    included.retain(|id| *id != chat);
    excluded.retain(|id| *id != chat);
    if include {
        if !pinned.contains(&chat) {
            included.push(chat)
        }
    } else {
        pinned.retain(|id| *id != chat);
        excluded.push(chat)
    }
    folder["included_chat_ids"] = json!(included);
    folder["excluded_chat_ids"] = json!(excluded);
    folder["pinned_chat_ids"] = json!(pinned);
}
pub fn membership(chat: i64) {
    actions::run(chat, "folder-membership", move |scope| async move {
        let (dialog, body) = w::dialog("Chat folders", 420);
        let list = scroll(&body);
        for info in infos() {
            let id = info["id"].as_i64().unwrap_or(0) as i32;
            let folder = scope.call("getChatFolder", json!({"chat_folder_id":id})).await?;
            let check = gtk::CheckButton::with_label(&name(&info));
            let contained = window::chat(chat).is_some_and(|c| c.list_position(ChatListId::Folder(id)).is_some())
                || ids(&folder, "included_chat_ids").contains(&chat)
                || ids(&folder, "pinned_chat_ids").contains(&chat);
            check.set_active(contained);
            let scope = scope.clone();
            check.connect_toggled(move |b| {
                if !scope.valid() || !b.is_sensitive() {
                    return;
                }
                let include = b.is_active();
                b.set_sensitive(false);
                let b = b.clone();
                let scope = scope.clone();
                glib::spawn_future_local(async move {
                    let result = async {
                        let mut folder = scope.call("getChatFolder", json!({"chat_folder_id":id})).await?;
                        apply_membership(&mut folder, chat, include);
                        scope.call("editChatFolder", json!({"chat_folder_id":id,"folder":folder})).await?;
                        window::with_service(|s| s.load_chat(chat));
                        Ok::<(), String>(())
                    }
                    .await;
                    if let Err(e) = result {
                        scope.error(&e);
                        b.set_active(!include);
                    }
                    b.set_sensitive(true);
                });
            });
            list.append(&check);
        }
        if infos().is_empty() {
            list.append(&w::label("Create a folder first", "dim"));
        }
        dialog.present();
        Ok(())
    });
}
fn fresh_folder() -> Value {
    json!({"@type":"chatFolder","name":{"@type":"chatFolderName","text":{"@type":"formattedText","text":"","entities":[]},"animate_custom_emoji":false},"icon":{"@type":"chatFolderIcon","name":"Custom"},"color_id":-1,"is_shareable":false,"pinned_chat_ids":[],"included_chat_ids":[],"excluded_chat_ids":[],"exclude_muted":false,"exclude_read":false,"exclude_archived":false,"include_contacts":false,"include_non_contacts":false,"include_bots":false,"include_groups":false,"include_channels":false})
}
fn edit(id: Option<i32>) {
    actions::run(0, "folder-edit", move |scope| async move {
        let baseline =
            if let Some(id) = id { scope.call("getChatFolder", json!({"chat_folder_id":id})).await? } else { fresh_folder() };
        let (dialog, body) = w::dialog(if id.is_some() { "Edit folder" } else { "New folder" }, 520);
        let fields = scroll(&body);
        let name_entry = gtk::Entry::new();
        name_entry.set_max_length(12);
        name_entry.set_placeholder_text(Some("Folder name (1–12 characters)"));
        name_entry.set_text(&name(&baseline));
        if id.is_none() {
            name_entry.set_text("");
        }
        fields.append(&name_entry);
        let flags = [
            ("include_contacts", "Contacts"),
            ("include_non_contacts", "Other private chats"),
            ("include_bots", "Bots"),
            ("include_groups", "Groups"),
            ("include_channels", "Channels"),
            ("exclude_muted", "Exclude muted chats"),
            ("exclude_read", "Exclude read chats"),
            ("exclude_archived", "Exclude archived chats"),
        ]
        .into_iter()
        .map(|(key, label)| {
            let b = gtk::CheckButton::with_label(label);
            b.set_active(baseline[key] == true);
            fields.append(&b);
            (key, b)
        })
        .collect::<Vec<_>>();
        fields.append(&w::label("Individual chats · Default / Always include / Always exclude", "dim"));
        let mut chats = window::list_chats(usize::MAX).into_iter().map(|c| (c.id, c.title)).collect::<BTreeMap<_, _>>();
        for chat in ids(&baseline, "included_chat_ids")
            .into_iter()
            .chain(ids(&baseline, "excluded_chat_ids"))
            .chain(ids(&baseline, "pinned_chat_ids"))
        {
            if !chats.contains_key(&chat) {
                let data = scope.call("getChat", json!({"chat_id":chat})).await?;
                chats.insert(chat, data["title"].as_str().unwrap_or("Conversation").into());
            }
        }
        let mut choices = Vec::new();
        for (chat, title) in chats {
            let choice = gtk::DropDown::from_strings(&["Default", "Include", "Exclude"]);
            choice.set_selected(if ids(&baseline, "excluded_chat_ids").contains(&chat) {
                2
            } else if ids(&baseline, "included_chat_ids").contains(&chat) || ids(&baseline, "pinned_chat_ids").contains(&chat) {
                1
            } else {
                0
            });
            let row = w::row(&title, "", Some(choice.upcast_ref()));
            if let Some(label) =
                row.first_child().and_downcast::<gtk::Box>().and_then(|b| b.first_child()).and_downcast::<gtk::Label>()
            {
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.set_max_width_chars(28);
            }
            fields.append(&row);
            choices.push((chat, choice));
        }
        let error = w::label("", "error");
        error.set_wrap(true);
        body.append(&error);
        let save = gtk::Button::with_label("Save folder");
        body.append(&save);
        let weak = dialog.downgrade();
        save.connect_clicked(move |button| {
            let Some(dialog) = weak.upgrade() else { return };
            if !scope.valid() {
                dialog.close();
                return;
            }
            let name = name_entry.text().trim().to_owned();
            if name.is_empty() || name.chars().count() > 12 || name.contains('\n') {
                error.set_text("Use a folder name of 1–12 characters");
                return;
            }
            let mut folder = baseline.clone();
            if folder["name"]["text"]["text"] != name {
                folder["name"]["text"] = json!({"@type":"formattedText","text":name,"entities":[]});
            }
            for (key, check) in &flags {
                folder[*key] = json!(check.is_active());
            }
            let pinned = ids(&baseline, "pinned_chat_ids");
            let mut included = Vec::new();
            let mut excluded = Vec::new();
            let mut kept_pins = Vec::new();
            for (chat, choice) in &choices {
                match choice.selected() {
                    1 => {
                        if pinned.contains(chat) {
                            kept_pins.push(*chat)
                        } else {
                            included.push(*chat)
                        }
                    }
                    2 => excluded.push(*chat),
                    _ => {}
                }
            }
            folder["included_chat_ids"] = json!(included);
            folder["excluded_chat_ids"] = json!(excluded);
            folder["pinned_chat_ids"] = json!(kept_pins);
            let (scope, baseline, error, button) = (scope.clone(), baseline.clone(), error.clone(), button.clone());
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                let result = async {
                    if let Some(id) = id {
                        let current = scope.call("getChatFolder", json!({"chat_folder_id":id})).await?;
                        if current != baseline {
                            return Err("This folder changed in another client. Reopen the editor to keep those changes.".into());
                        }
                        scope.call("editChatFolder", json!({"chat_folder_id":id,"folder":folder})).await?;
                    } else {
                        scope.call("createChatFolder", json!({"folder":folder})).await?;
                    }
                    Ok::<(), String>(())
                }
                .await;
                if !scope.valid() {
                    dialog.close();
                    return;
                }
                match result {
                    Ok(()) => dialog.close(),
                    Err(e) => {
                        error.set_text(&e);
                        button.set_sensitive(true);
                    }
                }
            });
        });
        dialog.present();
        Ok(())
    });
}
fn sharing(id: i32) {
    actions::run(0, "folder-sharing", move |scope| async move {
        let links = scope.call("getChatFolderInviteLinks", json!({"chat_folder_id":id})).await?;
        let (dialog, body) = w::dialog("Share folder", 520);
        let list = scroll(&body);
        let refresh = gtk::Button::with_label("Refresh links");
        let s = scope.clone();
        let weak = dialog.downgrade();
        refresh.connect_clicked(move |_| {
            if s.valid() {
                if let Some(d) = weak.upgrade() {
                    d.close();
                }
                sharing(id);
            }
        });
        body.append(&refresh);
        let explanation = w::label(
            "Links share only the chats you choose. Creating links for basic groups may convert those groups to supergroups.",
            "dim",
        );
        explanation.set_wrap(true);
        explanation.set_max_width_chars(46);
        body.append(&explanation);
        button(&body, "New invite link…", scope.clone(), move || edit_link(id, None));
        for link in links["invite_links"].as_array().into_iter().flatten() {
            let row = w::vbox(4);
            let text = link["invite_link"].as_str().unwrap_or("").to_owned();
            let label = w::label(link["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(&text), "");
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_max_width_chars(40);
            row.append(&label);
            let controls = w::hbox(6);
            let uri = text.clone();
            button(&controls, "Copy link", scope.clone(), move || {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&uri)
                }
            });
            let original = link.clone();
            button(&controls, "Edit", scope.clone(), move || edit_link(id, Some(original.clone())));
            button(&controls, "Revoke…", scope.clone(), move || {
                let uri = text.clone();
                actions::operation(
                    "Revoke folder link?",
                    "This link will stop working. Existing memberships are kept.",
                    vec![],
                    "Revoke",
                    move |scope, _| {
                        let uri = uri.clone();
                        async move {
                            scope.call("deleteChatFolderInviteLink", json!({"chat_folder_id":id,"invite_link":uri})).await?;
                            Ok(())
                        }
                    },
                )
            });
            row.append(&controls);
            list.append(&row);
        }
        dialog.present();
        Ok(())
    });
}
fn edit_link(id: i32, original: Option<Value>) {
    actions::run(0, "folder-link-edit", move |scope| async move {
        let eligible = scope.call("getChatsForChatFolderInviteLink", json!({"chat_folder_id":id})).await?;
        let (dialog, body) = w::dialog("Folder invite link", 500);
        let list = scroll(&body);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("Link name (optional)"));
        name.set_max_length(32);
        name.set_text(original.as_ref().and_then(|v| v["name"].as_str()).unwrap_or(""));
        body.append(&name);
        let mut choices = Vec::new();
        for chat in eligible["chat_ids"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
            let data = scope.call("getChat", json!({"chat_id":chat})).await?;
            let check = gtk::CheckButton::with_label(data["title"].as_str().unwrap_or("Conversation"));
            check.set_active(original.as_ref().is_some_and(|v| ids(v, "chat_ids").contains(&chat)));
            list.append(&check);
            choices.push((chat, check));
        }
        let error = w::label("", "error");
        error.set_wrap(true);
        body.append(&error);
        let save = gtk::Button::with_label(if original.is_some() { "Save link" } else { "Create link" });
        body.append(&save);
        let weak = dialog.downgrade();
        save.connect_clicked(move|b|{let Some(dialog)=weak.upgrade()else{return};if !scope.valid(){dialog.close();return}let chats=choices.iter().filter(|(_,c)|c.is_active()).map(|(id,_)|*id).collect::<Vec<_>>();if chats.is_empty(){error.set_text("Choose at least one eligible chat");return}let(scope,original,error,b,name)=(scope.clone(),original.clone(),error.clone(),b.clone(),name.text().to_string());b.set_sensitive(false);glib::spawn_future_local(async move{let result=scope.call(if original.is_some(){"editChatFolderInviteLink"}else{"createChatFolderInviteLink"},json!({"chat_folder_id":id,"invite_link":original.as_ref().map(|v|v["invite_link"].clone()).unwrap_or(Value::Null),"name":name,"chat_ids":chats})).await;
        match result{Ok(value)=>{if let(Some(display),Some(link))=(gdk::Display::default(),value["invite_link"].as_str()){display.clipboard().set_text(link);window::show_status("Folder invite link copied");}dialog.close();},Err(e)=>{if !scope.valid(){dialog.close();return}error.set_text(&e);b.set_sensitive(true);}}
    });});
        dialog.present();
        Ok(())
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn membership_preserves_pins_and_exclusions() {
        let mut folder = fresh_folder();
        folder["pinned_chat_ids"] = json!([1, 2]);
        folder["included_chat_ids"] = json!([3]);
        apply_membership(&mut folder, 1, false);
        assert_eq!(folder["pinned_chat_ids"], json!([2]));
        assert_eq!(folder["excluded_chat_ids"], json!([1]));
        apply_membership(&mut folder, 1, true);
        assert_eq!(folder["included_chat_ids"], json!([3, 1]));
        assert_eq!(folder["excluded_chat_ids"], json!([]));
        apply_membership(&mut folder, 2, true);
        assert_eq!(folder["pinned_chat_ids"], json!([2]));
        assert!(!ids(&folder, "included_chat_ids").contains(&2));
    }
}

pub fn import_link(link: String) {
    actions::run(0, "folder-import", move |scope| async move {
        let info = scope.call("checkChatFolderInviteLink", json!({"invite_link":link})).await?;
        let (dialog, body) = w::dialog("Add shared folder", 500);
        let explanation = w::label(
            &format!(
                "Add {}? Any checked conversations you have not joined will be joined. No chats are joined until you choose Add folder.",
                name(&info["chat_folder_info"])
            ),
            "dim",
        );
        explanation.set_wrap(true);
        explanation.set_max_width_chars(44);
        body.append(&explanation);
        let list = scroll(&body);
        let mut choices = Vec::new();
        let existing = ids(&info, "added_chat_ids");
        for chat in existing.iter().copied().chain(ids(&info, "missing_chat_ids")) {
            let data = scope.call("getChat", json!({"chat_id":chat})).await?;
            let check = gtk::CheckButton::with_label(data["title"].as_str().unwrap_or("Conversation"));
            check.set_active(existing.contains(&chat));
            check.set_sensitive(!existing.contains(&chat));
            list.append(&check);
            choices.push((chat, check));
        }
        let error = w::label("", "error");
        error.set_wrap(true);
        body.append(&error);
        let add = gtk::Button::with_label("Add folder");
        body.append(&add);
        let weak = dialog.downgrade();
        add.connect_clicked(move |button| {
            let Some(dialog) = weak.upgrade() else { return };
            let chats = choices.iter().filter(|(_, c)| c.is_active()).map(|(id, _)| *id).collect::<Vec<_>>();
            let (scope, link, error, button) = (scope.clone(), link.clone(), error.clone(), button.clone());
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                match scope.call("addChatFolderByInviteLink", json!({"invite_link":link,"chat_ids":chats})).await {
                    Ok(_) => dialog.close(),
                    Err(e) => {
                        if !scope.valid() {
                            dialog.close();
                            return;
                        }
                        error.set_text(&e);
                        button.set_sensitive(true);
                    }
                }
            });
        });
        dialog.present();
        Ok(())
    });
}

fn shared_updates(id: i32) {
    actions::run(0, "shared-folder-updates", move |scope| async move {
        let option = scope.call("getOption", json!({"name":"chat_folder_new_chats_update_period"})).await?;
        let period = option["value"]
            .as_u64()
            .or_else(|| option["value"].as_str().and_then(|s| s.parse().ok()))
            .filter(|s| *s > 0)
            .ok_or("Telegram did not provide a folder update interval")?;
        let remaining =
            STATE.with(|s| s.borrow().checked.get(&id).map(|last| period.saturating_sub(last.elapsed().as_secs())).unwrap_or(0));
        if remaining > 0 {
            return Err(format!("Check this shared folder again in {remaining} seconds"));
        }
        STATE.with(|s| {
            s.borrow_mut().checked.insert(id, std::time::Instant::now());
        });
        let data = scope.call("getChatFolderNewChats", json!({"chat_folder_id":id})).await?;
        let (dialog, body) = w::dialog("Shared folder updates", 500);
        let list = scroll(&body);
        let explanation = w::label(
            "Select conversations to add and join. Unchecked conversations will be skipped when you apply your choices.",
            "dim",
        );
        explanation.set_wrap(true);
        explanation.set_max_width_chars(44);
        body.append(&explanation);
        let mut choices = Vec::new();
        for chat in ids(&data, "chat_ids") {
            let c = scope.call("getChat", json!({"chat_id":chat})).await?;
            let check = gtk::CheckButton::with_label(c["title"].as_str().unwrap_or("Conversation"));
            list.append(&check);
            choices.push((chat, check));
        }
        if choices.is_empty() {
            list.append(&w::label("This folder is up to date", "dim"));
            dialog.present();
            return Ok(());
        }
        let error = w::label("", "error");
        error.set_wrap(true);
        body.append(&error);
        let apply = gtk::Button::with_label("Apply choices");
        body.append(&apply);
        let weak = dialog.downgrade();
        apply.connect_clicked(move |button| {
            let Some(dialog) = weak.upgrade() else { return };
            let chats = choices.iter().filter(|(_, c)| c.is_active()).map(|(id, _)| *id).collect::<Vec<_>>();
            let (scope, error, button) = (scope.clone(), error.clone(), button.clone());
            button.set_sensitive(false);
            glib::spawn_future_local(async move {
                match scope.call("processChatFolderNewChats", json!({"chat_folder_id":id,"added_chat_ids":chats})).await {
                    Ok(_) => {
                        window::with_service(|s| s.load_chat_list(ChatListId::Folder(id), 50));
                        dialog.close();
                    }
                    Err(e) => {
                        if !scope.valid() {
                            dialog.close();
                            return;
                        }
                        error.set_text(&e);
                        button.set_sensitive(true);
                    }
                }
            });
        });
        dialog.present();
        Ok(())
    });
}
