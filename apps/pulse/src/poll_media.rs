//! Poll attachment selection and rendering through the existing media viewer.
use crate::{actions::Scope, groups::note, window};
use gtk::{gio, glib, prelude::*};
use nixe_telegram_core::{client::parse_message, models::MessageSummary};
use nixe_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Clone)]
pub struct Attachment {
    kind: String,
    path: PathBuf,
    metadata: Value,
}
impl Attachment {
    pub fn input(&self) -> Result<Value, String> {
        if matches!(self.kind.as_str(), "Link" | "Location" | "Venue") {
            return Ok(self.metadata.clone());
        }
        if !self.path.is_file() {
            return Err("An attached file is no longer available. Choose it again.".into());
        }
        let mut content = input(&self.kind, &self.path)?;
        let key = self.kind.to_ascii_lowercase();
        let stream = &self.metadata["streams"][0];
        for field in ["width", "height"] {
            if let Some(n) = stream[field].as_i64().filter(|n| *n > 0) {
                content[&key][field] = json!(n);
            }
        }
        if let Some(duration) =
            self.metadata["format"]["duration"].as_str().and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.)
        {
            content[&key]["duration"] = json!(duration.ceil() as i64);
        }
        Ok(content)
    }
}
fn input(kind: &str, path: &Path) -> Result<Value, String> {
    let file = json!({"@type":"inputFileLocal","path":path.to_string_lossy()});
    let media = match kind {
        "Checking" => return Err("Wait for the attachment check to finish.".into()),
        "Photo" => {
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 10 * 1024 * 1024 {
                return Err("Poll photos must be at most 10 MB.".into());
            }
            json!({"@type":"inputPhoto","photo":file,"thumbnail":null,"video":null,"added_sticker_file_ids":[],"width":0,"height":0})
        }
        "Video" => {
            json!({"@type":"inputVideo","video":file,"thumbnail":null,"cover":null,"start_timestamp":0,"added_sticker_file_ids":[],"duration":0,"width":0,"height":0,"supports_streaming":false})
        }
        "Animation" => {
            json!({"@type":"inputAnimation","animation":file,"thumbnail":null,"added_sticker_file_ids":[],"duration":0,"width":0,"height":0})
        }
        "Audio" => json!({"@type":"inputAudio","audio":file,"album_cover_thumbnail":null,"duration":0,"title":"","performer":""}),
        "Sticker" => json!({"@type":"inputSticker","sticker":file,"thumbnail":null,"width":512,"height":512}),
        "Document" => json!({"@type":"inputDocument","document":file,"thumbnail":null,"disable_content_type_detection":true}),
        _ => return Err("Unsupported poll attachment.".into()),
    };
    let key = kind.to_ascii_lowercase();
    let mut result = json!({"@type":format!("inputPollMedia{kind}")});
    result[&key] = media;
    Ok(result)
}
pub fn picker(
    body: &gtk::Box,
    dialog: &gtk::Window,
    scope: &Scope,
    label: &str,
    answer: bool,
) -> Rc<RefCell<Option<Attachment>>> {
    let kinds = if answer {
        vec!["Photo", "Video", "Animation", "Link", "Location", "Venue", "Sticker"]
    } else {
        vec!["Photo", "Video", "Animation", "Audio", "Document", "Location", "Venue"]
    };
    picker_types(body, dialog, scope, label, kinds)
}
pub fn sticker_picker(body: &gtk::Box, dialog: &gtk::Window, scope: &Scope, label: &str) -> Rc<RefCell<Option<Attachment>>> {
    picker_types(body, dialog, scope, label, vec!["Sticker"])
}
pub fn story_picker(body: &gtk::Box, dialog: &gtk::Window, scope: &Scope, label: &str) -> Rc<RefCell<Option<Attachment>>> {
    picker_types(body, dialog, scope, label, vec!["Photo", "Video"])
}
fn picker_types(
    body: &gtk::Box,
    dialog: &gtk::Window,
    scope: &Scope,
    label: &str,
    kinds: Vec<&'static str>,
) -> Rc<RefCell<Option<Attachment>>> {
    let expander = gtk::Expander::new(Some(label));
    let inner = w::vbox(4);
    expander.set_child(Some(&inner));
    body.append(&expander);
    let body = &inner;
    let row = w::hbox(4);
    let kind = gtk::DropDown::from_strings(&kinds);
    let choose = gtk::Button::with_label("Attach…");
    let clear = w::bar_button("edit-clear-symbolic", "Remove attachment");
    row.append(&kind);
    row.append(&choose);
    row.append(&clear);
    body.append(&row);
    let name = note(body, "No attachment");
    name.set_max_width_chars(32);
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let selected: Rc<RefCell<Option<Attachment>>> = Default::default();
    let (state, name_) = (selected.clone(), name.clone());
    clear.connect_clicked(move |_| {
        state.borrow_mut().take();
        name_.set_text("No attachment");
    });
    let (state, weak, scope) = (selected.clone(), dialog.downgrade(), scope.clone());
    let clear = clear.clone();
    choose.connect_clicked(move |button| {
        let Some(dialog) = weak.upgrade().filter(|d| d.is_visible()) else {
            return;
        };
        if !scope.valid() {
            return;
        }
        let kind = kinds.get(kind.selected() as usize).copied().unwrap_or("Photo").to_string();
        if matches!(kind.as_str(), "Link" | "Location" | "Venue") {
            data_attachment(&dialog, &scope, kind, state.clone(), name.clone());
            return;
        }
        let chooser = gtk::FileDialog::builder().title("Choose attachment").build();
        let filter = gtk::FileFilter::new();
        match kind.as_str() {
            "Photo" => {
                for ext in ["jpg", "jpeg", "png", "webp"] {
                    filter.add_suffix(ext);
                }
            }
            "Video" => filter.add_suffix("mp4"),
            "Animation" => filter.add_suffix("gif"),
            "Sticker" => {
                for ext in ["webp", "tgs", "webm"] {
                    filter.add_suffix(ext);
                }
            }
            "Audio" => filter.add_mime_type("audio/*"),
            _ => filter.add_pattern("*"),
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        chooser.set_filters(Some(&filters));
        chooser.set_default_filter(Some(&filter));
        let (state, name, scope, weak, button, clear) =
            (state.clone(), name.clone(), scope.clone(), weak.clone(), button.clone(), clear.clone());
        button.set_sensitive(false);
        chooser.open(Some(&dialog), gio::Cancellable::NONE, move |result| {
            button.set_sensitive(true);
            if !scope.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) || !button.is_sensitive() {
                return;
            }
            let Ok(file) = result else { return };
            let Some(path) = file.path() else {
                name.set_text("Choose a local file.");
                return;
            };
            button.set_sensitive(false);
            clear.set_sensitive(false);
            let previous =
                state.borrow_mut().replace(Attachment { kind: "Checking".into(), path: path.clone(), metadata: Value::Null });
            name.set_text("Checking attachment…");
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || prepare(kind, path))
                    .await
                    .unwrap_or_else(|_| Err("Attachment check stopped.".into()));
                button.set_sensitive(true);
                clear.set_sensitive(true);
                if !scope.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                match result {
                    Ok(attachment) => {
                        name.set_text(&attachment.path.file_name().unwrap_or_default().to_string_lossy());
                        *state.borrow_mut() = Some(attachment);
                    }
                    Err(e) => {
                        *state.borrow_mut() = previous;
                        name.set_text(&format!("{e} Attachment unchanged."));
                    }
                }
            });
        });
    });
    selected
}
fn prepare(kind: String, path: PathBuf) -> Result<Attachment, String> {
    let mut attachment = Attachment { kind, path, metadata: Value::Null };
    attachment.input()?;
    if attachment.kind == "Sticker" {
        use std::io::Read;
        let size = std::fs::metadata(&attachment.path).map_err(|e| e.to_string())?.len();
        let ext = attachment.path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "tgs" {
            if size > 64 * 1024 {
                return Err("TGS stickers must be at most 64 KB.".into());
            }
            let mut bytes = Vec::new();
            flate2::read::GzDecoder::new(std::fs::File::open(&attachment.path).map_err(|e| e.to_string())?)
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err("Sticker animation is too large.".into());
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Unreadable TGS sticker.")?;
            let (w, h) = (value["w"].as_i64().unwrap_or(0), value["h"].as_i64().unwrap_or(0));
            if w <= 0 || h <= 0 || w > 512 || h > 512 {
                return Err("Sticker dimensions must fit within 512 pixels.".into());
            }
            attachment.metadata = json!({"streams":[{"width":w,"height":h}]});
            return Ok(attachment);
        }
        if !matches!(ext.as_str(), "webp" | "webm") || size > if ext == "webm" { 256 * 1024 } else { 512 * 1024 } {
            return Err("Choose a WebP sticker up to 512 KB, WebM up to 256 KB, or TGS up to 64 KB.".into());
        }
    }

    if matches!(attachment.kind.as_str(), "Photo" | "Video" | "Animation" | "Sticker") {
        let output = std::process::Command::new("timeout")
            .args([
                "10s",
                "ffprobe",
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height:format=format_name,duration",
                "-of",
                "json",
            ])
            .arg(&attachment.path)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("This media could not be read.".into());
        }
        let data: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
        validate_media(&attachment.kind, &data)?;
        if attachment.kind == "Sticker" {
            let stream = &data["streams"][0];
            if stream["width"].as_i64().unwrap_or(0) > 512 || stream["height"].as_i64().unwrap_or(0) > 512 {
                return Err("Sticker dimensions must fit within 512 pixels.".into());
            }
        }
        attachment.metadata = data;
    }
    Ok(attachment)
}
fn validate_media(kind: &str, data: &Value) -> Result<(), String> {
    let stream = &data["streams"][0];
    let (width, height) = (stream["width"].as_i64().unwrap_or(0), stream["height"].as_i64().unwrap_or(0));
    if width <= 0 || height <= 0 {
        return Err("This media has no usable picture.".into());
    }
    match kind {
        "Video"
            if stream["codec_name"] != "h264"
                || !data["format"]["format_name"].as_str().unwrap_or("").split(',').any(|f| f == "mp4") =>
        {
            return Err("Choose an MP4 video encoded with H.264.".into());
        }
        "Photo" if width + height > 10000 || width > height * 20 || height > width * 20 => {
            return Err("Photo dimensions exceed Telegram's limits.".into());
        }
        "Photo" if !matches!(stream["codec_name"].as_str(), Some("mjpeg" | "png" | "webp")) => {
            return Err("Choose a JPEG, PNG or WebP photo.".into());
        }
        "Animation" if stream["codec_name"] != "gif" => return Err("Choose a GIF animation.".into()),
        _ => {}
    }
    Ok(())
}
fn as_content(media: &Value) -> Option<Value> {
    let kind = media["@type"].as_str()?.strip_prefix("pollMedia")?;
    let key = match kind {
        "Photo" => "photo",
        "Video" => "video",
        "Animation" => "animation",
        "Audio" => "audio",
        "Document" => "document",
        "Sticker" => "sticker",
        "Location" => "location",
        "Venue" => "venue",
        _ => return None,
    };
    let mut content = json!({"@type":format!("message{kind}"),"caption":{"@type":"formattedText","text":"","entities":[]}});
    content[key] = media[key].clone();
    Some(content)
}
pub fn render(body: &gtk::Box, message: &MessageSummary, media: &Value) {
    if media.is_null() {
        return;
    }
    if media["@type"] == "pollMediaLink" {
        if let Some(url) = media["url"].as_str().filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
            body.append(&gtk::LinkButton::with_label(url, "Open answer link"));
        }
        return;
    }
    let Some(content) = as_content(media) else {
        note(body, "Unsupported poll attachment");
        return;
    };
    let mut data = message.data.clone();
    data["content"] = content;
    let Some(m) = parse_message(&data) else { return };
    if matches!(media["@type"].as_str(), Some("pollMediaLocation" | "pollMediaVenue")) {
        crate::rich_messages::render(body, &m);
        return;
    }
    let Some(scope) = Scope::current() else { return };
    if let Some(file) = m
        .files
        .iter()
        .find(|f| f.role == "thumbnail")
        .or_else(|| m.files.iter().find(|f| f.role == "original" && media["@type"] == "pollMediaPhoto"))
    {
        let pic = gtk::Picture::new();
        pic.set_can_shrink(true);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_size_request(180, 120);
        if let Some(path) = &file.path {
            pic.set_filename(Some(path));
        } else {
            let (scope, weak, id) = (scope.clone(), pic.downgrade(), file.id);
            glib::spawn_future_local(async move {
                if let Ok(file) =
                    scope.call("downloadFile", json!({"file_id":id,"priority":1,"offset":0,"limit":0,"synchronous":true})).await
                {
                    if let Some(pic) = weak.upgrade().filter(|p| p.root().is_some()) {
                        if let Some(path) = file["local"]["path"].as_str().filter(|p| !p.is_empty()) {
                            pic.set_filename(Some(path));
                        }
                    }
                }
            });
        }
        body.append(&pic);
    }
    let title = media["document"]["file_name"].as_str().or(media["audio"]["file_name"].as_str()).unwrap_or("attachment");
    let button = gtk::Button::with_label(&format!("Open {title}"));
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        label.set_max_width_chars(32);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    }
    button.connect_clicked(move |_| {
        if scope.valid() {
            window::show_media(&m);
        }
    });
    body.append(&button);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage3f_poll_media_schema() {
        let sticker_path = std::env::temp_dir().join(format!("pulse-sticker-{}.tgs", uuid::Uuid::new_v4()));
        let write_sticker = |width: i64| {
            use std::io::Write;
            let mut gz =
                flate2::write::GzEncoder::new(std::fs::File::create(&sticker_path).unwrap(), flate2::Compression::default());
            gz.write_all(json!({"w":width,"h":128}).to_string().as_bytes()).unwrap();
            gz.finish().unwrap();
        };
        write_sticker(1024);
        assert!(prepare("Sticker".into(), sticker_path.clone()).is_err());
        write_sticker(128);
        let sticker = prepare("Sticker".into(), sticker_path.clone()).unwrap().input().unwrap();
        assert_eq!(sticker["sticker"]["width"], 128);
        assert_eq!(sticker["sticker"]["sticker"]["@type"], "inputFileLocal");
        std::fs::remove_file(sticker_path).unwrap();
        let video = input("Video", Path::new("/tmp/video.mp4")).unwrap();
        assert_eq!(video["video"]["@type"], "inputVideo");
        assert_eq!(video["video"]["video"]["path"], "/tmp/video.mp4");
        assert!(
            validate_media(
                "Video",
                &json!({"streams":[{"codec_name":"vp9","width":320,"height":240}],"format":{"format_name":"webm"}})
            )
            .is_err()
        );
        assert!(validate_media("Photo", &json!({"streams":[{"codec_name":"png","width":10000,"height":10000}]})).is_err());
        let content = as_content(&json!({"@type":"pollMediaPhoto","photo":{"sizes":[]}})).unwrap();
        assert_eq!(content["@type"], "messagePhoto");
        assert!(as_content(&json!({"@type":"pollMediaUnknown"})).is_none());
    }
}

fn data_attachment(parent: &gtk::Window, scope: &Scope, kind: String, state: Rc<RefCell<Option<Attachment>>>, name: gtk::Label) {
    let (d, b) = w::dialog("Poll attachment", 440);
    d.set_transient_for(Some(parent));
    let fields = w::vbox(6);
    b.append(&fields);
    let url = crate::groups::entry(&fields, "HTTPS URL", "", 2048);
    url.set_visible(kind == "Link");
    if let Some(l) = url.prev_sibling() {
        l.set_visible(kind == "Link");
    }
    let lat = crate::groups::entry(&fields, "Latitude", "", 32);
    let lon = crate::groups::entry(&fields, "Longitude", "", 32);
    for e in [&lat, &lon] {
        e.set_visible(kind != "Link");
        if let Some(l) = e.prev_sibling() {
            l.set_visible(kind != "Link");
        }
    }
    let title = crate::groups::entry(&fields, "Venue name", "", 255);
    let address = crate::groups::entry(&fields, "Venue address", "", 1024);
    for e in [&title, &address] {
        e.set_visible(kind == "Venue");
        if let Some(l) = e.prev_sibling() {
            l.set_visible(kind == "Venue");
        }
    }
    crate::groups::submit(&d, &b, &fields, scope.clone(), "Attach", move |_| {
        let (kind, state, name, url, lat, lon, title, address) = (
            kind.clone(),
            state.clone(),
            name.clone(),
            url.text().to_string(),
            lat.text().to_string(),
            lon.text().to_string(),
            title.text().to_string(),
            address.text().to_string(),
        );
        async move {
            let media = if kind == "Link" {
                if !url.starts_with("https://") {
                    return Err("Enter an HTTPS URL.".into());
                }
                json!({"@type":"inputPollMediaLink","url":url})
            } else {
                let (lat, lon) =
                    (lat.parse::<f64>().map_err(|_| "Invalid latitude.")?, lon.parse::<f64>().map_err(|_| "Invalid longitude.")?);
                if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90. || lon.abs() > 180. {
                    return Err("Coordinates are out of range.".into());
                }
                let location = json!({"@type":"location","latitude":lat,"longitude":lon,"horizontal_accuracy":0.});
                if kind == "Venue" {
                    if title.trim().is_empty() {
                        return Err("Enter a venue name.".into());
                    }
                    json!({"@type":"inputPollMediaVenue","venue":{"@type":"venue","location":location,"title":title,"address":address,"provider":"","id":"","type":""}})
                } else {
                    json!({"@type":"inputPollMediaLocation","location":location})
                }
            };
            name.set_text(&format!("{kind} attached"));
            *state.borrow_mut() = Some(Attachment { kind, path: PathBuf::new(), metadata: media });
            Ok(())
        }
    });
    d.present();
}
