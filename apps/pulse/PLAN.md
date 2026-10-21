# Nixe Pulse Development Plan

## Phase Overview

```
Phase 0: Research & Feasibility (Architecture, Features Matrix, Plan, Risks)
   │
   ▼
Phase 1: Telegram Foundation (TDLib Core, Auth, Multi-Account, Basic Text Messaging MVP)
   │
   ▼
Phase 2: Complete Messaging (Media Attachments, Stickers, Reactions, Folders, Voice Notes)
   │
   ▼
Phase 3: Voice and Video Calling (Signaling, tgcalls/ntgcalls, PipeWire, Screen Sharing)
   │
   ▼
Phase 4: Advanced Telegram Features (Secret Chats, Stories, Mini Apps, Channel Admin)
   │
   ▼
Phase 5: Nixe Suite Integration (Contacts, Assistant, Glimpse, Capture, Settings)
   │
   ▼
Phase 6: Release Preparation (Security Review, Profiling, Packaging, Testing)
```

---

## Phase 0: Research and Feasibility

**Goal**: Establish system architecture, audit dependencies, evaluate calling stacks, and deliver planning documentation.

### Milestones & Tasks:
- [x] Inspect existing Nixe codebase and application conventions (`apps/chat`, `apps/contacts`, `apps/music`, `STYLE.md`).
- [x] Audit licenses (TDLib: BSL-1.0, tgcalls: LGPL-3.0, Nixe: MIT).
- [x] Evaluate TDLib integration options and C JSON FFI binding strategy.
- [x] Investigate Telegram Desktop calling architecture and Linux media streaming options (`tgcalls`, `ntgcalls`, WebRTC, PipeWire, XDG Desktop Portal).
- [x] Deliver `ARCHITECTURE.md`.
- [x] Deliver `FEATURES.md`.
- [x] Deliver `PLAN.md`.
- [x] Deliver `RISKS.md`.

**Exit Criteria**: Comprehensive architecture and planning documentation approved and committed.

---

## Phase 1: Telegram Foundation & Messaging MVP

**Goal**: Functional Telegram messaging MVP with native GTK4 three-column UI, TDLib authentication, chat synchronization, and real-time text message exchange.

### Milestones & Tasks:
1. **Workspace Setup**:
   - Create `crates/nixe-telegram-core` and configure dependencies (`tokio`, `serde`, `serde_json`, `libloading`, `dashmap`).
   - Create `apps/pulse` and configure dependencies (`nixe-core`, `nixe-ui`, `nixe-theme`, `gtk4`, `oo7`).
   - Add packages to root `Cargo.toml`.
2. **TDLib Bridge & Client (`nixe-telegram-core`)**:
   - Implement FFI bindings to `libtdjson.so` with dynamic loading and path detection.
   - Implement asynchronous worker thread polling `td_json_client_receive`.
   - Implement request dispatcher with `@extra` tokens and `tokio::sync::oneshot` channels.
   - Implement typed Telegram update event stream.
3. **Authentication & Multi-Account (`nixe-telegram-core`)**:
   - State machine for TDLib initialization (`setTdlibParameters`).
   - Phone number authentication, verification code, and 2FA password check.
   - QR code login initiation and SVG barcode rendering.
   - Account directory isolation in `~/.local/share/nixe-pulse/accounts/<id>/`.
   - Credential & database encryption key storage via Linux Secret Service (`oo7`).
4. **Native GTK4 Three-Column UI (`apps/pulse`)**:
   - Window setup: Flat `bg` surface, no titlebar (`window.set_titlebar(Some(&gtk::Box::new()))`), 1120x800 default size.
   - Navigation sidebar: 250 px width (`hexpand(false)`), collapsible to 52 px icon-only mode below 980 px window width.
   - Conversation list: Virtualized `gtk::ListView` with `gio::ListStore` displaying chat avatars, titles, snippets, timestamps, and unread badges.
   - Active chat view: Conversation header, virtualized message history (`gtk::ListView`), and multiline composer.
   - Status bar: 34 px with `F1 Shortcuts`, status readout, and settings gear.
   - Omarchy live theme synchronization polling `~/.local/state/omarchy/current/theme/colors.toml`.
5. **Basic Text Messaging**:
   - Send and receive plain text messages in real-time.
   - Message bubble styling according to `STYLE.md` (`@surface` incoming, `@nav_active` outgoing).
   - Read status receipt updates (single tick, double tick).
   - Typing indicator broadcast and display.
   - Desktop notifications via `gio::Notification`.

**Exit Criteria**: User can log into Telegram, view their chat list, open conversations, and send/receive real-time text messages in a polished Nixe GTK4 window.

---

## Phase 2: Complete Messaging

**Goal**: Full daily-use messaging client with rich media, stickers, reactions, chat folders, and voice notes.

### Milestones & Tasks:
1. **Rich Media Support**:
   - Image attachments: Thumbnail rendering, full-size zoom viewer, caching in `~/.cache/nixe-pulse/`.
   - Video attachments: Inline playback via GStreamer `gtk4paintablesink`.
   - File attachments: Document cards with file type icons, size, and download progress.
   - Drag-and-drop file sending (<kbd>DropTarget</kbd>) and clipboard image pasting (<kbd>Ctrl</kbd>+<kbd>V</kbd>).
2. **Interactive Messaging Actions**:
   - Message replies with preview banner in composer and jump-to-target scrolling.
   - Message editing with edit banner and update synchronization.
   - Message forwarding dialog.
   - Message deletion (for me / for everyone).
   - Message pinned bar.
   - Emoji and animated sticker picker popover.
   - Message reactions (tap to react, emoji reaction counters).
3. **Voice & Audio Messages**:
   - Audio recording via PipeWire with waveform amplitude visualizer.
   - Voice note playback with interactive scrub bar.
4. **Chat Management**:
   - Chat folders and tabs in navigation sidebar.
   - Archived chats collapsible section.
   - Mute and notification duration controls.
   - Saved Messages cloud shortcut.

**Exit Criteria**: User can use Nixe Pulse as their daily Telegram messenger for all routine multimedia and group chat tasks.

---

## Phase 3: Voice and Video Calling

**Goal**: Native GTK4 voice and video calling infrastructure replacing Telegram Desktop for calling.

### Milestones & Tasks:
1. **Calling Protocol & Media Bridge (`nixe-telegram-calls`)**:
   - Intercept TDLib `updateCall` and `CallStateReady`.
   - Extract Diffie-Hellman derived encryption key, server endpoints, and protocol options.
   - Bind Telegram's C++ calling engine (`tgcalls`/`ntgcalls`) via Rust FFI.
2. **1-on-1 Voice Calling**:
   - Outgoing call creation and incoming call ringing notification toast.
   - P2P / Reflector Opus audio transmission via PipeWire audio graph.
   - Hardware/software microphone mute and speaker volume control.
   - 4-emoji visual encryption key verification display.
3. **1-on-1 Video Calling**:
   - Camera input capture via PipeWire / V4L2.
   - VP8/H.264 video encoding and decoding.
   - Local self-view picture-in-picture and remote participant video rendering.
4. **Group Voice & Video Chats**:
   - TDLib `joinGroupCall` SDP/ICE payload negotiation with Telegram SFU servers.
   - Multi-participant audio mixing and video tile grid.
   - Raise-hand, participant list, and volume adjustment per user.
5. **Wayland Screen Sharing**:
   - Integrate XDG Desktop Portal `org.freedesktop.portal.ScreenCast` via `ashpd`.
   - Stream PipeWire video frames into the call video track.
   - Screen and window selection under Hyprland.
6. **Native GTK4 Calling UI**:
   - Floating incoming call notification.
   - Compact floating call window and fullscreen expandable video room.
   - Audio device picker (headset, speakers, microphone).

**Exit Criteria**: Successful end-to-end voice, video, and screen-sharing calls verified with standard Telegram Desktop and mobile users through Telegram's native servers.

---

## Phase 4: Advanced Feature Parity

**Goal**: Close remaining feature gaps with Telegram Desktop.

### Milestones & Tasks:
1. **Secret Chats**:
   - End-to-end encrypted secret chats via TDLib (where supported).
   - Self-destruct timers and screenshot alerts.
2. **Stories**:
   - Story carousel above conversation list.
   - Fullscreen story viewer with progress bars and reactions.
3. **Telegram Mini Apps**:
   - Sandboxed WebKitGTK web view.
   - Telegram WebApp JavaScript bridge implementation.
4. **Channel & Group Administration**:
   - Edit group permissions, slow mode, invite links.
   - Channel subscriber management and post analytics.
5. **Telegram Premium Features**:
   - Large file transfers (up to 4 GB).
   - Voice-to-text transcription display.
   - Custom emoji reactions.

**Exit Criteria**: Parity with all primary advanced Telegram Desktop features documented in `FEATURES.md`.

---

## Phase 5: Nixe Suite Integration

**Goal**: Deep native integration with the rest of the Nixe application suite on Omarchy Linux.

### Milestones & Tasks:
1. **Nixe Contacts**:
   - Associate Telegram users with local contacts via phone number or `@username`.
   - Action button in Contacts to "Send Telegram Message" or "Call on Telegram" via Pulse.
2. **Nixe Assistant**:
   - Implement `io.github.design_nexus.nixe.Control` D-Bus tools:
     - `pulse.chats.list`
     - `pulse.chats.search`
     - `pulse.messages.send` (with safety policy / user confirmation)
     - `pulse.calls.start`
3. **Nixe Glimpse**:
   - Create `apps/pulse-glimpse` search plugin using `glimpse-plugin-sdk`.
   - Index contact names and recent message snippets for quick desktop launching.
4. **Nixe Capture**:
   - Direct screenshot pasting and attachment workflow.
5. **Nixe Settings**:
   - Configuration panel in Nixe Settings for Pulse preferences.

**Exit Criteria**: Seamless interoperability between Pulse, Contacts, Assistant, Glimpse, and Capture.

---

## Phase 6: Release Preparation

**Goal**: Hardening, security auditing, performance profiling, and desktop packaging.

### Milestones & Tasks:
1. **Security Audit**:
   - Validate Secret Service credential encryption.
   - Verify scrubbing of tokens, auth codes, and passwords from logs.
   - Memory leak inspection on long-running TDLib sessions.
2. **Performance Profiling**:
   - Measure startup time, idle RAM, and CPU usage.
   - Optimize GTK4 `ListView` recycling for large chat histories.
3. **Packaging & Installation**:
   - Desktop entry `io.github.design_nexus.nixe.Pulse.desktop`.
   - Icon asset in `crates/nixe-icons`.
   - Installation integration in `install.sh` and `bin/nixe`.
   - Hyprland window rules in `~/.config/hypr/nixe.lua`.

**Exit Criteria**: Stable, production-ready release of Nixe Pulse running on Omarchy Linux.
