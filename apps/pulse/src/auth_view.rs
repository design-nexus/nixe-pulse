//! Native Telegram authentication view: QR code login, phone entry, SMS code, and 2FA cloud password.

use gtk::prelude::*;
use nixe_telegram_core::models::AuthState;
use nixe_ui::widgets as w;
use std::cell::RefCell;
use std::rc::Rc;

pub struct AuthView {
    pub root: gtk::Box,
    stack: gtk::Stack,
    qr_btn: gtk::Button,
    phone_btn: gtk::Button,
    qr_link: Rc<RefCell<String>>,
    qr_area: gtk::DrawingArea,
    phone_entry: gtk::Entry,
    phone_error: gtk::Label,
    code_entry: gtk::Entry,
    code_error: gtk::Label,
    password_entry: gtk::PasswordEntry,
    password_hint: gtk::Label,
    password_error: gtk::Label,
    on_submit_phone: RefCell<Option<Box<dyn Fn(String)>>>,
    on_submit_code: RefCell<Option<Box<dyn Fn(String)>>>,
    on_submit_password: RefCell<Option<Box<dyn Fn(String)>>>,
    on_request_qr: RefCell<Option<Box<dyn Fn()>>>,
}

impl AuthView {
    pub fn new() -> Rc<Self> {
        let root = w::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);

        // Center modal card
        let card = w::vbox(16);
        card.add_css_class("auth-card");

        // Header: Icon + Title
        let header = w::vbox(6);
        header.set_halign(gtk::Align::Center);

        let icon = gtk::Image::from_icon_name("io.github.design_nexus.nixe.Pulse");
        icon.set_pixel_size(48);
        header.append(&icon);

        let title = w::label("Log in to Telegram", "section-title");
        title.set_halign(gtk::Align::Center);
        header.append(&title);

        let subtitle = w::label("Use your phone to scan or sign in with your phone number", "dim");
        subtitle.set_halign(gtk::Align::Center);
        subtitle.set_wrap(true);
        header.append(&subtitle);
        card.append(&header);

        // Mode Switcher (Phone Number / QR Code)
        let mode_switcher = w::hbox(4);
        mode_switcher.add_css_class("segmented");
        mode_switcher.set_halign(gtk::Align::Center);

        let phone_btn = gtk::Button::with_label("Phone Number");
        phone_btn.add_css_class("active");
        let qr_btn = gtk::Button::with_label("QR Code");

        mode_switcher.append(&phone_btn);
        mode_switcher.append(&qr_btn);
        card.append(&mode_switcher);

        // Page Stack
        let stack = gtk::Stack::new();
        if std::env::var_os("PULSE_SNAPSHOT").is_none() {
            stack.set_transition_type(gtk::StackTransitionType::Crossfade);
            stack.set_transition_duration(160);
        } else {
            stack.set_transition_type(gtk::StackTransitionType::None);
        }

        // 1. QR Code Page
        let qr_page = w::vbox(12);
        qr_page.set_halign(gtk::Align::Center);

        let qr_frame = w::vbox(0);
        qr_frame.add_css_class("auth-qr-frame");
        qr_frame.set_halign(gtk::Align::Center);

        let qr_link = Rc::new(RefCell::new(String::new()));
        let qr_area = gtk::DrawingArea::new();
        qr_area.set_size_request(200, 200);
        qr_area.set_halign(gtk::Align::Center);

        let link_for_draw = qr_link.clone();
        qr_area.set_draw_func(move |_, cr, width, height| {
            let link = link_for_draw.borrow();
            if link.is_empty() {
                // Draw loading or placeholder indicator
                cr.set_source_rgb(0.5, 0.5, 0.5);
                cr.rectangle(10.0, 10.0, width as f64 - 20.0, height as f64 - 20.0);
                let _ = cr.stroke();
                return;
            }

            if let Ok(code) = qrcode::QrCode::new(link.as_bytes()) {
                let n = code.width();
                let module_size = (width.min(height) as f64) / (n as f64);
                let offset_x = (width as f64 - (n as f64 * module_size)) / 2.0;
                let offset_y = (height as f64 - (n as f64 * module_size)) / 2.0;

                // QR code standard: black modules on white card background
                cr.set_source_rgb(0.08, 0.08, 0.12);
                for y in 0..n {
                    for x in 0..n {
                        if code[(x, y)] == qrcode::Color::Dark {
                            cr.rectangle(
                                offset_x + (x as f64 * module_size),
                                offset_y + (y as f64 * module_size),
                                module_size,
                                module_size,
                            );
                        }
                    }
                }
                let _ = cr.fill();
            }
        });
        qr_frame.append(&qr_area);
        qr_page.append(&qr_frame);

        let qr_help = w::label(
            "1. Open Telegram on your phone\n2. Go to Settings → Devices → Link Desktop Device\n3. Point your camera at this screen",
            "dim",
        );
        qr_help.set_justify(gtk::Justification::Center);
        qr_page.append(&qr_help);

        stack.add_named(&qr_page, Some("qr"));

        // 2. Phone Number Page
        let phone_page = w::vbox(10);
        let phone_lbl = w::label("PHONE NUMBER", "nav-group-label");
        phone_lbl.set_xalign(0.0);
        phone_page.append(&phone_lbl);

        let phone_entry = gtk::Entry::new();
        phone_entry.set_placeholder_text(Some("+1 234 567 8900"));
        phone_entry.add_css_class("settings-search");
        phone_page.append(&phone_entry);

        let phone_error = w::label("", "danger");
        phone_error.set_visible(false);
        phone_page.append(&phone_error);

        let phone_submit_btn = gtk::Button::with_label("Next");
        phone_submit_btn.add_css_class("suggested-action");
        phone_submit_btn.set_halign(gtk::Align::Fill);
        phone_page.append(&phone_submit_btn);

        stack.add_named(&phone_page, Some("phone"));
        stack.set_visible_child_name("phone");

        // 3. Verification Code Page
        let code_page = w::vbox(10);
        let code_lbl = w::label("VERIFICATION CODE", "nav-group-label");
        code_lbl.set_xalign(0.0);
        code_page.append(&code_lbl);

        let code_desc = w::label("Enter the code sent to your Telegram app or via SMS", "dim");
        code_desc.set_xalign(0.0);
        code_page.append(&code_desc);

        let code_entry = gtk::Entry::new();
        code_entry.set_placeholder_text(Some("Code"));
        code_entry.add_css_class("settings-search");
        code_page.append(&code_entry);

        let code_error = w::label("", "danger");
        code_error.set_visible(false);
        code_page.append(&code_error);

        let code_actions = w::hbox(8);
        let code_back_btn = gtk::Button::with_label("Back");
        let code_submit_btn = gtk::Button::with_label("Verify");
        code_submit_btn.add_css_class("suggested-action");
        code_submit_btn.set_hexpand(true);

        code_actions.append(&code_back_btn);
        code_actions.append(&code_submit_btn);
        code_page.append(&code_actions);
        let passkey = gtk::Button::with_label("Sign in with a security key");
        passkey.connect_clicked(|_|crate::account_extras::login_passkey());
        phone_page.append(&passkey);
        let resend = gtk::Button::with_label("Resend login code");
        resend.connect_clicked(|_|crate::actions::request(0,"resendAuthenticationCode",serde_json::json!({"reason":null})));code_page.append(&resend);

        stack.add_named(&code_page, Some("code"));

        // 4. Two-Step Verification (Password) Page
        let password_page = w::vbox(10);
        let password_lbl = w::label("TWO-STEP VERIFICATION", "nav-group-label");
        password_lbl.set_xalign(0.0);
        password_page.append(&password_lbl);

        let password_desc = w::label("Your account is protected with a cloud password.", "dim");
        password_desc.set_xalign(0.0);
        password_page.append(&password_desc);

        let password_hint = w::label("", "dim");
        password_hint.set_xalign(0.0);
        password_hint.set_visible(false);
        password_page.append(&password_hint);

        let password_entry = gtk::PasswordEntry::new();
        password_entry.set_show_peek_icon(true);
        password_entry.add_css_class("settings-search");
        password_page.append(&password_entry);

        let password_error = w::label("", "danger");
        password_error.set_visible(false);
        password_page.append(&password_error);

        let password_submit_btn = gtk::Button::with_label("Submit");
        password_submit_btn.add_css_class("suggested-action");
        password_submit_btn.set_halign(gtk::Align::Fill);
        password_page.append(&password_submit_btn);
        let recover = gtk::Button::with_label("Forgot password? Recover by email");
        recover.connect_clicked(|_|crate::auth_tools::recover_login()); password_page.append(&recover);

        stack.add_named(&password_page, Some("password"));

        card.append(&stack);
        root.append(&card);

        let view = Rc::new(Self {
            root,
            stack,
            qr_btn: qr_btn.clone(),
            phone_btn: phone_btn.clone(),
            qr_link,
            qr_area,
            phone_entry: phone_entry.clone(),
            phone_error: phone_error.clone(),
            code_entry: code_entry.clone(),
            code_error: code_error.clone(),
            password_entry: password_entry.clone(),
            password_hint: password_hint.clone(),
            password_error: password_error.clone(),
            on_submit_phone: RefCell::new(None),
            on_submit_code: RefCell::new(None),
            on_submit_password: RefCell::new(None),
            on_request_qr: RefCell::new(None),
        });

        // Mode switch handlers
        {
            let v = view.clone();
            qr_btn.connect_clicked(move |b| {
                v.qr_btn.add_css_class("active");
                v.phone_btn.remove_css_class("active");
                v.stack.set_visible_child_name("qr");
                if let Some(ref cb) = *v.on_request_qr.borrow() {
                    cb();
                }
                b.grab_focus();
            });
        }
        {
            let v = view.clone();
            phone_btn.connect_clicked(move |b| {
                v.phone_btn.add_css_class("active");
                v.qr_btn.remove_css_class("active");
                v.stack.set_visible_child_name("phone");
                v.phone_entry.grab_focus();
                b.grab_focus();
            });
        }

        // Phone submit
        {
            let v = view.clone();
            let pe = phone_entry.clone();
            let submit_action = move || {
                let text = pe.text().trim().to_string();
                if !text.is_empty() {
                    if let Some(ref cb) = *v.on_submit_phone.borrow() {
                        cb(text);
                    }
                }
            };

            let sa = submit_action.clone();
            phone_submit_btn.connect_clicked(move |_| sa());
            phone_entry.connect_activate(move |_| submit_action());
        }

        // Code back & submit
        {
            let v = view.clone();
            code_back_btn.connect_clicked(move |_| {
                v.stack.set_visible_child_name("phone");
                v.phone_entry.grab_focus();
            });
        }
        {
            let v = view.clone();
            let ce = code_entry.clone();
            let submit_code = move || {
                let code = ce.text().trim().to_string();
                if !code.is_empty() {
                    if let Some(ref cb) = *v.on_submit_code.borrow() {
                        cb(code);
                    }
                }
            };

            let sc = submit_code.clone();
            code_submit_btn.connect_clicked(move |_| sc());
            code_entry.connect_activate(move |_| submit_code());
        }

        // Password submit
        {
            let v = view.clone();
            let pe = password_entry.clone();
            let submit_pwd = move || {
                let pwd = pe.text().to_string();
                if !pwd.is_empty() {
                    if let Some(ref cb) = *v.on_submit_password.borrow() {
                        cb(pwd);
                    }
                }
            };

            let sp = submit_pwd.clone();
            password_submit_btn.connect_clicked(move |_| sp());
            password_entry.connect_activate(move |_| submit_pwd());
        }

        view
    }

    /// Sets the QR code payload link (e.g. `tg://login?token=...`) and redraws the barcode.
    pub fn set_qr_link(&self, link: &str) {
        *self.qr_link.borrow_mut() = link.to_string();
        self.qr_area.queue_draw();
        self.qr_btn.add_css_class("active");
        self.phone_btn.remove_css_class("active");
        self.stack.set_visible_child_name("qr");
    }

    /// Explicitly activates and displays the phone number entry screen.
    pub fn show_phone(&self) {
        self.phone_btn.add_css_class("active");
        self.qr_btn.remove_css_class("active");
        self.stack.set_visible_child_name("phone");
        self.phone_entry.grab_focus();
    }

    /// Explicitly activates and displays the QR code login screen.
    #[allow(dead_code)]
    pub fn show_qr(&self) {
        self.qr_btn.add_css_class("active");
        self.phone_btn.remove_css_class("active");
        self.stack.set_visible_child_name("qr");
    }

    /// Explicitly activates and displays the SMS/Telegram verification code prompt.
    pub fn show_code(&self) {
        self.stack.set_visible_child_name("code");
        self.code_entry.grab_focus();
    }

    /// Explicitly activates and displays the two-step cloud password prompt.
    pub fn show_password(&self, hint: Option<&str>) {
        self.stack.set_visible_child_name("password");
        if let Some(h) = hint {
            self.password_hint.set_text(&format!("Hint: {h}"));
            self.password_hint.set_visible(true);
        } else {
            self.password_hint.set_visible(false);
        }
        self.password_entry.grab_focus();
    }

    /// Updates the view state in response to TDLib `AuthState` transitions.
    pub fn set_state(&self, state: &AuthState) {
        self.clear_errors();
        match state {
            AuthState::WaitOtherDeviceConfirmation { link } => {
                self.set_qr_link(link);
            }
            AuthState::WaitPhoneNumber => {
                // If user selected phone, stay on phone, else show QR
                if self.phone_btn.has_css_class("active") {
                    self.show_phone();
                } else {
                    self.stack.set_visible_child_name("qr");
                }
            }
            AuthState::WaitCode { .. } => self.show_code(),
            AuthState::WaitPassword { hint } => self.show_password(hint.as_deref()),
            _ => {}
        }
    }

    /// Sets an error message on the active page.
    pub fn set_error(&self, message: &str) {
        match self.stack.visible_child_name().as_deref() {
            Some("phone") => {
                self.phone_error.set_text(message);
                self.phone_error.set_visible(true);
            }
            Some("code") => {
                self.code_error.set_text(message);
                self.code_error.set_visible(true);
            }
            Some("password") => {
                self.password_error.set_text(message);
                self.password_error.set_visible(true);
            }
            _ => {}
        }
    }

    /// Clears any visible error messages across all pages.
    pub fn clear_errors(&self) {
        self.phone_error.set_visible(false);
        self.code_error.set_visible(false);
        self.password_error.set_visible(false);
    }

    pub fn connect_submit_phone<F: Fn(String) + 'static>(&self, callback: F) {
        *self.on_submit_phone.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_submit_code<F: Fn(String) + 'static>(&self, callback: F) {
        *self.on_submit_code.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_submit_password<F: Fn(String) + 'static>(&self, callback: F) {
        *self.on_submit_password.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_request_qr<F: Fn() + 'static>(&self, callback: F) {
        *self.on_request_qr.borrow_mut() = Some(Box::new(callback));
    }
}
