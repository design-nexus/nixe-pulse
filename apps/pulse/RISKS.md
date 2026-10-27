# Nyx Pulse Risk Analysis & Mitigation Matrix

This document identifies primary technical, architectural, and operational risks in the Nyx Pulse project, along with mitigation strategies and contingencies.

---

## 1. Technical Risk Matrix

| Risk ID | Risk Summary | Impact | Likelihood | Mitigation Strategy | Contingency Plan |
|---|---|---|---|---|---|
| **R-01** | TDLib C++ build time and binary distribution burden | High | High | Dynamic loading (`libloading`) of system `libtdjson.so` (available via `aur/telegram-tdlib`) with custom fallback paths. | Provide automated binary download script in `install.sh` pulling precompiled `libtdjson.so`. |
| **R-02** | tgcalls / WebRTC calling stack complexity & upstream drift | High | Medium | Isolate calling into `crates/nyx-telegram-calls` with clean FFI abstractions (`ntgcalls` / `tgcalls`). | Fallback to audio-only VoIP bridge first; maintain pinned C++ submodule. |
| **R-03** | Wayland / Hyprland screencasting portal permission failures | Medium | Medium | Use standard `ashpd` / XDG Desktop Portal `ScreenCast` D-Bus API; test against `xdg-desktop-portal-hyprland`. | Display clear user-facing error toast with portal troubleshooting guide. |
| **R-04** | GTK4 UI hitching or memory explosion on large chat histories | High | Low | Enforce strict list virtualization (`gtk::ListView` + `SignalListItemFactory`); lazy fetch 50 messages at a time; zero whole-history DOM loading. | Implement hard memory caps and proactive item eviction in `gio::ListStore`. |
| **R-05** | Telegram API key revoking or strict flood wait limits | High | Medium | Store credentials securely in Linux Secret Service; enforce exponential backoff on TDLib flood wait errors. | Guide users through obtaining personal `api_id`/`api_hash` at `my.telegram.org`. |
| **R-06** | License mismatch (LGPL-3.0 tgcalls vs MIT Nyx workspace) | Medium | Low | Dynamic linking of tgcalls / ntgcalls shared libraries or process isolation; keeps core Nyx crates MIT. | Compliance with LGPL-3.0 by providing relinking capability. |
| **R-07** | Audio device desynchronization or PipeWire latency issues | Medium | Low | Use standard PipeWire native audio streams or PulseAudio emulation layer over WirePlumber. | Allow explicit audio device selection in Nyx Pulse settings card. |
| **R-08** | Secret chats protocol changes or partial TDLib support | Low | Medium | Identify and mark secret chats as experimental; ensure ordinary cloud chats are never mislabeled as E2EE. | Disable secret chats if TDLib reports unsupported environment. |

---

## 2. In-Depth Risk Analyses

### R-01: TDLib Compilation & Dependency Footprint
- **Description**: Compiling TDLib from source requires CMake, gperf, OpenSSL, and clang/gcc, taking 20–30 minutes and consuming over 16 GB of RAM during linking. Forcing every Nyx user to compile TDLib would lead to install failures on lower-spec machines.
- **Mitigation**:
  - `nyx-telegram-core` will dynamically resolve `libtdjson.so` at runtime using `libloading` or `pkg-config`.
  - Search order:
    1. Environment variable `TDLIB_PATH`
    2. Arch system libraries: `/usr/lib/libtdjson.so`, `/usr/local/lib/libtdjson.so` (from package `telegram-tdlib`)
    3. User local library: `~/.local/share/nyx/lib/libtdjson.so`
  - The installation script (`install.sh`) checks for `telegram-tdlib` and prompts the user or installs the package.

### R-02: Calling Stack (tgcalls / ntgcalls) Maintenance
- **Description**: TDLib provides the signaling protocol (Diffie-Hellman keys, server IP/port reflectors, WebRTC SDP payloads), but the media transport requires Telegram's C++ calling stack (`TelegramMessenger/tgcalls` or `ntgcalls`). Calling protocols are occasionally updated by Telegram.
- **Mitigation**:
  - Encapsulate all calling logic in `crates/nyx-telegram-calls`.
  - Use stable C ABI bindings (`ntgcalls-sys`) or `cxx` to communicate with the C++ WebRTC engine.
  - Implement 1-on-1 audio calling first, followed by video, followed by group SFU calls.
  - Check protocol layer negotiation in `CallStateReady` (`min_layer` / `max_layer`).

### R-03: Wayland Screen Sharing under Hyprland
- **Description**: Under Wayland, applications cannot directly grab the screen buffer or read other windows for security reasons. Screen sharing must go through the XDG Desktop Portal (`org.freedesktop.portal.ScreenCast`).
- **Mitigation**:
  - Use `ashpd` to request screen casting sessions from `xdg-desktop-portal-hyprland`.
  - The portal dialog prompts the user to select an output or window.
  - The portal yields a PipeWire node ID, which is consumed directly into the calling media pipeline.

### R-04: Memory & Performance with Large Media
- **Description**: High-resolution images, long videos, animated stickers, and endless chat histories can quickly consume gigabytes of RAM if not carefully managed.
- **Mitigation**:
  - Virtualized `gtk::ListView` widgets recycle cells; only messages currently visible on screen (plus a small buffer) exist in the GTK widget tree.
  - Media thumbnails are stored in `~/.cache/nyx-pulse/thumbnails/` and scaled on load.
  - Full-resolution media is only downloaded when explicitly viewed or saved.
  - Memory buffers for video streaming are streamed directly via GStreamer rather than buffered in RAM.

### R-05: Telegram API Policy & Security
- **Description**: Hardcoding a single public Telegram API ID/Hash in open-source repositories often leads to Telegram revoking the application key due to spam bots.
- **Mitigation**:
  - Never commit hardcoded API credentials.
  - Nyx Pulse provides a setup wizard where users input their `api_id` and `api_hash`.
  - Store credentials in the Linux Secret Service keyring (`org.freedesktop.secrets`).
  - Provide clear in-app documentation on how to generate personal API credentials in 60 seconds from `my.telegram.org`.
