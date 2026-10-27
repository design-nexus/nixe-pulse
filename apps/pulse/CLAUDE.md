# Pulse — notes for working on this app

- GTK4 (gtk4-rs 0.11) + Rust, no libadwaita, built strictly to `STYLE.md`. App id
  `io.github.design_nexus.nyx.Pulse`; data in `~/.local/share/nyx-pulse`, config in
  `~/.config/nyx-pulse`.
- Three-column layout:
  1. `.settings-navigation`: accounts, folders, settings button.
  2. Conversation list: virtualized `gtk::ListView` with `gio::ListStore`.
  3. Active chat: header, virtualized message history, rich composer.
  4. Collapsible right detail panel: contact profile, shared media tabs.
- TDLib protocol engine in `crates/nyx-telegram-core`.
- Calling media engine in `crates/nyx-telegram-calls`.
- Control interface (`control.rs`): `nyx_core::control` on the app's D-Bus name, for Nyx
  Assistant: `pulse.chats.list`, `pulse.chats.search`, `pulse.messages.send`, `pulse.calls.start`.
- Checks: `cargo test -p nyx-pulse`, `cargo test -p nyx-telegram-core`, `cargo test -p nyx-telegram-calls`.
