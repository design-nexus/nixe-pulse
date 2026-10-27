//! Explicit, initially muted group audio transport using the bundled NTgCalls engine.
use crate::{
    actions::{self, Scope},
    features::{self, alive, button},
    groups::{entry, note, submit},
    service::TelegramService,
    window,
};
use gtk::{glib, prelude::*};
use nyx_telegram_calls::engine::{Engine, EngineEvent};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
struct Session {
    engine: Arc<Mutex<Engine>>,
    service: TelegramService,
    key: i64,
    id: i64,
    muted: Cell<bool>,
    camera: Cell<bool>,
    screen: RefCell<Option<Arc<AtomicBool>>>,
    streams: RefCell<BTreeMap<String, Value>>,
    frames: RefCell<BTreeMap<u32, gtk::Picture>>,
    video_grid: RefCell<Option<gtk::FlowBox>>,
}
thread_local! {
static SESSION:RefCell<Option<Rc<Session>>>=const{
RefCell::new(None)}
;
static JOINING:Cell<bool>=const{
Cell::new(false)}
;
static MEMBERS:RefCell<BTreeMap<(i64,String),Value>>=const{
RefCell::new(BTreeMap::new())}
;
}
pub fn active() -> bool {
    SESSION.with(|v| v.borrow().is_some()) || JOINING.with(Cell::get)
}
pub fn stop() {
    if let Some(session) = SESSION.with(|v| v.borrow_mut().take()) {
        if let Some(flag) = session.screen.borrow_mut().take() {
            flag.store(true, Ordering::Relaxed);
        }
        if let Ok(mut e) = session.engine.lock() {
            e.stop(session.key);
        }
        session.service.request(
            "leaveGroupCall",
            json!({
            "group_call_id":session.id}
            ),
        );
    }
    MEMBERS.with(|m| m.borrow_mut().clear());
}
pub fn update(v: &Value) {
    if v["@type"] == "updateGroupCallParticipant" {
        if let Some(id) = v["group_call_id"].as_i64() {
            let p = &v["participant"];
            MEMBERS.with(|m| {
                let key=(id,p["participant_id"].to_string());if p["order"].as_str()==Some(""){m.borrow_mut().remove(&key);}else{m.borrow_mut().insert(key,p.clone());}
            });
            if let Some(session) = SESSION.with(|s| s.borrow().as_ref().filter(|s| s.id == id).cloned()) {
                sync_video(&session);
            }
        }
    }
    if v["@type"] == "updateGroupCall" {
        let c = &v["group_call"];
        if c["is_active"] == false {
            let id = c["id"].as_i64();
            if SESSION.with(|s| s.borrow().as_ref().is_some_and(|s| Some(s.id) == id)) {
                stop();
            }
        }
    }
}
pub fn open(chat: i64) {
    features::load("Group call", move |s, d, b| async move {
        let c = s
            .call(
                "getChat",
                json!({
                "chat_id":chat}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let id = c["video_chat"]["group_call_id"].as_i64().unwrap_or(0);
        if id == 0 {
            note(&b, "No active group call.");
            let scope = s.clone();
            button(&b, "Create group call…", move || {
                if scope.valid() {
                    create(chat);
                }
            });
            return Ok(());
        }
        let call = s
            .call(
                "getGroupCall",
                json!({
                "group_call_id":id}
                ),
            )
            .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        note(&b, &format!("{} · {} participants", call["title"].as_str().unwrap_or("Group call"), call["participant_count"]));
        let status = note(
            &b,
            if active() {
                "A call transport is already active."
            } else {
                "Join with microphone muted. Camera and screen sharing are not started."
            },
        );
        let scope = s.clone();
        button(&b, "Join audio (muted)", move || {
            if scope.valid() {
                join(chat, id);
            }
        });
        let scope = s.clone();
        button(&b, "Mute / unmute microphone", move || {
            if !scope.valid() {
                return;
            }
            let session = SESSION.with(|v| v.borrow().clone());
            if let Some(session) = session.filter(|v| v.id == id) {
                let muted = !session.muted.get();
                let me = json!({
                "@type":"messageSenderUser","user_id":0}
                );
                let _ = me;
                actions::run(chat, "group-call-mute", move |s| async move {
                    let me = s.call("getMe", json!({})).await?;
                    s.call(
                        "toggleGroupCallParticipantIsMuted",
                        json!({
                        "group_call_id":id,"participant_id":{
                        "@type":"messageSenderUser","user_id":me["id"]}
                        ,"is_muted":muted}
                        ),
                    )
                    .await?;
                    session.engine.lock().map_err(|_| "Call engine unavailable.")?.mute(session.key, muted)?;
                    session.muted.set(muted);
                    window::show_status(if muted { "Microphone muted" } else { "Microphone unmuted" });
                    Ok(())
                });
            }
        });
        button(&b, "Leave call", stop);
        let scope = s.clone();
        button(&b, "Camera on / off", move || {
            if scope.valid() {
                camera(id);
            }
        });
        let scope = s.clone();
        button(&b, "Share screen / stop sharing…", move || {
            if scope.valid() {
                screen(id);
            }
        });
        let scope = s.clone();
        button(&b, "View participant videos", move || {
            if scope.valid() {
                video_window(id);
            }
        });
        let scope = s.clone();
        button(&b, "Participants / refresh", move || {
            if scope.valid() {
                participants(id);
            }
        });
        if call["can_be_managed"] == true {
            let scope = s.clone();
            button(&b, "End call for everyone…", move || {
                if !scope.valid() {
                    return;
                }
                actions::operation(
                    "End group call?",
                    "Everyone will be disconnected.",
                    vec![],
                    "End call",
                    move |s, _| async move {
                        if s.call(
                            "getGroupCall",
                            json!({
                            "group_call_id":id}
                            ),
                        )
                        .await?["can_be_managed"]
                            != true
                        {
                            return Err("Your call management permission changed.".into());
                        }
                        s.call(
                            "endGroupCall",
                            json!({
                            "group_call_id":id}
                            ),
                        )
                        .await?;
                        stop();
                        Ok(())
                    },
                );
            });
            let scope = s.clone();
            button(&b, "Copy invite link", move || {
                if !scope.valid() {
                    return;
                }
                actions::run(chat, "group-call-link", move |s| async move {
                    let url = s
                        .call(
                            "getVideoChatInviteLink",
                            json!({
                            "group_call_id":id,"can_self_unmute":false}
                            ),
                        )
                        .await?;
                    if let (Some(d), Some(url)) = (gtk::gdk::Display::default(), url["url"].as_str()) {
                        d.clipboard().set_text(url);
                    }
                    Ok(())
                });
            });
        }
        let weak = d.downgrade();
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            if weak.upgrade().is_none_or(|d| !d.is_visible()) {
                return glib::ControlFlow::Break;
            }
            if !s.valid() {
                return glib::ControlFlow::Break;
            }
            let text = SESSION.with(|v| {
                v.borrow()
                    .as_ref()
                    .filter(|v| v.id == id)
                    .map(|v| if v.muted.get() { "Joined · microphone muted" } else { "Joined · microphone on" })
            });
            status.set_text(text.unwrap_or("Not joined"));
            glib::ControlFlow::Continue
        });
        Ok(())
    });
}
fn create(chat: i64) {
    let Some(s) = Scope::current() else { return };
    let (d, b) = nyx_ui::widgets::dialog("Create group call", 440);
    let fields = nyx_ui::widgets::vbox(8);
    b.append(&fields);
    let title = entry(&fields, "Title", "", 128);
    submit(&d, &b, &fields, s, "Create call", move |s| {
        let title = title.text().to_string();
        async move {
            let c = crate::groups::context(&s, chat).await?;
            c.require("can_manage_video_chats")?;
            crate::groups::commit(
                &s,
                "createVideoChat",
                json!({
                "chat_id":chat,"title":title,"start_date":0,"is_rtmp_stream":false}
                ),
            )
            .await?;
            Ok(())
        }
    });
    d.present();
}
fn join(chat: i64, id: i64) {
    join_mode(chat, id, false)
}
pub fn join_live(chat: i64, id: i64) {
    join_mode(chat, id, true)
}
fn join_mode(chat: i64, id: i64, live: bool) {
    if active() || crate::calls::active() {
        window::show_status("Finish the current call first.");
        return;
    }
    let Some(s) = Scope::current() else { return };
    let Some(service) = window::service() else { return };
    JOINING.with(|v| v.set(true));
    glib::spawn_future_local(async move {
        let result = async {
            let ((mut engine, rx), payload) = gtk::gio::spawn_blocking(move || {
                let (mut engine, rx) = Engine::new()?;
                let payload = engine.create_group(chat)?;
                let devices = Engine::devices()?;
                let p = crate::prefs::get();
                let mic = if p.microphone.is_empty() {
                    devices["microphone"][0]["metadata"].as_str().unwrap_or("")
                } else {
                    &p.microphone
                };
                let speaker =
                    if p.speaker.is_empty() { devices["speaker"][0]["metadata"].as_str().unwrap_or("") } else { &p.speaker };
                if (!live && mic.is_empty()) || speaker.is_empty() {
                    return Err("Choose microphone and speaker devices first.".to_string());
                }
                engine.mute(chat, true)?;
                engine.media(chat, if live { "" } else { mic }, speaker, None, false)?;
                Ok(((engine, rx), payload))
            })
            .await
            .map_err(|_| "Call engine initialization stopped.")??;
            if !s.valid() {
                return Err("Account changed.".into());
            }
            let data: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            let source = data["ssrc"].as_u64().ok_or("Invalid native group call source.")? as u32 as i32;
            let response=s.call(if live{
"joinLiveStory"}
else{
"joinVideoChat"}
,json!({
"group_call_id":id,"participant_id":null,"join_parameters":{
"@type":"groupCallJoinParameters","audio_source_id":source,"payload":payload,"is_muted":true,"is_my_video_enabled":false}
,"invite_hash":""}
)).await;
            let response = match response {
                Ok(v) => v,
                Err(e) => {
                    service.request(
                        "leaveGroupCall",
                        json!({
                        "group_call_id":id}
                        ),
                    );
                    return Err(e);
                }
            };
            let joined = gtk::gio::spawn_blocking(move || {
                engine.connect_group(chat, response["text"].as_str().ok_or("No group transport payload.")?)?;
                Ok::<_, String>(engine)
            })
            .await
            .map_err(|_| "Call connection stopped.")?;
            let engine = match joined {
                Ok(v) => v,
                Err(e) => {
                    service.request(
                        "leaveGroupCall",
                        json!({
                        "group_call_id":id}
                        ),
                    );
                    return Err(e);
                }
            };
            if !s.valid() {
                service.request(
                    "leaveGroupCall",
                    json!({
                    "group_call_id":id}
                    ),
                );
                return Err("Account changed.".into());
            }
            let session = Rc::new(Session {
                engine: Arc::new(Mutex::new(engine)),
                service: service.clone(),
                key: chat,
                id,
                muted: Cell::new(true),
                camera: Cell::new(false),
                screen: RefCell::new(None),
                streams: Default::default(),
                frames: Default::default(),
                video_grid: Default::default(),
            });
            SESSION.with(|v| *v.borrow_mut() = Some(session.clone()));
            if live {
                video_window(id);
            }
            let weak = Rc::downgrade(&session);
            glib::spawn_future_local(async move {
                while let Ok(event) = rx.recv().await {
                    let Some(session) = weak.upgrade() else { break };
                    if matches!(&event, EngineEvent::BroadcastTime(_) | EngineEvent::BroadcastPart(..)) {
                        broadcast(&session, event);
                        continue;
                    }
                    if let EngineEvent::Frame(_, source, w, h, rotation, data) = &event {
                        if !session.frames.borrow().contains_key(source) {
                            if let Some(grid) = session.video_grid.borrow().as_ref() {
                                add_video_tile(
                                    &session,
                                    grid,
                                    &json!({
                                    "user":"Live stream"}
                                    ),
                                    *source,
                                );
                            }
                        }

                        if let Some(pic) = session.frames.borrow().get(source) {
                            if let Some(bytes) = crate::calls::i420_rgba(*w, *h, data) {
                                let (w, h, bytes) = crate::calls::rotate_rgba(*w, *h, bytes, *rotation);
                                let tex = gtk::gdk::MemoryTexture::new(
                                    w as i32,
                                    h as i32,
                                    gtk::gdk::MemoryFormat::R8g8b8a8,
                                    &glib::Bytes::from_owned(bytes),
                                    w as usize * 4,
                                );
                                pic.set_paintable(Some(&tex));
                            }
                        }
                    }
                    if let EngineEvent::Connection(_, state) = event {
                        if state >= 2 {
                            if SESSION.with(|s| s.borrow().as_ref().is_some_and(|s| Rc::ptr_eq(s, &session))) {
                                stop();
                                window::show_status("Group call disconnected.");
                            }
                            break;
                        } else if state == 1 {
                            window::show_status("Group audio connected · microphone muted");
                        }
                    }
                }
            });
            Ok::<(), String>(())
        }
        .await;
        JOINING.with(|v| v.set(false));
        if let Err(e) = result {
            s.error(&e);
        }
    });
}
fn participants(id: i64) {
    features::load("Call participants", move |s, d, b| async move {
        s.call(
            "loadGroupCallParticipants",
            json!({
            "group_call_id":id,"limit":100}
            ),
        )
        .await?;
        if !alive(&s, &d) {
            return Ok(());
        }
        let refresh = gtk::Button::with_label("Refresh");
        b.append(&refresh);
        let list = nyx_ui::widgets::vbox(6);
        b.append(&list);
        refresh.connect_clicked(move |_| {
            if !s.valid() {
                return;
            }
            while let Some(c) = list.first_child() {
                list.remove(&c)
            }
            let rows = MEMBERS
                .with(|v| v.borrow().iter().filter(|((call, _), _)| *call == id).map(|(_, p)| p.clone()).collect::<Vec<_>>());
            for p in rows {
                note(
                    &list,
                    &format!(
                        "{}{}{}",
                        features::summary(&p["participant_id"]),
                        if p["is_speaking"] == true { " · speaking" } else { "" },
                        if p["is_muted_for_all_users"] == true { " · muted" } else { "" }
                    ),
                );
                if p["can_be_muted_for_all_users"]==true||p["can_be_unmuted_for_all_users"]==true{let(scope,participant,muted)=(s.clone(),p["participant_id"].clone(),p["is_muted_for_all_users"]!=true);button(&list,if muted{"Mute for everyone"}else{"Allow speaking"},move||{if !scope.valid(){return}let participant=participant.clone();actions::run(0,"participant-moderation",move|s|async move{s.call("toggleGroupCallParticipantIsMuted",json!({"group_call_id":id,"participant_id":participant,"is_muted":muted})).await?;Ok(())});});}
                if p["is_current_user"]==true{let(scope,raised,participant)=(s.clone(),p["is_hand_raised"]!=true,p["participant_id"].clone());button(&list,if raised{"Raise hand"}else{"Lower hand"},move||{if !scope.valid(){return}let participant=participant.clone();actions::run(0,"call-hand",move|s|async move{s.call("toggleGroupCallParticipantIsHandRaised",json!({"group_call_id":id,"participant_id":participant,"is_hand_raised":raised})).await?;Ok(())});});}else{let volume=gtk::Scale::with_range(gtk::Orientation::Horizontal,1.,200.,1.);volume.set_value(p["volume_level"].as_f64().unwrap_or(10000.)/100.);volume.set_digits(0);note(&list,"Participant volume (%)");list.append(&volume);let(scope,participant)=(s.clone(),p["participant_id"].clone());button(&list,"Apply volume",move||{if !scope.valid(){return}let(volume,participant)=((volume.value()*100.)as i32,participant.clone());actions::run(0,"call-volume",move|s|async move{s.call("setGroupCallParticipantVolumeLevel",json!({"group_call_id":id,"participant_id":participant,"volume_level":volume})).await?;Ok(())});});}
                if p["can_be_muted_for_current_user"] == true || p["can_be_unmuted_for_current_user"] == true {
                    let (scope, participant, muted) =
                        (s.clone(), p["participant_id"].clone(), p["is_muted_for_current_user"] != true);
                    button(&list, if muted { "Mute for me" } else { "Unmute for me" }, move || {
                        if !scope.valid() {
                            return;
                        }
                        let participant = participant.clone();
                        actions::run(0, "participant-mute", move |s| async move {
                            s.call(
                                "toggleGroupCallParticipantIsMuted",
                                json!({
                                "group_call_id":id,"participant_id":participant,"is_muted":muted}
                                ),
                            )
                            .await?;
                            Ok(())
                        });
                    });
                }
            }
        });
        refresh.emit_clicked();
        Ok(())
    });
}
fn configure(session: &Session, screen: bool) -> Result<(), String> {
    let prefs = crate::prefs::get();
    let devices = Engine::devices()?;
    let selected = |key: &str, value: &str| {
        if value.is_empty() { devices[key][0]["metadata"].as_str().unwrap_or("").to_string() } else { value.to_string() }
    };
    let camera = if session.camera.get() { Some(selected("camera", &prefs.camera)) } else { None };
    session.engine.lock().map_err(|_| "Call engine unavailable.")?.media(
        session.key,
        &selected("microphone", &prefs.microphone),
        &selected("speaker", &prefs.speaker),
        camera.as_deref(),
        screen,
    )
}
fn camera(id: i64) {
    let Some(session) = SESSION.with(|s| s.borrow().as_ref().filter(|s| s.id == id).cloned()) else {
        window::show_status("Join this call first.");
        return;
    };
    let target = !session.camera.get();
    actions::run(session.key, "group-camera", move |s| async move {
        let call = s
            .call(
                "getGroupCall",
                json!({
                "group_call_id":id}
                ),
            )
            .await?;
        if target && call["can_enable_video"] != true {
            return Err("Telegram does not permit enabling video in this call.".into());
        }
        if session.screen.borrow().is_some() {
            return Err("Stop sharing your screen before enabling the camera.".into());
        }
        session.camera.set(target);
        if let Err(e) = configure(&session, false) {
            session.camera.set(!target);
            return Err(e);
        }
        if let Err(e) = s
            .call(
                "toggleGroupCallIsMyVideoEnabled",
                json!({
                "group_call_id":id,"is_my_video_enabled":target}
                ),
            )
            .await
        {
            session.camera.set(!target);
            let _ = configure(&session, false);
            return Err(e);
        }
        window::show_status(if target { "Group camera enabled" } else { "Group camera disabled" });
        Ok(())
    });
}
fn screen(id: i64) {
    let Some(session) = SESSION.with(|s| s.borrow().as_ref().filter(|s| s.id == id).cloned()) else {
        window::show_status("Join this call first.");
        return;
    };
    if let Some(flag) = session.screen.borrow_mut().take() {
        flag.store(true, Ordering::Relaxed);
        session.service.request(
            "endGroupCallScreenSharing",
            json!({
            "group_call_id":id}
            ),
        );
        if let Ok(mut e) = session.engine.lock() {
            e.stop_presentation(session.key);
        }
        let _ = configure(&session, false);
        window::show_status("Screen sharing stopped.");
        return;
    }
    actions::run(session.key, "group-screen", move |s| async move {
        configure(&session, true)?;
        let payload = session.engine.lock().map_err(|_| "Call engine unavailable.")?.create_presentation(session.key)?;
        let parsed: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        let source = parsed["ssrc"].as_u64().ok_or("Invalid screen-sharing source.")? as u32 as i32;
        let response = match s
            .call(
                "startGroupCallScreenSharing",
                json!({
                "group_call_id":id,"audio_source_id":source,"payload":payload}
                ),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                if let Ok(mut engine) = session.engine.lock() {
                    engine.stop_presentation(session.key);
                }
                let _ = configure(&session, false);
                return Err(e);
            }
        };
        let connected=session.engine.lock().map_err(|_|"Call engine unavailable.".to_string()).and_then(|mut engine|engine.connect_presentation(session.key,response["text"].as_str().ok_or("No screen-sharing response.")?));
        if let Err(e)=connected{session.service.request("endGroupCallScreenSharing",json!({"group_call_id":id}));if let Ok(mut engine)=session.engine.lock(){engine.stop_presentation(session.key);}let _=configure(&session,false);return Err(e)}
        let flag = Arc::new(AtomicBool::new(false));
        *session.screen.borrow_mut() = Some(flag.clone());
        let (engine, key, service) = (session.engine.clone(), session.key, session.service.clone());
        let token = flag.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = tokio::runtime::Runtime::new()
                .map_err(|e| e.to_string())
                .and_then(|rt| rt.block_on(nyx_telegram_calls::capture::run(engine, key, flag)));
            let _ = tx.send_blocking(result);
        });
        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            if session.screen.borrow().as_ref().is_some_and(|f| Arc::ptr_eq(f, &token)) {
                session.screen.borrow_mut().take();
                service.request(
                    "endGroupCallScreenSharing",
                    json!({
                    "group_call_id":id}
                    ),
                );
                if let Ok(mut e) = session.engine.lock() {
                    e.stop_presentation(key);
                }
                let _ = configure(&session, false);
                if let Ok(Err(e)) = result {
                    window::show_status(&e);
                }
            }
        });
        Ok(())
    });
}
fn sync_video(session: &Rc<Session>) {
    let rows =
        MEMBERS.with(|m| m.borrow().iter().filter(|((id, _), _)| *id == session.id).map(|(_, p)| p.clone()).collect::<Vec<_>>());
    let mut wanted = BTreeMap::new();
    for p in rows {
        for kind in ["video_info", "screen_sharing_video_info"] {
            let info = &p[kind];
            if let Some(endpoint) = info["endpoint_id"].as_str().filter(|e| !e.is_empty()) {
                let user = p["participant_id"]["user_id"].as_i64().or(p["participant_id"]["chat_id"].as_i64()).unwrap_or(0);
                wanted.insert(
                    endpoint.to_string(),
                    json!({
                    "user":user,"info":info}
                    ),
                );
            }
        }
    }
    let old = session
        .streams
        .borrow()
        .keys()
        .filter(|k| {
            !wanted.contains_key(*k)
                || wanted.get(*k).is_some_and(|w| {
                    session.streams.borrow().get(*k).is_some_and(|old| old["user"] != w["user"] || old["info"] != w["info"])
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    for endpoint in old {
        let _ =
            session.engine.lock().map_err(|_| ()).and_then(|mut e| e.remove_group_video(session.key, &endpoint).map_err(|_| ()));
        if let Some(old) = session.streams.borrow_mut().remove(&endpoint) {
            let mut ids = video_ids(&old);
            ids.push(old["_ssrc"].as_u64().unwrap_or(0) as u32);
            let mut pictures = Vec::new();
            for id in ids {
                if let Some(pic) = session.frames.borrow_mut().remove(&id) {
                    pictures.push(pic);
                }
            }
            if let Some(grid) = session.video_grid.borrow().as_ref() {
                for pic in pictures {
                    if let Some(child) = pic.ancestor(gtk::FlowBoxChild::static_type()).and_downcast::<gtk::FlowBoxChild>() {
                        if child.parent().as_ref() == Some(grid.upcast_ref()) {
                            grid.remove(&child);
                        }
                    }
                }
            }
        }
    }
    for (endpoint, source) in wanted {
        if session
            .streams
            .borrow()
            .get(&endpoint)
            .is_some_and(|old| old["user"] == source["user"] && old["info"] == source["info"])
        {
            continue;
        }
        let result = session
            .engine
            .lock()
            .map_err(|_| "Call engine unavailable.".to_string())
            .and_then(|mut e| e.add_group_video(session.key, source["user"].as_i64().unwrap_or(0), &source["info"]));
        if let Ok(ssrc) = result {
            let mut stored = source.clone();
            stored["_ssrc"] = json!(ssrc);
            session.streams.borrow_mut().insert(endpoint, stored);
            if let Some(grid) = session.video_grid.borrow().as_ref() {
                add_video_tile(session, grid, &source, ssrc);
            }
        }
    }
}
fn add_video_tile(session: &Session, grid: &gtk::FlowBox, source: &Value, ssrc: u32) {
    let row = nyx_ui::widgets::vbox(4);
    note(&row, &format!("Participant {}", source["user"]));
    let pic = gtk::Picture::new();
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic.set_size_request(240, 180);
    row.append(&pic);
    grid.insert(&row, -1);
    let mut ids = source["info"]["source_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["source_ids"].as_array().into_iter().flatten())
        .filter_map(Value::as_i64)
        .map(|n| n as i32 as u32)
        .collect::<Vec<_>>();
    ids.push(ssrc);
    for id in ids {
        session.frames.borrow_mut().insert(id, pic.clone());
    }
}
fn video_window(id: i64) {
    let Some(session) = SESSION.with(|s| s.borrow().as_ref().filter(|s| s.id == id).cloned()) else {
        window::show_status("Join the group call first.");
        return;
    };
    let (d, b) = nyx_ui::widgets::dialog("Group video", 800);
    let grid = gtk::FlowBox::new();
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_min_children_per_line(1);
    grid.set_max_children_per_line(3);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(360)
        .max_content_height(720)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .build();
    b.append(&scroll);
    session.frames.borrow_mut().clear();
    *session.video_grid.borrow_mut() = Some(grid.clone());
    let sources = session.streams.borrow().values().cloned().collect::<Vec<_>>();
    for source in sources {
        add_video_tile(&session, &grid, &source, source["_ssrc"].as_u64().unwrap_or(0) as u32);
    }
    let weak = Rc::downgrade(&session);
    d.connect_close_request(move |_| {
        if let Some(session) = weak.upgrade() {
            if session.video_grid.borrow().as_ref() == Some(&grid) {
                session.video_grid.borrow_mut().take();
                session.frames.borrow_mut().clear();
            }
        }
        glib::Propagation::Proceed
    });
    session.service.request(
        "loadGroupCallParticipants",
        json!({
        "group_call_id":id,"limit":100}
        ),
    );
    sync_video(&session);
    d.present();
}

fn video_ids(source: &Value) -> Vec<u32> {
    source["info"]["source_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|g| g["source_ids"].as_array().into_iter().flatten())
        .filter_map(Value::as_i64)
        .map(|n| n as i32 as u32)
        .collect()
}
fn broadcast(session: &Rc<Session>, event: EngineEvent) {
    let weak = Rc::downgrade(session);
    let service = session.service.clone();
    let id = session.id;
    glib::spawn_future_local(async move {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        match event {
            EngineEvent::BroadcastTime(chat) => {
                let result = service
                    .request(
                        "getGroupCallStreams",
                        json!({
                        "group_call_id":id}
                        ),
                    )
                    .recv()
                    .await;
                let timestamp =
                    result.ok().and_then(Result::ok).and_then(|v| v["streams"][0]["time_offset"].as_i64()).unwrap_or(0);
                if let Some(s) = weak.upgrade() {
                    if let Ok(mut engine) = s.engine.lock() {
                        let _ = engine.broadcast_time(chat, timestamp);
                    }
                }
            }
            EngineEvent::BroadcastPart(chat, segment, part, limit, timestamp, quality_update, channel, quality) => {
                let quality = match quality {
                    1 => "groupCallVideoQualityThumbnail",
                    2 => "groupCallVideoQualityMedium",
                    _ => "groupCallVideoQualityFull",
                };
                let result = service
                    .request(
                        "getGroupCallStreamSegment",
                        json!({
                        "group_call_id":id,"time_offset":timestamp,"scale":0,"channel_id":channel.max(0),"video_quality":{
                        "@type":quality}
                        }
                        ),
                    )
                    .recv()
                    .await;
                let (status, mut data) = match result {
                    Ok(Ok(v)) => match STANDARD.decode(v["data"].as_str().unwrap_or("")) {
                        Ok(data) if !data.is_empty() => (3, data),
                        _ => (0, Vec::new()),
                    },
                    _ => (1, Vec::new()),
                };
                if limit > 0 {
                    data.truncate(limit as usize);
                }
                if let Some(s) = weak.upgrade() {
                    if let Ok(mut engine) = s.engine.lock() {
                        let _ = engine.broadcast_part(chat, segment, part, status, quality_update, &data);
                    }
                }
            }
            _ => {}
        }
    });
}
