//! Emoji, Sticker, and GIF picker popover for Nixe Pulse message composer.
//!
//! Complies with STYLE.md: flat @theme_surface card, tabbed navigation,
//! and clean hover effects.

use gtk::prelude::*;
use nixe_ui::widgets as w;
use std::cell::RefCell;
use std::rc::Rc;

pub struct PickerPopover {
    pub popover: gtk::Popover,
    on_emoji: RefCell<Option<Box<dyn Fn(&str)>>>,
    on_sticker: RefCell<Option<Box<dyn Fn(&str, &str)>>>,
    on_gif: RefCell<Option<Box<dyn Fn(&str, &str)>>>,
}

impl PickerPopover {
    pub fn new(parent: &gtk::Button) -> Rc<Self> {
        let popover = gtk::Popover::new();
        popover.set_parent(parent);
        popover.add_css_class("picker-popover");
        popover.set_position(gtk::PositionType::Top);

        let root_box = w::vbox(8);
        root_box.set_size_request(340, 360);
        root_box.set_margin_top(8);
        root_box.set_margin_bottom(8);
        root_box.set_margin_start(8);
        root_box.set_margin_end(8);

        // Tab Switcher Bar (Emoji, Stickers, GIFs)
        let tab_bar = w::hbox(4);
        tab_bar.add_css_class("picker-tab-bar");
        tab_bar.set_halign(gtk::Align::Center);

        let emoji_tab_btn = gtk::Button::with_label("😃 Emoji");
        emoji_tab_btn.add_css_class("flat");
        emoji_tab_btn.add_css_class("active");

        let sticker_tab_btn = gtk::Button::with_label("🐱 Stickers");
        sticker_tab_btn.add_css_class("flat");

        let gif_tab_btn = gtk::Button::with_label("🎬 GIFs");
        gif_tab_btn.add_css_class("flat");

        tab_bar.append(&emoji_tab_btn);
        tab_bar.append(&sticker_tab_btn);
        tab_bar.append(&gif_tab_btn);
        root_box.append(&tab_bar);

        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);

        let picker = Rc::new(Self {
            popover: popover.clone(),
            on_emoji: RefCell::new(None),
            on_sticker: RefCell::new(None),
            on_gif: RefCell::new(None),
        });

        // 1. Emoji View
        let emoji_page = build_emoji_page(picker.clone());
        stack.add_named(&emoji_page, Some("emoji"));

        // 2. Sticker View
        let sticker_page = build_sticker_page(picker.clone());
        stack.add_named(&sticker_page, Some("stickers"));

        // 3. GIF View
        let gif_page = build_gif_page(picker.clone());
        stack.add_named(&gif_page, Some("gifs"));

        root_box.append(&stack);
        popover.set_child(Some(&root_box));

        // Connect Tab Switching
        let s_emoji = stack.clone();
        let b_emoji = emoji_tab_btn.clone();
        let b_stick = sticker_tab_btn.clone();
        let b_gif = gif_tab_btn.clone();
        emoji_tab_btn.connect_clicked(move |_| {
            s_emoji.set_visible_child_name("emoji");
            b_emoji.add_css_class("active");
            b_stick.remove_css_class("active");
            b_gif.remove_css_class("active");
        });

        let s_stick = stack.clone();
        let b_emoji2 = emoji_tab_btn.clone();
        let b_stick2 = sticker_tab_btn.clone();
        let b_gif2 = gif_tab_btn.clone();
        sticker_tab_btn.connect_clicked(move |_| {
            s_stick.set_visible_child_name("stickers");
            b_stick2.add_css_class("active");
            b_emoji2.remove_css_class("active");
            b_gif2.remove_css_class("active");
        });

        let s_gif = stack.clone();
        let b_emoji3 = emoji_tab_btn.clone();
        let b_stick3 = sticker_tab_btn.clone();
        let b_gif3 = gif_tab_btn.clone();
        gif_tab_btn.connect_clicked(move |_| {
            s_gif.set_visible_child_name("gifs");
            b_gif3.add_css_class("active");
            b_emoji3.remove_css_class("active");
            b_stick3.remove_css_class("active");
        });

        picker
    }

    pub fn connect_emoji<F: Fn(&str) + 'static>(&self, callback: F) {
        *self.on_emoji.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_sticker<F: Fn(&str, &str) + 'static>(&self, callback: F) {
        *self.on_sticker.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_gif<F: Fn(&str, &str) + 'static>(&self, callback: F) {
        *self.on_gif.borrow_mut() = Some(Box::new(callback));
    }

    pub fn popup(&self) {
        self.popover.popup();
    }
}

fn build_emoji_page(picker: Rc<PickerPopover>) -> gtk::Widget {
    let container = w::vbox(8);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .build();

    let grid_box = w::vbox(12);
    grid_box.set_margin_start(4);
    grid_box.set_margin_end(4);

    let categories: &[(&str, &[&str])] = &[
        (
            "SMILEYS & EMOTIONS",
            &[
                "😀", "😃", "😄", "😁", "😆", "🥹", "😅", "😂", "🤣", "😊", "😇", "🙂", "🙃", "😉", "😌", "😍", "🥰", "😘", "😋",
                "😛", "😜", "🤪", "😝", "🤗", "🤭", "🤫", "🤔", "🫡", "🤐", "🤨", "😐", "😑", "😶", "🫥", "😏", "😒", "🙄", "😬",
                "🤥", "😔", "😪", "🤤", "😴", "😷", "🤒", "🤕", "🤢", "🤮", "🤧", "🥵", "🥶", "🥴", "😵", "🤯", "🤠", "🥳", "🥸",
                "😎", "🤓", "🧐",
            ],
        ),
        (
            "GESTURES & HEARTS",
            &[
                "👍", "👎", "👏", "🙌", "🤝", "✌️", "🤞", "🤟", "🤘", "🤙", "👈", "👉", "👆", "👇", "☝️", "✋", "🤚", "🖐️", "🖖",
                "🫲", "🫱", "🫳", "🫴", "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣️", "💕", "💞", "💓", "💗",
                "💖", "💘", "💝", "🔥", "✨", "💥", "💯",
            ],
        ),
        (
            "NATURE & ANIMALS",
            &[
                "🐱", "🐶", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵", "🐔", "🐧", "🐦", "🦆",
                "🦅", "🦉", "🦇", "🐺", "🐗", "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞", "🐜", "🌲", "🌳", "🌴", "🌵", "🌾", "🌿",
                "🍀", "🍁", "🍂", "🍃", "🍄", "🌰", "🦀", "🦞", "🦐", "🐙",
            ],
        ),
    ];

    for (cat_name, emojis) in categories {
        let cat_label = w::label(cat_name, "picker-cat-title");
        cat_label.set_xalign(0.0);
        grid_box.append(&cat_label);

        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_max_children_per_line(8);
        flow.set_min_children_per_line(8);
        flow.set_homogeneous(true);
        flow.set_row_spacing(4);
        flow.set_column_spacing(4);

        for &emoji in *emojis {
            let btn = gtk::Button::with_label(emoji);
            btn.add_css_class("flat");
            btn.add_css_class("emoji-btn");

            let p_click = Rc::downgrade(&picker);
            let e_str = emoji.to_string();
            btn.connect_clicked(move |_| {
                if let Some(p_click) = p_click.upgrade() {
                    if let Some(ref cb) = *p_click.on_emoji.borrow() {
                        cb(&e_str);
                    }
                }
            });
            flow.append(&btn);
        }
        grid_box.append(&flow);
    }

    scroll.set_child(Some(&grid_box));
    container.append(&scroll);
    container.upcast()
}

fn build_sticker_page(picker: Rc<PickerPopover>) -> gtk::Widget {
    remote_page(picker, false)
}
fn build_gif_page(picker: Rc<PickerPopover>) -> gtk::Widget {
    remote_page(picker, true)
}
fn remote_page(picker: Rc<PickerPopover>, gif: bool) -> gtk::Widget {
    let body = w::vbox(8);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(if gif { "Search GIFs on Telegram" } else { "Search stickers by emoji" }));
    body.append(&search);
    let results = w::vbox(4);
    let sc = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&results)
        .build();
    body.append(&sc);
    let picker = Rc::downgrade(&picker);
    let generation = Rc::new(std::cell::Cell::new(0u64));
    let fetch: Rc<dyn Fn(String)> = Rc::new(move |query| {
        let Some(service) = crate::window::service() else { return };
        let serial = generation.get() + 1;
        generation.set(serial);
        let generation = generation.clone();
        let weak_results = results.downgrade();
        let picker = picker.clone();
        let chat = crate::window::active_chat_id().unwrap_or(0);
        let account = service.account.clone();
        gtk::glib::spawn_future_local(async move {
            let items = if gif {
                gif_results(&service, chat, &query).await
            } else {
                let (method, args) = if query.is_empty() {
                    ("getRecentStickers", serde_json::json!({"is_attached":false}))
                } else {
                    (
                        "searchStickers",
                        serde_json::json!({"sticker_type":{"@type":"stickerTypeRegular"},"emojis":query,"query":"","input_language_codes":[],"offset":0,"limit":50}),
                    )
                };
                match service.request(method, args).recv().await {
                    Ok(Ok(data)) => Ok(data["stickers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|s| (s["emoji"].as_str().unwrap_or("Sticker").to_owned(), s["sticker"]["id"].to_string()))
                        .collect::<Vec<_>>()),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err("Service stopped".into()),
                }
            };
            if generation.get() != serial || crate::prefs::get().active_account_id != account {
                return;
            }
            let Some(results) = weak_results.upgrade() else { return };
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            match items {
                Ok(items) => {
                    if items.is_empty() {
                        results.append(&w::label("No items found", "dim"));
                    }
                    for (name, token) in items {
                        let button = gtk::Button::with_label(&name);
                        let picker = picker.clone();
                        button.connect_clicked(move |_| {
                            if let Some(p) = picker.upgrade() {
                                if gif {
                                    if let Some(cb) = p.on_gif.borrow().as_ref() {
                                        cb(&name, &token);
                                    }
                                } else {
                                    if let Some(cb) = p.on_sticker.borrow().as_ref() {
                                        cb(&token, &name);
                                    }
                                }
                                p.popover.popdown();
                            }
                        });
                        results.append(&button);
                    }
                }
                Err(e) => {
                    let label = w::label(&e, "dim");
                    label.set_wrap(true);
                    results.append(&label);
                }
            }
        });
    });
    let f = fetch.clone();
    search.connect_search_changed(move |e| f(e.text().to_string()));
    body.connect_map(move |_| fetch(String::new()));
    body.upcast()
}
async fn gif_results(service: &crate::service::TelegramService, chat: i64, query: &str) -> Result<Vec<(String, String)>, String> {
    async fn request(
        service: &crate::service::TelegramService,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        service.request(method, args).recv().await.map_err(|_| "Service stopped".to_owned())?
    }
    if query.is_empty() {
        let saved = request(service, "getSavedAnimations", serde_json::json!({})).await?;
        let items = saved["animations"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| (s["file_name"].as_str().unwrap_or("Saved GIF").to_owned(), s["animation"]["id"].to_string()))
            .collect::<Vec<_>>();
        if !items.is_empty() {
            return Ok(items);
        }
    }
    let option = request(service, "getOption", serde_json::json!({"name":"animation_search_bot_username"})).await?;
    let username = option["value"].as_str().filter(|s| !s.is_empty()).ok_or("GIF search is unavailable")?;
    let bot = request(service, "searchPublicChat", serde_json::json!({"username":username})).await?;
    let user = bot["type"]["user_id"].as_i64().ok_or("GIF search provider was not found")?;
    let data = request(
        service,
        "getInlineQueryResults",
        serde_json::json!({"bot_user_id":user,"chat_id":chat,"query":query,"offset":""}),
    )
    .await?;
    Ok(data["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["@type"] == "inlineQueryResultAnimation")
        .map(|r| {
            (
                r["title"].as_str().filter(|s| !s.is_empty()).unwrap_or("GIF").to_owned(),
                serde_json::json!({"query_id":data["inline_query_id"],"result_id":r["id"]}).to_string(),
            )
        })
        .collect())
}
impl Drop for PickerPopover {
    fn drop(&mut self) {
        if self.popover.parent().is_some() {
            self.popover.unparent();
        }
    }
}
