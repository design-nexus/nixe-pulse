//! Text sizing, contrast and configurable native keyboard shortcuts.
use crate::{
    features::{self, button, check},
    groups::note,
    window,
};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};
thread_local! {
static STYLE:RefCell<Option<gtk::CssProvider>>=const{
RefCell::new(None)}
;
}
pub fn apply() {
    let p = crate::prefs::get();
    if let Some(display) = gdk::Display::default() {
        STYLE.with(|s| {
            if let Some(old) = s.borrow_mut().take() {
                gtk::style_context_remove_provider_for_display(&display, &old);
            }
            let css = gtk::CssProvider::new();
            css.load_from_string(&format!(
                ".pulse-window {{font-size: {}pt;}} {}",
                11. * p.text_scale.clamp(0.75, 2.),
                if p.high_contrast {
                    ".pulse-window button, .pulse-window entry, .pulse-window textview {border: 1px solid currentColor;}"
                } else {
                    ""
                }
            ));
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            *s.borrow_mut() = Some(css);
        });
    }
}
const ACTIONS: &[(&str, &str, &str)] = &[
    ("search", "Search chats", "<Control>f"),
    ("messages", "Search all messages", "<Control><Shift>f"),
    ("settings", "Settings", "<Control>comma"),
    ("tools", "Tools", "<Control><Shift>p"),
    ("downloads", "Downloads", "<Control>j"),
    ("stories", "Stories", "<Control><Shift>s"),
    ("sidebar", "Toggle sidebar", "<Control>b"),
    ("quit", "Quit Pulse", "<Control>q"),
];
fn parsed(text: &str) -> Option<(gdk::Key, gdk::ModifierType)> {
    let (key, mods) = gtk::accelerator_parse(text)?;
    let mods = mods
        & (gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK);
    if mods.is_empty()
        && !key
            .name()
            .is_some_and(|s| s.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=35).contains(&n)))
    {
        return None;
    }
    Some((key.to_lower(), mods))
}
pub fn shortcut(key: gdk::Key, mods: gdk::ModifierType) -> bool {
    let prefs = crate::prefs::get();
    let mods = mods
        & (gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK);
    for (action, _, default) in ACTIONS {
        let configured = prefs.shortcuts.get(*action).map(String::as_str).unwrap_or(default);
        if parsed(configured) != Some((key.to_lower(), mods)) {
            continue;
        }
        match *action {
            "search" => window::focus_chat_search(),
            "messages" => crate::message_tools::search(None),
            "settings" => window::open_settings(),
            "tools" => crate::features::hub(),
            "downloads" => crate::desktop_tools::downloads(),
            "stories" => crate::stories::browse(),
            "sidebar" => window::toggle_sidebar(),
            "quit" => {
                if let Some(app) = gtk::gio::Application::default() {
                    app.quit();
                }
            }
            _ => {}
        }
        return true;
    }
    false
}
pub fn open() {
    features::load("Accessibility and shortcuts", |s, d, b| async move {
        let p = crate::prefs::get();
        note(&b, "Text size");
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.75, 2., 0.05);
        scale.set_value(p.text_scale);
        scale.set_digits(2);
        b.append(&scale);
        scale.connect_value_changed(|v| {
            crate::prefs::update(|p| p.text_scale = v.value());
            apply();
        });
        let contrast = check(&b, "Stronger control outlines", p.high_contrast);
        contrast.connect_toggled(|c| {
            crate::prefs::update(|p| p.high_contrast = c.is_active());
            apply();
        });
        note(&b, "Choose Record, then press a shortcut. Use a modifier or function key. Escape cancels recording.");
        let recording: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
        let entries = Rc::new(RefCell::new(Vec::<gtk::Entry>::new()));
        for (i, (action, title, default)) in ACTIONS.iter().enumerate() {
            let row = nixe_ui::widgets::hbox(6);
            let label = nixe_ui::widgets::label(title, "");
            label.set_hexpand(true);
            row.append(&label);
            let entry = gtk::Entry::new();
            entry.set_editable(false);
            entry.set_text(p.shortcuts.get(*action).map(String::as_str).unwrap_or(default));
            row.append(&entry);
            let record = gtk::Button::with_label("Record");
            row.append(&record);
            let recording = recording.clone();
            record.connect_clicked(move |_| recording.set(Some(i)));
            entries.borrow_mut().push(entry);
            b.append(&row);
        }
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let rec = recording.clone();
        let fields = entries.clone();
        controller.connect_key_pressed(move |_, key, _, mods| {
            let Some(i) = rec.get() else { return glib::Propagation::Proceed };
            if key == gdk::Key::Escape {
                rec.set(None);
                return glib::Propagation::Stop;
            }
            let text = gtk::accelerator_name(key, mods);
            if parsed(&text).is_some() {
                fields.borrow()[i].set_text(&text);
                rec.set(None);
            }
            glib::Propagation::Stop
        });
        d.add_controller(controller);
        let scope = s.clone();
        button(&b, "Save shortcuts", move || {
            if !scope.valid() {
                return;
            }
            let mut configured = BTreeMap::new();
            let mut seen = Vec::new();
            for ((action, _, _), entry) in ACTIONS.iter().zip(entries.borrow().iter()) {
                let text = entry.text().to_string();
                let Some(shortcut) = parsed(&text) else {
                    window::show_status("A shortcut is invalid.");
                    return;
                };
                if seen.contains(&shortcut) {
                    window::show_status("Two actions use the same shortcut.");
                    return;
                }
                seen.push(shortcut);
                configured.insert(action.to_string(), text);
            }
            crate::prefs::update(|p| p.shortcuts = configured);
            window::show_status("Shortcuts saved.");
        });
        button(&b, "Restore default shortcuts", || {
            crate::prefs::update(|p| p.shortcuts.clear());
            window::show_status("Default shortcuts restored. Reopen this page to view them.");
        });
        Ok(())
    });
}
#[cfg(test)]
pub(crate) fn check_shortcuts(){assert!(parsed("<Control>j").is_some());assert!(parsed("a").is_none());assert!(parsed("F8").is_some());}
