//! Profile history, birthday/phone changes, passkeys and session capability controls.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button, check},
    groups::{entry, note, submit},
    window,
};
use gtk::{gio, prelude::*};
use serde_json::{Value, json};
use std::io::Write;
pub fn open() {
    features::load("Profile and security tools", |_s, _d, b| async move {
        button(&b, "Profile photo history", photos);
        button(&b, "Birthday", birthday);
        button(&b, "Change phone number…", phone);
        button(&b, "Passkeys", passkeys);
        button(&b, "Session permissions and confirmation", sessions);
        Ok(())
    });
}
fn birthday() {
    features::load("Birthday", |s, d, b| async move {
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
        let enabled = check(&fields, "Show birthday", !info["birthdate"].is_null());
        let day = entry(&fields, "Day", &info["birthdate"]["day"].as_i64().unwrap_or(1).to_string(), 2);
        let month = entry(&fields, "Month", &info["birthdate"]["month"].as_i64().unwrap_or(1).to_string(), 2);
        let year = entry(&fields, "Year (0 to hide)", &info["birthdate"]["year"].as_i64().unwrap_or(0).to_string(), 4);
        submit(&d, &b, &fields, s, "Save birthday", move |s| {
            let (enabled, day, month, year) =
                (enabled.is_active(), day.text().parse::<u32>(), month.text().parse::<u32>(), year.text().parse::<i32>());
            async move {
                let birthdate = if !enabled {
                    Value::Null
                } else {
                    let (day, month, year) = (
                        day.map_err(|_| "Invalid day.")?,
                        month.map_err(|_| "Invalid month.")?,
                        year.map_err(|_| "Invalid year.")?,
                    );
                    if chrono::NaiveDate::from_ymd_opt(if year == 0 { 2000 } else { year }, month, day).is_none() {
                        return Err("Enter a valid birthday.".into());
                    }
                    json!({
                    "@type":"birthdate","day":day,"month":month,"year":year}
                    )
                };
                s.call(
                    "setBirthdate",
                    json!({
                    "birthdate":birthdate}
                    ),
                )
                .await?;
                Ok(())
            }
        });
        Ok(())
    });
}
fn photos() {
    features::load("Profile photos", |s, d, b| async move {
        let me = s.call("getMe", json!({})).await?;
        let more = gtk::Button::with_label("Load more");
        b.append(&more);
        let list = nyx_ui::widgets::vbox(6);
        b.append(&list);
        let offset = std::rc::Rc::new(std::cell::Cell::new(0));
        let weak = d.downgrade();
        more.connect_clicked(move|button|{
if !s.valid(){
return}
button.set_sensitive(false);
let(s,me,button,list,offset,weak)=(s.clone(),me.clone(),button.clone(),list.clone(),offset.clone(),weak.clone());
gtk::glib::spawn_future_local(async move{
let result=s.call("getUserProfilePhotos",json!({
"user_id":me["id"],"offset":offset.get(),"limit":30}
)).await;
button.set_sensitive(true);
if !s.valid()||weak.upgrade().is_none_or(|d|!d.is_visible()){
return}
match result{
Err(e)=>{
note(&list,&e);
}
,Ok(data)=>{
let photos=data["photos"].as_array().cloned().unwrap_or_default();
offset.set(offset.get()+photos.len()as i32);
for photo in &photos{
let id=photo["id"].clone();
let message=json!({
"id":1,"chat_id":me["id"],"date":photo["added_date"].as_i64().unwrap_or(0),"can_be_saved":true,"content":{
"@type":"messagePhoto","photo":{
"sizes":photo["sizes"]}
}
}
);
if let Some(message)=nyx_telegram_core::client::parse_message(&message){
crate::poll_media::render(&list,&message,&json!({
"@type":"pollMediaPhoto","photo":{
"sizes":photo["sizes"]}
}
));
}
let(scope,id_)=(s.clone(),id.clone());
button_fn(&list,"Set as current photo",move||{
if !scope.valid(){
return}
let id=id_.clone();
actions::run(0,"profile-photo",move|s|async move{
s.call("setProfilePhoto",json!({
"photo":{
"@type":"inputChatPhotoPrevious","chat_photo_id":id}
,"is_public":false}
)).await?;
Ok(())}
);
}
);
let scope=s.clone();
button_fn(&list,"Delete photo…",move||{
if !scope.valid(){
return}
let id=id.clone();
actions::operation("Delete profile photo?","This removes the photo from your Telegram profile history.",vec![],"Delete photo",move|s,_|{
let id=id.clone();
async move{
s.call("deleteProfilePhoto",json!({
"profile_photo_id":id}
)).await?;
Ok(())}
}
);
}
);
}
button.set_visible(!photos.is_empty()&&(offset.get()as i64)<data["total_count"].as_i64().unwrap_or(0));
}
}
}
);
}
);
        more.emit_clicked();
        Ok(())
    });
}
fn button_fn(b: &gtk::Box, title: &str, f: impl Fn() + 'static) {
    button(b, title, f);
}
fn phone() {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Change Telegram phone number", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    note(
        &fields,
        "This changes the phone number for your existing account. Messages and contacts are retained. A verification code will be sent to the new number.",
    );
    let phone = entry(&fields, "New phone including country code", "", 32);
    submit(&d, &b, &fields, s, "Send verification code", move |s| {
        let phone = phone.text().trim().to_string();
        async move {
            if phone.chars().filter(char::is_ascii_digit).count() < 5 {
                return Err("Enter a phone number with country code.".into());
            }
            s.call("sendPhoneNumberCode",json!({
"phone_number":phone,"settings":{
"@type":"phoneNumberAuthenticationSettings","allow_flash_call":false,"allow_missed_call":false,"is_current_phone_number":false,"allow_sms_retriever_api":false,"firebase_authentication_settings":null,"authentication_tokens":[]}
,"type":{
"@type":"phoneNumberCodeTypeChange"}
}
)).await?;
            phone_code(s);
            Ok(())
        }
    });
    d.present();
}
fn phone_code(scope: Scope) {
    let (d, b) = nyx_ui::widgets::dialog("Verify new phone number", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let code = entry(&fields, "Verification code", "", 64);
    submit(&d, &b, &fields, scope, "Change phone number", move |s| {
        let code = code.text().trim().to_string();
        async move {
            s.call(
                "checkPhoneNumberCode",
                json!({
                "code":code}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
fn sessions() {
    features::load("Session permissions", |s, d, b| async move {
        let data = s.call("getActiveSessions", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        for session in data["sessions"].as_array().into_iter().flatten() {
            note(
                &b,
                &format!(
                    "{} · {} · {}",
                    session["application_name"].as_str().unwrap_or("Device"),
                    session["device_model"].as_str().unwrap_or(""),
                    session["location"].as_str().unwrap_or("")
                ),
            );
            let id = session["id"].clone();
            if session["is_unconfirmed"] == true {
                let (scope, id) = (s.clone(), id.clone());
                button(&b, "Confirm this device…", move || {
                    if !scope.valid() {
                        return;
                    }
                    let id = id.clone();
                    actions::operation(
                        "Confirm this session?",
                        "Only confirm a device you recognize. It will become an authorized session.",
                        vec![],
                        "Confirm",
                        move |s, _| {
                            let id = id.clone();
                            async move {
                                s.call(
                                    "confirmSession",
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
            for (label, method, key, current) in [
                ("Calls", "toggleSessionCanAcceptCalls", "can_accept_calls", session["can_accept_calls"] == true),
                (
                    "Secret chats",
                    "toggleSessionCanAcceptSecretChats",
                    "can_accept_secret_chats",
                    session["can_accept_secret_chats"] == true,
                ),
            ] {
                let (scope, id) = (s.clone(), id.clone());
                button(&b, &format!("{} {label}", if current { "Disable" } else { "Enable" }), move || {
                    if !scope.valid() {
                        return;
                    }
                    let mut args = json!({
                    "session_id":id}
                    );
                    args[key] = json!(!current);
                    actions::request(0, method, args);
                });
            }
        }
        Ok(())
    });
}
fn helper(create: bool, device: String, pin: String, parameters: Value) -> Result<Value, String> {
    let script = nyx_core::paths::cache_dir().join("passkey-helper.py");
    std::fs::create_dir_all(script.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&script, include_str!("../helpers/passkey.py")).map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new("/usr/bin/python3")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    writeln!(
        child.stdin.take().ok_or("Passkey helper failed.")?,
        "{}",
        json!({
        "create":create,"device":device,"pin":pin,"parameters":parameters}
        )
    )
    .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let result: Value = serde_json::from_slice(&output.stdout).map_err(|_| "Passkey helper returned no response.")?;
    if let Some(error) = result["error"].as_str() {
        return Err(error.to_string());
    }
    Ok(result["result"].clone())
}
pub fn login_passkey() {
    passkey_operation(false);
}
fn passkeys() {
    features::load("Passkeys", |s, d, b| async move {
        let data = s.call("getLoginPasskeys", json!({})).await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        for key in data["passkeys"].as_array().into_iter().flatten() {
            note(&b, key["name"].as_str().unwrap_or("Passkey"));
            let (scope, id) = (s.clone(), key["id"].clone());
            button(&b, "Remove passkey…", move || {
                if !scope.valid() {
                    return;
                }
                let id = id.clone();
                actions::operation(
                    "Remove passkey?",
                    "This key will no longer be able to log in to your account.",
                    vec![],
                    "Remove",
                    move |s, _| {
                        let id = id.clone();
                        async move {
                            s.call(
                                "removeLoginPasskey",
                                json!({
                                "passkey_id":id}
                                ),
                            )
                            .await?;
                            Ok(())
                        }
                    },
                );
            });
        }
        button(&b, "Add hardware security-key passkey…", || passkey_operation(true));
        Ok(())
    });
}
fn passkey_operation(create: bool) {
    features::load(if create { "Add passkey" } else { "Log in with passkey" }, move |s, d, b| async move {
        let devices = gio::spawn_blocking(|| {
            let path = nyx_core::paths::cache_dir().join("passkey-helper.py");
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, include_str!("../helpers/passkey.py")).map_err(|e| e.to_string())?;
            let output =
                std::process::Command::new("/usr/bin/python3").arg(path).arg("--devices").output().map_err(|e| e.to_string())?;
            let data: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
            data["result"]
                .as_array()
                .cloned()
                .ok_or_else(|| data["error"].as_str().unwrap_or("Security-key enumeration failed.").to_string())
        })
        .await
        .map_err(|_| "Security-key enumeration stopped.")??;
        if !alive(&s, &d) {
            return Ok(());
        }
        if devices.is_empty() {
            return Err("Connect a FIDO2 security key and reopen this dialog.".into());
        }
        let fields = nyx_ui::widgets::vbox(8);
        b.append(&fields);
        let names = devices.iter().map(|d| d["name"].as_str().unwrap_or("Security key")).collect::<Vec<_>>();
        let selected = gtk::DropDown::from_strings(&names);
        fields.append(&selected);
        note(&fields, "Security-key PIN (if configured). Touch the key when requested.");
        let pin = gtk::PasswordEntry::new();
        pin.set_show_peek_icon(true);
        fields.append(&pin);
        submit(&d, &b, &fields, s, if create { "Create passkey" } else { "Authenticate" }, move |s| {
            let (device, pin) =
                (devices[selected.selected() as usize]["path"].as_str().unwrap_or("").to_string(), pin.text().to_string());
            async move {
                let result =
                    s.call(if create { "getPasskeyParameters" } else { "getAuthenticationPasskeyParameters" }, json!({})).await?;
                let parameters: Value =
                    serde_json::from_str(result["text"].as_str().ok_or("No WebAuthn parameters.")?).map_err(|e| e.to_string())?;
                window::show_status("Touch your security key to continue…");
                let response = gio::spawn_blocking(move || helper(create, device, pin, parameters))
                    .await
                    .map_err(|_| "Security-key operation stopped.")??;
                if create {
                    s.call("addLoginPasskey", response).await?;
                } else {
                    s.call("checkAuthenticationPasskey", response).await?;
                }
                Ok(())
            }
        });
        Ok(())
    });
}
