//! Privacy categories and human-readable user/group exception selection.
use crate::{
    actions::Scope,
    features::{self, alive, button, check},
    groups::{note, submit},
    window,
};
use gtk::prelude::*;
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};
type Selection = Rc<RefCell<Vec<i64>>>;
fn add_row(list: &gtk::Box, scope: &Scope, selection: &Selection, id: i64, name: String) {
    let row = nyx_ui::widgets::hbox(4);
    let label = nyx_ui::widgets::label(&name, "");
    label.set_hexpand(true);
    label.set_wrap(true);
    row.append(&label);
    let remove = gtk::Button::with_label("Remove");
    row.append(&remove);
    let (state, weak, container, s) = (Rc::downgrade(selection), row.downgrade(), list.downgrade(), scope.clone());
    remove.connect_clicked(move |_| {
        if !s.valid() {
            return;
        }
        if let (Some(state), Some(row), Some(list)) = (state.upgrade(), weak.upgrade(), container.upgrade()) {
            state.borrow_mut().retain(|v| *v != id);
            list.remove(&row);
        }
    });
    list.append(&row);
}
fn choose(body: &gtk::Box, scope: &Scope, title: &str, initial: Vec<i64>, groups: bool) -> Selection {
    note(body, title);
    let state = Rc::new(RefCell::new(initial.clone()));
    let list = nyx_ui::widgets::vbox(4);
    body.append(&list);
    for id in initial {
        if groups {
            let name = window::chat(id).map(|c| c.title).unwrap_or_else(|| format!("Group {id}"));
            add_row(&list, scope, &state, id, name);
        } else {
            let (s, list, state) = (scope.clone(), list.clone(), state.clone());
            gtk::glib::spawn_future_local(async move {
                let name = s
                    .call(
                        "getUser",
                        json!({
                        "user_id":id}
                        ),
                    )
                    .await
                    .map(|u| format!("{} {}", u["first_name"].as_str().unwrap_or(""), u["last_name"].as_str().unwrap_or("")))
                    .unwrap_or_else(|_| format!("Unavailable user {id}"));
                if s.valid() && state.borrow().contains(&id) {
                    add_row(&list, &s, &state, id, name);
                }
            });
        }
    }
    let (s, list, state_) = (scope.clone(), list.clone(), state.clone());
    button(body, if groups { "Add group…" } else { "Add person…" }, move || {
        if !s.valid() {
            return;
        }
        if groups {
            let (d, b) = nyx_ui::widgets::dialog("Choose group", 440);
            crate::dialog_dismissal::add(&d, &b, "Cancel");
            let rows = crate::groups::scroll(&b);
            for chat in window::list_chats(10000).into_iter().filter(|c| {
                matches!(
                    c.chat_type,
                    nyx_telegram_core::models::ChatType::BasicGroup | nyx_telegram_core::models::ChatType::Supergroup
                )
            }) {
                let (s, list, state, weak) = (s.clone(), list.clone(), state_.clone(), d.downgrade());
                let id = chat.id;
                let name = chat.title.clone();
                button(&rows, &chat.title, move || {
                    if s.valid() && !state.borrow().contains(&id) {
                        state.borrow_mut().push(id);
                        add_row(&list, &s, &state, id, name.clone());
                    }
                    if let Some(d) = weak.upgrade() {
                        d.close();
                    }
                });
            }
            d.present();
        } else {
            let (s, list, state) = (s.clone(), list.clone(), state_.clone());
            crate::groups::choose_user("Choose person", move |user| {
                if !s.valid() {
                    return;
                }
                if let Some(id) = user["id"].as_i64() {
                    if !state.borrow().contains(&id) {
                        state.borrow_mut().push(id);
                        add_row(
                            &list,
                            &s,
                            &state,
                            id,
                            format!("{} {}", user["first_name"].as_str().unwrap_or(""), user["last_name"].as_str().unwrap_or("")),
                        );
                    }
                }
            });
        }
    });
    state
}
fn members(rules: &[Value], kind: &str, key: &str) -> Vec<i64> {
    let mut ids = Vec::new();
    for rule in rules.iter().filter(|r| r["@type"] == kind) {
        for id in rule[key].as_array().into_iter().flatten().filter_map(Value::as_i64) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}
pub fn edit(title: &str, setting: &str) {
    let setting = setting.to_owned();
    features::load(title, move |s, d, b| async move {
        let key = json!({
        "@type":setting}
        );
        let original = s
            .call(
                "getUserPrivacySettingRules",
                json!({
                "setting":key}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let rules = original["rules"].as_array().cloned().unwrap_or_default();
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let base = gtk::DropDown::from_strings(&["Everyone", "Contacts", "Nobody"]);
        base.set_selected(if rules.iter().any(|r| r["@type"] == "userPrivacySettingRuleAllowAll") {
            0
        } else if rules.iter().any(|r| r["@type"] == "userPrivacySettingRuleAllowContacts") {
            1
        } else {
            2
        });
        note(&fields, "Base rule");
        fields.append(&base);
        let premium = check(
            &fields,
            "Always allow Premium users",
            rules.iter().any(|r| r["@type"] == "userPrivacySettingRuleAllowPremiumUsers"),
        );
        let bots = gtk::DropDown::from_strings(&["Bots follow base rule", "Always allow bots", "Never allow bots"]);
        bots.set_selected(if rules.iter().any(|r| r["@type"] == "userPrivacySettingRuleRestrictBots") {
            2
        } else if rules.iter().any(|r| r["@type"] == "userPrivacySettingRuleAllowBots") {
            1
        } else {
            0
        });
        fields.append(&bots);
        let restrict_contacts = check(
            &fields,
            "Never allow contacts except individual exceptions",
            rules.iter().any(|r| r["@type"] == "userPrivacySettingRuleRestrictContacts"),
        );
        let allow =
            choose(&fields, &s, "Always allow people", members(&rules, "userPrivacySettingRuleAllowUsers", "user_ids"), false);
        let deny =
            choose(&fields, &s, "Never allow people", members(&rules, "userPrivacySettingRuleRestrictUsers", "user_ids"), false);
        let allow_groups = choose(
            &fields,
            &s,
            "Always allow members of groups",
            members(&rules, "userPrivacySettingRuleAllowChatMembers", "chat_ids"),
            true,
        );
        let deny_groups = choose(
            &fields,
            &s,
            "Never allow members of groups",
            members(&rules, "userPrivacySettingRuleRestrictChatMembers", "chat_ids"),
            true,
        );
        note(
            &fields,
            "Individual exceptions take precedence, then group exceptions, then bot/Premium rules, then the base rule. Telegram enforces account eligibility for each category.",
        );
        submit(&d, &b, &fields, s, "Save privacy rules", move |s| {
            let (base, premium, bots, contacts, allow, deny, allow_groups, deny_groups, key, original) = (
                base.selected(),
                premium.is_active(),
                bots.selected(),
                restrict_contacts.is_active(),
                allow.borrow().clone(),
                deny.borrow().clone(),
                allow_groups.borrow().clone(),
                deny_groups.borrow().clone(),
                key.clone(),
                original.clone(),
            );
            async move {
                if allow.iter().any(|v| deny.contains(v)) || allow_groups.iter().any(|v| deny_groups.contains(v)) {
                    return Err("The same person or group cannot be both always allowed and never allowed.".into());
                }
                if s.call(
                    "getUserPrivacySettingRules",
                    json!({
                    "setting":key}
                    ),
                )
                .await?
                    != original
                {
                    return Err("Privacy rules changed. Reopen this editor.".into());
                }
                let mut rules = Vec::new();
                for (kind, field, ids) in [
                    ("AllowUsers", "user_ids", allow),
                    ("RestrictUsers", "user_ids", deny),
                    ("RestrictChatMembers", "chat_ids", deny_groups),
                    ("AllowChatMembers", "chat_ids", allow_groups),
                ] {
                    if !ids.is_empty() {
                        let mut rule = json!({
                        "@type":format!("userPrivacySettingRule{kind}")}
                        );
                        rule[field] = json!(ids);
                        rules.push(rule);
                    }
                }
                if bots != 0 {
                    rules.push(json!({
                    "@type":if bots==1{
                    "userPrivacySettingRuleAllowBots"}
                    else{
                    "userPrivacySettingRuleRestrictBots"}
                    }
                    ));
                }
                if premium {
                    rules.push(json!({
                    "@type":"userPrivacySettingRuleAllowPremiumUsers"}
                    ));
                }
                if contacts {
                    rules.push(json!({
                    "@type":"userPrivacySettingRuleRestrictContacts"}
                    ));
                }
                if base == 1 {
                    rules.push(json!({
                    "@type":"userPrivacySettingRuleAllowContacts"}
                    ));
                }
                rules.push(json!({
                "@type":if base==0{
                "userPrivacySettingRuleAllowAll"}
                else{
                "userPrivacySettingRuleRestrictAll"}
                }
                ));
                s.call(
                    "setUserPrivacySettingRules",
                    json!({
                    "setting":key,"rules":{
                    "@type":"userPrivacySettingRules","rules":rules}
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
