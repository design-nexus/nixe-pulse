//! Actual photo and audiovisual viewer using the suite's playback engine and controls.
use gtk::{gdk, gio, glib, prelude::*};
use nyx_playback::{controls, mpv, video};
use nyx_telegram_core::models::*;
use nyx_ui::widgets as w;
use std::{cell::RefCell, rc::Rc};
pub struct MediaViewer {
    pub root: gtk::Box,
    title: gtk::Label,
    save_button: gtk::Button,
    copy_button: gtk::Button,
    current_message: RefCell<Option<MessageSummary>>,
    meta: gtk::Label,
    picture: gtk::Picture,
    viewport: gtk::ScrolledWindow,
    path: RefCell<Option<String>>,
    player: RefCell<Option<mpv::Mpv>>,
    controls: controls::OsdControls,
    seek: controls::SeekWidgets,
    zoom: std::cell::Cell<f64>,
    image_size: std::cell::Cell<(i32, i32)>,
    gallery: RefCell<Vec<MessageSummary>>,
    gallery_previous: gtk::Button,
    gallery_next: gtk::Button,
    current_id: std::cell::Cell<i64>,
    generation: std::cell::Cell<u64>,
}
impl MediaViewer {
    pub fn new() -> Rc<Self> {
        controls::install_icons();
        let root = w::vbox(8);
        root.add_css_class("media-viewer-backdrop");
        root.set_visible(false);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let header = w::hbox(8);
        let title = w::label("Media", "media-viewer-title");
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.append(&title);
        let gallery_previous = w::bar_button("go-previous-symbolic", "Previous media");
        let gallery_next = w::bar_button("go-next-symbolic", "Next media");
        header.append(&gallery_previous);
        header.append(&gallery_next);
        let zoom_out = w::bar_button("zoom-out-symbolic", "Zoom out");
        let zoom_in = w::bar_button("zoom-in-symbolic", "Zoom in");
        let save = w::bar_button("document-save-symbolic", "Save as (Ctrl+S)");
        let copy = w::bar_button("edit-copy-symbolic", "Copy (Ctrl+C)");
        let close = w::bar_button("window-close-symbolic", "Close (Esc)");
        for b in [&zoom_out, &zoom_in, &save, &copy, &close] {
            header.append(b);
        }
        root.append(&header);
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        let viewport = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&picture)
            .build();
        root.append(&viewport);
        let meta = w::label("", "dim");
        root.append(&meta);
        let seek = controls::SeekWidgets::new();
        let controls = controls::OsdControls::new(&seek.root, 1.0, 1.0, false);
        for b in [&controls.shape, &controls.speed, &controls.audio, &controls.subs, &controls.chapters] {
            b.set_visible(false);
        }
        controls.full.set_visible(false);
        root.append(&controls.root);
        let v = Rc::new(Self {
            root,
            save_button: save.clone(),
            copy_button: copy.clone(),
            current_message: Default::default(),
            title,
            meta,
            picture,
            viewport,
            path: Default::default(),
            player: Default::default(),
            controls,
            seek,
            zoom: std::cell::Cell::new(1.0),
            image_size: std::cell::Cell::new((0, 0)),
            gallery: Default::default(),
            gallery_previous,
            gallery_next,
            current_id: std::cell::Cell::new(0),
            generation: std::cell::Cell::new(0),
        });
        let drag = gtk::GestureDrag::new();
        let start = std::rc::Rc::new(std::cell::Cell::new((0.0, 0.0)));
        let pos = start.clone();
        let weak = Rc::downgrade(&v);
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(v) = weak.upgrade() {
                pos.set((v.viewport.hadjustment().value(), v.viewport.vadjustment().value()));
            }
        });
        let weak = Rc::downgrade(&v);
        drag.connect_drag_update(move |_, x, y| {
            if let Some(v) = weak.upgrade() {
                let (sx, sy) = start.get();
                v.viewport.hadjustment().set_value(sx - x);
                v.viewport.vadjustment().set_value(sy - y);
            }
        });
        v.picture.add_controller(drag);
        macro_rules! click {
            ($button:expr,$f:expr) => {{
                let weak = Rc::downgrade(&v);
                $button.connect_clicked(move |_| {
                    if let Some(v) = weak.upgrade() {
                        ($f)(v);
                    }
                });
            }};
        }
        click!(v.gallery_previous, |v: Rc<Self>| v.navigate(-1));
        click!(v.gallery_next, |v: Rc<Self>| v.navigate(1));
        click!(v.controls.prev, |v: Rc<Self>| v.navigate(-1));
        click!(v.controls.next, |v: Rc<Self>| v.navigate(1));
        click!(close, |v: Rc<Self>| v.hide());
        click!(v.controls.close, |v: Rc<Self>| v.hide());
        click!(save, |v: Rc<Self>| v.save());
        click!(copy, |v: Rc<Self>| v.copy());
        click!(zoom_in, |v: Rc<Self>| v.zoom(1.25));
        click!(zoom_out, |v: Rc<Self>| v.zoom(0.8));
        click!(v.controls.play, |v: Rc<Self>| {
            if let Some(p) = v.player.borrow().as_ref() {
                let _ = p.command(&["cycle", "pause"]);
            }
        });
        click!(v.controls.back, |v: Rc<Self>| {
            if let Some(p) = v.player.borrow().as_ref() {
                let _ = p.command(&["seek", "-10", "relative"]);
            }
        });
        click!(v.controls.fwd, |v: Rc<Self>| {
            if let Some(p) = v.player.borrow().as_ref() {
                let _ = p.command(&["seek", "10", "relative"]);
            }
        });
        click!(v.controls.mute, |v: Rc<Self>| {
            if let Some(p) = v.player.borrow().as_ref() {
                let _ = p.command(&["cycle", "mute"]);
            }
        });
        v.controls.prev.set_visible(false);
        v.controls.next.set_visible(false);
        let weak = Rc::downgrade(&v);
        v.controls.volume.connect_value_changed(move |s| {
            if let Some(v) = weak.upgrade() {
                if let Some(p) = v.player.borrow().as_ref() {
                    let _ = p.set_f64("volume", s.value() * 100.0);
                }
            }
        });
        let weak = Rc::downgrade(&v);
        v.seek.seek.connect_change_value(move |_, _, value| {
            if let Some(v) = weak.upgrade() {
                if let Some(p) = v.player.borrow().as_ref() {
                    let _ = p.set_f64("time-pos", value);
                }
            }
            glib::Propagation::Proceed
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&v);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(v) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !v.root.is_visible() {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Escape => v.hide(),
                gdk::Key::s if mods.contains(gdk::ModifierType::CONTROL_MASK) => v.save(),
                gdk::Key::c if mods.contains(gdk::ModifierType::CONTROL_MASK) => v.copy(),
                gdk::Key::plus | gdk::Key::equal => v.zoom(1.25),
                gdk::Key::minus => v.zoom(0.8),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        v.root.add_controller(keys);
        let weak = Rc::downgrade(&v);
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let Some(v) = weak.upgrade() else { return glib::ControlFlow::Break };
            if v.root.is_visible() {
                if let Some(p) = v.player.borrow().as_ref() {
                    let pos = p.get_f64("time-pos").unwrap_or(0.0);
                    let duration = p.get_f64("duration").unwrap_or(0.0);
                    v.seek.seek.set_range(0.0, duration.max(1.0));
                    v.seek.seek.set_value(pos);
                    v.seek.pos.set_text(&controls::time(pos));
                    v.seek.dur.set_text(&controls::time(duration));
                    v.controls.play.set_icon_name(if p.get_str("pause").as_deref() == Some("yes") {
                        "media-playback-start-symbolic"
                    } else {
                        "media-playback-pause-symbolic"
                    });
                    while p.next_event().is_some() {}
                }
            }
            glib::ControlFlow::Continue
        });
        v
    }
    pub fn show_message(self: &Rc<Self>, m: &MessageSummary) {
        let path = m
            .files
            .iter()
            .find(|f| f.role == "original")
            .and_then(|f| f.path.clone())
            .filter(|p| std::path::Path::new(p).exists());
        let Some(path) = path else {
            let Some(file) = m.files.iter().find(|f| f.role == "original") else { return };
            let Some(s) = crate::window::service() else { return };
            let generation = self.generation.get() + 1;
            self.generation.set(generation);
            let rx = s.request(
                "downloadFile",
                serde_json::json!({"file_id":file.id,"priority":32,"offset":0,"limit":0,"synchronous":true}),
            );
            let weak = Rc::downgrade(self);
            let mut message = m.clone();
            let file_id = file.id;
            crate::window::show_status("Downloading media…");
            glib::spawn_future_local(async move {
                if let Some(v) = weak.upgrade() {
                    if let Ok(result) = rx.recv().await {
                        if v.generation.get() != generation {
                            return;
                        }
                        match result {
                            Ok(file) => {
                                if let Some(path) = file["local"]["path"].as_str().filter(|p| !p.is_empty()) {
                                    for f in &mut message.files {
                                        if f.id == file_id {
                                            f.path = Some(path.into());
                                        }
                                    }
                                    v.show_message(&message);
                                }
                            }
                            Err(e) => crate::window::show_status(&e),
                        }
                    }
                }
            });
            return;
        };
        self.hide();
        self.current_id.set(m.id);
        *self.current_message.borrow_mut() = Some(m.clone());
        let export = crate::actions::exports_allowed(m);
        self.save_button.set_sensitive(export);
        self.copy_button.set_sensitive(export);
        let gallery = self.gallery.borrow();
        let index = gallery.iter().position(|m| m.id == self.current_id.get()).unwrap_or(0);
        self.gallery_previous.set_sensitive(index > 0);
        self.gallery_next.set_sensitive(index + 1 < gallery.len());
        drop(gallery);
        *self.path.borrow_mut() = Some(path.clone());
        self.title.set_text(&crate::chat_view::preview(m));
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let date = chrono::DateTime::from_timestamp(m.date, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%b %-d, %Y · %H:%M").to_string())
            .unwrap_or_default();
        let dimensions = match &m.content {
            MessageContent::Photo { width, height, .. } | MessageContent::Video { width, height, .. } => {
                format!(" · {width} × {height}")
            }
            _ => String::new(),
        };
        self.meta.set_text(&format!("{} · {date}{dimensions} · {:.1} KB", m.sender_name, size as f64 / 1024.0));
        self.zoom.set(1.0);
        self.image_size.set((0, 0));
        self.picture.set_widget_name("media-picture");
        let animation = m.data["content"]["@type"] == "messageAnimation";
        let still = if animation { None } else { gdk::Texture::from_file(&gio::File::for_path(&path)).ok() };
        if let Some(texture) = still {
            self.image_size.set((texture.width(), texture.height()));
            self.picture.set_paintable(Some(&texture));
            self.controls.root.set_visible(false);
        } else if path.ends_with(".tgs") {
            crate::animation::play(&self.picture, &path);
            self.controls.root.set_visible(false);
        } else if matches!(m.content, MessageContent::Document { .. }) {
            crate::actions::open_external_attachment(m.clone());
            return;
        } else {
            if self.player.borrow().is_none() {
                match mpv::Mpv::new(&[("vo", "libmpv"), ("terminal", "no"), ("idle", "yes")]) {
                    Ok((p, _)) => {
                        if let Some(win) = self.root.root().and_downcast::<gtk::ApplicationWindow>() {
                            let weak = Rc::downgrade(self);
                            if let Err(e) = video::init(&win, &p, move || {
                                weak.upgrade()
                                    .map(|v| (v.viewport.width().max(1), v.viewport.height().max(1)))
                                    .unwrap_or((640, 480))
                            }) {
                                crate::window::show_status(&e);
                                return;
                            }
                        }
                        *self.player.borrow_mut() = Some(p);
                    }
                    Err(e) => {
                        crate::window::show_status(&e);
                        return;
                    }
                }
            }
            self.picture.set_paintable(Some(&video::paintable()));
            if let Some(p) = self.player.borrow().as_ref() {
                if let Err(e) = p.command(&["loadfile", &path]) {
                    crate::window::show_status(&e);
                    return;
                }
                let _ = p.set_str("loop-file", if animation { "inf" } else { "no" });
                let _ = p.set_flag("pause", false);
            }
            self.controls.root.set_visible(true);
        }
        self.picture.set_size_request(-1, -1);
        self.root.set_visible(true);
        self.root.set_focusable(true);
        self.root.grab_focus();
    }
    fn zoom(&self, factor: f64) {
        let z = (self.zoom.get() * factor).clamp(0.1, 8.0);
        self.zoom.set(z);
        let (w, h) = self.image_size.get();
        if w > 0 {
            self.picture.set_size_request((w as f64 * z) as i32, (h as f64 * z) as i32);
        }
    }
    fn save(&self) {
        if let Some(message) = self.current_message.borrow().as_ref().filter(|m| crate::actions::exports_allowed(m)) {
            crate::actions::save_attachment(message.clone());
        }
    }
    fn copy(&self) {
        if let Some(message) = self.current_message.borrow().as_ref().filter(|m| crate::actions::exports_allowed(m)) {
            crate::actions::copy_attachment(message.clone());
        }
    }
    pub fn set_gallery(&self, messages: Vec<MessageSummary>) {
        *self.gallery.borrow_mut() = messages;
    }
    fn navigate(self: &Rc<Self>, direction: i32) {
        let gallery = self.gallery.borrow();
        let index = gallery.iter().position(|m| m.id == self.current_id.get()).unwrap_or(0) as i32 + direction;
        let message = if index >= 0 { gallery.get(index as usize).cloned() } else { None };
        drop(gallery);
        if let Some(m) = message {
            self.show_message(&m);
        }
    }
    pub fn hide(&self) {
        self.current_message.borrow_mut().take();
        self.save_button.set_sensitive(false);
        self.copy_button.set_sensitive(false);
        self.generation.set(self.generation.get() + 1);
        self.picture.set_widget_name("media-picture");
        if let Some(p) = self.player.borrow().as_ref() {
            let _ = p.command(&["stop"]);
        }
        self.picture.set_paintable(gdk::Paintable::NONE);
        self.root.set_visible(false);
    }
}
