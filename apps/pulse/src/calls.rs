//! TDLib signaling with an actual native one-to-one media transport.
use crate::service::TelegramService;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gtk::{gdk, glib, prelude::*};
use nyx_telegram_calls::engine::{Engine, EngineEvent};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
thread_local! {static SESSION:RefCell<Option<Rc<Session>>>=const{RefCell::new(None)};static LAST_CALL:RefCell<Value>=const{RefCell::new(Value::Null)};}
struct Session {
    engine: Arc<Mutex<Engine>>,
    service: TelegramService,
    user: i64,
    id: Cell<i32>,
    video: Cell<bool>,
    ready: Cell<bool>,
    call: RefCell<Value>,
    screen: RefCell<Option<Arc<AtomicBool>>>,
}
fn initialize(s: TelegramService, user: i64, video: bool) -> Result<Rc<Session>, String> {
    if crate::group_calls::active() || SESSION.with(|v| v.borrow().is_some()) {
        return Err("Finish the current call first".into());
    }
    let (mut engine, rx) = Engine::new()?;
    engine.create(user)?;
    let devices = Engine::devices()?;
    let p = crate::prefs::get();
    let selected = |kind: &str, pref: &str| {
        if pref.is_empty() { devices[kind][0]["metadata"].as_str().unwrap_or("").to_owned() } else { pref.to_owned() }
    };
    engine.media(
        user,
        &selected("microphone", &p.microphone),
        &selected("speaker", &p.speaker),
        if video { Some(selected("camera", &p.camera)) } else { None }.as_deref(),
        false,
    )?;
    let session = Rc::new(Session {
        engine: Arc::new(Mutex::new(engine)),
        service: s,
        user,
        id: Cell::new(0),
        video: Cell::new(video),
        ready: Cell::new(false),
        call: RefCell::new(Value::Null),
        screen: RefCell::new(None),
    });
    SESSION.with(|v| *v.borrow_mut() = Some(session.clone()));
    let weak = Rc::downgrade(&session);
    glib::spawn_future_local(async move {
        while let Ok(e) = rx.recv().await {
            let Some(s) = weak.upgrade() else { break };
            match e {
                EngineEvent::BroadcastTime(_)|EngineEvent::BroadcastPart(..)=>{},
                EngineEvent::Signal(_, bytes) => {
                    if s.id.get() != 0 {
                        s.service.request("sendCallSignalingData", json!({"call_id":s.id.get(),"data":STANDARD.encode(bytes)}));
                    }
                }
                EngineEvent::Connection(_, state) => {
                    if state == 1 {
                        if let Some(ui) = crate::window::call_ui() {
                            let call = s.call.borrow();
                            let emojis = call["state"]["emojis"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect::<Vec<_>>();
                            ui.show_active(s.id.get(), s.user, &format!("User {}", s.user), s.video.get(), &emojis);
                            resolve_peer(&s.service, s.id.get(), s.user);
                        }
                    } else if state >= 2 {
                        s.service.discard_call(s.id.get(), 0, s.video.get());
                        crate::window::show_status("Call disconnected");
                        stop();
                        if let Some(ui) = crate::window::call_ui() {
                            ui.dismiss();
                        }
                    } else {
                        crate::window::show_status("Connecting call…");
                    }
                }
                EngineEvent::Frame(_, _, w, h, rotation, data) => {
                    if let Some(bytes) = i420_rgba(w, h, &data) {
                        let (w, h, bytes) = rotate_rgba(w, h, bytes, rotation);
                        if let Some(ui) = crate::window::call_ui() {
                            let tex = gdk::MemoryTexture::new(
                                w as i32,
                                h as i32,
                                gdk::MemoryFormat::R8g8b8a8,
                                &glib::Bytes::from_owned(bytes),
                                w as usize * 4,
                            );
                            ui.set_video(tex.upcast_ref());
                        }
                    }
                }
            }
        }
    });
    Ok(session)
}
pub fn start(s: TelegramService, user: i64, video: bool) -> bool {
    match initialize(s.clone(), user, video) {
        Ok(session) => {
            let protocol = match Engine::protocol() {
                Ok(p) => p,
                Err(e) => {
                    crate::window::show_status(&e);
                    stop();
                    return false;
                }
            };
            let rx = s.request("createCall", json!({"user_id":user,"protocol":protocol,"is_video":video}));
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(v)) => {
                        session.id.set(v["id"].as_i64().unwrap_or(0) as i32);
                        if let Some(ui) = crate::window::call_ui() {
                            ui.show_connecting(session.id.get(), user, video);
                        }
                        crate::window::show_status("Calling…");
                    }
                    Ok(Err(e)) => {
                        crate::window::show_status(&e);
                        stop();
                    }
                    _ => stop(),
                }
            });
            true
        }
        Err(e) => {
            crate::window::show_status(&e);
            false
        }
    }
}
pub fn accept(s: TelegramService, id: i32) {
    let call = LAST_CALL.with(|v| v.borrow().clone());
    if call["id"].as_i64() != Some(id as i64) {
        return;
    }
    let user = call["user_id"].as_i64().unwrap_or(0);
    match initialize(s.clone(), user, call["is_video"] == true) {
        Ok(session) => {
            session.id.set(id);
            *session.call.borrow_mut() = call;
            match Engine::protocol() {
                Ok(protocol) => {
                    let rx = s.request("acceptCall", json!({"call_id":id,"protocol":protocol}));
                    glib::spawn_future_local(async move {
                        if let Ok(Err(e)) = rx.recv().await {
                            crate::window::show_status(&e);
                            stop();
                        }
                    });
                }
                Err(e) => {
                    crate::window::show_status(&e);
                    stop();
                }
            }
        }
        Err(e) => {
            crate::window::show_status(&e);
            if let Some(ui) = crate::window::call_ui() {
                ui.connection_error(&e);
            }
        }
    }
}
pub fn update(v: &Value) {
    match v["@type"].as_str() {
        Some("updateCall") => {
            let call = &v["call"];
            let previous = LAST_CALL.with(|c| c.borrow().clone());
            LAST_CALL.with_borrow_mut(|v| *v = call.clone());
            let id = call["id"].as_i64().unwrap_or(0) as i32;
            let state = call["state"]["@type"].as_str().unwrap_or("");
            if state != "callStatePending" {
                if let Some(app) = gtk::gio::Application::default() {
                    app.withdraw_notification(&format!("call-{id}"));
                }
            }
            if call["is_outgoing"] != true
                && state == "callStatePending"
                && (previous["id"] != call["id"] || previous["state"]["@type"] != "callStatePending")
            {
                let prefs = crate::prefs::get();
                if prefs.notifications_enabled {
                    if let Some(app) = gtk::gio::Application::default() {
                        let notification = gtk::gio::Notification::new("Incoming Telegram call");
                        notification.set_body(Some(if call["is_video"] == true {
                            "Incoming video call"
                        } else {
                            "Incoming voice call"
                        }));
                        notification
                            .set_default_action_and_target_value("app.present-call", Some(&prefs.active_account_id.to_variant()));
                        app.send_notification(Some(&format!("call-{id}")), &notification);
                    }
                }
                if prefs.sound_enabled {
                    let _ = std::process::Command::new("canberra-gtk-play").args(["-i", "phone-incoming-call"]).spawn();
                }
            }

            if state == "callStateDiscarded" || state == "callStateError" {
                if SESSION.with(|v| v.borrow().as_ref().is_some_and(|s| s.id.get() != id)) {
                    return;
                }
                stop();
                if let Some(ui) = crate::window::call_ui() {
                    ui.dismiss();
                }
                if state == "callStateError" {
                    crate::window::show_status(call["state"]["error"]["message"].as_str().unwrap_or("Call failed"));
                }
                return;
            }
            let session = SESSION.with(|v| v.borrow().clone());
            if let Some(s) = session {
                if s.user != call["user_id"].as_i64().unwrap_or(0) {
                    return;
                }
                s.id.set(id);
                *s.call.borrow_mut() = call.clone();
                if state == "callStateReady" && !s.ready.replace(true) {
                    let engine = s.engine.clone();
                    let user = s.user;
                    let state = call["state"].clone();
                    let outgoing = call["is_outgoing"] == true;
                    let (tx, rx) = async_channel::bounded(1);
                    std::thread::spawn(move || {
                        let r = engine.lock().unwrap().connect(user, &state, outgoing);
                        let _ = tx.send_blocking(r);
                    });
                    glib::spawn_future_local(async move {
                        if let Ok(Err(e)) = rx.recv().await {
                            crate::window::show_status(&e);
                            stop();
                        }
                    });
                }
            } else if call["is_outgoing"] != true && state == "callStatePending" {
                if let Some(service) = crate::window::service() {
                    resolve_peer(&service, id, call["user_id"].as_i64().unwrap_or(0));
                }
                if let Some(ui) = crate::window::call_ui() {
                    ui.show_incoming(
                        id,
                        call["user_id"].as_i64().unwrap_or(0),
                        "Incoming Telegram call",
                        call["is_video"] == true,
                    );
                }
            }
        }
        Some("updateNewCallSignalingData") => {
            if let Some(s) = SESSION.with(|v| v.borrow().clone()) {
                if s.id.get() == v["call_id"].as_i64().unwrap_or(0) as i32 {
                    if let Ok(data) = STANDARD.decode(v["data"].as_str().unwrap_or("")) {
                        let engine = s.engine.clone();
                        let user = s.user;
                        std::thread::spawn(move || {
                            let _ = engine.lock().unwrap().signal(user, &data);
                        });
                    }
                }
            }
        }
        _ => {}
    }
}
pub fn stop() {
    if let Some(s) = SESSION.with(|v| v.borrow_mut().take()) {
        if let Some(flag) = s.screen.borrow_mut().take() {
            flag.store(true, Ordering::Relaxed);
        }
        let e = s.engine.clone();
        let user = s.user;
        std::thread::spawn(move || e.lock().unwrap().stop(user));
    }
}
pub fn mute(on: bool) -> bool {
    SESSION.with(|v| {
        v.borrow()
            .as_ref()
            .map(|s| match s.engine.lock().unwrap().mute(s.user, on) {
                Ok(()) => true,
                Err(e) => {
                    crate::window::show_status(&e);
                    false
                }
            })
            .unwrap_or(false)
    })
}
pub fn camera(on: bool) -> bool {
    let session = SESSION.with(|v| v.borrow().clone());
    let Some(s) = session else { return false };
    s.video.set(on);
    let sharing = s.screen.borrow().is_some();
    configure(&s, sharing)
}
fn configure(s: &Session, screen: bool) -> bool {
    let p = crate::prefs::get();
    let devices = Engine::devices().unwrap_or_default();
    let select = |kind: &str, pref: &str| {
        if pref.is_empty() { devices[kind][0]["metadata"].as_str().unwrap_or("").to_owned() } else { pref.to_owned() }
    };
    match s.engine.lock().unwrap().media(
        s.user,
        &select("microphone", &p.microphone),
        &select("speaker", &p.speaker),
        if s.video.get() { Some(select("camera", &p.camera)) } else { None }.as_deref(),
        screen,
    ) {
        Ok(()) => true,
        Err(e) => {
            crate::window::show_status(&e);
            false
        }
    }
}
pub fn share(on: bool) {
    let Some(s) = SESSION.with(|v| v.borrow().clone()) else { return };
    if let Some(flag) = s.screen.borrow_mut().take() {
        flag.store(true, Ordering::Relaxed);
    }
    if !on {
        configure(&s, false);
        return;
    }
    if !configure(&s, true) {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    *s.screen.borrow_mut() = Some(stop.clone());
    let token = stop.clone();
    let engine = s.engine.clone();
    let user = s.user;
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())
            .and_then(|rt| rt.block_on(nyx_telegram_calls::capture::run(engine, user, stop)));
        let _ = tx.send_blocking(result);
    });
    glib::spawn_future_local(async move {
        let result = rx.recv().await;
        if let Some(s) = SESSION.with(|v| v.borrow().clone()) {
            let current = s.screen.borrow().as_ref().is_some_and(|f| Arc::ptr_eq(f, &token));
            if current {
                s.screen.borrow_mut().take();
                configure(&s, false);
                if let Ok(Err(e)) = result {
                    crate::window::show_status(&e);
                }
                if let Some(ui) = crate::window::call_ui() {
                    ui.screen_stopped();
                }
            }
        }
    });
}
pub fn devices(button: &gtk::Button) {
    let Some(parent) = button.root().and_downcast::<gtk::Window>() else { return };
    let d = gtk::Window::builder().title("Call devices").transient_for(&parent).modal(true).default_width(440).build();
    let body = nyx_ui::widgets::vbox(12);
    let data = match Engine::devices() {
        Ok(d) => d,
        Err(e) => {
            crate::window::show_status(&e);
            return;
        }
    };
    let p = crate::prefs::get();
    for (kind, title, selected) in
        [("microphone", "Microphone", p.microphone), ("speaker", "Speakers", p.speaker), ("camera", "Camera", p.camera)]
    {
        body.append(&nyx_ui::widgets::label(title, "dim"));
        let options = data[kind].as_array().cloned().unwrap_or_default();
        let names = options.iter().map(|v| v["name"].as_str().unwrap_or("Device")).collect::<Vec<_>>();
        let drop = gtk::DropDown::from_strings(&names);
        if let Some(index) = options.iter().position(|v| v["metadata"].as_str() == Some(&selected)) {
            drop.set_selected(index as u32);
        }
        drop.connect_selected_notify(move |d| {
            if let Some(value) = options.get(d.selected() as usize).and_then(|v| v["metadata"].as_str()) {
                crate::prefs::update(|p| match kind {
                    "microphone" => p.microphone = value.into(),
                    "speaker" => p.speaker = value.into(),
                    _ => p.camera = value.into(),
                });
                if let Some(s) = SESSION.with(|v| v.borrow().clone()) {
                    let sharing = s.screen.borrow().is_some();
                    configure(&s, sharing);
                }
            }
        });
        body.append(&drop);
    }
    d.set_child(Some(&body));
    crate::dialog_dismissal::add(&d, &body, "Close");
    crate::dialog_dismissal::escape(&d);
    d.present();
}
pub(crate) fn i420_rgba(w: u16, h: u16, data: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (w as usize, h as usize);
    let cw = w.div_ceil(2);
    let chroma = cw * h.div_ceil(2);
    if w == 0 || h == 0 || data.len() < w * h + 2 * chroma {
        return None;
    }
    let mut out = vec![0; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let yy = (data[y * w + x] as f32 - 16.0) * 1.164383;
            let u = data[w * h + (y / 2) * cw + x / 2] as f32 - 128.0;
            let v = data[w * h + chroma + (y / 2) * cw + x / 2] as f32 - 128.0;
            let i = (y * w + x) * 4;
            out[i] = (yy + 1.596027 * v).clamp(0.0, 255.0) as u8;
            out[i + 1] = (yy - 0.391762 * u - 0.812968 * v).clamp(0.0, 255.0) as u8;
            out[i + 2] = (yy + 2.017232 * u).clamp(0.0, 255.0) as u8;
            out[i + 3] = 255;
        }
    }
    Some(out)
}

fn resolve_peer(service: &TelegramService, id: i32, user: i64) {
    let generation = service.generation.clone();
    let rx = service.request("getUser", json!({"user_id":user}));
    glib::spawn_future_local(async move {
        if let Ok(Ok(user)) = rx.recv().await {
            if crate::window::service().is_some_and(|s| s.generation == generation) {
                if let Some(ui) = crate::window::call_ui() {
                    let name =
                        format!("{} {}", user["first_name"].as_str().unwrap_or(""), user["last_name"].as_str().unwrap_or(""));
                    ui.set_peer_name(id, name.trim());
                }
            }
        }
    });
}

pub(crate) fn rotate_rgba(w: u16, h: u16, data: Vec<u8>, rotation: u32) -> (u16, u16, Vec<u8>) {
    let turns = match rotation {
        1 | 90 => 1,
        2 | 180 => 2,
        3 | 270 => 3,
        _ => 0,
    };
    if turns == 0 {
        return (w, h, data);
    }
    let (width, height) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    let mut out = vec![0; data.len()];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (nx, ny) = match turns {
                1 => (h as usize - 1 - y, x),
                2 => (w as usize - 1 - x, h as usize - 1 - y),
                _ => (y, w as usize - 1 - x),
            };
            let old = (y * w as usize + x) * 4;
            let new = (ny * width as usize + nx) * 4;
            out[new..new + 4].copy_from_slice(&data[old..old + 4]);
        }
    }
    (width, height, out)
}
#[cfg(test)]
mod video_regression {
    use super::*;
    #[test]
    fn limited_range_colors_and_portrait_rotation() {
        assert_eq!(i420_rgba(1, 1, &[16, 128, 128]).unwrap(), vec![0, 0, 0, 255]);
        assert_eq!(i420_rgba(1, 1, &[235, 128, 128]).unwrap(), vec![254, 254, 254, 255]);
        assert!(i420_rgba(2, 2, &[0]).is_none());
        let (w, h, data) = rotate_rgba(1, 2, vec![255, 0, 0, 255, 0, 0, 255, 255], 1);
        assert_eq!((w, h), (2, 1));
        assert_eq!(data, vec![0, 0, 255, 255, 255, 0, 0, 255]);
    }
}

pub(crate) fn active() -> bool { SESSION.with(|v| v.borrow().is_some()) }
