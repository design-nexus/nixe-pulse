//! Poll interaction and contact/location message cards and composition.
use crate::{
    actions::{self, Scope},
    groups::{entry, note, scroll, submit},
    window,
};
use gtk::{glib, prelude::*};
use nyx_telegram_core::{client::parse_message, models::MessageSummary};
use nyx_ui::widgets as w;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

fn text(value: &str) -> Value {
    json!({"@type":"formattedText","text":value,"entities":[]})
}
async fn send(scope: &Scope, chat: i64, content: Value, reply: Value, topic: Value) -> Result<(), String> {
    let keyboard = window::chat(chat).and_then(|c| c.data["reply_markup_message_id"].as_i64()).unwrap_or(0);
    let result=crate::groups::commit(scope,"sendMessage",json!({"chat_id":chat,"input_message_content":content,"reply_to":reply,"topic_id":topic,"options":{"@type":"messageSendOptions","paid_message_star_count":0}})).await?;
    if keyboard != 0 {
        crate::bots::consume_keyboard(chat, keyboard);
    }
    if let Some(message) = parse_message(&result) {
        window::display_message(&message);
    }
    Ok(())
}
fn build_poll(
    question: &str,
    options: &[String],
    quiz: bool,
    correct: usize,
    anonymous: bool,
    multiple: bool,
    explanation: Value,
    period: i32,
) -> Result<Value, String> {
    if question.trim().is_empty() {
        return Err("Enter a question.".into());
    }
    let options = options.iter().map(|s| s.trim()).collect::<Vec<_>>();
    if options.len() < 2 || options.iter().any(|s| s.is_empty()) {
        return Err("Add at least two nonempty answers, and remove unused answers.".into());
    }
    if quiz && correct >= options.len() {
        return Err("Choose the correct answer.".into());
    }
    if options.iter().collect::<HashSet<_>>().len() != options.len() {
        return Err("Answers must be different.".into());
    }
    let kind = if quiz {
        json!({"@type":"inputPollTypeQuiz","correct_option_ids":[correct],"explanation":explanation,"explanation_media":null})
    } else {
        json!({"@type":"inputPollTypeRegular","allow_adding_options":false})
    };
    Ok(
        json!({"@type":"inputMessagePoll","question":text(question.trim()),"options":options.into_iter().map(|s|json!({"@type":"inputPollOption","text":text(s),"media":null})).collect::<Vec<_>>(),"description":null,"media":null,"is_anonymous":anonymous,"allows_multiple_answers":multiple&&!quiz,"allows_revoting":!quiz,"members_only":false,"country_codes":[],"shuffle_options":false,"hide_results_until_closes":false,"type":kind,"open_period":period,"close_date":0,"is_closed":false}),
    )
}
pub fn compose_poll(chat: i64) {
    let reply = window::composer_reply(chat);
    let topic = window::composer_topic(chat);
    let Some(scope) = Scope::current() else { return };
    if window::chat(chat).is_some_and(|c| c.is_secret) {
        scope.error("Polls cannot be sent to secret chats.");
        return;
    }
    let (dialog, body) = w::dialog("Send poll", 480);
    let fields = scroll(&body);
    let question = entry(&fields, "Question", "", 255);
    let quiz = gtk::CheckButton::with_label("Quiz");
    fields.append(&quiz);
    let anonymous = gtk::CheckButton::with_label("Anonymous voting");
    anonymous.set_active(true);
    let channel = window::chat(chat).is_some_and(|c| c.chat_type == nyx_telegram_core::models::ChatType::Channel);
    anonymous.set_sensitive(!channel);
    fields.append(&anonymous);
    let multiple = gtk::CheckButton::with_label("Allow multiple answers");
    fields.append(&multiple);
    let options_box = w::vbox(4);
    fields.append(&options_box);
    let max_options = Rc::new(Cell::new(10usize));
    let limit = max_options.clone();
    let weak = dialog.downgrade();
    let limits_scope = scope.clone();
    glib::spawn_future_local(async move {
        if let Ok(value) = limits_scope.call("getOption", json!({"name":"poll_answer_count_max"})).await {
            if limits_scope.valid() && weak.upgrade().is_some_and(|d| d.is_visible()) {
                if let Some(max) = value["value"].as_u64().filter(|n| *n > 0 && *n <= 1024) {
                    limit.set(max as usize);
                }
            }
        }
    });
    let entries: Rc<RefCell<Vec<(gtk::Entry, gtk::CheckButton, gtk::Box, Rc<RefCell<Option<crate::poll_media::Attachment>>>)>>> =
        Default::default();
    let media = crate::poll_media::picker(&fields, &dialog, &scope, "Poll attachment (optional)", false);
    let add: Rc<dyn Fn()> = Rc::new({
        let options_box = options_box.clone();
        let entries = entries.clone();
        let quiz = quiz.clone();
        let dialog = dialog.downgrade();
        let scope = scope.clone();
        let max_options = max_options.clone();
        move || {
            if entries.borrow().len() >= max_options.get() {
                return;
            }
            let Some(dialog) = dialog.upgrade() else { return };
            let outer = w::vbox(4);
            let row = w::hbox(4);
            outer.append(&row);
            let correct = gtk::CheckButton::new();
            correct.set_tooltip_text(Some("Correct quiz answer"));
            correct.set_visible(quiz.is_active());
            let weak = correct.downgrade();
            quiz.connect_toggled(move |q| {
                if let Some(c) = weak.upgrade() {
                    c.set_visible(q.is_active());
                }
            });
            row.append(&correct);
            let e = gtk::Entry::new();
            e.set_placeholder_text(Some("Answer"));
            e.set_max_length(100);
            e.set_hexpand(true);
            row.append(&e);
            let remove = w::bar_button("list-remove-symbolic", "Remove answer");
            row.append(&remove);
            let media = crate::poll_media::picker(&outer, &dialog, &scope, "Answer attachment (optional)", true);
            entries.borrow_mut().push((e, correct, outer.clone(), media));
            options_box.append(&outer);
            let (entries, options_box, weak) = (Rc::downgrade(&entries), options_box.downgrade(), outer.downgrade());
            remove.connect_clicked(move |_| {
                if let (Some(row), Some(entries), Some(options_box)) = (weak.upgrade(), entries.upgrade(), options_box.upgrade())
                {
                    entries.borrow_mut().retain(|(_, _, r, _)| r != &row);
                    options_box.remove(&row);
                }
            });
        }
    });
    add();
    add();
    let b = gtk::Button::with_label("Add answer");
    let a = add.clone();
    b.connect_clicked(move |_| a());
    fields.append(&b);
    note(&fields, "For a quiz, select every correct answer. More than one enables multiple-answer voting.");
    let explanation = entry(&fields, "Quiz explanation (optional)", "", 200);
    let explanation_media = crate::poll_media::picker(&fields, &dialog, &scope, "Quiz explanation attachment (optional)", false);
    let description = entry(&fields, "Description (optional)", "", 4096);
    let revote = gtk::CheckButton::with_label("Allow changing votes");
    revote.set_active(true);
    fields.append(&revote);
    let adding = gtk::CheckButton::with_label("Let voters add answers (public regular polls)");
    fields.append(&adding);
    let members = gtk::CheckButton::with_label("Only channel members of over one day can vote");
    members.set_sensitive(channel);
    fields.append(&members);
    let countries = entry(&fields, "Allowed country codes (channels; optional, e.g. US, CA)", "", 256);
    countries.set_sensitive(channel);
    let shuffle = gtk::CheckButton::with_label("Shuffle answers");
    fields.append(&shuffle);
    let hide = gtk::CheckButton::with_label("Hide results until poll closes");
    fields.append(&hide);
    let expiry = gtk::SpinButton::with_range(0.0, 10080.0, 1.0);
    note(&fields, "Close after minutes (0 means no automatic close)");
    fields.append(&expiry);
    let a = adding.clone();
    let q = quiz.clone();
    anonymous.connect_toggled(move |anonymous| {
        let available = !channel && !anonymous.is_active() && !q.is_active();
        a.set_sensitive(available);
        if !available {
            a.set_active(false);
        }
    });
    adding.set_sensitive(false);
    let (a, anonymous_, revote_) = (adding.clone(), anonymous.clone(), revote.clone());
    quiz.connect_toggled(move |quiz| {
        let available = !channel && !anonymous_.is_active() && !quiz.is_active();
        a.set_sensitive(available);
        if !available {
            a.set_active(false);
        }
        revote_.set_sensitive(!quiz.is_active());
    });
    let m = multiple.clone();
    quiz.connect_toggled(move |q| {
        m.set_sensitive(!q.is_active());
        if q.is_active() {
            m.set_active(false);
        }
    });
    let markdown = gtk::CheckButton::with_label("Format poll text as Markdown (including custom emoji links)");
    fields.append(&markdown);
    submit(&dialog, &body, &fields, scope, "Send poll", move |scope| {
        let entries = entries.borrow();
        let options = entries.iter().map(|(e, _, _, _)| e.text().to_string()).collect::<Vec<_>>();
        let correct_ids =
            entries.iter().enumerate().filter(|(_, (_, c, _, _))| c.is_active()).map(|(i, _)| i).collect::<Vec<_>>();
        let correct = correct_ids.first().copied().unwrap_or(usize::MAX);
        let content = build_poll(
            &question.text(),
            &options,
            quiz.is_active(),
            correct,
            anonymous.is_active(),
            multiple.is_active(),
            text(&explanation.text()),
            expiry.value_as_int() * 60,
        )
        .and_then(|mut p| {
            if adding.is_active() && (quiz.is_active() || anonymous.is_active() || channel) {
                return Err("Adding answers requires a public regular poll outside channels.".into());
            }
            p["description"] = text(&description.text());
            p["media"] = media.borrow().as_ref().map(|m| m.input()).transpose()?.unwrap_or(Value::Null);
            for (i, (_, _, _, media)) in entries.iter().enumerate() {
                p["options"][i]["media"] = media.borrow().as_ref().map(|m| m.input()).transpose()?.unwrap_or(Value::Null);
            }
            if quiz.is_active() {
                p["type"]["correct_option_ids"] = json!(correct_ids);
                p["type"]["explanation_media"] =
                    explanation_media.borrow().as_ref().map(|m| m.input()).transpose()?.unwrap_or(Value::Null);
                p["allows_multiple_answers"] = json!(correct_ids.len() > 1);
            }
            p["country_codes"] = json!(country_codes(&countries.text())?);
            p["allows_revoting"] = json!(revote.is_active() && !quiz.is_active());
            p["members_only"] = json!(members.is_active());
            p["shuffle_options"] = json!(shuffle.is_active());
            p["hide_results_until_closes"] = json!(hide.is_active());
            if !quiz.is_active() {
                p["type"]["allow_adding_options"] = json!(adding.is_active());
            }
            Ok(p)
        });
        {
            let reply = reply.clone();
            let markdown = markdown.is_active();
            let topic = topic.clone();
            async move {
                let mut content = content?;
                if markdown {
                    for key in ["question", "description"] {
                        let text = content[key]["text"].as_str().unwrap_or("");
                        content[key] = scope
                            .call(
                                "parseTextEntities",
                                json!({"text":text,"parse_mode":{"@type":"textParseModeMarkdown","version":2}}),
                            )
                            .await?;
                    }
                    if let Some(options) = content["options"].as_array_mut() {
                        for option in options {
                            let text = option["text"]["text"].as_str().unwrap_or("");
                            option["text"] = scope
                                .call(
                                    "parseTextEntities",
                                    json!({"text":text,"parse_mode":{"@type":"textParseModeMarkdown","version":2}}),
                                )
                                .await?;
                        }
                    }
                    if content["type"]["@type"] == "inputPollTypeQuiz" {
                        let text = content["type"]["explanation"]["text"].as_str().unwrap_or("");
                        content["type"]["explanation"] = scope
                            .call(
                                "parseTextEntities",
                                json!({"text":text,"parse_mode":{"@type":"textParseModeMarkdown","version":2}}),
                            )
                            .await?;
                    }
                }
                send(&scope, chat, content, reply, topic).await
            }
        }
    });
    dialog.present();
}
pub fn compose_contact(chat: i64) {
    let reply = window::composer_reply(chat);
    let topic = window::composer_topic(chat);
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Share contact", 440);
    let fields = w::vbox(8);
    body.append(&fields);
    let first = entry(&fields, "First name", "", 255);
    let last = entry(&fields, "Last name", "", 255);
    let phone = entry(&fields, "Phone number (include country code)", "", 64);
    let user = Rc::new(Cell::new(0i64));
    let pick = gtk::Button::with_label("Choose contact…");
    fields.append(&pick);
    let (f, l, p, u) = (first.clone(), last.clone(), phone.clone(), user.clone());
    let s = scope.clone();
    let weak = dialog.downgrade();
    pick.connect_clicked(move |_| {
        if !s.valid() {
            return;
        }
        let (f, l, p, u, s, weak) = (f.clone(), l.clone(), p.clone(), u.clone(), s.clone(), weak.clone());
        crate::groups::choose_user("Choose contact", move |contact| {
            if !s.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                return;
            }
            f.set_text(contact["first_name"].as_str().unwrap_or(""));
            l.set_text(contact["last_name"].as_str().unwrap_or(""));
            p.set_text(contact["phone_number"].as_str().unwrap_or(""));
            u.set(contact["id"].as_i64().unwrap_or(0));
        });
    });
    // A manually edited contact must not retain another person's user identifier.
    for e in [&first, &last, &phone] {
        let u = user.clone();
        e.connect_changed(move |_| u.set(0));
    }
    submit(&dialog, &body, &fields, scope, "Share contact", move |s| {
        let reply = reply.clone();
        let topic = topic.clone();
        let (first, last, phone, user) =
            (first.text().trim().to_string(), last.text().trim().to_string(), phone.text().trim().to_string(), user.get());
        async move {
            if first.is_empty() || phone.chars().filter(char::is_ascii_digit).count() < 3 {
                return Err("Enter a first name and valid phone number.".into());
            }
            send(&s,chat,json!({"@type":"inputMessageContact","contact":{"@type":"contact","first_name":first,"last_name":last,"phone_number":phone,"vcard":"","user_id":user}}),reply,topic).await
        }
    });
    dialog.present();
}
fn coordinates(latitude: &str, longitude: &str) -> Result<(f64, f64), String> {
    let lat = latitude.trim().parse::<f64>().map_err(|_| "Enter a latitude between −90 and 90.")?;
    let lon = longitude.trim().parse::<f64>().map_err(|_| "Enter a longitude between −180 and 180.")?;
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err("Coordinates are outside the valid range.".into());
    }
    Ok((lat, lon))
}
pub fn compose_location(chat: i64) {
    let reply = window::composer_reply(chat);
    let topic = window::composer_topic(chat);
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Share location", 440);
    let fields = w::vbox(8);
    body.append(&fields);
    note(
        &fields,
        "Enter coordinates from a map. Enable automatic updates for sent live locations from Tools → Live locations. GeoClue and location permission are required.",
    );
    let live = gtk::CheckButton::with_label("Share live location");
    fields.append(&live);
    note(&fields, "Live duration (minutes)");
    let duration = gtk::SpinButton::with_range(1.0, 1440.0, 1.0);
    duration.set_value(15.0);
    fields.append(&duration);
    let lat = entry(&fields, "Latitude", "", 32);
    let lon = entry(&fields, "Longitude", "", 32);
    let device_status = note(&fields, "");
    let device = gtk::Button::with_label("Use device location");
    fields.append(&device);
    let (s, l, o, st) = (scope.clone(), lat.clone(), lon.clone(), device_status.clone());
    device.connect_clicked(move |_| {
        if s.valid() {
            crate::locations::fill(s.clone(), l.clone(), o.clone(), st.clone());
        }
    });
    submit(&dialog, &body, &fields, scope, "Share location", move |s| {
        let reply = reply.clone();
        let topic = topic.clone();
        let point = coordinates(&lat.text(), &lon.text());
        let period = if live.is_active() { duration.value_as_int() * 60 } else { 0 };
        async move {
            let (latitude, longitude) = point?;
            send(&s, chat, location_content(latitude, longitude, period), reply, topic).await
        }
    });
    dialog.present();
}
fn location_content(latitude: f64, longitude: f64, period: i32) -> Value {
    let point = json!({"@type":"location","latitude":latitude,"longitude":longitude,"horizontal_accuracy":0.0});
    if period == 0 {
        json!({"@type":"inputMessageLocation","location":point})
    } else {
        json!({"@type":"inputMessageLiveLocation","location":{"@type":"liveLocation","location":point,"live_period":period,"heading":0,"proximity_alert_radius":0}})
    }
}
async fn editable_location(s: &Scope, chat: i64, id: i64) -> Result<Value, String> {
    if s.call("getMessageProperties", json!({"chat_id":chat,"message_id":id})).await?["can_be_edited"] != true {
        return Err("Telegram does not permit editing this location.".into());
    }
    let message = s.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
    if message["content"]["@type"] != "messageLiveLocation" || message["content"]["expires_in"].as_i64().unwrap_or(0) <= 0 {
        return Err("Live location sharing has ended.".into());
    }
    Ok(message["content"]["location"].clone())
}
fn edit_location(chat: i64, id: i64) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Update live location", 440);
    let fields = w::vbox(8);
    body.append(&fields);
    note(&fields, "Enter your current position. This does not extend sharing time.");
    let lat = entry(&fields, "Latitude", "", 32);
    let lon = entry(&fields, "Longitude", "", 32);
    submit(&dialog, &body, &fields, scope, "Update position", move |s| {
        let point = coordinates(&lat.text(), &lon.text());
        async move {
            let (latitude, longitude) = point?;
            let mut live = editable_location(&s, chat, id).await?;
            live["location"] = json!({"@type":"location","latitude":latitude,"longitude":longitude,"horizontal_accuracy":0.0});
            live["heading"] = json!(0);
            crate::groups::commit(
                &s,
                "editMessageLiveLocation",
                json!({"chat_id":chat,"message_id":id,"reply_markup":null,"location":live}),
            )
            .await?;
            reload(&s, chat, id).await
        }
    });
    dialog.present();
}
pub fn render(body: &gtk::Box, message: &MessageSummary) -> bool {
    let content = &message.data["content"];
    match content["@type"].as_str().unwrap_or("") {
        "messagePoll" => {
            poll(body, message);
            true
        }
        "messageContact" => {
            let contact = &content["contact"];
            note(
                body,
                &format!("{} {}", contact["first_name"].as_str().unwrap_or(""), contact["last_name"].as_str().unwrap_or("")),
            );
            note(body, contact["phone_number"].as_str().unwrap_or(""));
            if let Some(user) = contact["user_id"].as_i64().filter(|v| *v != 0) {
                let b = gtk::Button::with_label("Open conversation");
                b.connect_clicked(move |_| {
                    actions::run(0, "contact-open", move |s| async move {
                        let chat = s.call("createPrivateChat", json!({"user_id":user,"force":false})).await?;
                        if let Some(id) = chat["id"].as_i64() {
                            window::open_chat(id);
                        }
                        Ok(())
                    })
                });
                body.append(&b);
            }
            true
        }
        "messageLocation" | "messageVenue" | "messageLiveLocation" => {
            let venue = &content["venue"];
            let point = if content["@type"] == "messageVenue" {
                &venue["location"]
            } else if content["@type"] == "messageLiveLocation" {
                &content["location"]["location"]
            } else {
                &content["location"]
            };
            if let (Some(lat), Some(lon)) = (point["latitude"].as_f64(), point["longitude"].as_f64()) {
                if content["@type"] == "messageVenue" {
                    note(body, venue["title"].as_str().unwrap_or("Place"));
                    note(body, venue["address"].as_str().unwrap_or(""));
                }
                note(body, &format!("{lat:.6}, {lon:.6}"));
                if coordinates(&lat.to_string(), &lon.to_string()).is_ok() {
                    let url = format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=16/{lat}/{lon}");
                    let link = gtk::LinkButton::with_label(&url, "Open map");
                    body.append(&link);
                }
            }
            if content["@type"] == "messageLiveLocation" {
                note(body, if content["expires_in"].as_i64().unwrap_or(0) > 0 { "Live location" } else { "Live location ended" });
                if message.is_outgoing && content["expires_in"].as_i64().unwrap_or(0) > 0 {
                    let (chat, id) = (message.chat_id, message.id);
                    let update = gtk::Button::with_label("Update position…");
                    update.connect_clicked(move |_| edit_location(chat, id));
                    body.append(&update);
                    let stop = gtk::Button::with_label("Stop sharing…");
                    stop.connect_clicked(move |_| {
                        actions::operation(
                            "Stop sharing location?",
                            "Your last position will remain visible in the chat.",
                            vec![],
                            "Stop sharing",
                            move |s, _| async move {
                                editable_location(&s, chat, id).await?;
                                crate::groups::commit(
                                    &s,
                                    "editMessageLiveLocation",
                                    json!({"chat_id":chat,"message_id":id,"reply_markup":null,"location":null}),
                                )
                                .await?;
                                reload(&s, chat, id).await
                            },
                        )
                    });
                    body.append(&stop);
                }
            }
            true
        }
        _ => false,
    }
}
fn poll(body: &gtk::Box, message: &MessageSummary) {
    let poll = &message.data["content"]["poll"];
    let chat = message.chat_id;
    let id = message.id;
    note(body, poll["question"]["text"].as_str().or(poll["question"].as_str()).unwrap_or("Poll"));
    note(
        body,
        &format!(
            "{} · {}{} · {} votes",
            if poll["type"]["@type"] == "pollTypeQuiz" { "Quiz" } else { "Poll" },
            if poll["is_anonymous"] == true { "Anonymous" } else { "Public" },
            if poll["is_closed"] == true { " · Closed" } else { "" },
            poll["total_voter_count"]
        ),
    );
    let description = message.data["content"]["description"]["text"].as_str().unwrap_or("");
    if !description.is_empty() {
        note(body, description);
    }
    if !poll["vote_restriction_reason"].is_null() {
        let reason = match poll["vote_restriction_reason"]["@type"].as_str().unwrap_or("") {
            "pollVoteRestrictionReasonMembershipRequired" => "Join the chat to vote.",
            "pollVoteRestrictionReasonCountryRestricted" => "Voting is unavailable in your country.",
            "pollVoteRestrictionReasonClosed" => "This poll has closed.",
            "pollVoteRestrictionReasonScheduled" | "pollVoteRestrictionReasonYetUnsent" => {
                "Voting opens after this message is sent."
            }
            _ => "Telegram currently restricts voting in this poll.",
        };
        note(body, reason);
    }
    crate::poll_media::render(body, message, &message.data["content"]["media"]);
    let options = poll["options"].as_array().cloned().unwrap_or_default();
    let mut order = poll["option_order"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|i| i as usize)
        .filter(|i| *i < options.len())
        .collect::<Vec<_>>();
    if order.len() != options.len() || order.iter().collect::<HashSet<_>>().len() != options.len() {
        order = (0..options.len()).collect();
    }
    let selected = Rc::new(RefCell::new(Vec::<(usize, gtk::CheckButton)>::new()));
    let group = gtk::CheckButton::new();
    let can_vote = poll["is_closed"] != true
        && poll["vote_restriction_reason"].is_null()
        && (poll["allows_revoting"] == true || !options.iter().any(|o| o["is_chosen"] == true));
    for i in order {
        let option = &options[i];
        let title = option["text"]["text"].as_str().or(option["text"].as_str()).unwrap_or("Answer");
        let correct = poll["can_see_results"] == true
            && poll["type"]["correct_option_ids"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id.as_u64() == Some(i as u64)));
        let title = if correct { format!("✓ {title}") } else { title.to_string() };
        let title = if poll["can_see_results"] == true || poll["is_closed"] == true {
            format!("{title} · {}% ({})", option["vote_percentage"], option["voter_count"])
        } else {
            title.to_string()
        };
        let c = gtk::CheckButton::with_label(&title);
        if poll["allows_multiple_answers"] != true {
            c.set_group(Some(&group));
        }
        c.set_active(option["is_chosen"] == true);
        c.set_sensitive(can_vote);
        if let Some(label) = c.child().and_downcast::<gtk::Label>() {
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_max_width_chars(32);
        }
        body.append(&c);
        selected.borrow_mut().push((i, c));
        crate::poll_media::render(body, message, &option["media"]);
        if let Some(option_id) = option["id"].as_str().filter(|s| !s.is_empty()) {
            let button = gtk::Button::with_label("Answer actions…");
            let (message, option_id) = (message.clone(), option_id.to_string());
            button.connect_clicked(move |_| answer_actions(message.clone(), option_id.clone()));
            body.append(&button);
        }
        if poll["can_get_voters"] == true {
            let b = gtk::Button::with_label("View voters");
            let original = options.clone();
            b.connect_clicked(move |_| voters(chat, id, i, original.clone()));
            body.append(&b);
        }
    }
    if can_vote {
        let b = gtk::Button::with_label("Submit vote");
        let original = options.clone();
        b.connect_clicked(move |_| {
            let options = selected.borrow().iter().filter(|(_, c)| c.is_active()).map(|(i, _)| *i).collect::<Vec<_>>();
            if options.is_empty() {
                window::show_status("Select an answer first.");
                return;
            }
            let original = original.clone();
            actions::run(chat, "poll-vote", move |s| async move {
                let fresh = s.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
                if fresh["content"]["poll"]["is_closed"] == true {
                    return Err("This poll has closed.".into());
                }
                let options = remap_answers(&original, &fresh["content"]["poll"]["options"], &options)?;
                s.call("setPollAnswer", json!({"chat_id":chat,"message_id":id,"option_ids":options})).await?;
                reload(&s, chat, id).await
            });
        });
        body.append(&b);
    }
    if poll["allows_revoting"] == true && poll["is_closed"] != true && options.iter().any(|o| o["is_chosen"] == true) {
        let b = gtk::Button::with_label("Retract vote");
        b.connect_clicked(move |_| {
            actions::run(chat, "poll-vote", move |s| async move {
                s.call("setPollAnswer", json!({"chat_id":chat,"message_id":id,"option_ids":[]})).await?;
                reload(&s, chat, id).await
            })
        });
        body.append(&b);
    }
    if message.data["content"]["can_add_option"] == true {
        let b = gtk::Button::with_label("Add answer…");
        b.connect_clicked(move |_| add_poll_option(chat, id));
        body.append(&b);
    }
    let explanation = poll["type"]["explanation"]["text"].as_str().unwrap_or("");
    if !explanation.is_empty() && poll["can_see_results"] == true {
        note(body, explanation);
    }
    if poll["can_see_results"] == true {
        crate::poll_media::render(body, message, &poll["type"]["explanation_media"]);
    }
    if poll["is_closed"] != true && message.is_outgoing {
        let b = gtk::Button::with_label("Stop poll…");
        b.connect_clicked(move |_| stop(chat, id));
        body.append(&b);
    }
}
fn country_codes(input: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for code in input.split(|c: char| c == ',' || c.is_whitespace()).filter(|c| !c.is_empty()) {
        let code = code.to_ascii_uppercase();
        if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err("Use two-letter country codes, separated by commas.".into());
        }
        if !result.contains(&code) {
            result.push(code);
        }
    }
    Ok(result)
}
fn answer_actions(message: MessageSummary, option: String) {
    let (chat, id) = (message.chat_id, message.id);
    actions::run(chat, "poll-answer-actions", move |scope| async move {
        let properties =
            scope.call("getPollOptionProperties", json!({"chat_id":chat,"message_id":id,"poll_option_id":option})).await?;
        let (dialog, body) = w::dialog("Poll answer", 400);
        let fresh = scope.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
        let answer = fresh["content"]["poll"]["options"]
            .as_array()
            .and_then(|options| options.iter().find(|o| o["id"] == option))
            .ok_or("This answer was removed.")?;
        note(&body, answer["text"]["text"].as_str().unwrap_or("Answer"));
        if properties["can_be_replied"] == true {
            let button = gtk::Button::with_label("Reply to answer…");
            let (scope, option, weak) = (scope.clone(), option.clone(), dialog.downgrade());
            button.connect_clicked(move |_| {
                if scope.valid() {
                    reply_to_answer(scope.clone(), chat, id, option.clone());
                    if let Some(d) = weak.upgrade() {
                        d.close();
                    }
                }
            });
            body.append(&button);
        }
        if properties["can_get_link"] == true {
            let button = gtk::Button::with_label("Copy answer link");
            let (scope, option) = (scope.clone(), option.clone());
            button.connect_clicked(move |_| {if !scope.valid(){return} let option=option.clone(); actions::run(chat,"poll-answer-link",move |s|async move {
                if s.call("getPollOptionProperties",json!({"chat_id":chat,"message_id":id,"poll_option_id":option})).await?["can_get_link"]!=true {return Err("This answer no longer has a link.".into());}
                let result=s.call("getMessageLink",json!({"chat_id":chat,"message_id":id,"media_timestamp":0,"checklist_task_id":0,"poll_option_id":option,"for_album":false,"in_message_thread":false})).await?;
                if let (Some(display),Some(link))=(gtk::gdk::Display::default(),result["link"].as_str()) {display.clipboard().set_text(link);}
                Ok(())
            });});
            body.append(&button);
        }
        if properties["can_be_deleted"] == true {
            let button = gtk::Button::with_label("Delete answer…");
            let (scope, option, weak) = (scope.clone(), option.clone(), dialog.downgrade());
            button.connect_clicked(move |_| {
                if !scope.valid() {
                    return;
                }
                let option = option.clone();
                let weak = weak.clone();
                actions::operation(
                    "Delete poll answer?",
                    "This removes the answer and its votes for everyone.",
                    vec![],
                    "Delete answer",
                    move |s, _| {
                        let option = option.clone();
                        let weak = weak.clone();
                        async move {
                            if s.call("getPollOptionProperties", json!({"chat_id":chat,"message_id":id,"poll_option_id":option}))
                                .await?["can_be_deleted"]
                                != true
                            {
                                return Err("Telegram no longer permits deleting this answer.".into());
                            }
                            crate::groups::commit(
                                &s,
                                "deletePollOption",
                                json!({"chat_id":chat,"message_id":id,"option_id":option}),
                            )
                            .await?;
                            if let Some(d) = weak.upgrade() {
                                d.close();
                            }
                            reload(&s, chat, id).await
                        }
                    },
                );
            });
            body.append(&button);
        }
        if properties["can_get_link"] != true && properties["can_be_deleted"] != true && properties["can_be_replied"] != true {
            note(&body, "No actions are available for this answer.");
        }
        dialog.present();
        Ok(())
    });
}
fn reply_to_answer(scope: Scope, chat: i64, id: i64, option: String) {
    let (dialog, body) = w::dialog("Reply to poll answer", 440);
    let fields = w::vbox(8);
    body.append(&fields);
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(100)
        .max_content_height(240)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build();
    fields.append(&scroll);
    let format = gtk::Button::with_label("Format reply");
    let editor = view.clone();
    format.connect_clicked(move |b| crate::compose::menu(b.upcast_ref(), &editor));
    fields.append(&format);
    let mut destinations = vec![(chat, "This conversation".to_string())];
    destinations.extend(window::list_chats(10000).into_iter().filter(|c| c.id != chat).map(|c| (c.id, c.title)));
    let names = destinations.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>();
    note(&fields, "Reply destination");
    let destination = gtk::DropDown::from_strings(&names);
    fields.append(&destination);
    submit(&dialog, &body, &fields, scope, "Send reply", move |s| {
        let text = crate::compose::formatted(&view.buffer());
        let option = option.clone();
        let destination = destinations.get(destination.selected() as usize).map(|v| v.0).unwrap_or(chat);
        async move {
            if text["text"].as_str().unwrap_or("").trim().is_empty() {
                return Err("Enter a reply.".into());
            }
            let props =
                s.call("getPollOptionProperties", json!({"chat_id":chat,"message_id":id,"poll_option_id":option})).await?;
            if props[if destination == chat { "can_be_replied" } else { "can_be_replied_in_another_chat" }] != true {
                return Err("Telegram no longer permits replying to this answer.".into());
            }
            let fresh = s.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
            let reply = json!({"@type":if destination==chat{"inputMessageReplyToMessage"}else{"inputMessageReplyToExternalMessage"},"chat_id":chat,"message_id":id,"quote":null,"checklist_task_id":0,"poll_option_id":option});
            send(
                &s,
                destination,
                json!({"@type":"inputMessageText","text":text,"link_preview_options":null,"clear_draft":false}),
                reply,
                if destination == chat { fresh["topic_id"].clone() } else { Value::Null },
            )
            .await
        }
    });
    dialog.present();
}
fn remap_answers(original: &[Value], fresh: &Value, chosen: &[usize]) -> Result<Vec<usize>, String> {
    let current = fresh.as_array().ok_or("This poll has changed. Reopen it before voting.")?;
    chosen
        .iter()
        .map(|i| {
            let option = original.get(*i).ok_or("This answer is no longer available.")?;
            let id = option["id"].as_str().filter(|s| !s.is_empty());
            if let Some(id) = id {
                current.iter().position(|o| o["id"] == id).ok_or("An answer was removed. Review this poll before voting.".into())
            } else if current == original {
                Ok(*i)
            } else {
                Err("This poll has changed. Review it before voting.".into())
            }
        })
        .collect()
}
fn add_poll_option(chat: i64, id: i64) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Add poll answer", 440);
    let fields = w::vbox(8);
    body.append(&fields);
    let answer = entry(&fields, "Answer", "", 100);
    let media = crate::poll_media::picker(&fields, &dialog, &scope, "Answer attachment (optional)", true);
    submit(&dialog, &body, &fields, scope, "Add answer", move |s| {
        let answer = answer.text().trim().to_string();
        let media = media.borrow().as_ref().map(|m| m.input()).transpose();
        async move {
            if answer.is_empty() {
                return Err("Enter an answer.".into());
            }
            let fresh = s.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
            if fresh["content"]["can_add_option"] != true {
                return Err("Telegram no longer permits adding answers.".into());
            }
            crate::groups::commit(
                &s,
                "addPollOption",
                json!({"chat_id":chat,"message_id":id,"option":{"@type":"inputPollOption","text":text(&answer),"media":media?.unwrap_or(Value::Null)}}),
            )
            .await?;
            reload(&s, chat, id).await
        }
    });
    dialog.present();
}
async fn reload(scope: &Scope, chat: i64, id: i64) -> Result<(), String> {
    let data = scope.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
    if let Some(m) = parse_message(&data) {
        window::display_message(&m);
    }
    Ok(())
}
pub(crate) fn stop(chat: i64, id: i64) {
    actions::operation(
        "Stop poll?",
        "Voting will close for everyone. This cannot be undone.",
        vec![],
        "Stop poll",
        move |s, _| async move {
            if s.call("getMessageProperties", json!({"chat_id":chat,"message_id":id})).await?["can_be_edited"] != true {
                return Err("Telegram does not permit stopping this poll.".into());
            }
            s.call("stopPoll", json!({"chat_id":chat,"message_id":id,"reply_markup":null})).await?;
            reload(&s, chat, id).await
        },
    );
}
fn voters(chat: i64, id: i64, option: usize, original: Vec<Value>) {
    let Some(scope) = Scope::current() else { return };
    let (dialog, body) = w::dialog("Poll voters", 420);
    let list = scroll(&body);
    let more = gtk::Button::with_label("Load more");
    body.append(&more);
    let offset = Rc::new(Cell::new(0));
    let seen: Rc<RefCell<HashSet<String>>> = Default::default();
    let weak = dialog.downgrade();
    more.connect_clicked(move |b| {
        if !scope.valid() {
            return;
        }
        b.set_sensitive(false);
        let (scope, list, b, offset, seen, weak) =
            (scope.clone(), list.clone(), b.clone(), offset.clone(), seen.clone(), weak.clone());
        let original = original.clone();
        glib::spawn_future_local(async move {
            let start = offset.get();
            let result = async {
                let current = scope.call("getMessage", json!({"chat_id":chat,"message_id":id})).await?;
                let mapped = remap_answers(&original, &current["content"]["poll"]["options"], &[option])?;
                scope
                    .call(
                        "getPollVoters",
                        json!({"chat_id":chat,"message_id":id,"option_id":mapped[0],"offset":start,"limit":50}),
                    )
                    .await
            }
            .await;
            if !scope.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                return;
            }
            b.set_sensitive(true);
            let data = match result {
                Ok(v) => v,
                Err(e) => {
                    note(&list, &e);
                    return;
                }
            };
            let rows = data["voters"].as_array().cloned().unwrap_or_default();
            offset.set(start + rows.len() as i32);
            for row in &rows {
                let voter = &row["voter_id"];
                if !seen.borrow_mut().insert(voter.to_string()) {
                    continue;
                }
                let title = if let Some(user) = voter["user_id"].as_i64() {
                    scope
                        .call("getUser", json!({"user_id":user}))
                        .await
                        .map(|u| format!("{} {}", u["first_name"].as_str().unwrap_or(""), u["last_name"].as_str().unwrap_or("")))
                        .unwrap_or_else(|_| format!("User {user}"))
                } else {
                    let chat = voter["chat_id"].as_i64().unwrap_or(0);
                    scope
                        .call("getChat", json!({"chat_id":chat}))
                        .await
                        .map(|c| c["title"].as_str().unwrap_or("Chat").to_string())
                        .unwrap_or_else(|_| format!("Chat {chat}"))
                };
                if !scope.valid() || weak.upgrade().is_none_or(|d| !d.is_visible()) {
                    return;
                }
                note(&list, &title);
            }
            b.set_visible(!rows.is_empty() && (offset.get() as i64) < data["total_count"].as_i64().unwrap_or(i64::MAX));
            if rows.is_empty() && start == 0 {
                note(&list, "No voters available.");
            }
        });
    });
    dialog.present();
    more.emit_clicked();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage3f_country_restrictions() {
        assert_eq!(country_codes("us, CA us").unwrap(), vec!["US", "CA"]);
        assert!(country_codes("USA").is_err());
        assert!(country_codes("1A").is_err());
    }
    #[test]
    fn stage3e_live_location_and_stable_poll_answers() {
        assert_eq!(location_content(1.0, 2.0, 900)["location"]["live_period"], 900);
        assert_eq!(location_content(1.0, 2.0, 0)["@type"], "inputMessageLocation");
        let original = vec![json!({"id":"a"}), json!({"id":"b"})];
        assert_eq!(remap_answers(&original, &json!([{"id":"b"},{"id":"a"}]), &[0]).unwrap(), vec![1]);
        assert!(remap_answers(&original, &json!([{"id":"b"}]), &[0]).is_err());
    }
    #[test]
    fn poll_payload_and_location_validation() {
        let options = vec!["Yes".into(), "No".into()];
        let poll = build_poll("Choose", &options, true, 1, false, true, text("Because"), 0).unwrap();
        assert_eq!(poll["options"][1]["text"]["text"], "No");
        assert_eq!(poll["type"]["correct_option_ids"], json!([1]));
        assert_eq!(poll["allows_multiple_answers"], false);
        assert!(build_poll("Choose", &options, true, 2, true, false, Value::Null, 0).is_err());
        assert!(build_poll("Choose", &["Yes".into(), " Yes ".into()], false, 0, true, false, Value::Null, 0).is_err());
        assert!(coordinates("NaN", "0").is_err());
        assert!(coordinates("91", "0").is_err());
        assert_eq!(coordinates("-90", "180").unwrap(), (-90.0, 180.0));
    }
}
