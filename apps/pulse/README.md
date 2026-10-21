# Pulse

Native Telegram client for the Nixe suite, using Rust, GTK4, TDLib, shared Nixe widgets, and Media Player's playback engine and controls.

Install from the repository root with `./install.sh pulse`. The installer prepares the SHA-256-pinned NTgCalls 3.0.5 runtime, installs required system dependencies, and installs Pulse, its launcher/icon, and its Glimpse plugin. It does not launch Pulse.

Pulse supports account-isolated sessions, paginated chat history, drafts and replies, message search, receipts and failed-send retry, file/photo/video/animation uploads, voice recording, downloads, real media previews, albums, custom emoji, stickers, and media save/copy/navigation. Account, privacy, proxy, notification, membership, shared-media, and secret-chat information comes from Telegram rather than demonstration data.

One-to-one calls use TDLib for signaling/key exchange and a native NTgCalls C ABI for media transport. Microphone/speaker/camera controls and portal-based Wayland screen sharing use real devices. Local native peer negotiation is covered by a loopback regression test; interoperability with another Telegram client still requires an authorized remote test contact. Group calls add initially muted audio, camera/video reception, PipeWire screen sharing, participant mute/hand/volume controls and native broadcast playback. These transports are compiled locally; live cross-client interoperability remains unverified.

New database keys are stored in Secret Service using `secret-tool`. Existing sessions are retained and database encryption is upgraded through TDLib's supported key-change method, with a recovery marker for interrupted migrations. Production startup reports missing TDLib instead of selecting a mock backend.

The Tools menu includes profile history, birthday/phone changes, hardware passkeys, privacy pickers, notification exceptions, accessibility and shortcut settings, detached conversation windows, HTML/media exports, embedded Mini Apps, provider checkout, stories/albums/interactions, collectible gifts and Business controls. See [PARITY.md](PARITY.md) for coverage and remaining limitations. Mini Apps use a separate WebKitGTK process; passkeys use libfido2; device locations use GeoClue.

Notifications run while Pulse is open. Account switching selects the active account's client and data directories.

- Preferences: `$XDG_CONFIG_HOME/nixe-pulse/settings.toml`
- Account databases: `$XDG_DATA_HOME/nixe-pulse/accounts/<id>/data/`
- Media cache: `$XDG_CACHE_HOME/nixe-pulse/accounts/<id>/files/`

Shortcuts: Ctrl+F searches chats; Ctrl+B toggles the sidebar; Ctrl+, opens settings; F1 shows shortcuts; Escape closes overlays; Enter sends; Shift+Enter inserts a newline; Ctrl+Q quits and Ctrl+W closes the window. Tool shortcuts are configurable under Tools → Accessibility and shortcuts. The media viewer also supports Ctrl+S, Ctrl+C, and +/-.
