//! Main window assembly for Nixe Pulse, complying with STYLE.md.

use crate::auth_view::AuthView;
use crate::calling_ui::CallingUi;
use crate::chat_list::ChatList;
use crate::chat_view::ChatView;
use crate::detail_panel::DetailPanel;
use crate::media_viewer::MediaViewer;
use crate::service::{ServiceEvent, TelegramService};
use crate::{prefs, settings_dialog, theme};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use nixe_telegram_core::models::{AuthState, ChatSummary, MessageContent, TelegramUpdate};
use nixe_ui::chrome;
use nixe_ui::widgets as w;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static CONTEXT_SELECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static NAVIGATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static DELETING_CHATS: RefCell<std::collections::HashSet<(String, i64)>> = RefCell::new(Default::default());
    static REMOVED_CHATS: RefCell<std::collections::HashSet<i64>> = RefCell::new(Default::default());
    static LOGGING_OUT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
    static PENDING_CHAT:RefCell<Option<i64>>=const{RefCell::new(None)};
    static STATUS: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static EVENT_TX: RefCell<Option<async_channel::Sender<ServiceEvent>>> = const { RefCell::new(None) };
    static WINDOW: RefCell<Option<gtk::ApplicationWindow>> = const { RefCell::new(None) };
    static OVERLAY: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
    static CONVERSATION_LAYOUT: RefCell<Option<(gtk::Box, gtk::Overlay)>> = const { RefCell::new(None) };
    static NAV: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
    static SEARCH: RefCell<Option<gtk::SearchEntry>> = const { RefCell::new(None) };
    static CONTENT_STACK: RefCell<Option<gtk::Stack>> = const { RefCell::new(None) };
    static AUTH_VIEW: RefCell<Option<Rc<AuthView>>> = const { RefCell::new(None) };
    static CHAT_VIEW: RefCell<Option<Rc<ChatView>>> = const { RefCell::new(None) };
    static CHAT_LIST: RefCell<Option<Rc<ChatList>>> = const { RefCell::new(None) };
    static DETAIL_PANEL: RefCell<Option<Rc<DetailPanel>>> = const { RefCell::new(None) };
    static CALLING_UI: RefCell<Option<Rc<CallingUi>>> = const { RefCell::new(None) };
    static MEDIA_VIEWER: RefCell<Option<Rc<MediaViewer>>> = const { RefCell::new(None) };
    static SERVICE: RefCell<Option<TelegramService>> = const { RefCell::new(None) };
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn present(app: &gtk::Application) {
    if let Some(win) = WINDOW.with(|w| w.borrow().clone()) {
        win.present();
        return;
    }

    theme::install();
    crate::desktop_extras::apply();
    build_window(app);

    if let Some(out) = std::env::var_os("PULSE_SNAPSHOT") {
        snapshot_and_quit(app, std::path::PathBuf::from(out));
        return;
    }

    if let Some(win) = WINDOW.with(|w| w.borrow().clone()) {
        win.present();
    }
}

pub fn log_out() {
    LOGGING_OUT.with(|v| v.set(true));
    if let Some(s) = SERVICE.with(|s| s.borrow().clone()) {
        s.log_out();
    }
    if let Some(stack) = CONTENT_STACK.with(|s| s.borrow().clone()) {
        stack.set_visible_child_name("auth");
    }
    if let Some(av) = AUTH_VIEW.with(|a| a.borrow().clone()) {
        av.set_state(&AuthState::WaitPhoneNumber);
    }
}

pub fn add_account() {
    switch_account(nixe_telegram_core::account::AccountManager::create_new_account_id());
}

pub fn list_chats(limit: usize) -> Vec<ChatSummary> {
    CHAT_LIST
        .with(|cl| cl.borrow().as_ref().map(|list| list.get_all_chats().into_iter().take(limit).collect()).unwrap_or_default())
}

pub fn search_chats(query: &str) -> Vec<ChatSummary> {
    let q = query.to_lowercase();
    CHAT_LIST.with(|cl| {
        cl.borrow()
            .as_ref()
            .map(|list| {
                list.get_all_chats()
                    .into_iter()
                    .filter(|c| {
                        c.title.to_lowercase().contains(&q)
                            || c.last_message
                                .as_ref()
                                .map(|m| match &m.content {
                                    MessageContent::Text(t) => t.text.to_lowercase().contains(&q),
                                    _ => false,
                                })
                                .unwrap_or(false)
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

pub fn open_chat(chat_id: i64) {
    advance_navigation();
    REMOVED_CHATS.with(|r| {
        r.borrow_mut().remove(&chat_id);
    });
    if let Some(win) = WINDOW.with(|w| w.borrow().clone()) {
        win.present();
    }
    let list = CHAT_LIST.with(|v| v.borrow().clone());
    let Some(list) = list else {
        PENDING_CHAT.with(|p| *p.borrow_mut() = Some(chat_id));
        return;
    };
    if list.get_chat(chat_id).is_some() {
        list.select_chat(chat_id);
        CONTENT_STACK.with(|s| {
            if let Some(s) = s.borrow().as_ref() {
                s.set_visible_child_name("main");
            }
        });
    } else {
        PENDING_CHAT.with(|p| *p.borrow_mut() = Some(chat_id));
        with_service(|s| s.load_chat(chat_id));
    }
}

pub fn service() -> Option<TelegramService> {
    SERVICE.with(|s| s.borrow().clone())
}
pub fn with_service(f: impl FnOnce(&TelegramService)) {
    if let Some(s) = service() {
        f(&s);
    }
}
pub fn show_status(text: &str) {
    STATUS.with(|s| {
        if let Some(label) = s.borrow().as_ref() {
            label.set_text(text);
        }
    });
}
pub fn send_message_from_assistant(chat_id: i64, text: &str) -> bool {
    if let Some(s) = service() {
        s.send_text_message(chat_id, text.to_owned(), None);
        true
    } else {
        false
    }
}
pub fn start_call_from_assistant(chat_id: i64, is_video: bool) -> bool {
    let user = CHAT_LIST
        .with(|c| c.borrow().as_ref().and_then(|c| c.get_chat(chat_id)))
        .and_then(|c| c.data["type"]["user_id"].as_i64());
    if let (Some(s), Some(user)) = (service(), user) { s.start_call(user, is_video) } else { false }
}
pub fn request_delete_chat(id: i64) {
    let Some(service) = service() else {
        show_status("Connect to Telegram first");
        return;
    };
    let Some(list) = CHAT_LIST.with(|l| l.borrow().clone()) else { return };
    let Some(chat) = list.get_chat(id) else { return };
    let key = (service.generation.clone(), id);
    if !DELETING_CHATS.with(|pending| pending.borrow_mut().insert(key.clone())) {
        return;
    }
    let leave = chat.chat_type != nixe_telegram_core::models::ChatType::Private;
    let (dialog, card) = w::dialog(if leave { "Leave and delete chat?" } else { "Delete chat?" }, 420);
    let title = w::label(&chat.title, "");
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_max_width_chars(36);
    card.append(&title);
    let explanation = w::label(
        if leave {
            "You will leave this group or channel and remove its history from your chat list. Other members keep their messages."
        } else {
            "This removes the chat and its history only for you. This cannot be undone."
        },
        "dim",
    );
    explanation.set_wrap(true);
    explanation.set_max_width_chars(36);
    card.append(&explanation);
    let error = w::label("", "error");
    error.set_wrap(true);
    error.set_max_width_chars(36);
    card.append(&error);
    let actions = w::hbox(8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let confirm = gtk::Button::with_label(if leave { "Leave and delete" } else { "Delete chat" });
    confirm.add_css_class("destructive-action");
    actions.append(&cancel);
    actions.append(&confirm);
    card.append(&actions);
    let busy = Rc::new(std::cell::Cell::new(false));
    let b = busy.clone();
    dialog.connect_close_request(move |_| {
        if b.get() {
            return glib::Propagation::Stop;
        }
        DELETING_CHATS.with(|pending| {
            pending.borrow_mut().remove(&key);
        });
        glib::Propagation::Proceed
    });
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = weak.upgrade() {
            d.close();
        }
    });
    let weak = dialog.downgrade();
    let cancel_action = cancel.clone();
    confirm.connect_clicked(move |button| {
        let Some(dialog) = weak.upgrade() else { return };
        if !service_is_current(&service.generation) {
            dialog.close();
            return;
        }
        if busy.replace(true) {
            return;
        }
        button.set_sensitive(false);
        cancel_action.set_sensitive(false);
        error.set_text("");
        let rx = service.delete_chat(id, leave);
        let generation = service.generation.clone();
        let account = service.account.clone();
        let button = button.clone();
        let cancel = cancel_action.clone();
        let error = error.clone();
        let busy = busy.clone();
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("Telegram service stopped".into()));
            busy.set(false);
            if !service_is_current(&generation) {
                dialog.close();
                return;
            }
            match result {
                Ok(_) => {
                    REMOVED_CHATS.with(|r| {
                        r.borrow_mut().insert(id);
                    });
                    CHAT_LIST.with(|l| {
                        if let Some(l) = l.borrow().as_ref() {
                            l.remove_chat(id);
                        }
                    });
                    CHAT_VIEW.with(|v| {
                        if let Some(v) = v.borrow().as_ref() {
                            if *v.current_chat_id.borrow() == Some(id) {
                                v.reset_account();
                                DETAIL_PANEL.with(|p| {
                                    if let Some(p) = p.borrow().as_ref() {
                                        p.clear();
                                    }
                                });
                                MEDIA_VIEWER.with(|m| {
                                    if let Some(m) = m.borrow().as_ref() {
                                        m.hide();
                                    }
                                });
                            }
                        }
                    });
                    PENDING_CHAT.with(|p| {
                        if *p.borrow() == Some(id) {
                            p.borrow_mut().take();
                        }
                    });
                    prefs::update(|p| {
                        p.drafts.remove(&format!("{account}:{id}"));
                        if p.last_chat_id == Some(id) {
                            p.last_chat_id = None;
                        }
                    });
                    with_service(|s| {
                        s.request("closeChat", serde_json::json!({"chat_id":id}));
                    });
                    show_status(if leave { "Left and deleted chat" } else { "Chat deleted" });
                    dialog.close();
                }
                Err(e) => {
                    error.set_text(&e);
                    button.set_sensitive(true);
                    cancel.set_sensitive(true);
                }
            }
        });
    });
    dialog.present();
    cancel.grab_focus();
}

pub fn chat(id: i64) -> Option<ChatSummary> {
    CHAT_LIST.with(|l| l.borrow().as_ref().and_then(|l| l.get_chat(id)))
}
pub fn current_chat_list() -> nixe_telegram_core::models::ChatListId {
    CHAT_LIST.with(|l| l.borrow().as_ref().map(|l| l.current_list()).unwrap_or_default())
}
pub fn leave_conversation(id: i64) {
    CHAT_VIEW.with(|v| {
        if let Some(v) = v.borrow().as_ref() {
            if *v.current_chat_id.borrow() == Some(id) {
                v.reset_account();
            }
        }
    });
    CHAT_LIST.with(|l| {
        if let Some(l) = l.borrow().as_ref() {
            l.clear_selection();
        }
    });
    DETAIL_PANEL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.clear();
        }
    });
    prefs::update(|p| {
        if p.last_chat_id == Some(id) {
            p.last_chat_id = None;
        }
    });
    with_service(|s| {
        s.request("closeChat", serde_json::json!({"chat_id":id}));
    });
}
pub fn show_chat_info(id: i64) {
    open_chat(id);
    DETAIL_PANEL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.root.set_visible(true);
        }
    });
}
pub fn refresh_conversation(id: i64) {
    CHAT_VIEW.with(|v| {
        if let Some(v) = v.borrow().as_ref() {
            if *v.current_chat_id.borrow() == Some(id) {
                v.clear_messages();
                v.clear_reply();
                v.load_history(0);
            }
        }
    });
    with_service(|s| s.load_chat(id));
}
pub fn search_conversation(id: i64) {
    open_chat(id);
    CHAT_VIEW.with(|v| {
        if let Some(v) = v.borrow().as_ref() {
            v.search_messages();
        }
    });
}

pub fn service_is_current(generation: &str) -> bool {
    service().is_some_and(|s| s.generation == generation)
}

pub fn switch_account(id: String) {
    let tx = EVENT_TX.with(|t| t.borrow().clone());
    let Some(tx) = tx else { return };
    WINDOW.with(|main| {
        if let Some(app) = main.borrow().as_ref().and_then(|w| w.application()) {
            for dialog in app.windows().into_iter().filter(|w| w.is_modal()) {
                dialog.close();
            }
        }
    });
    crate::conversation_windows::reset();
    crate::webapps::reset();
    crate::group_calls::stop();
    crate::locations::reset();
    crate::stories::reset();
    crate::business::reset();
    crate::calls::stop();
    crate::rich_text::clear_cache();
    crate::folders::reset();
    crate::message_tools::reset();
    crate::actions::update_default_reaction(serde_json::Value::Null);
    REMOVED_CHATS.with(|r| r.borrow_mut().clear());
    DETAIL_PANEL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.clear();
        }
    });
    MEDIA_VIEWER.with(|v| {
        if let Some(v) = v.borrow().as_ref() {
            v.hide();
        }
    });
    if let Some(s) = service() {
        s.close();
    }
    SERVICE.with(|v| v.borrow_mut().take());
    CHAT_VIEW.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.reset_account();
        }
    });
    prefs::update(|p| p.active_account_id = id.clone());
    CHAT_LIST.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.reset();
        }
    });
    CHAT_VIEW.with(|c| {
        if let Some(c) = c.borrow().as_ref() {
            c.clear_messages();
            *c.current_chat_id.borrow_mut() = None;
        }
    });
    CONTENT_STACK.with(|s| {
        if let Some(s) = s.borrow().as_ref() {
            s.set_visible_child_name("auth");
        }
    });
    match TelegramService::start_account(tx, id) {
        Ok(s) => SERVICE.with(|v| *v.borrow_mut() = Some(s)),
        Err(e) => show_status(&e.to_string()),
    }
}

fn build_window(app: &gtk::Application) {
    let window =
        gtk::ApplicationWindow::builder().application(app).title("Pulse").default_width(1120).default_height(800).build();

    window.add_css_class("pulse-window");
    // No client-side titlebar: Hyprland manages windows
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some("io.github.design_nexus.nixe.Pulse"));

    let overlay = gtk::Overlay::new();
    let frame = w::vbox(0);
    frame.add_css_class("window-frame");

    let body = w::hbox(0);
    body.set_vexpand(true);

    // 1. Navigation Sidebar (.settings-navigation, 250px expanded / 52px compact)
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);

    let head = chrome::sidebar_head("Pulse", Some("Search chats (Ctrl+F)"));
    nav.append(&head.root);

    let search = search_entry();
    nav.append(&search);

    // The chat list is inside the sidebar scroller
    let chat_list = ChatList::new(&search);
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hexpand(false)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&chat_list.root)
        .build();
    nav.append(&nav_scroll);
    body.append(&nav);

    // 2. Main Content Area (Active Conversation View OR Authentication View)
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.add_css_class("settings-content");
    content.set_hexpand(true);
    content.set_vexpand(true);

    let content_stack = gtk::Stack::new();
    if std::env::var_os("PULSE_SNAPSHOT").is_none() {
        content_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        content_stack.set_transition_duration(160);
    } else {
        content_stack.set_transition_type(gtk::StackTransitionType::None);
    }
    content_stack.set_hexpand(true);
    content_stack.set_vexpand(true);

    // Page: Authentication
    let auth_view = AuthView::new();
    content_stack.add_named(&auth_view.root, Some("auth"));

    // Page: Active Conversation View + Collapsible Detail Panel
    let main_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    main_box.set_hexpand(true);
    main_box.set_vexpand(true);

    let chat_view = ChatView::new();
    main_box.append(&chat_view.root);

    let detail_panel = DetailPanel::new();
    main_box.append(&detail_panel.root);

    let conversation_overlay = gtk::Overlay::new();
    conversation_overlay.set_child(Some(&main_box));
    CONVERSATION_LAYOUT.with(|layout| *layout.borrow_mut() = Some((main_box, conversation_overlay.clone())));
    content_stack.add_named(&conversation_overlay, Some("main"));

    // Default to main conversation view (or auth view if login required)
    content_stack.set_visible_child_name("main");
    content.append(&content_stack);
    body.append(&content);
    frame.append(&body);

    // 3. Status Bar (34px)
    let (status_bar, status_readout) = status_bar(&overlay, &window);
    frame.append(&status_bar);

    overlay.set_child(Some(&frame));
    window.set_child(Some(&overlay));

    // Calling UI overlay (cockpit & incoming notification banner)
    let calling_ui = CallingUi::new();
    overlay.add_overlay(&calling_ui.root);
    overlay.set_measure_overlay(&calling_ui.root, false);

    // Media Viewer Lightbox overlay
    let media_viewer = MediaViewer::new();
    overlay.add_overlay(&media_viewer.root);
    overlay.set_measure_overlay(&media_viewer.root, false);

    // Store references in thread-local storage
    NAV.with(|n| *n.borrow_mut() = Some(nav.clone()));
    SEARCH.with(|s| *s.borrow_mut() = Some(search.clone()));
    CONTENT_STACK.with(|s| *s.borrow_mut() = Some(content_stack.clone()));
    AUTH_VIEW.with(|a| *a.borrow_mut() = Some(auth_view.clone()));
    CHAT_VIEW.with(|c| *c.borrow_mut() = Some(chat_view.clone()));
    CHAT_LIST.with(|l| *l.borrow_mut() = Some(chat_list.clone()));
    DETAIL_PANEL.with(|d| *d.borrow_mut() = Some(detail_panel.clone()));
    CALLING_UI.with(|c| *c.borrow_mut() = Some(calling_ui.clone()));
    MEDIA_VIEWER.with(|m| *m.borrow_mut() = Some(media_viewer.clone()));
    WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
    OVERLAY.with(|o| *o.borrow_mut() = Some(overlay.clone()));

    // Sidebar head toggle button
    head.toggle.connect_clicked(|_| toggle_sidebar());

    // Sidebar head search button
    if let Some(find) = &head.search {
        let s = search.clone();
        let n = nav.clone();
        find.connect_clicked(move |_| {
            open_search(&s, &n);
        });
    }

    // Connect conversation selection to view and detail panel
    let cv = chat_view.clone();
    let dp = detail_panel.clone();
    let cl_for_select = chat_list.clone();
    chat_list.connect_chat_selected(move |chat_id| {
        if CONTEXT_SELECT.with(|v|v.get()){return}
        let Some(chat) = cl_for_select.get_chat(chat_id) else { return };
        advance_navigation();
        let previous = *cv.current_chat_id.borrow();
        if previous == Some(chat_id) && cv.topic_id().is_null() {
            return;
        }
        if let Some(previous) = previous {
            with_service(|s| {
                if !crate::conversation_windows::is_open(previous){s.request("closeChat", serde_json::json!({"chat_id":previous}));}
            });
        }
        cv.set_chat(&chat);
        dp.set_chat(&chat);

        // Fetch real chat history from background service
        if let Some(s) = SERVICE.with(|srv| srv.borrow().clone()) {
            cv.load_history(0);
            s.request("openChat", serde_json::json!({"chat_id":chat_id}));
        }
    });

    chat_view.connect_start_call(|chat, video| {
        if !start_call_from_assistant(chat, video) {
            show_status("Calling is only available for a Telegram contact");
        }
    });
    calling_ui.connect_accept(|id| with_service(|s| s.accept_call(id)));
    calling_ui.connect_hangup(|id, duration, video| with_service(|s| s.discard_call(id, duration, video)));
    let dp = detail_panel.clone();
    chat_view.connect_toggle_details(move || dp.toggle_visible());
    let mv = media_viewer.clone();
    let weak = Rc::downgrade(&chat_view);
    chat_view.connect_open_media(move |message| {
        if let Some(cv) = weak.upgrade() {
            mv.set_gallery(cv.media_messages());
        }
        mv.show_message(message)
    });
    detail_panel.connect_start_secret_chat(|user| with_service(|s| s.create_secret_chat(user)));
    let mv = media_viewer.clone();
    detail_panel.connect_media(move |message| mv.show_message(message));

    // Start background Telegram service and async event bridge
    let (event_tx, event_rx) = async_channel::unbounded::<ServiceEvent>();
    EVENT_TX.with(|t| *t.borrow_mut() = Some(event_tx.clone()));
    STATUS.with(|t| *t.borrow_mut() = Some(status_readout.clone()));
    let service = match TelegramService::start(event_tx) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("TDLib background service initialization notice: {:?}", e);
            status_readout.set_text(&format!("Telegram unavailable: {e}"));
            None
        }
    };

    if let Some(ref s) = service {
        SERVICE.with(|v| *v.borrow_mut() = Some(s.clone()));
    }
    auth_view.connect_submit_phone(|phone| with_service(|s| s.submit_phone(phone)));
    auth_view.connect_submit_code(|code| with_service(|s| s.submit_code(code)));
    auth_view.connect_submit_password(|pwd| with_service(|s| s.submit_password(pwd)));
    auth_view.connect_request_qr(|| with_service(|s| s.request_qr()));
    chat_view.connect_typing(|chat| with_service(|s| {
        s.request("sendChatAction",serde_json::json!({"chat_id":chat,"topic_id":composer_topic(chat),"action":{"@type":"chatActionTyping"}}));
    }));
    let chat_limit = Rc::new(std::cell::Cell::new(50));
    let limit = chat_limit.clone();
    nav_scroll.connect_edge_reached(move |_, edge| {
        if edge == gtk::PositionType::Bottom {
            let n = (limit.get() + 50).min(10000);
            limit.set(n);
            with_service(|s| s.load_chats(n));
        }
    });
    let cv_read = chat_view.clone();
    let weak = window.downgrade();
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        let Some(win) = weak.upgrade() else { return glib::ControlFlow::Break };
        if win.is_active() {
            if let Some(chat) = *cv_read.current_chat_id.borrow() {
                with_service(|s| s.mark_chat_read(chat, cv_read.topic_id(), cv_read.visible_ids()));
            }
        }
        glib::ControlFlow::Continue
    });
    window.connect_close_request(|_| {
        crate::calls::stop();
        MEDIA_VIEWER.with(|m| {
            if let Some(m) = m.borrow().as_ref() {
                m.hide();
            }
        });
        nixe_playback::video::shutdown();
        with_service(|s| s.close());
        glib::Propagation::Proceed
    });

    // Process background events on the GTK main thread
    let av = auth_view.clone();
    let stack = content_stack.clone();
    let readout = status_readout.clone();
    let cv_events = chat_view.clone();
    let cl_events = chat_list.clone();
    let calling_ui_events = calling_ui.clone();

    let dp_events = detail_panel.clone();
    let app_events = app.clone();
    let win_events = window.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = event_rx.recv().await {
            let event = match event {
                ServiceEvent::Account(id, generation, event)
                    if id == prefs::get().active_account_id
                        && crate::window::service().is_some_and(|s| s.generation == generation) =>
                {
                    *event
                }
                ServiceEvent::Account(_, _, _) => continue,
                e => e,
            };
            crate::conversation_windows::event(&event);
            match event {
                ServiceEvent::DraftSynced { chat_id, topic, draft } => {
                    let key = crate::topics::draft_key(&prefs::get().active_account_id,chat_id,&topic);
                    prefs::update(|p| {
                        if let Some(local) = p.drafts.get_mut(&key) {
                            if local.to_td() == draft {
                                local.pending = false;
                                if local.empty() {
                                    p.drafts.remove(&key);
                                }
                            }
                        }
                    });
                }
                ServiceEvent::ContextReady { mut chat, topic, title, draft, navigation, jump } => {
                    if navigation != navigation_version() { continue; }
                    cl_events.upsert_chat(&chat);
                    CONTEXT_SELECT.with(|v|v.set(true));
                    cl_events.select_chat(chat.id);
                    CONTEXT_SELECT.with(|v|v.set(false));
                    let previous=*cv_events.current_chat_id.borrow();
                    if let Some(previous)=previous.filter(|id|*id!=chat.id){with_service(|s|{if !crate::conversation_windows::is_open(previous){s.request("closeChat",serde_json::json!({"chat_id":previous}));}});}
                    with_service(|s|{s.request("openChat",serde_json::json!({"chat_id":chat.id}));});
                    dp_events.set_chat(&chat);
                    if !topic.is_null(){chat.data["draft_message"] = draft;}
                    cv_events.set_context(&chat,topic,&title);
                    cv_events.context_jump(jump);
                    cv_events.load_history(jump.unwrap_or(0));
                    stack.set_visible_child_name("main");
                }
                ServiceEvent::TopicHistoryLoaded {chat_id,topic,revision,from,result} => cv_events.topic_history(chat_id,&topic,revision,from,result),
                ServiceEvent::SecretChatCreated(chat) => {
                    cl_events.upsert_chat(&chat);
                    cl_events.select_chat(chat.id);
                }
                ServiceEvent::ChatsLoaded(chats) => {
                    let pending = PENDING_CHAT.with(|p| p.borrow_mut().take());
                    let chats =
                        chats.into_iter().filter(|c| !REMOVED_CHATS.with(|r| r.borrow().contains(&c.id))).collect::<Vec<_>>();
                    cl_events.update_chats(&chats);
                    if let Some(id) = pending {
                        if cl_events.get_chat(id).is_some() {
                            cl_events.select_chat(id);
                        } else {
                            PENDING_CHAT.with(|p| *p.borrow_mut() = Some(id));
                            with_service(|s| s.load_chat(id));
                        }
                    }
                }
                ServiceEvent::Update(TelegramUpdate::AuthStateChanged(state)) => {
                    if matches!(state,AuthState::WaitRegistration){crate::auth_tools::registration();}
                    if matches!(state, AuthState::Closed) && LOGGING_OUT.with(|v| v.replace(false)) {
                        switch_account(prefs::get().active_account_id);
                        continue;
                    }
                    av.set_state(&state);
                    match state {
                        AuthState::Ready => {
                            sync_pending_drafts();
                            stack.set_visible_child_name("main");
                            readout.set_text("Connected to Telegram");
                        }
                        AuthState::LoggingOut
                        | AuthState::Closed
                        | AuthState::WaitPhoneNumber
                        | AuthState::WaitCode { .. }
                        | AuthState::WaitPassword { .. }
                        | AuthState::WaitOtherDeviceConfirmation { .. } => {
                            stack.set_visible_child_name("auth");
                            readout.set_text("Awaiting authentication");
                        }
                        _ => {}
                    }
                }
                ServiceEvent::Update(TelegramUpdate::NewMessage(msg)) => {
                    cv_events.bot_force_reply(&msg);
                    crate::message_tools::changed(msg.chat_id);
                    if msg.data["scheduling_state"].is_object() {
                        continue;
                    }
                    let removed = REMOVED_CHATS.with(|r| r.borrow_mut().remove(&msg.chat_id));
                    if removed {
                        with_service(|s| s.load_chat(msg.chat_id));
                    }
                    let new = cl_events.get_chat(msg.chat_id).and_then(|c| c.last_message).is_none_or(|m| m.id != msg.id);
                    if !msg.is_outgoing
                        && new
                        && (!win_events.is_active() || *cv_events.current_chat_id.borrow() != Some(msg.chat_id) || !crate::topics::matches_message(&cv_events.topic_id(),&msg.data))
                        && prefs::get().notifications_enabled
                    {
                        let chat = cl_events.get_chat(msg.chat_id);
                        if !chat.as_ref().is_some_and(|c| c.is_muted) {
                            let n = gtk::gio::Notification::new(
                                chat.as_ref().map(|c| c.title.as_str()).unwrap_or("New Telegram message"),
                            );
                            n.set_body(Some(&crate::chat_view::preview(&msg)));
                            n.set_default_action_and_target_value(
                                "app.open-chat",
                                Some(&(prefs::get().active_account_id, msg.chat_id).to_variant()),
                            );
                            app_events.send_notification(Some(&format!("chat-{}", msg.chat_id)), &n);
                            if prefs::get().sound_enabled {
                                let _ =
                                    std::process::Command::new("canberra-gtk-play").args(["-i", "message-new-instant"]).spawn();
                            }
                        }
                    }
                    cv_events.append_message(&msg);
                    if let Some(mut c) = cl_events.get_chat(msg.chat_id) {
                        c.last_message = Some(msg);
                        cl_events.upsert_chat(&c);
                    }
                }
                ServiceEvent::Update(TelegramUpdate::MessageReplaced { old_id, message }) => {
                    crate::message_tools::changed(message.chat_id);
                    cv_events.replace_message(old_id, &message)
                }
                ServiceEvent::Update(TelegramUpdate::MessagesDeleted { chat_id, message_ids }) => {
                    crate::message_tools::changed(chat_id);
                    if *cv_events.current_chat_id.borrow() == Some(chat_id) {
                        cv_events.delete_messages(&message_ids);
                    }
                }
                ServiceEvent::Update(TelegramUpdate::ChatUpdated(chat)) => {
                    if REMOVED_CHATS.with(|r| r.borrow().contains(&chat.id)) {
                        continue;
                    }
                    if chat.data["positions"].as_array().is_some_and(|positions| {
                        positions
                            .iter()
                            .all(|p| p["order"].as_str().unwrap_or("0") == "0" && p["order"].as_u64().unwrap_or(0) == 0)
                    }) && *cv_events.current_chat_id.borrow() != Some(chat.id)
                        && PENDING_CHAT.with(|p| *p.borrow()) != Some(chat.id)
                    {
                        cl_events.remove_chat(chat.id);
                        continue;
                    }
                    cl_events.upsert_chat(&chat);
                    if PENDING_CHAT.with(|p| *p.borrow()) == Some(chat.id) {
                        PENDING_CHAT.with(|p| p.borrow_mut().take());
                        cl_events.select_chat(chat.id);
                    }
                    if *cv_events.current_chat_id.borrow() == Some(chat.id) {
                        cv_events.update_chat(&chat);
                        dp_events.refresh(&chat);
                    }
                }
                ServiceEvent::Update(TelegramUpdate::ChatPhotoUpdated { chat_id, path }) => {
                    if let Some(mut c) = cl_events.get_chat(chat_id) {
                        c.avatar_path = Some(path);
                        cl_events.upsert_chat(&c);
                        if *cv_events.current_chat_id.borrow() == Some(chat_id) {
                            cv_events.update_avatar(&c);
                        }
                    }
                }
                ServiceEvent::Update(TelegramUpdate::UserStatusChanged { user_id, is_online }) => {
                    let chats = cl_events.get_all_chats();
                    for mut chat in chats.into_iter().filter(|c| c.data["type"]["user_id"].as_i64() == Some(user_id)) {
                        chat.data["pulse_user_online"] = serde_json::json!(is_online);
                        cl_events.upsert_chat(&chat);
                        if *cv_events.current_chat_id.borrow() == Some(chat.id) {
                            cv_events.update_chat(&chat);
                        }
                    }
                }
                ServiceEvent::Update(TelegramUpdate::ChatReadInbox { chat_id, unread_count, .. }) => {
                    if let Some(mut c) = cl_events.get_chat(chat_id) {
                        c.unread_count = unread_count;
                        cl_events.upsert_chat(&c);
                    }
                }
                ServiceEvent::Update(TelegramUpdate::UserTyping { chat_id, .. }) => {
                    if *cv_events.current_chat_id.borrow() == Some(chat_id) {
                        cv_events.set_typing(true);
                        let cv = cv_events.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                            if *cv.current_chat_id.borrow() == Some(chat_id) {
                                cv.set_typing(false);
                            }
                        });
                    }
                }
                ServiceEvent::Update(TelegramUpdate::Raw(v)) => {
                    crate::folders::update(&v);
                    crate::groups::update(&v);
                    crate::topics::update(&v);
                    crate::auth_tools::update(&v);
                    crate::stories::update(&v);
                    crate::business::update(&v);
                    crate::group_calls::update(&v);
                    crate::locations::update(&v);
                    crate::message_tools::update(&v);
                    if v["@type"] == "updateDefaultReactionType" {
                        crate::actions::update_default_reaction(v["reaction_type"].clone());
                    }
                    if v["@type"] == "updateScopeNotificationSettings" {
                        with_service(|s| s.load_chats(100));
                    }
                    if v["@type"] == "updateConnectionState" {
                        if v["state"]["@type"] == "connectionStateReady" {
                            sync_pending_drafts();
                        }
                        readout.set_text(match v["state"]["@type"].as_str() {
                            Some("connectionStateReady") => "Connected to Telegram",
                            Some("connectionStateConnecting") => "Connecting…",
                            Some("connectionStateUpdating") => "Syncing…",
                            _ => "Waiting for network…",
                        });
                    }
                    crate::calls::update(&v);
                }
                ServiceEvent::Update(TelegramUpdate::CallUpdated { call_id, user_id, is_outgoing, is_video, state, emojis }) => {
                    let name = cl_events.get_chat(user_id).map(|c| c.title).unwrap_or_else(|| format!("User {user_id}"));
                    if state.contains("Discarded") || state.contains("Error") {
                        calling_ui_events.hang_up();
                    } else if state.contains("Ready") {
                        calling_ui_events.show_active(call_id, user_id, &name, is_video, &emojis);
                    } else if !is_outgoing {
                        calling_ui_events.show_incoming(call_id, user_id, &name, is_video);
                    }
                }
                ServiceEvent::AuthError(e) => {
                    LOGGING_OUT.with(|v| v.set(false));
                    av.set_error(&e);
                    readout.set_text(&e);
                }
                ServiceEvent::Error(e) => {
                    readout.set_text(&e);
                    cv_events.set_error(&e);
                }
                _ => {}
            }
        }
    });

    // Responsive layout handling: compositor tiling and window sizing
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    probe.set_can_focus(false);
    overlay.add_overlay(&probe);
    overlay.set_measure_overlay(&probe, false);

    let w2 = window.clone();
    probe.connect_resize(move |_, _, _| {
        let w = w2.clone();
        glib::idle_add_local_once(move || relayout(&w));
    });
    window.connect_default_width_notify(relayout);
    window.connect_realize(relayout);

    let w3 = window.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
        relayout(&w3);
        glib::ControlFlow::Continue
    });

    // Keyboard shortcuts & drop target
    install_shortcuts(&window, &overlay, &search, &nav, &detail_panel, &chat_view, &chat_list);
}

fn search_entry() -> gtk::SearchEntry {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search chats"));
    search.add_css_class("sidebar-search");
    search.add_css_class("compact-hide");
    search.set_visible(false);
    search.connect_stop_search(|e| {
        e.set_text("");
        e.set_visible(false);
    });
    search
}

fn open_search(search: &gtk::SearchEntry, nav: &gtk::Box) {
    if nav.has_css_class("compact") {
        prefs::update(|p| p.sidebar_collapsed = false);
        if let Some(w) = WINDOW.with(|w| w.borrow().clone()) {
            relayout(&w);
        }
    }
    if search.is_visible() && search.text().is_empty() {
        search.set_visible(false);
    } else {
        search.set_visible(true);
        search.grab_focus();
    }
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    if let Some(w) = WINDOW.with(|w| w.borrow().clone()) {
        relayout(&w);
    }
}

fn relayout(w: &gtk::ApplicationWindow) {
    let Some(nav) = NAV.with(|n| n.borrow().clone()) else { return };
    let width = if w.width() > 0 { w.width() } else { w.default_width() };
    if width <= 0 {
        return;
    }

    let narrow = width < 980 && std::env::var_os("PULSE_DEBUG_WIDE").is_none();
    settings_dialog::fit(w);
    CONVERSATION_LAYOUT.with(|layout| {
        DETAIL_PANEL.with(|panel| {
            let layout = layout.borrow();
            let panel = panel.borrow();
            if let (Some((row, overlay)), Some(panel)) = (layout.as_ref(), panel.as_ref()) {
                if narrow && panel.root.parent().as_ref() == Some(row.upcast_ref()) {
                    row.remove(&panel.root);
                    panel.root.set_halign(gtk::Align::End);
                    panel.root.set_valign(gtk::Align::Fill);
                    overlay.add_overlay(&panel.root);
                    overlay.set_measure_overlay(&panel.root, false);
                } else if !narrow && panel.root.parent().as_ref() == Some(overlay.upcast_ref()) {
                    overlay.remove_overlay(&panel.root);
                    panel.root.set_halign(gtk::Align::Fill);
                    row.append(&panel.root);
                }
            }
        });
    });

    if narrow != NARROW.with(|n| n.get()) || !nav.has_css_class("sized") {
        nav.add_css_class("sized");
        NARROW.with(|n| n.set(narrow));
    }

    let compact = narrow || prefs::get().sidebar_collapsed;
    if compact != nav.has_css_class("compact") || !nav.has_css_class("laid-out") {
        nav.add_css_class("laid-out");
        apply_compact(&nav, compact);
    }
}

fn apply_compact(nav: &gtk::Box, compact: bool) {
    if let Some(head) = nav.first_child().and_downcast::<gtk::Box>() {
        chrome::set_head_compact(&head, compact);
        if let Some(search) = head.next_sibling().and_downcast::<gtk::SearchEntry>() {
            search.set_visible(!compact);
        }
    }
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }

    fn walk(w: &gtk::Widget, compact: bool) {
        if w.has_css_class("compact-hide") {
            w.set_visible(!compact);
        }
        if w.has_css_class("compact-show") {
            w.set_visible(compact);
        }
        if w.has_css_class("nav-item")
            && let Some(content) = w.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
            if let Some(bx) = content.downcast_ref::<gtk::Box>() {
                bx.set_spacing(if compact { 0 } else { 10 });
            }
        }
        // Do not recurse into chat-list-root: it manages its own items, headers, and empty state
        if w.has_css_class("chat-list-root") {
            return;
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, compact);
            child = c.next_sibling();
        }
    }
    walk(nav.upcast_ref(), compact);

    if let Some(chat_list) = CHAT_LIST.with(|l| l.borrow().clone()) {
        chat_list.set_compact(compact);
    }
}

fn status_bar(overlay: &gtk::Overlay, window: &gtk::ApplicationWindow) -> (gtk::Box, gtk::Label) {
    let bar = w::hbox(16);
    bar.add_css_class("status-bar");

    let shortcuts_btn = chrome::shortcuts_button(|| {
        show_shortcuts();
    });
    bar.append(&shortcuts_btn);

    let readout = w::label("Connecting…", "status-readout");
    readout.set_hexpand(true);
    readout.set_xalign(1.0);
    bar.append(&readout);

    let ov = overlay.clone();
    let win = window.clone();
    let gear_btn = chrome::gear(move || {
        settings_dialog::open(&ov, &win);
    });
    bar.append(&gear_btn);

    (bar, readout)
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+F", "Search chats"),
    ("Ctrl+Shift+F", "Search all messages"),
    ("Ctrl+B / I / U", "Format selected composer text"),
    ("Ctrl+B", "Collapse or expand sidebar"),
    ("Ctrl+,", "Open settings"),
    ("F1", "Show keyboard shortcuts"),
    ("Esc", "Close dialogs, panels, or search"),
    ("Ctrl+Q", "Quit application"),
    ("Enter", "Send message"),
    ("Shift+Enter", "Insert newline in message"),
    ("Delete", "Delete focused chat (with confirmation)"),
    ("Shift+F10", "Open focused chat menu"),
];

fn keys_widget(combo: &str) -> gtk::Box {
    let b = w::hbox(4);
    for (i, k) in combo.split('+').enumerate() {
        if i > 0 {
            b.append(&w::label("+", "dim"));
        }
        b.append(&w::label(k.trim(), "key-cap"));
    }
    b
}

pub fn show_shortcuts() {
    let (dialog, card) = w::dialog("Keyboard shortcuts", 520);
    let list = w::vbox(0);
    list.add_css_class("group-list");
    for (keys, what) in SHORTCUTS {
        list.append(&w::row(what, "", Some(keys_widget(keys).upcast_ref())));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&list)
        .build();
    card.append(&scroll);

    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);

    dialog.present();
}

fn install_shortcuts(
    window: &gtk::ApplicationWindow,
    _overlay: &gtk::Overlay,
    search: &gtk::SearchEntry,
    _nav: &gtk::Box,
    detail_panel: &Rc<DetailPanel>,
    chat_view: &Rc<ChatView>,
    _chat_list: &Rc<ChatList>,
) {
    let key_controller = gtk::EventControllerKey::new();

    let win = window.clone();
    let s = search.clone();
    let dp = detail_panel.clone();
    let cv_key = chat_view.clone();

    key_controller.connect_key_pressed(move |_, key, _, modifier| {
        let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
        let typing = gtk::prelude::GtkWindowExt::focus(&win).is_some_and(|f| {
            f.is::<gtk::Text>()
                || f.ancestor(gtk::Entry::static_type()).is_some()
                || f.is::<gtk::SearchEntry>()
                || f.is::<gtk::TextView>()
        });

        if settings_dialog::is_open() {
            match key {
                gdk::Key::Escape => {
                    settings_dialog::escape();
                    return glib::Propagation::Stop;
                }
                gdk::Key::f | gdk::Key::F if ctrl => {
                    settings_dialog::focus_search();
                    return glib::Propagation::Stop;
                }
                gdk::Key::q | gdk::Key::Q | gdk::Key::w | gdk::Key::W if ctrl => {
                    win.close();
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            }
        }

        if crate::desktop_extras::shortcut(key,modifier){return glib::Propagation::Stop;}

        if (key == gdk::Key::F1) || (!typing && key == gdk::Key::question) {
            show_shortcuts();
            return glib::Propagation::Stop;
        }

        if key == gdk::Key::Escape {
            if let Some(mv) = MEDIA_VIEWER.with(|m| m.borrow().clone()) {
                if mv.root.is_visible() {
                    mv.hide();
                    return glib::Propagation::Stop;
                }
            }
            if cv_key.selection_active() {
                cv_key.clear_selection();
                return glib::Propagation::Stop;
            }
            if s.is_visible() {
                s.set_text("");
                s.set_visible(false);
                return glib::Propagation::Stop;
            }
            if dp.root.is_visible() {
                dp.root.set_visible(false);
                return glib::Propagation::Stop;
            }
        }

        if ctrl && (key == gdk::Key::w || key == gdk::Key::W) {
            win.close();
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });

    window.add_controller(key_controller);

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let cv = chat_view.clone();
    drop.connect_drop(move |_, value, _, _| {
        let Ok(files) = value.get::<gdk::FileList>() else { return false };
        for file in files.files() {
            if let Some(path) = file.path() {
                cv.attach(path);
            }
        }
        true
    });
    window.add_controller(drop);
}

fn snapshot_and_quit(_app: &gtk::Application, _out: std::path::PathBuf) {
    show_status("Use an isolated test account for UI captures");
}

pub fn resolve_link(uri: &str) {
    if let Some(id) = uri
        .strip_prefix("nixe-pulse://chat/")
        .or_else(|| uri.strip_prefix("tg://pulse?chat_id="))
        .and_then(|s| s.parse::<i64>().ok())
    {
        open_chat(id);
        return;
    }

    let Some(s) = service() else { return };
    let session = s.generation.clone();
    let rx = s.request("getInternalLinkType", serde_json::json!({"link":uri}));
    glib::spawn_future_local(async move {
        if let Ok(Ok(v)) = rx.recv().await {
            if !service_is_current(&session) {
                return;
            }
            if v["@type"] == "internalLinkTypeChatInvite" {
                if let Some(link) = v["invite_link"].as_str() {
                    crate::groups::import_link(link.into());
                }
                return;
            }
            if v["@type"] == "internalLinkTypeChatFolderInvite" {
                if let Some(link) = v["invite_link"].as_str() {
                    crate::folders::import_link(link.into());
                }
                return;
            }
            if let Some(name) = v["username"].as_str() {
                if let Some(s) = service() {
                    let rx = s.request("searchPublicChat", serde_json::json!({"username":name}));
                    if let Ok(Ok(chat)) = rx.recv().await {
                        if !service_is_current(&session) {
                            return;
                        }
                        if let Some(id) = chat["id"].as_i64() {
                            open_chat(id);
                        }
                    }
                }
            } else if v["@type"] == "internalLinkTypeMessage" {
                let rx = s.request("getMessageLinkInfo", serde_json::json!({"url":v["url"]}));
                if let Ok(Ok(info)) = rx.recv().await {
                    if !service_is_current(&session) {
                        return;
                    }
                    crate::topics::open_link(info);
                }
            } else {
                show_status("This Telegram link is not supported");
            }
        }
    });
}

pub fn call_ui() -> Option<Rc<CallingUi>> {
    CALLING_UI.with(|v| v.borrow().clone())
}

pub fn composer_reply(chat: i64) -> serde_json::Value {
    if let Some(v)=crate::conversation_windows::focused(chat){return v.reply_target()}
    CHAT_VIEW.with(|v| v.borrow().as_ref().filter(|v| *v.current_chat_id.borrow() == Some(chat)).map(|v| v.reply_target()).unwrap_or(serde_json::Value::Null))
}

pub fn display_message(message: &nixe_telegram_core::models::MessageSummary) {
    CHAT_VIEW.with(|v| { if let Some(v) = v.borrow().as_ref() { v.append_message(message); } });
}

pub fn jump_message(message: &nixe_telegram_core::models::MessageSummary) {
    if active_chat_id()!=Some(message.chat_id)||!crate::topics::matches_message(&composer_topic(message.chat_id),&message.data){
        crate::topics::open_link(serde_json::json!({"chat_id":message.chat_id,"topic_id":message.data["topic_id"],"message":{"id":message.id}}));return;
    }
    CHAT_VIEW.with(|v| {if let Some(v)=v.borrow().as_ref(){v.append_message(message);v.jump_to(message.id);}});
}

pub fn active_chat_id() -> Option<i64> {
    CHAT_VIEW.with(|v| v.borrow().as_ref().and_then(|v| *v.current_chat_id.borrow()))
}

pub fn select_messages_for_report() {
    CHAT_VIEW.with(|v| {
        if let Some(v) = v.borrow().as_ref() {
            v.start_selection();
        }
    });
}

pub fn refresh_folders() {
    CHAT_LIST.with(|list| {
        if let Some(list) = list.borrow().as_ref() {
            list.set_folders(&crate::folders::infos());
        }
    });
}

fn sync_pending_drafts() {
    let Some(service) = service() else { return };
    for (key, mut draft) in prefs::get().drafts {
        let Some((chat, topic)) = crate::topics::parse_key(&service.account, &key) else { continue };
        if draft.pending || (draft.formatted.is_null() && !draft.empty()) {
            if draft.formatted.is_null() {
                draft.formatted = draft.formatted_text();
                draft.pending = true;
                draft.updated_at = chrono::Utc::now().timestamp();
                prefs::update(|p| {
                    p.drafts.insert(key, draft.clone());
                });
            }
            service.set_topic_draft(chat, topic, draft.to_td());
        }
    }
}

pub fn refresh_folder_labels() {
    CHAT_LIST.with(|list| {
        if let Some(list) = list.borrow().as_ref() {
            list.refresh_folder_labels(&crate::folders::infos());
        }
    });
}

pub fn discard_topic_draft(id: i64,topic:&serde_json::Value) {
    let key=crate::topics::draft_key(&prefs::get().active_account_id,id,topic);
    prefs::update(|p|{p.drafts.remove(&key);});
    CHAT_VIEW.with(|v|{if let Some(v)=v.borrow().as_ref(){if *v.current_chat_id.borrow()==Some(id)&&v.topic_id()==*topic{v.discard_draft();}}});
}

pub fn navigation_version()->u64 { NAVIGATION.with(|v|v.get()) }
fn advance_navigation()->u64 { NAVIGATION.with(|v|{v.set(v.get().wrapping_add(1));v.get()}) }
pub fn open_context(chat:i64, topic:serde_json::Value, title:String, draft:serde_json::Value) {
    open_context_at(chat,topic,title,draft,None);
}
pub fn open_context_at(chat:i64,topic:serde_json::Value,title:String,draft:serde_json::Value,jump:Option<i64>){
    let navigation=advance_navigation();with_service(|s|s.resolve_context(chat,topic,title,draft,navigation,jump));
}
pub fn composer_topic(chat:i64)->serde_json::Value {
    if let Some(v)=crate::conversation_windows::focused(chat){return v.topic_id()}
    CHAT_VIEW.with(|v|v.borrow().as_ref().filter(|v|*v.current_chat_id.borrow()==Some(chat)).map(|v|v.topic_id()).unwrap_or(serde_json::Value::Null))
}
pub fn remote_topic_draft(chat:i64, topic:&serde_json::Value, draft:&serde_json::Value){
    CHAT_VIEW.with(|v|{if let Some(v)=v.borrow().as_ref(){v.remote_topic_draft(chat,topic,draft);}});
}
pub fn topic_name(chat:i64,id:i64,name:&str){
    CHAT_VIEW.with(|v|{if let Some(v)=v.borrow().as_ref(){v.topic_name(chat,id,name);}});
}
pub fn topic_deleted(chat:i64,id:i64){
    let topic=serde_json::json!({"@type":"messageTopicForum","forum_topic_id":id});
    with_service(|s|s.cancel_topic_draft(chat,&topic));
    discard_topic_draft(chat,&topic);
    if composer_topic(chat)==topic{open_chat(chat);}
}

pub(crate) fn show_media(message: &nixe_telegram_core::models::MessageSummary) {
    MEDIA_VIEWER.with(|m| { if let Some(m) = m.borrow().as_ref() { m.show_message(message); } });
}

pub fn focus_chat_search(){SEARCH.with(|s|NAV.with(|n|{if let(Some(s),Some(n))=(s.borrow().as_ref(),n.borrow().as_ref()){open_search(s,n);}}));}
pub fn open_settings(){OVERLAY.with(|o|WINDOW.with(|w|{if let(Some(o),Some(w))=(o.borrow().as_ref(),w.borrow().as_ref()){settings_dialog::open(o,w);}}));}
