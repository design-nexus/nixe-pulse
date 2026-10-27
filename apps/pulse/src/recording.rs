//! Explicit voice/video note capture, validated uploads, and shared suite preview controls.
use crate::actions::Scope;
use gtk::{gio, glib, prelude::*};
use nyx_playback::controls::{OsdControls, SeekWidgets};
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    time::Instant,
};
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Voice,
    Video,
}
impl Kind {
    fn limit(self) -> u64 {
        if self == Self::Video { 60 } else { 600 }
    }
    fn extension(self) -> &'static str {
        if self == Self::Video { "mp4" } else { "ogg" }
    }
}
fn filter() -> String {
    let circle = "lte((X-W/2)*(X-W/2)+(Y-H/2)*(Y-H/2),(W/2)*(W/2))";
    format!(
        "crop='min(iw,ih)':'min(iw,ih)',scale=384:384,setsar=1,format=rgb24,geq=r='if({circle},r(X,Y),0)':g='if({circle},g(X,Y),0)':b='if({circle},b(X,Y),0)',format=yuv420p"
    )
}
fn video_args() -> Vec<String> {
    vec![
        "-vf".into(),
        filter(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "24".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "64k".into(),
        "-movflags".into(),
        "+faststart".into(),
    ]
}
fn payload(kind: Kind, path: &Path, probe: &Value) -> Result<Value, String> {
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
        .ok_or("The recording has no usable duration.")?;
    if duration > kind.limit() as f64 + 0.25 {
        return Err("The recording exceeds Telegram's duration limit.".into());
    }
    let streams = probe["streams"].as_array().ok_or("The recording has no media streams.")?;
    let local = json!({"@type":"inputFileLocal","path":path.to_string_lossy()});
    if kind == Kind::Video {
        let video = streams.iter().find(|s| s["codec_type"] == "video").ok_or("The recording contains no video.")?;
        let width = video["width"].as_i64().unwrap_or(0);
        let height = video["height"].as_i64().unwrap_or(0);
        if video["codec_name"] != "h264" || width <= 0 || width > 640 || width != height {
            return Err("Video messages must be square H.264 video, up to 640 pixels.".into());
        }
        Ok(
            json!({"@type":"inputMessageVideoNote","video_note":{"@type":"inputVideoNote","video_note":local,"thumbnail":null,"duration":duration.ceil().min(60.) as i32,"length":width},"self_destruct_type":null}),
        )
    } else {
        if !streams.iter().any(|s| s["codec_type"] == "audio" && s["codec_name"] == "opus") {
            return Err("The voice recording is not Opus audio.".into());
        }
        Ok(
            json!({"@type":"inputMessageVoiceNote","voice_note":{"@type":"inputVoiceNote","voice_note":local,"duration":duration.ceil().min(600.) as i32,"waveform":""},"caption":null,"self_destruct_type":null}),
        )
    }
}
fn probe(path: &Path) -> Result<Value, String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_name,codec_type,width,height:format=duration", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("The recording could not be read. Try another camera or microphone.".into());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}
struct Capture {
    child: Child,
    began: Instant,
    stop_at: Option<Instant>,
}
fn request_stop(capture: &Rc<RefCell<Option<Capture>>>) {
    if let Some(c) = capture.borrow_mut().as_mut() {
        if c.stop_at.is_none() {
            if let Some(stdin) = c.child.stdin.as_mut() {
                let _ = stdin.write_all(b"q\n");
                let _ = stdin.flush();
            }
            c.stop_at = Some(Instant::now());
        }
    }
}
fn cameras() -> Vec<(String, String)> {
    let mut devices = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") {
        for item in entries.flatten() {
            let node = item.file_name().to_string_lossy().to_string();
            let label = std::fs::read_to_string(item.path().join("name")).unwrap_or_else(|_| node.clone());
            devices.push((format!("/dev/{node}"), label.trim().to_string()));
        }
    }
    devices.sort_by(|a, b| a.0.cmp(&b.0));
    devices
}
fn microphones() -> Vec<(String, String)> {
    let mut devices = vec![("default".into(), "Default microphone".into())];
    if let Ok(out) = Command::new("timeout").args(["3s", "pactl", "-f", "json", "list", "sources"]).output() {
        if let Ok(value) = serde_json::from_slice::<Value>(&out.stdout) {
            for item in value.as_array().into_iter().flatten() {
                if let Some(name) = item["name"].as_str() {
                    devices.push((name.into(), item["description"].as_str().unwrap_or(name).into()));
                }
            }
        }
    }
    devices
}
pub fn open(parent: &gtk::Box, chat: Option<i64>) {
    if let Some(chat) = chat {
        show(parent, chat, Kind::Voice);
    }
}
pub fn open_video(parent: &gtk::Box, chat: Option<i64>) {
    if let Some(chat) = chat {
        show(parent, chat, Kind::Video);
    }
}
fn show(parent: &gtk::Box, chat: i64, kind: Kind) {
    let Some(scope) = Scope::current() else { return };
    let Some(parent) = parent.root().and_downcast::<gtk::Window>() else { return };
    let (dialog, body) = w::dialog(if kind == Kind::Video { "Video message" } else { "Voice message" }, 480);
    dialog.set_transient_for(Some(&parent));
    let status = w::label(
        if kind == Kind::Video {
            "Record up to 60 seconds, or use the first minute of a video file. The picture is cropped to a circle."
        } else {
            "Ready to record · maximum 10 minutes"
        },
        "dim",
    );
    status.set_wrap(true);
    status.set_max_width_chars(44);
    body.append(&status);
    let devices = w::vbox(6);
    body.append(&devices);
    let camera_devices = cameras();
    let audio_devices = microphones();
    let camera = gtk::DropDown::from_strings(&camera_devices.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>());
    if kind == Kind::Video {
        devices.append(&w::label("Camera", "dim"));
        devices.append(&camera);
    }
    let audio = gtk::DropDown::from_strings(&audio_devices.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>());
    devices.append(&w::label("Microphone", "dim"));
    devices.append(&audio);
    let preview_box = w::vbox(8);
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_size_request(280, 280);
    picture.set_visible(kind == Kind::Video);
    preview_box.append(&picture);
    let seek = SeekWidgets::new();
    let controls = OsdControls::new(&seek.root, 1., 1., false);
    for widget in [
        controls.prev.clone().upcast::<gtk::Widget>(),
        controls.next.clone().upcast(),
        controls.chapters.clone().upcast(),
        controls.shape.clone().upcast(),
        controls.speed.clone().upcast(),
        controls.speed_label.clone().upcast(),
        controls.audio.clone().upcast(),
        controls.subs.clone().upcast(),
        controls.full.clone().upcast(),
        controls.close.clone().upcast(),
    ] {
        widget.set_visible(false);
    }
    preview_box.append(&controls.root);
    preview_box.set_visible(false);
    body.append(&preview_box);
    let row = w::hbox(8);
    let start = gtk::Button::with_label("Record");
    let stop = gtk::Button::with_label("Stop");
    let preview = gtk::Button::with_label("Preview");
    let send = gtk::Button::with_label("Send");
    let cancel = gtk::Button::with_label("Cancel");
    for button in [&start, &stop, &preview, &send, &cancel] {
        row.append(button);
    }
    body.append(&row);
    stop.set_sensitive(false);
    preview.set_sensitive(false);
    send.set_sensitive(false);
    if kind == Kind::Video && camera_devices.is_empty() {
        start.set_sensitive(false);
        status.set_text("No camera was found. Connect a camera or use a recorded video file.");
    }
    let import = gtk::Button::with_label("Use video file…");
    import.set_visible(kind == Kind::Video);
    body.append(&import);
    let directory = nyx_core::paths::cache_dir().join("recordings");
    let _ = std::fs::create_dir_all(&directory);
    let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), kind.extension()));
    let log = path.with_extension("log");
    let capture: Rc<RefCell<Option<Capture>>> = Default::default();
    let content: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));
    let media: Rc<RefCell<Option<gtk::MediaFile>>> = Default::default();
    let busy = Rc::new(Cell::new(false));
    let uploading = Rc::new(Cell::new(false));
    let keep_file = Rc::new(Cell::new(false));
    let launch: Rc<dyn Fn(Option<PathBuf>)> = Rc::new({
        let (capture, content, media, busy, status, start, stop, preview, send, devices, import, preview_box, path, log, scope) = (
            capture.clone(),
            content.clone(),
            media.clone(),
            busy.clone(),
            status.clone(),
            start.clone(),
            stop.clone(),
            preview.clone(),
            send.clone(),
            devices.clone(),
            import.clone(),
            preview_box.clone(),
            path.clone(),
            log.clone(),
            scope.clone(),
        );
        move |file| {
            if !scope.valid() || busy.get() || capture.borrow().is_some() {
                return;
            }
            let mut command = Command::new("ffmpeg");
            command.args(["-hide_banner", "-loglevel", "error", "-nostats", "-y"]);
            if let Some(file) = file {
                command.arg("-i").arg(file).args(["-map", "0:v:0", "-map", "0:a?"]);
            } else if kind == Kind::Video {
                let Some((camera, _)) = camera_devices.get(camera.selected() as usize) else {
                    status.set_text("Choose a camera.");
                    return;
                };
                let source = &audio_devices[audio.selected() as usize].0;
                command.args([
                    "-thread_queue_size",
                    "512",
                    "-f",
                    "v4l2",
                    "-framerate",
                    "30",
                    "-video_size",
                    "640x480",
                    "-i",
                    camera,
                    "-thread_queue_size",
                    "512",
                    "-f",
                    "pulse",
                    "-i",
                    source,
                    "-map",
                    "0:v:0",
                    "-map",
                    "1:a:0",
                ]);
            } else {
                command.args(["-f", "pulse", "-i", &audio_devices[audio.selected() as usize].0]);
            }
            command.args(["-t", &kind.limit().to_string()]);
            if kind == Kind::Video {
                command.args(video_args());
            } else {
                command.args(["-ac", "1", "-c:a", "libopus", "-b:a", "32k"]);
            }
            let stderr = std::fs::File::create(&log).map(Stdio::from).unwrap_or(Stdio::null());
            match command.arg(&path).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(stderr).spawn() {
                Ok(child) => {
                    if let Some(player) = media.borrow_mut().take() {
                        player.pause();
                    }
                    preview_box.set_visible(false);
                    *content.borrow_mut() = Value::Null;
                    *capture.borrow_mut() = Some(Capture { child, began: Instant::now(), stop_at: None });
                    start.set_sensitive(false);
                    stop.set_sensitive(true);
                    preview.set_sensitive(false);
                    send.set_sensitive(false);
                    devices.set_sensitive(false);
                    import.set_sensitive(false);
                    status.set_text("Recording / converting…");
                }
                Err(e) => status.set_text(&format!("Unable to start recording: {e}")),
            }
        }
    });
    let l = launch.clone();
    start.connect_clicked(move |_| l(None));
    let c = capture.clone();
    stop.connect_clicked(move |b| {
        b.set_sensitive(false);
        request_stop(&c);
    });
    let weak = dialog.downgrade();
    let l = launch.clone();
    let s = scope.clone();
    import.connect_clicked(move |_| {
        let Some(d) = weak.upgrade() else { return };
        let (l, s, weak) = (l.clone(), s.clone(), weak.clone());
        let picker = gtk::FileDialog::builder().title("Choose video message source").build();
        picker.open(Some(&d), gio::Cancellable::NONE, move |result| {
            if s.valid() && weak.upgrade().is_some_and(|d| d.is_visible()) {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        l(Some(path));
                    }
                }
            }
        });
    });
    let (player, pic, box_, p) = (media.clone(), picture.clone(), preview_box.clone(), path.clone());
    preview.connect_clicked(move |_| {
        if let Some(old) = player.borrow_mut().take() {
            old.pause();
        }
        let file = gtk::MediaFile::for_filename(&p);
        pic.set_paintable(Some(&file));
        box_.set_visible(true);
        file.play();
        *player.borrow_mut() = Some(file);
    });
    let m = media.clone();
    controls.play.connect_clicked(move |_| {
        if let Some(m) = m.borrow().as_ref() {
            if m.is_playing() {
                m.pause()
            } else {
                if m.is_ended() {
                    m.seek(0);
                }
                m.play();
            }
        }
    });
    for (button, delta) in [(controls.back.clone(), -5_000_000i64), (controls.fwd.clone(), 5_000_000)] {
        let m = media.clone();
        button.connect_clicked(move |_| {
            if let Some(m) = m.borrow().as_ref() {
                m.seek((m.timestamp() + delta).clamp(0, m.duration().max(0)));
            }
        });
    }
    let m = media.clone();
    controls.mute.connect_clicked(move |_| {
        if let Some(m) = m.borrow().as_ref() {
            m.set_muted(!m.is_muted());
        }
    });
    let m = media.clone();
    controls.volume.connect_value_changed(move |v| {
        if let Some(m) = m.borrow().as_ref() {
            m.set_volume(v.value());
        }
    });
    let m = media.clone();
    seek.seek.connect_change_value(move |_, _, value| {
        if let Some(m) = m.borrow().as_ref() {
            m.seek((value * 1_000_000.) as i64);
        }
        glib::Propagation::Proceed
    });
    let weak = dialog.downgrade();
    let (c, b, st, pre, se, dev, im, star, path_, log_, content_, scope_, media_, play, stop_) = (
        capture.clone(),
        busy.clone(),
        status.clone(),
        preview.clone(),
        send.clone(),
        devices.clone(),
        import.clone(),
        start.clone(),
        path.clone(),
        log.clone(),
        content.clone(),
        scope.clone(),
        media.clone(),
        controls.play.clone(),
        stop.clone(),
    );
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if weak.upgrade().is_none_or(|d| !d.is_visible()) {
            return glib::ControlFlow::Break;
        }
        if !scope_.valid() {
            request_stop(&c);
            se.set_sensitive(false);
            star.set_sensitive(false);
            im.set_sensitive(false);
            st.set_text("Account changed. Close this recording and start again.");
        }
        let mut finished = None;
        {
            let mut state = c.borrow_mut();
            if let Some(capture) = state.as_mut() {
                if capture.stop_at.is_some_and(|t| t.elapsed().as_secs() > 3) {
                    let _ = capture.child.kill();
                }
                match capture.child.try_wait() {
                    Ok(Some(result)) => finished = Some(result.success()),
                    Err(_) => {
                        let _ = capture.child.kill();
                        let _ = capture.child.wait();
                        finished = Some(false);
                    }
                    Ok(None) => {}
                }
                if finished.is_none() && scope_.valid() {
                    st.set_text(&format!(
                        "Recording / converting · {}s / {}s",
                        capture.began.elapsed().as_secs().min(kind.limit()),
                        kind.limit()
                    ));
                }
            }
        }
        if let Some(success) = finished {
            stop_.set_sensitive(false);
            c.borrow_mut().take();
            se.set_sensitive(false);
            pre.set_sensitive(false);
            b.set(true);
            let (b, st, pre, se, dev, im, star, path, log, content, scope, weak) = (
                b.clone(),
                st.clone(),
                pre.clone(),
                se.clone(),
                dev.clone(),
                im.clone(),
                star.clone(),
                path_.clone(),
                log_.clone(),
                content_.clone(),
                scope_.clone(),
                weak.clone(),
            );
            glib::spawn_future_local(async move {
                let p = path.clone();
                let result = gio::spawn_blocking(move || probe(&p).and_then(|data| payload(kind, &p, &data)))
                    .await
                    .unwrap_or_else(|_| Err("Recording validation stopped.".into()));
                b.set(false);
                if weak.upgrade().is_none_or(|d| !d.is_visible()) || !scope.valid() {
                    return;
                }
                dev.set_sensitive(true);
                im.set_sensitive(true);
                star.set_sensitive(kind == Kind::Voice || !cameras().is_empty());
                match result {
                    Ok(value) => {
                        *content.borrow_mut() = value;
                        st.set_text("Ready to preview and send");
                        pre.set_sensitive(true);
                        se.set_sensitive(true);
                    }
                    Err(e) => {
                        let detail = std::fs::read_to_string(log).unwrap_or_default();
                        st.set_text(&format!(
                            "{e}{}",
                            if !success && !detail.is_empty() {
                                format!("\n{}", detail.chars().take(300).collect::<String>())
                            } else {
                                String::new()
                            }
                        ));
                    }
                }
            });
        }
        if let Some(m) = media_.borrow().as_ref() {
            let duration = m.duration().max(0) as f64 / 1_000_000.;
            let position = m.timestamp().max(0) as f64 / 1_000_000.;
            seek.seek.set_range(0., duration.max(1.));
            seek.seek.set_value(position);
            seek.pos.set_text(&format!("{}:{:02}", position as i64 / 60, position as i64 % 60));
            seek.dur.set_text(&format!("{}:{:02}", duration as i64 / 60, duration as i64 % 60));
            play.set_icon_name(if m.is_playing() { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
            if let Some(e) = m.error() {
                st.set_text(&format!("Preview unavailable: {e}"));
            }
        }
        glib::ControlFlow::Continue
    });
    let (topic, reply) = (crate::window::composer_topic(chat), crate::window::composer_reply(chat));
    let weak = dialog.downgrade();
    let (s, content_, uploading_, keep, st, can, devices_, import_, start_, pre) = (
        scope.clone(),
        content.clone(),
        uploading.clone(),
        keep_file.clone(),
        status.clone(),
        cancel.clone(),
        devices.clone(),
        import.clone(),
        start.clone(),
        preview.clone(),
    );
    send.connect_clicked(move |button|{
        if !s.valid()||content_.borrow().is_null()||uploading_.replace(true){return}let Some(d)=weak.upgrade()else{uploading_.set(false);return};
        keep.set(true);button.set_sensitive(false);can.set_sensitive(false);devices_.set_sensitive(false);import_.set_sensitive(false);start_.set_sensitive(false);pre.set_sensitive(false);
        let (s,content,topic,reply,uploading,st,button,can)=(s.clone(),content_.borrow().clone(),topic.clone(),reply.clone(),uploading_.clone(),st.clone(),button.clone(),can.clone());
        glib::spawn_future_local(async move{let result=crate::groups::commit(&s,"sendMessage",json!({"chat_id":chat,"topic_id":topic,"reply_to":reply,"options":{"@type":"messageSendOptions","paid_message_star_count":0},"input_message_content":content})).await;uploading.set(false);if !s.valid(){d.close();return}match result{Ok(data)=>{if let Some(m)=nyx_telegram_core::client::parse_message(&data){crate::window::display_message(&m);}d.close();},Err(e)=>{st.set_text(&e);button.set_sensitive(!e.contains("status is unknown"));can.set_sensitive(true);}}});
    });
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    dialog.connect_close_request(move |_| {
        if uploading.get() {
            return glib::Propagation::Stop;
        }
        if let Some(mut c) = capture.borrow_mut().take() {
            let _ = c.child.kill();
            let _ = c.child.wait();
        }
        if let Some(m) = media.borrow_mut().take() {
            m.pause();
        }
        if !keep_file.get() {
            let _ = std::fs::remove_file(&path);
        }
        let _ = std::fs::remove_file(&log);
        glib::Propagation::Proceed
    });
    dialog.present();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage3e_video_and_voice_payload_validation() {
        let path = Path::new("/tmp/example.mp4");
        let good =
            json!({"format":{"duration":"1.2"},"streams":[{"codec_type":"video","codec_name":"h264","width":384,"height":384}]});
        let content = payload(Kind::Video, path, &good).unwrap();
        assert_eq!(content["video_note"]["@type"], "inputVideoNote");
        assert_eq!(content["video_note"]["duration"], 2);
        let mut bad = good.clone();
        bad["streams"][0]["height"] = json!(480);
        assert!(payload(Kind::Video, path, &bad).is_err());
        bad = good;
        bad["format"]["duration"] = json!("61");
        assert!(payload(Kind::Video, path, &bad).is_err());
        assert!(payload(Kind::Voice, path, &bad).is_err());
        assert!(filter().contains("geq="));
    }
}
