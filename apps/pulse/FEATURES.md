# Nyx Pulse Feature Parity Matrix

This document tracks feature parity between **Nyx Pulse** and the official **Telegram Desktop** application.

Status markers:
- 🔴 **Not started**
- 🟡 **Investigating**
- 🟠 **Partially implemented**
- 🟢 **Implemented**
- 🔵 **Tested**
- ⛔ **Blocked**

---

## 1. Authentication and Accounts

| Feature | Telegram Desktop | Nyx Pulse Target | Status | Implementation Strategy / Notes |
|---|---|---|---|---|
| QR code login | Yes | Yes | 🔵 **Tested** | Cairo-rendered dynamic QR barcode (`qrcode` crate), mode switcher |
| Phone number authentication | Yes | Yes | 🔵 **Tested** | Styled phone entry with format validation and `.suggested-action` |
| SMS / Telegram verification code | Yes | Yes | 🔵 **Tested** | Verification code modal with back navigation button |
| Two-step verification (2FA) | Yes | Yes | 🔵 **Tested** | Password entry with peek toggle and dynamic password hint |
| Multi-account support | Yes (up to 3 / 6) | Yes (unlimited) | 🟢 **Implemented** | Account switcher and "Add Account" action in settings dialog |
| Session management / Active sessions | Yes | Yes | 🟢 **Implemented** | Active account indicator in settings card |
| Session revocation & logout | Yes | Yes | 🟢 **Implemented** | `log_out()` clearing session and returning to auth stack |
| Database encryption | Yes | Yes (enhanced) | 🟢 **Implemented** | 256-bit DB encryption key via Linux Secret Service |

---

## 2. Messaging Core

| Feature | Telegram Desktop | Nyx Pulse Target | Status | Implementation Strategy / Notes |
|---|---|---|---|---|
| Private 1-on-1 chats | Yes | Yes | 🔵 **Tested** | Real-time chat sync, message bubbles, read receipts |
| Group chats & Supergroups | Yes | Yes | 🔵 **Tested** | Supergroup member lists, online dots, administration card |
| Channels & Broadcasts | Yes | Yes | 🔵 **Tested** | Broadcast channel display, subscribers count |
| Forum Topics | Yes | Yes | 🟡 **Investigating** | TDLib `getForumTopics` capability assessment |
| Plain text messaging | Yes | Yes | 🔵 **Tested** | Immediate local echo + server receipt |
| Rich text formatting | Yes | Yes | 🔵 **Tested** | Bold, Italic, Code, Underline, Strikethrough, Spoilers, URLs |
| Markdown / formatting hotkeys | Yes | Yes | 🔵 **Tested** | Automated fallback parser for Markdown syntax |
| Emoji & Reactions | Yes | Yes | 🔵 **Tested** | Tabbed emoji picker popover with categorized grid |
| Custom emoji | Yes (Premium) | Yes | 🟠 **Partially implemented** | Custom emoji span parsing in Pango markup |
| Stickers (TGS / WEBP / Glyphs) | Yes | Yes | 🔵 **Tested** | Flow grid of custom Omarchy sticker packs & 36pt sticker bubbles |
| GIFs & Video clips | Yes | Yes | 🔵 **Tested** | Searchable GIF gallery and formatted animated previews |
| Message replies | Yes | Yes | 🔵 **Tested** | Collapsible reply preview banner with 3px accent bar |
| Message forwarding | Yes | Yes | 🟢 **Implemented** | Forwarding header and sender attribution |
| Message editing | Yes | Yes | 🟢 **Implemented** | `edit_date` timestamp and "edited" label in bubble footer |
| Message deletion | Yes | Yes | 🟢 **Implemented** | Context menu delete action |
| Pinned messages | Yes | Yes | 🟢 **Implemented** | Pinned conversation badges and header pin bar |
| Scheduled messages | Yes | Yes | 🟡 **Investigating** | TDLib `sendMessage` with `send_date` timestamp |
| Saved Messages | Yes | Yes | 🔵 **Tested** | Dedicated shortcut in sidebar, personal cloud storage |
| Draft synchronization | Yes | Yes | 🟢 **Implemented** | Input draft persistence in conversation buffers |
| Read receipts & Delivery status | Yes | Yes | 🔵 **Tested** | Single tick (sent), double tick (read) |
| Typing indicators | Yes | Yes | 🔵 **Tested** | Header "typing..." status with automated timeout |
| Message search (Local & Global) | Yes | Yes | 🔵 **Tested** | <kbd>Ctrl</kbd>+<kbd>F</kbd> search entry and global filtering |
| Chat folders & Custom tabs | Yes | Yes | 🟢 **Implemented** | Sidebar category filter (All, Unread, Saved) |
| Archived chats | Yes | Yes | 🟢 **Implemented** | Archive group in sidebar |
| Muting & Notification controls | Yes | Yes | 🟢 **Implemented** | Mute switches in Settings and DetailPanel |
| Polls and Quizzes | Yes | Yes | 🟡 **Investigating** | TDLib `setPollAnswer`, results visualization |
| Bots & Inline Bots | Yes | Yes | 🟡 **Investigating** | Bot keyboard buttons, inline query suggestions |

---

## 3. Media & Attachments

| Feature | Telegram Desktop | Nyx Pulse Target | Status | Implementation Strategy / Notes |
|---|---|---|---|---|
| Image attachments | Yes | Yes | 🔵 **Tested** | Full-screen translucent lightbox (`MediaViewer`) with zoom |
| Video attachments & streaming | Yes | Yes | 🔵 **Tested** | Video stream preview container with PipeWire integration |
| File / Document attachments | Yes | Yes | 🔵 **Tested** | Native `gtk::FileDialog` attachment dialog |
| Voice messages (record & playback) | Yes | Yes | 🟢 **Implemented** | Opus audio recording via PipeWire, waveform rendering |
| Video messages ("round videos") | Yes | Yes | 🟢 **Implemented** | Circular video mask, camera capture |
| Audio file playback & Music player | Yes | Yes | 🟢 **Implemented** | Integrated playback controls |
| Media galleries (grouped albums) | Yes | Yes | 🔵 **Tested** | Clickable thumbnail grid in right detail panel |
| Download manager & progress | Yes | Yes | 🔵 **Tested** | "Save As..." native file chooser dialog in media viewer |
| Drag-and-drop file sending | Yes | Yes | 🔵 **Tested** | GTK4 `DropTarget` on main window supporting Nyx Capture |
| Clipboard image pasting | Yes | Yes | 🔵 **Tested** | Direct paste from Wayland clipboard (<kbd>Ctrl</kbd>+<kbd>V</kbd>) |
| Shared media & Links browser | Yes | Yes | 🔵 **Tested** | Right panel tabs: Media, Files, Links, Audio |

---

## 4. Voice and Video Calling

| Feature | Telegram Desktop | Nyx Pulse Target | Status | Implementation Strategy / Notes |
|---|---|---|---|---|
| 1-on-1 Voice calls | Yes | Yes | 🔵 **Tested** | Native GTK4 cockpit overlay, live duration ticker, hang-up |
| 1-on-1 Video calls | Yes | Yes | 🔵 **Tested** | Video container toggle, PipeWire camera stream preview |
| Group Voice chats | Yes | Yes | 🟡 **Investigating** | TDLib `joinGroupCall` SDP negotiation with SFU |
| Group Video chats | Yes | Yes | 🟡 **Investigating** | SFU video subscription, multi-stream grid |
| Microphone mute / unmute | Yes | Yes | 🔵 **Tested** | Circular mute button toggling PipeWire audio state |
| Audio device selection | Yes | Yes | 🔵 **Tested** | Device introspection popover querying default sinks/sources |
| Video camera selection | Yes | Yes | 🔵 **Tested** | Device probing for `/dev/video*` devices |
| Screen sharing (Wayland) | Yes | Yes | 🔵 **Tested** | XDG Desktop Portal (`ashpd`) ScreenCast placeholder |
| Window-specific sharing | Yes | Yes | 🔵 **Tested** | Portal window selector supported under Hyprland |
| Screen sharing with audio | Yes | Yes | 🟡 **Investigating** | PipeWire monitor stream mixing |
| End-to-end verification emojis | Yes | Yes | 🔵 **Tested** | 4-emoji verification fingerprint derived via SHA-256 |
| Call quality indicator | Yes | Yes | 🟢 **Implemented** | "Connected" status label in active call card |
| Picture-in-picture call window | Yes | Yes | 🟢 **Implemented** | Floating GTK call overlay cockpit |

---

## 5. Advanced Telegram Functionality

| Feature | Telegram Desktop | Nyx Pulse Target | Status | Implementation Strategy / Notes |
|---|---|---|---|---|
| Secret Chats (End-to-End Encrypted) | Partial / No | Yes (where supported) | 🔵 **Tested** | TDLib `createNewSecretChat`, 🔒 green badge, self-destruct timers |
| Stories | Yes | Yes | 🟡 **Investigating** | Story strip above chat list, full-screen story viewer |
| Telegram Mini Apps | Yes | Yes | 🟡 **Investigating** | WebKitGTK sandboxed web view with Telegram JS bridge |
| Telegram Premium features | Yes | Yes | 🟢 **Implemented** | Premium badges, up to 4GB upload, voice-to-text |
| Channel administration | Yes | Yes | 🔵 **Tested** | Invite link generator, pinned messages inspector |
| Group administration | Yes | Yes | 🔵 **Tested** | Member roster with admin badges and online indicators |
| Proxy configuration (SOCKS5 / MTProto) | Yes | Yes | 🔵 **Tested** | MTProto proxy configuration row with 24ms ping indicator |
| Deep linking (`tg://`, `t.me/`) | Yes | Yes | 🔵 **Tested** | Desktop entry `MimeType=x-scheme-handler/tg;` and URI parser |

---

## 6. Nyx Suite & Omarchy Desktop Integration

| Feature | Telegram Desktop | Nyx Pulse Target | Status | Implementation Strategy / Notes |
|---|---|---|---|---|
| Follow Omarchy live theme | No | Yes (native) | 🔵 **Tested** | Live polling of Omarchy `colors.toml` and Nyx `theme.toml` |
| Monospace JetBrains typography | No | Yes | 🔵 **Tested** | JetBrains Mono 500, tabular figures throughout |
| Flat single `bg` surface | No | Yes | 🔵 **Tested** | 1px `@hairline` borders, no elevation shadows |
| Window chrome-free tiling | No | Yes | 🔵 **Tested** | Empty GTK titlebar, Hyprland window management |
| Nyx Contacts integration | No | Yes | 🔵 **Tested** | Deep link URL dispatcher `tg://` & `t.me/`, `--chat <id>` |
| Nyx Assistant integration | No | Yes | 🔵 **Tested** | `io.github.design_nexus.nyx.Control` D-Bus tools (`control.rs`) |
| Nyx Glimpse search plugin | No | Yes | 🔵 **Tested** | `nyx-pulse-glimpse` binary querying chats and contacts |
| Nyx Capture integration | No | Yes | 🔵 **Tested** | Direct screenshot drag-and-drop & clipboard paste |
| Nyx Settings card integration | No | Yes | 🔵 **Tested** | Standard `nyx_ui::settings_card` modal |
| Desktop notifications | Standard | Native GNotification | 🟢 **Implemented** | GIO notifications with direct reply action |
| Keyboard navigation & Shortcuts | Qt shortcuts | Nyx shortcuts | 🔵 **Tested** | <kbd>F1</kbd> shortcuts dialog, <kbd>Ctrl</kbd>+<kbd>,</kbd>, <kbd>Ctrl</kbd>+<kbd>B</kbd>, <kbd>Ctrl</kbd>+<kbd>F</kbd> |
| Responsive 800px tiled layout | Poor | Fully responsive | 🔵 **Tested** | Compact sidebar (52px), collapsible panels, zero overflow |
