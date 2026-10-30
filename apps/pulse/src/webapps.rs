//! Isolated WebKit Mini Apps with account-scoped, explicitly approved native privileges.
use crate::{actions::Scope, features::button, groups::note, window};
use gtk::{glib, prelude::*};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    rc::Rc,
};
struct Browser {
    child: RefCell<Child>,
    input: RefCell<ChildStdin>,
    scope: Scope,
    closed: Cell<bool>,
    close_native: Option<(crate::service::TelegramService, i64)>,
}
thread_local! {
static WINDOWS:RefCell<Vec<Rc<Browser>>>=const{
RefCell::new(Vec::new())}
;
}
impl Browser {
    fn emit(&self, event: &str, data: Value) {
        if self.closed.get() || !self.scope.valid() {
            return;
        }
        let _ = writeln!(
            self.input.borrow_mut(),
            "{}",
            json!({
            "event":event,"data":data}
            )
        );
    }
    fn close(&self) {
        if !self.closed.replace(true) {
            if let Some((service, launch)) = &self.close_native {
                service.request("closeWebApp", json!({"web_app_launch_id":launch}));
            }
            let _ = self.child.borrow_mut().kill();
            let _ = self.child.borrow_mut().wait();
        }
    }
}
pub fn reset() {
    let windows = WINDOWS.with(|v| std::mem::take(&mut *v.borrow_mut()));
    for window in windows {
        window.close();
    }
}
fn spawn(
    scope: Scope,
    url: &str,
    title: &str,
    html: Option<String>,
    close_native: Option<(crate::service::TelegramService, i64)>,
    callback: impl Fn(Rc<Browser>, String, Value) + 'static,
) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Mini Apps require an HTTPS URL.".into());
    }
    let path = nyx_core::paths::cache_dir().join("webview-helper.py");
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, include_str!("../helpers/webview.py")).map_err(|e| e.to_string())?;
    let mut child = Command::new("/usr/bin/python3")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut input = child.stdin.take().ok_or("No browser input stream.")?;
    let output = child.stdout.take().ok_or("No browser output stream.")?;
    let p = crate::theme::current_palette();
    writeln!(input,"{}",json!({
"url":url,"title":title,"html":html,"theme":{
"bg_color":p.bg,"secondary_bg_color":p.surface,"text_color":p.text,"hint_color":p.muted,"link_color":p.accent,"button_color":p.accent,"button_text_color":p.bg}
}
)).map_err(|e|e.to_string())?;
    let browser = Rc::new(Browser {
        child: RefCell::new(child),
        input: RefCell::new(input),
        scope,
        closed: Cell::new(false),
        close_native,
    });
    WINDOWS.with(|v| v.borrow_mut().push(browser.clone()));
    let (tx, rx) = async_channel::bounded(32);
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if line.len() > 1024 * 1024 {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send_blocking(v).is_err() {
                    break;
                }
            }
        }
    });
    let weak = Rc::downgrade(&browser);
    glib::spawn_future_local(async move {
        let mut reported_closed = false;
        while let Ok(v) = rx.recv().await {
            let Some(browser) = weak.upgrade() else { break };
            if !browser.scope.valid() {
                browser.close();
                break;
            }
            let event = v["event"].as_str().unwrap_or("").to_string();
            callback(browser.clone(), event.clone(), v["data"].clone());
            if event == "closed" {
                reported_closed = true;
                browser.close();
                break;
            }
        }
        if let Some(b) = weak.upgrade() {
            if !reported_closed {
                callback(b.clone(), "closed".into(), Value::Null);
            }
            b.close();
            WINDOWS.with(|v| v.borrow_mut().retain(|v| !Rc::ptr_eq(v, &b)));
        }
    });
    Ok(())
}
pub fn open(scope: Scope, chat: i64, bot: i64, url: String, button_text: String) {
    let (d, b) = nyx_ui::widgets::dialog("Open Mini App", 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    note(
        &b,
        "This bot will receive your basic Telegram profile and Mini App session data. Phone, location, clipboard, camera and microphone access require separate permission.",
    );
    note(&b, &url);
    let weak = d.downgrade();
    button(&b, "Open Mini App", move || {
        if !scope.valid() {
            return;
        }
        let (scope, url, button_text, weak) = (scope.clone(), url.clone(), button_text.clone(), weak.clone());
        glib::spawn_future_local(async move {
            let result=async{
let response=scope.call("openWebApp",json!({
"chat_id":chat,"bot_user_id":bot,"url":url,"topic_id":window::composer_topic(chat),"reply_to":window::composer_reply(chat),"parameters":{
"@type":"webAppOpenParameters","theme":null,"application_name":"Pulse","mode":{
"@type":"webAppOpenModeFullSize"}
}
}
)).await?;
let launch=response["launch_id"].as_i64().unwrap_or(0);
let url=response["url"]["url"].as_str().or(response["url"].as_str()).ok_or("Telegram returned no Mini App URL.")?;
let service=window::service();
spawn(scope.clone(),url,"Pulse Mini App",None,service.map(|s|(s,launch)),move|browser,event,data|{
if event=="closed"{return}
handle(browser,event,data,chat,bot,button_text.clone());
}
)?;
if let Some(d)=weak.upgrade(){
d.close();
}
Ok::<(),String>(())}
.await;
            if let Err(e) = result {
                scope.error(&e);
            }
        });
    });
    d.present();
}
fn permission(
    browser: Rc<Browser>,
    title: &str,
    message: &str,
    event: &str,
    data: Value,
    work: impl FnOnce(Scope, Rc<Browser>, Value) + 'static,
) {
    let (d, b) = nyx_ui::widgets::dialog(title, 440);
    crate::dialog_dismissal::add(&d, &b, "Cancel");
    note(&b, message);
    let pending = Rc::new(Cell::new(true));
    let state = pending.clone();
    let browser_ = browser.clone();
    let event_ = event.to_string();
    let data_ = data.clone();
    d.connect_close_request(move |_| {
        if state.replace(false) {
            browser_.emit(&event_, data_.clone());
        }
        glib::Propagation::Proceed
    });
    let work = Rc::new(RefCell::new(Some(work)));
    let weak = d.downgrade();
    button(&b, "Allow", move || {
        if !browser.scope.valid() || browser.closed.get() || !pending.replace(false) {
            return;
        }
        if let Some(work) = work.borrow_mut().take() {
            work(browser.scope.clone(), browser.clone(), data.clone());
        }
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    d.present();
}
fn handle(browser: Rc<Browser>, event: String, data: Value, chat: i64, bot: i64, button_text: String) {
    match event.as_str() {
        "bridge_error" => window::show_status(data["message"].as_str().unwrap_or("Mini App browser error")),
        "web_app_open_tg_link" => {
            let url = data["url"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("https://t.me{}", data["path_full"].as_str().unwrap_or("")));
            if url.starts_with("tg://") || url.starts_with("https://t.me/") {
                window::resolve_link(&url);
            }
        }
        "web_app_request_write_access" => permission(
            browser,
            "Allow bot messages?",
            "This bot will be allowed to send messages to you.",
            "write_access_requested",
            json!({
            "status":"cancelled"}
            ),
            move |s, b, _| {
                glib::spawn_future_local(async move {
                    let result = s
                        .call(
                            "allowBotToSendMessages",
                            json!({
                            "bot_user_id":bot}
                            ),
                        )
                        .await;
                    b.emit(
                        "write_access_requested",
                        json!({
                        "status":if result.is_ok(){
                        "allowed"}
                        else{
                        "cancelled"}
                        }
                        ),
                    );
                });
            },
        ),
        "web_app_request_phone" => permission(
            browser,
            "Share phone number?",
            "The bot will receive your Telegram phone number.",
            "phone_requested",
            json!({
            "status":"cancelled"}
            ),
            move |s, b, _| {
                glib::spawn_future_local(async move {
                    let result = s
                        .call(
                            "sharePhoneNumber",
                            json!({
                            "user_id":bot}
                            ),
                        )
                        .await;
                    b.emit(
                        "phone_requested",
                        json!({
                        "status":if result.is_ok(){
                        "sent"}
                        else{
                        "cancelled"}
                        }
                        ),
                    );
                });
            },
        ),
        "web_app_read_text_from_clipboard" => {
            let req = data["req_id"].clone();
            permission(
                browser,
                "Read clipboard?",
                "This Mini App will receive the text currently on your clipboard.",
                "clipboard_text_received",
                json!({
                "req_id":req}
                ),
                move |s, b, _| {
                    glib::spawn_future_local(async move {
                        if let Some(display) = gtk::gdk::Display::default() {
                            let text = display.clipboard().read_text_future().await.ok().flatten();
                            if s.valid() {
                                b.emit(
                                    "clipboard_text_received",
                                    json!({
                                    "req_id":req,"data":text.map(|s|s.to_string())}
                                    ),
                                );
                            }
                        }
                    });
                },
            );
        }
        "web_app_request_location" => permission(
            browser,
            "Share device location?",
            "This Mini App will receive one current device location fix. GeoClue permission may also be required.",
            "location_requested",
            json!({
            "location_available":false}
            ),
            move |s, b, _| {
                glib::spawn_future_local(async move {
                    let result = crate::locations::device_fix().await;
                    if !s.valid() {
                        return;
                    }
                    match result {
                        Ok((lat, lon, accuracy)) => b.emit(
                            "location_requested",
                            json!({
                            "location_available":true,"latitude":lat,"longitude":lon,"horizontal_accuracy":accuracy}
                            ),
                        ),
                        Err(e) => {
                            s.error(&e);
                            b.emit(
                                "location_requested",
                                json!({
                                "location_available":false}
                                ),
                            );
                        }
                    }
                });
            },
        ),
        "web_app_check_location" => browser.emit(
            "location_checked",
            json!({
            "available":true,"access_requested":false,"access_granted":false}
            ),
        ),
        "web_app_data_send" => {
            let scope = browser.scope.clone();
            let text = data["data"].as_str().unwrap_or("").to_owned();
            glib::spawn_future_local(async move {
                if let Err(e) = crate::groups::commit(
                    &scope,
                    "sendWebAppData",
                    json!({
                    "bot_user_id":bot,"button_text":button_text,"data":text}
                    ),
                )
                .await
                {
                    scope.error(&e);
                }
            });
        }
        "web_app_switch_inline_query" => {
            let query = data["query"].as_str().unwrap_or("").to_string();
            let scope = browser.scope.clone();
            glib::spawn_future_local(async move {
                if let Ok(user) = scope
                    .call(
                        "getUser",
                        json!({
                        "user_id":bot}
                        ),
                    )
                    .await
                {
                    let name = user["usernames"]["active_usernames"][0].as_str().unwrap_or("");
                    crate::bots::inline_prefilled(chat, name, &query);
                }
            });
        }
        "web_app_open_invoice" => {
            if let Some(slug) = data["slug"].as_str() {
                let slug = slug.to_string();
                let source = slug.clone();
                crate::commerce::invoice_input(
                    json!({
                    "@type":"inputInvoiceName","name":slug}
                    ),
                    Some(Rc::new(move |status| {
                        browser.emit(
                            "invoice_closed",
                            json!({
                            "slug":source,"status":status}
                            ),
                        )
                    })),
                );
            }
        }
        "web_app_invoke_custom_method" => {
            let (s, req_id, method, parameters, browser) = (
                browser.scope.clone(),
                data["req_id"].clone(),
                data["method"].as_str().unwrap_or("").to_string(),
                data["params"].clone(),
                browser.clone(),
            );
            if method.is_empty() || method.len() > 128 || parameters.to_string().len() > 1024 * 1024 {
                browser.emit("custom_method_invoked", json!({"req_id":req_id,"error":"Invalid custom request"}));
                return;
            }
            glib::spawn_future_local(async move {
                let result = s
                    .call(
                        "sendWebAppCustomRequest",
                        json!({"bot_user_id":bot,"method":method,"parameters":parameters.to_string()}),
                    )
                    .await;
                match result {
                    Ok(v) => match serde_json::from_str::<Value>(v["result"].as_str().unwrap_or("null")) {
                        Ok(result) => browser.emit("custom_method_invoked", json!({"req_id":req_id,"result":result})),
                        Err(_) => {
                            browser.emit("custom_method_invoked", json!({"req_id":req_id,"error":"Invalid server response"}))
                        }
                    },
                    Err(e) => browser.emit("custom_method_invoked", json!({"req_id":req_id,"error":e})),
                }
            });
        }
        "web_app_biometry_get_info" => browser.emit("biometry_info_received", json!({"available":false})),
        "web_app_start_accelerometer" => browser.emit("accelerometer_failed", json!({"error":"UNSUPPORTED"})),
        "web_app_start_gyroscope" => browser.emit("gyroscope_failed", json!({"error":"UNSUPPORTED"})),
        "web_app_start_device_orientation" => browser.emit("device_orientation_failed", json!({"error":"UNSUPPORTED"})),
        "web_app_request_file_download" => browser.emit(
            "file_download_requested",
            json!({
            "status":"cancelled"}
            ),
        ),
        _ => {}
    }
}
pub fn checkout(scope: Scope, provider: &Value, done: Rc<dyn Fn(Value)>) -> Result<(), String> {
    let (url, html) = match provider["@type"].as_str() {
        Some("paymentProviderOther") => (provider["url"].as_str().ok_or("Missing provider URL.")?.to_string(), None),
        Some("paymentProviderSmartGlocal") => {
            let url = provider["tokenize_url"]
                .as_str()
                .filter(|u| u.starts_with("https://"))
                .ok_or("Invalid provider tokenization URL.")?;
            let endpoint = serde_json::to_string(url).map_err(|e| e.to_string())?;
            let token = serde_json::to_string(provider["public_token"].as_str().ok_or("Missing provider public token.")?)
                .map_err(|e| e.to_string())?;
            let html = include_str!("../helpers/smartglocal.html")
                .replace("__ENDPOINT__", &endpoint.replace('<', "\\u003c"))
                .replace("__TOKEN__", &token.replace('<', "\\u003c"));
            (url.to_string(), Some(html))
        }
        Some("paymentProviderStripe") => {
            let key = serde_json::to_string(provider["publishable_key"].as_str().ok_or("Missing Stripe public key.")?)
                .map_err(|e| e.to_string())?;
            let html = include_str!("../helpers/stripe.html").replace("__PUBLIC_KEY__", &key.replace('<', "\\u003c"));
            ("https://js.stripe.com/".to_string(), Some(html))
        }
        _ => {
            return Err(
                "This provider requires an additional tokenization integration. Select a saved payment method if available."
                    .into(),
            );
        }
    };
    spawn(scope, &url, "Pulse payment provider", html, None, move |b, event, data| {
        if event == "payment_form_submit" {
            let credentials = data["credentials"].clone();
            if credentials.is_object() || credentials.is_string() {
                done(credentials);
                b.close();
            }
        } else if event == "bridge_error" {
            window::show_status(data["message"].as_str().unwrap_or("Payment provider failed."));
        }
    })
}
