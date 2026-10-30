mod dialog_dismissal;
mod account_extras;
mod story_extras;
mod business_extras;
mod gifts;
mod conversation_windows;
mod desktop_extras;
mod export_bundle;
mod privacy_tools;
mod notifications;
// Nyx Pulse: Native Telegram Desktop Client for Omarchy Linux and the Nyx Suite.

mod actions;
mod message_editing;
mod payments;
mod webapps;
mod auth_tools;
mod features;
mod account_tools;
mod desktop_tools;
mod bots;
mod stories;
mod commerce;
mod group_calls;
mod locations;
mod message_extras;
mod statistics;
mod business;
mod animation;
mod clip;
mod auth_view;
mod calling_ui;
mod calls;
mod chat_list;
mod chat_view;
mod control;
mod compose;
mod folders;
mod groups;
mod identities;
mod audit;
mod group_settings;
mod topics;
mod topic_art;
mod message_tools;
mod detail_panel;
mod media_viewer;
mod picker;
mod prefs;
mod recording;
mod rich_text;
mod rich_messages;
mod poll_media;
mod secrets;
mod service;
mod settings_dialog;
mod theme;
mod window;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

fn main() -> glib::ExitCode {
    nyx_core::app::init(nyx_core::app::App { slug: "nyx-pulse", name: "Pulse" });

    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|s| s == "--help" || s == "-h") {
        println!("Usage: nyx-pulse [--help] [--version]");
        println!("Native Telegram desktop client for Omarchy Linux and the Nyx Suite");
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|s| s == "--version" || s == "-V") {
        println!("nyx-pulse 0.1.0");
        return glib::ExitCode::SUCCESS;
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nyx_pulse=info,nyx_telegram_core=info".into()),
        )
        .init();

    // Developer aid: a different id gives a separate instance (a test copy on a virtual
    // monitor), so it doesn't hand its arguments to the copy that's open.
    let id = std::env::var("NYX_PULSE_APP_ID").unwrap_or_else(|_| "io.github.design_nexus.nyx.Pulse".to_string());
    let app = gtk::Application::builder()
        .application_id(id)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|app| {
        nyx_core::control::serve_async(app, control::tools(), control::handle_async);
    });

    app.connect_shutdown(|_| {crate::conversation_windows::reset(); crate::group_calls::stop(); crate::locations::reset(); crate::webapps::reset(); });

    app.connect_activate(|app| {
        window::present(app);
    });

    app.connect_command_line(|app, cmdline| {
        let args = cmdline.arguments();
        let mut target_chat: Option<i64> = None;
        let mut i = 0;
        while i < args.len() {
            let s = args[i].to_string_lossy();
            if s == "--chat" && i + 1 < args.len() {
                if let Ok(id) = args[i + 1].to_string_lossy().parse::<i64>() {
                    target_chat = Some(id);
                }
                i += 1;
            } else if s.starts_with("nyx-pulse://") || s.starts_with("tg://") || s.starts_with("https://t.me/") {
                let uri = s.to_string();
                glib::idle_add_local_once(move || window::resolve_link(&uri));
            }
            i += 1;
        }

        app.activate();

        if let Some(chat_id) = target_chat {
            glib::idle_add_local_once(move || {
                window::open_chat(chat_id);
            });
        }

        glib::ExitCode::SUCCESS
    });

    let open_chat = gio::SimpleAction::new("open-chat", Some(&<(String, i64)>::static_variant_type()));
    open_chat.connect_activate(|_, p| {
        if let Some((account, id)) = p.and_then(|v| v.get::<(String, i64)>()) {
            if prefs::get().active_account_id != account {
                window::switch_account(account);
            }
            window::open_chat(id);
        }
    });
    app.add_action(&open_chat);
    let present_call = gio::SimpleAction::new("present-call", Some(&String::static_variant_type()));
    let weak = app.downgrade();
    present_call.connect_activate(move |_, p| {
        if let Some(account) = p.and_then(|v| v.get::<String>()) {
            if account != prefs::get().active_account_id {
                window::switch_account(account);
            }
            if let Some(app) = weak.upgrade() {
                app.activate();
            }
        }
    });
    app.add_action(&present_call);
    app.connect_open(|app, files, _| {
        app.activate();
        for file in files {
            window::resolve_link(file.uri().as_str());
        }
    });
    app.run()
}
