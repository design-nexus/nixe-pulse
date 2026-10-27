//! Settings as a modal card over the window (`nyx_ui::settings_card`).

use crate::prefs;
use crate::theme;
use gtk::prelude::*;
use nyx_ui::settings_card::{self, Spec};
use nyx_ui::widgets as w;

use gtk::glib;
pub use nyx_ui::settings_card::{close, escape, fit, focus_search, is_open};
use serde_json::{Value, json};

const GROUPS: &[(&str, &str, &str)] = &[
    ("Account", "avatar-default-symbolic", "Phone number, active sessions, logout"),
    ("Appearance", "applications-graphics-symbolic", "Follow Omarchy, theme, motion"),
    ("Privacy and security", "channel-secure-symbolic", "2FA password, last seen, phone visibility"),
    ("Connection and proxy", "preferences-system-network-symbolic", "MTProto proxy, SOCKS5, ping latency"),
    ("Notifications", "preferences-system-notifications-symbolic", "Desktop alerts, badges, sounds"),
    ("Voice and calls", "call-start-symbolic", "Microphone, speakers, camera"),
    ("About", "help-about-symbolic", "Pulse, TDLib, licenses"),
];

const SPEC: Spec = Spec { groups: GROUPS, search_text: None };

pub fn open(overlay: &gtk::Overlay, window: &gtk::ApplicationWindow) {
    settings_card::open(overlay, window, SPEC, || {
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .vexpand(true)
            .build();

        let body = w::vbox(24);
        body.set_margin_top(16);
        body.set_margin_bottom(24);
        body.set_margin_start(20);
        body.set_margin_end(20);

        let tools = gtk::Button::with_label("More account and app tools…");
        tools.connect_clicked(|_| crate::features::hub()); body.append(&tools);
        build_account_group(&body);
        build_appearance_group(&body);
        build_privacy_group(&body);
        build_proxy_group(&body);
        build_notifications_group(&body);
        build_calls_group(&body);
        build_about_group(&body);

        root.set_child(Some(&body));
        (root.upcast(), body)
    });
}

fn group_box(title: &str) -> (gtk::Box, gtk::Box) {
    let wrap = w::vbox(10);
    wrap.set_widget_name(title);
    wrap.add_css_class("settings-group");

    let card = w::vbox(0);
    card.add_css_class("group-list");
    wrap.append(&card);

    (wrap, card)
}

fn request(method: &str, args: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
    let Some(service) = crate::window::service() else {
        done(Err("Telegram is unavailable".into()));
        return;
    };
    let generation = service.generation.clone();
    let rx = service.request(method, args);
    glib::spawn_future_local(async move {
        let result = rx.recv().await.unwrap_or_else(|_| Err("Service closed".into()));
        if crate::window::service_is_current(&generation) {
            done(result);
        }
    });
}
fn build_account_group(body: &gtk::Box) {
    let (wrap, card) = group_box("Account");
    let who = w::label("Loading account…", "dim");
    card.append(&who);
    request("getMe", json!({}), move |r| {
        who.set_text(
            &r.map(|u| {
                format!(
                    "{} {} · +{}",
                    u["first_name"].as_str().unwrap_or(""),
                    u["last_name"].as_str().unwrap_or(""),
                    u["phone_number"].as_str().unwrap_or("")
                )
            })
            .unwrap_or_else(|e| e),
        );
    });
    let mgr = nyx_telegram_core::account::AccountManager::new(nyx_core::paths::data_dir(), nyx_core::paths::cache_dir());
    for id in mgr.list_accounts() {
        let b = gtk::Button::with_label(&if id == prefs::get().active_account_id {
            format!("● {id}")
        } else {
            format!("Switch to {id}")
        });
        b.connect_clicked(move |_| {
            close();
            crate::window::switch_account(id.clone());
        });
        card.append(&b);
    }
    let add = gtk::Button::with_label("Add account");
    add.connect_clicked(|_| {
        close();
        crate::window::add_account();
    });
    card.append(&add);
    let logout = gtk::Button::with_label("Log out of this account");
    logout.connect_clicked(|_| {
        close();
        crate::window::log_out();
    });
    card.append(&logout);
    let sessions = w::vbox(4);
    card.append(&sessions);
    request("getActiveSessions", json!({}), move |r| {
        if let Ok(data) = r {
            for session in data["sessions"].as_array().into_iter().flatten() {
                let row = w::hbox(4);
                let label = w::label(
                    &format!(
                        "{} · {}{}",
                        session["application_name"].as_str().unwrap_or("Session"),
                        session["device_model"].as_str().unwrap_or(""),
                        if session["is_current"] == true { " (this device)" } else { "" }
                    ),
                    "dim",
                );
                label.set_wrap(true);
                label.set_hexpand(true);
                row.append(&label);
                if session["is_current"] != true {
                    let id = session["id"].clone();
                    let b = gtk::Button::with_label("Revoke");
                    b.connect_clicked(move |_| {
                        request("terminateSession", json!({"session_id":id}), |r| {
                            crate::window::show_status(&r.map(|_| "Session revoked".to_owned()).unwrap_or_else(|e| e));
                        });
                    });
                    row.append(&b);
                }
                sessions.append(&row);
            }
        }
    });
    body.append(&wrap);
}

fn refresh_swatches(row: &gtk::Box) {
    while let Some(c) = row.first_child() {
        row.remove(&c);
    }
    let p = theme::current_palette();
    for (colour, name) in [
        (&p.bg, "Background"),
        (&p.surface, "Surface"),
        (&p.muted, "Muted"),
        (&p.text, "Text"),
        (&p.accent, "Accent"),
        (&p.danger, "Danger"),
    ] {
        let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        s.add_css_class("swatch");
        s.set_tooltip_text(Some(&format!("{name} {colour}")));
        w::paint(&s, colour);
        row.append(&s);
    }
}

fn build_appearance_group(body: &gtk::Box) {
    let (wrap, card) = group_box("Appearance");

    let p = prefs::get();
    let omarchy = theme::omarchy_available();
    let following = omarchy && p.mode == prefs::ThemeMode::Omarchy;

    let themes = theme::all();
    let options: Vec<(String, String)> = themes.iter().map(|t| (t.id.clone(), t.name.clone())).collect();
    let swatches = w::hbox(8);
    swatches.set_halign(gtk::Align::End);
    refresh_swatches(&swatches);

    let (theme_row, dropdown) =
        w::choice_row("Theme", "Colors for the window, when it isn't following Omarchy.", options, &p.theme, {
            let swatches = swatches.clone();
            move |id| {
                prefs::update(|p| {
                    p.mode = prefs::ThemeMode::Theme;
                    p.theme = id.clone();
                });
                theme::apply();
                refresh_swatches(&swatches);
            }
        });
    dropdown.set_sensitive(!following);

    if omarchy {
        let (row, switch) =
            w::switch_row("Follow Omarchy theme", "Match the colors of the current Omarchy theme as it changes.", following, {
                let (dropdown, swatches) = (dropdown.clone(), swatches.clone());
                move |on| {
                    prefs::update(|p| p.mode = if on { prefs::ThemeMode::Omarchy } else { prefs::ThemeMode::Theme });
                    theme::apply();
                    dropdown.set_sensitive(!on);
                    refresh_swatches(&swatches);
                }
            });
        card.append(&row);
        dropdown.connect_selected_notify(move |_| {
            if switch.is_active() {
                switch.set_active(false);
            }
        });
    }
    card.append(&theme_row);
    card.append(&w::row("Current colors", "", Some(swatches.upcast_ref())));

    let (glow, _) = w::switch_row("Glow", "A soft light around selected and focused items.", p.glow, |on| {
        prefs::update(|p| p.glow = on);
        theme::apply();
    });
    card.append(&glow);

    let (motion_row, _) = w::switch_row("Reduce Motion", "Disable UI animations and crossfades", p.reduce_motion, |active| {
        prefs::update(|p| {
            p.reduce_motion = active;
        });
        theme::apply();
    });
    card.append(&motion_row);

    body.append(&wrap);
}

fn privacy_row(card:&gtk::Box,title:&str,setting:&str) {
    let setting=setting.to_string();let title=title.to_string();let label=title.clone();
    let button=gtk::Button::with_label(&format!("{label} and exceptions…"));
    button.connect_clicked(move |_|crate::account_tools::privacy_edit(&title,&setting));card.append(&button);
}
fn build_privacy_group(body: &gtk::Box) {
    let (wrap, card) = group_box("Privacy and security");
    let status = w::label("Loading two-step verification…", "dim");
    card.append(&status);
    request("getPasswordState", json!({}), move |r| {
        status.set_text(
            &r.map(|v| {
                if v["has_password"] == true {
                    "Two-step verification enabled".into()
                } else {
                    "Two-step verification disabled".into()
                }
            })
            .unwrap_or_else(|e| e),
        )
    });
    let password = gtk::Button::with_label("Configure two-step verification");
    password.connect_clicked(|b|{
 let Some(parent)=b.root().and_downcast::<gtk::Window>()else{return};let d=gtk::Window::builder().title("Two-step verification").transient_for(&parent).modal(true).default_width(420).build();let box_=w::vbox(8);let old=gtk::PasswordEntry::new();old.set_show_peek_icon(true);old.set_tooltip_text(Some("Current password (empty if none)"));box_.append(&old);let new=gtk::PasswordEntry::new();new.set_show_peek_icon(true);new.set_tooltip_text(Some("New password (empty removes password)"));box_.append(&new);let hint=gtk::Entry::builder().placeholder_text("Password hint").build();box_.append(&hint);let email=gtk::Entry::builder().placeholder_text("Recovery email (optional)").build();box_.append(&email);let status=w::label("","dim");status.set_wrap(true);box_.append(&status);let save=gtk::Button::with_label("Save password");box_.append(&save);d.set_child(Some(&box_));save.connect_clicked(move |_|{let status=status.clone();request("setPassword",json!({"old_password":old.text().to_string(),"new_password":new.text().to_string(),"new_hint":hint.text().to_string(),"set_recovery_email_address":!email.text().is_empty(),"new_recovery_email_address":email.text().to_string()}),move |r|status.set_text(&r.map(|v|if v["recovery_email_address_code_info"].is_object(){"Password saved; verify your recovery email in Telegram".into()}else{"Password saved".into()}).unwrap_or_else(|e|e)));});d.present();});
    card.append(&password);
    privacy_row(&card, "Phone number visibility", "userPrivacySettingShowPhoneNumber");
    privacy_row(&card, "Last seen and online", "userPrivacySettingShowStatus");
    privacy_row(&card, "Who can call", "userPrivacySettingAllowCalls");
    privacy_row(&card, "Peer-to-peer calls", "userPrivacySettingAllowPeerToPeerCalls");
    let blocked = w::vbox(4);
    card.append(&blocked);
    request("getBlockedMessageSenders", json!({"block_list":{"@type":"blockListMain"},"offset":0,"limit":50}), move |result| {
        if let Ok(data) = result {
            blocked.append(&w::label(&format!("{} blocked users", data["total_count"].as_i64().unwrap_or(0)), "dim"));
            for sender in data["senders"].as_array().into_iter().flatten() {
                let id = sender["user_id"].as_i64().unwrap_or(0);
                let row = gtk::Button::with_label(&format!("Unblock user {id}"));
                let sender = sender.clone();
                row.connect_clicked(move |button| {
                    let button = button.clone();
                    request("setMessageSenderBlockList", json!({"sender_id":sender,"block_list":null}), move |r| match r {
                        Ok(_) => button.set_sensitive(false),
                        Err(e) => crate::window::show_status(&e),
                    });
                });
                blocked.append(&row);
            }
        }
    });
    let user = gtk::Entry::builder().placeholder_text("Telegram user ID to block").build();
    card.append(&user);
    let block = gtk::Button::with_label("Block user");
    block.connect_clicked(move |_| {
        if let Ok(id) = user.text().parse::<i64>() {
            request(
                "setMessageSenderBlockList",
                json!({"sender_id":{"@type":"messageSenderUser","user_id":id},"block_list":{"@type":"blockListMain"}}),
                |r| crate::window::show_status(&r.map(|_| "User blocked".into()).unwrap_or_else(|e| e)),
            );
        }
    });
    card.append(&block);
    let recovery = gtk::Entry::builder().placeholder_text("Recovery email verification code").build();
    card.append(&recovery);
    let confirm = gtk::Button::with_label("Verify recovery email");
    confirm.connect_clicked(move |_| {
        request("checkRecoveryEmailAddressCode", json!({"code":recovery.text().to_string()}), |r| {
            crate::window::show_status(&r.map(|_| "Recovery email verified".into()).unwrap_or_else(|e| e))
        })
    });
    card.append(&confirm);
    body.append(&wrap);
}
fn build_proxy_group(body: &gtk::Box) {
    let (wrap, card) = group_box("Connection and proxy");
    let list = w::vbox(4);
    card.append(&list);
    request("getProxies", json!({}), move |r| match r {
        Ok(data) => {
            let mut enabled = false;
            for proxy in data["proxies"].as_array().into_iter().flatten() {
                enabled |= proxy["is_enabled"] == true;
                let p = proxy.get("proxy").unwrap_or(proxy);
                let id = proxy["id"].clone();
                let b = gtk::Button::with_label(&format!(
                    "{}:{}{}",
                    p["server"].as_str().unwrap_or(""),
                    p["port"],
                    if proxy["is_enabled"] == true { " · active" } else { " · enable" }
                ));
                b.connect_clicked(move |_| {
                    request("enableProxy", json!({"proxy_id":id}), |r| {
                        crate::window::show_status(&r.map(|_| "Proxy enabled".into()).unwrap_or_else(|e| e))
                    })
                });
                list.append(&b);
            }
            if !enabled {
                list.append(&w::label("Direct connection", "dim"));
            }
        }
        Err(e) => list.append(&w::label(&e, "dim")),
    });
    let direct = gtk::Button::with_label("Use direct connection");
    direct.connect_clicked(|_| {
        request("disableProxy", json!({}), |r| {
            crate::window::show_status(&r.map(|_| "Proxy disabled".into()).unwrap_or_else(|e| e))
        })
    });
    card.append(&direct);
    let server = gtk::Entry::builder().placeholder_text("Proxy hostname").build();
    let port = gtk::Entry::builder().placeholder_text("Port").build();
    let user = gtk::Entry::builder().placeholder_text("SOCKS5 username (optional)").build();
    let secret = gtk::PasswordEntry::new();
    secret.set_tooltip_text(Some("SOCKS5 password or MTProto secret"));
    let kind = gtk::DropDown::from_strings(&["SOCKS5", "MTProto"]);
    for field in [&server, &port, &user] {
        card.append(field);
    }
    card.append(&kind);
    card.append(&secret);
    let add = gtk::Button::with_label("Add and enable proxy");
    let status = w::label("", "dim");
    status.set_wrap(true);
    card.append(&status);
    add.connect_clicked(move |_| {
        let Ok(port) = port.text().parse::<u16>() else {
            status.set_text("Enter a valid port");
            return;
        };
        if server.text().trim().is_empty() {
            status.set_text("Enter a hostname");
            return;
        }
        let typ = if kind.selected() == 1 {
            json!({"@type":"proxyTypeMtproto","secret":secret.text().to_string()})
        } else {
            json!({"@type":"proxyTypeSocks5","username":user.text().to_string(),"password":secret.text().to_string()})
        };
        let proxy = json!({"@type":"proxy","server":server.text().to_string(),"port":port,"type":typ});
        let saved = proxy.clone();
        let status = status.clone();
        let account = prefs::get().active_account_id;
        request("addProxy", json!({"proxy":proxy,"enable":true,"comment":"Pulse"}), move |result| match result {
            Ok(value) => {
                status.set_text("Proxy enabled");
                if let Some(id) = value["id"].as_i64() {
                    std::thread::spawn(move || {
                        if let Err(e) = crate::secrets::store_proxy(&account, id, &saved) {
                            tracing::error!("{e}");
                        }
                    });
                }
            }
            Err(e) => status.set_text(&e),
        });
    });
    card.append(&add);
    body.append(&wrap);
}

fn build_notifications_group(body: &gtk::Box) {
    let (wrap, card) = group_box("Notifications");

    let (notify_row, _) = w::switch_row(
        "Desktop Notifications",
        "Show incoming message banners and call alerts",
        prefs::get().notifications_enabled,
        |active| {
            prefs::update(|p| {
                p.notifications_enabled = active;
            });
        },
    );
    card.append(&notify_row);

    let (sound_row, _) =
        w::switch_row("Sound Effects", "Play sound when receiving messages or calls", prefs::get().sound_enabled, |active| {
            prefs::update(|p| {
                p.sound_enabled = active;
            });
        });
    card.append(&sound_row);

    body.append(&wrap);
}

fn build_calls_group(body: &gtk::Box) {
    let (wrap, card) = group_box("Voice and calls");
    let button = gtk::Button::with_label("Audio and video devices");
    button.connect_clicked(|b| crate::calls::devices(b));
    card.append(&button);
    body.append(&wrap);
}

fn build_about_group(body: &gtk::Box) {
    let (wrap, card) = group_box("About");

    let ver_lbl = w::label("v0.1.0", "accent");
    let version_row =
        w::row("Pulse", "Native Telegram desktop client for the Nyx application suite", Some(ver_lbl.upcast_ref()));
    card.append(&version_row);

    let td_lbl = w::label("TDLib JSON API", "dim");
    let backend_row = w::row("Protocol Engine", "Official Telegram Database Library (TDLib v1.8)", Some(td_lbl.upcast_ref()));
    card.append(&backend_row);

    let lic_lbl = w::label("Open Source", "dim");
    let license_row = w::row("License", "MIT License (Nyx) · BSL-1.0 (TDLib) · LGPL-3.0 (tgcalls)", Some(lic_lbl.upcast_ref()));
    card.append(&license_row);

    body.append(&wrap);
}
