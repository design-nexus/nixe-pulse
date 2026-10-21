//! Call state and controls driven by the native media engine.
use gtk::{gdk, glib, prelude::*};
use nixe_ui::widgets as w;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};
pub struct CallingUi {
    pub root: gtk::Box,
    title: gtk::Label,
    status: gtk::Label,
    emojis: gtk::Label,
    picture: gtk::Picture,
    accept: gtk::Button,
    active: Cell<Option<i32>>,
    mute: gtk::ToggleButton,
    camera: gtk::ToggleButton,
    share: gtk::ToggleButton,
    video: Cell<bool>,
    started: Cell<Option<Instant>>,
    on_accept: RefCell<Option<Box<dyn Fn(i32)>>>,
    on_hangup: RefCell<Option<Box<dyn Fn(i32, i32, bool)>>>,
}
impl CallingUi {
    pub fn new() -> Rc<Self> {
        let root = w::vbox(12);
        root.add_css_class("call-active-card");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        root.set_visible(false);
        let title = w::label("", "call-peer-name");
        root.append(&title);
        let status = w::label("", "dim");
        root.append(&status);
        let emojis = w::label("", "call-encryption-emojis");
        root.append(&emojis);
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_size_request(360, 220);
        picture.set_visible(false);
        root.append(&picture);
        let buttons = w::hbox(8);
        let accept = gtk::Button::with_label("Answer");
        buttons.append(&accept);
        let mute = gtk::ToggleButton::with_label("Mute");
        buttons.append(&mute);
        let camera = gtk::ToggleButton::with_label("Camera");
        buttons.append(&camera);
        let share = gtk::ToggleButton::with_label("Share screen");
        buttons.append(&share);
        let devices = w::bar_button("preferences-system-symbolic", "Devices");
        devices.connect_clicked(crate::calls::devices);
        buttons.append(&devices);
        let hangup = gtk::Button::with_label("End call");
        hangup.add_css_class("destructive-action");
        buttons.append(&hangup);
        root.append(&buttons);
        let ui = Rc::new(Self {
            root,
            title,
            status,
            emojis,
            picture,
            accept: accept.clone(),
            active: Cell::new(None),
            mute: mute.clone(),
            camera: camera.clone(),
            share: share.clone(),
            video: Cell::new(false),
            started: Cell::new(None),
            on_accept: Default::default(),
            on_hangup: Default::default(),
        });
        let weak = Rc::downgrade(&ui);
        accept.connect_clicked(move |b| {
            if let Some(ui) = weak.upgrade() {
                b.set_sensitive(false);
                ui.status.set_text("Connecting…");
                if let Some(id) = ui.active.get() {
                    if let Some(cb) = ui.on_accept.borrow().as_ref() {
                        cb(id);
                    }
                }
            }
        });
        let weak = Rc::downgrade(&ui);
        hangup.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.hang_up();
            }
        });
        mute.connect_toggled(|b| {
            if !crate::calls::mute(b.is_active()) {
                b.set_active(false);
            }
        });
        camera.connect_toggled(|b| {
            if !crate::calls::camera(b.is_active()) {
                b.set_active(false);
            }
        });
        let weak = Rc::downgrade(&ui);
        share.connect_toggled(move |b| {
            if let Some(ui) = weak.upgrade() {
                if b.is_active() {
                    ui.camera.set_active(false);
                }
                ui.camera.set_sensitive(!b.is_active());
            }
            crate::calls::share(b.is_active());
        });
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            if let Some(start) = ui.started.get() {
                ui.status.set_text(&format!(
                    "Connected · {}",
                    nixe_telegram_calls::format_call_duration(start.elapsed().as_secs() as u32)
                ));
            }
            glib::ControlFlow::Continue
        });
        ui
    }
    pub fn show_incoming(&self, id: i32, _user: i64, name: &str, video: bool) {
        self.active.set(Some(id));
        self.video.set(video);
        self.title.set_text(name);
        self.status.set_text(if video { "Incoming video call" } else { "Incoming voice call" });
        self.started.set(None);
        self.mute.set_sensitive(false);
        self.camera.set_sensitive(false);
        self.share.set_sensitive(false);
        self.accept.set_visible(true);
        self.accept.set_sensitive(true);
        self.emojis.set_text("");
        self.root.set_visible(true);
    }
    pub fn show_active(&self, id: i32, _user: i64, name: &str, video: bool, emojis: &[String]) {
        self.active.set(Some(id));
        self.video.set(video);
        self.title.set_text(name);
        self.emojis.set_text(&emojis.join(" "));
        self.mute.set_sensitive(true);
        self.camera.set_sensitive(true);
        self.share.set_sensitive(true);
        self.camera.set_active(video);
        self.accept.set_visible(false);
        if self.started.get().is_none() {
            self.started.set(Some(Instant::now()));
        }
        self.status.set_text("Connected");
        self.root.set_visible(true);
    }
    pub fn show_connecting(&self, id: i32, user: i64, video: bool) {
        self.active.set(Some(id));
        self.video.set(video);
        self.title.set_text(&format!("User {user}"));
        self.status.set_text("Calling / connecting…");
        self.mute.set_sensitive(false);
        self.camera.set_sensitive(false);
        self.share.set_sensitive(false);
        self.accept.set_visible(false);
        self.emojis.set_text("");
        self.root.set_visible(true);
    }
    pub fn set_peer_name(&self, id: i32, name: &str) {
        if self.active.get() == Some(id) {
            self.title.set_text(name);
        }
    }
    pub fn screen_stopped(&self) {
        self.share.set_active(false);
    }
    pub fn connection_error(&self, error: &str) {
        self.status.set_text(error);
        self.accept.set_sensitive(true);
    }
    pub fn set_video(&self, texture: &gdk::Texture) {
        self.picture.set_paintable(Some(texture));
        self.picture.set_visible(true);
    }
    pub fn hang_up(&self) {
        if let Some(id) = self.active.take() {
            if let Some(cb) = self.on_hangup.borrow().as_ref() {
                cb(id, self.started.get().map(|t| t.elapsed().as_secs() as i32).unwrap_or(0), self.video.get());
            }
        }
        crate::calls::stop();
        self.dismiss();
    }
    pub fn dismiss(&self) {
        self.active.set(None);
        self.started.set(None);
        self.mute.set_active(false);
        self.camera.set_active(false);
        self.share.set_active(false);
        self.picture.set_paintable(gdk::Paintable::NONE);
        self.picture.set_visible(false);
        self.root.set_visible(false);
    }
    pub fn connect_accept<F: Fn(i32) + 'static>(&self, f: F) {
        *self.on_accept.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_hangup<F: Fn(i32, i32, bool) + 'static>(&self, f: F) {
        *self.on_hangup.borrow_mut() = Some(Box::new(f));
    }
}
