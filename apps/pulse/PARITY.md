# Telegram Desktop parity

Reference: [Telegram Desktop v7.3.1](https://github.com/telegramdesktop/tdesktop/releases/tag/v7.3.1), frozen for this roadmap. Pulse retains native GTK and Nixe styling. “Implemented” means connected to TDLib; live interoperability is listed separately from local checks.

## Release 1: chat and message actions

| Feature | Status and behavior |
| --- | --- |
| Shared chat menus | Implemented in sidebar and conversation header; information, search, read/unread, pin, archive, mute, history, block, report, delete/leave. |
| Archive | Implemented as an independent list with its own pinning/order and additional loading. Removing Main position no longer deletes the account cache entry. |
| Read/unread | Explicit unread marker is independent of counts and participates in the Unread filter. Marking an open chat unread preserves its draft and exits the conversation. |
| Notifications | Unmute; 1 hour, 8 hours, 2 days, forever, custom minutes. Preserve other notification options; resolve inherited scope mute settings. |
| Clear/delete | Clear retains chat membership/list entry. Destructive dialogs default to current-user scope; broader scope requires an explicit choice and current Telegram permission. Existing leave/delete behavior remains. |
| Message permissions | Query `getMessageProperties` when opening menus and recheck destructive/forward/edit actions before submission. Removed obsolete inferred message flags. |
| Forward/save | Search recipients, multiple destinations, previews, album grouping, attribution/caption options, Saved Messages, protected-content restrictions. Track accepted source IDs to avoid repeating confirmed messages after partial failures. Unknown send status requires inspection before retry. |
| Message editing | Separate formatted text/caption editor preserves the composer draft and existing formatting. |
| Reply/quote | Reply and quote selected text with UTF-16 quote positions; reply and quote drafts synchronize with Telegram. |
| Reactions | Available emoji/custom-emoji selection, quick reaction synchronized from Telegram, chosen state, toggling/removal, counts, participants and pagination. Paid reactions remain Release 5 work. |
| Bulk actions | Selection, copy, forward, delete with explicit scope, report, Escape cancellation, keyboard context menus. |
| Attachment actions | Save/download, copy supported images, show downloaded files, cancel downloads, retry/cancel pending messages where Telegram permits. Viewer buttons/shortcuts and external document opening also respect export permission and recheck it before use. |
| Reporting/blocking | Server-provided reporting options/text steps and message selection; block/unblock contacts. |
| State isolation | Account-generation checks on async actions; dialog/menu cleanup and stale search-result guards. |

Local verification: compilation; targeted fixtures covering partial forwarding/source identity, permission defaults, deletion failures/scopes; isolated GTK checks covering Archive transitions, manual unread, selection, quotes, renderer ownership and tiled width. No real chats were deleted, reported, joined, left, or contacted during these checks. Cross-client synchronization and live forwarding/reaction checks remain unverified.

## Release 2: organization and composition

| Feature | Status and behavior |
| --- | --- |
| Folders | Create, edit, reorder, rules, explicit membership/exclusions, per-folder unread counts and tags; preserve pinned chats and detect conflicting edits. |
| Shared folders | Create/edit/revoke/copy invite links, preview imports, choose chats explicitly, and review shared-folder additions. Server eligibility and update intervals apply. |
| Composition | Rich formatting, links, mentions and formatted message/caption editing; formatting shortcuts and UTF-16 entity handling. |
| Drafts | Debounced cloud synchronization, local pending edits, formatted text, replies and quotes; stale acknowledgements cannot overwrite newer edits. Preserve preview/effect metadata. Unsupported voice/video and suggested-post drafts remain read-only until explicitly discarded. |
| Search | Global/chat search, media/sender/date filters, secret-chat search, result navigation and pagination; full pinned-message browsing. |
| Delivery | Silent sending, scheduling, Premium repeat schedules, online delivery where eligible; scheduled-message editing, rescheduling, send-now and deletion. |
| Auto-delete | Chat timer controls with separate regular/secret-chat ranges; Telegram enforces permissions. |
| Saved Messages | Source topics/history, source pinning, tag search/counts and tag renaming where eligible. |

Local verification: compilation, targeted folder/draft/date fixtures, and isolated GTK regression for formatted offsets, quote/reply restoration and stale remote drafts. Installed without opening or restarting Pulse. Live cross-client draft synchronization, scheduling, folder sharing and Premium eligibility have not been exercised against an account.

## Release 3, first update: group controls and interactive messages

| Feature | Status and behavior |
| --- | --- |
| Creation and joining | Create private groups, forums and channels; preview ordinary/shared-folder invitations before joining; public-chat join control. Paid invitations remain explicit dependency/workflow gaps and do not initiate payment. Guard-bot verification is reported for completion in Telegram. |
| Members | Search/filter and paginate group/channel member lists; open member conversations; add contacts or resolved usernames without forwarding earlier history; show privacy/Premium invite failures. |
| Administration | Edit names/descriptions, default member permissions, administrator rights and individual restrictions. Preserve unedited fields, reject conflicting permission/status edits, recheck current privileges and protect owners/self/uneditable administrators. |
| Moderation | Explicit remove/ban/unban confirmations, restriction expiry and lifting restrictions. Banning warns that Telegram deletes the member's messages in supergroups/channels. Unbanning does not silently re-add membership. |
| Invites and requests | Create/edit/revoke/delete invite links, expiration/member limits/approval requirements, copy links, owner access to other administrators' links; paginated join requests with individual approval/decline. Refresh controls recover stale lists. |
| Ownership | Eligibility checks, named recipient confirmation, two-step verification password and fresh owner validation before transfer. |
| Polls and quizzes | Create standard polls/quizzes (up to ten answers), anonymous/public and multiple-answer controls, explanations and automatic closing; interactive voting/retraction, results, paginated voter lists and permission-checked stopping. Advanced poll media/options remain future work. |
| Contacts and locations | Compose contact cards and fixed coordinate locations; render contact/venue/location cards with conversation/map actions. Keep composer drafts and snapshot reply targets for these sends. Live location sending remains future work. |
| State and UI | Account-scoped requests, stale result guards, bounded scrolling and browser lifetime cleanup; group status updates refresh chat controls. Ambiguous create/send timeouts require inspecting Telegram before retrying. Info uses the paginated member/pinned browsers. |

Local verification: Pulse/Glimpse compilation, targeted permission/member-protection fixtures and poll payload/coordinate validation. No live groups were created or modified, invitations sent, requests processed, votes cast or messages sent during checks. Ownership, live permissions, joins and poll interoperability still require account-based verification. Implementation follows the installed [TDLib API schema](https://github.com/tdlib/td/blob/master/td/generate/scheme/td_api.tl).

## Release 3, second update: topics, comments and group settings

| Feature | Status and behavior |
| --- | --- |
| Forum topics | Search and paginate topics; open them in the existing conversation view; create/rename, pin/unpin, close/reopen, hide/show General, delete with confirmation, mute/unmute with inherited settings, and copy topic links. Telegram enforces current topic privileges. Custom topic icons and pinned-topic ordering are covered in the fourth update below. |
| Threads and comments | Permission-checked replies/comments from message menus and reply-count buttons; resolve channel posts to their discussion chat; load the thread's own paginated history and send to the native topic destination. Deep links preserve the topic and target message. |
| Draft/history isolation | Separate local/cloud draft keys and debounce versions for each topic/thread; receive native forum draft updates; discard only the selected draft; reject history results after changing conversations. Main-chat updates cannot overwrite a topic draft. Media uses the existing download/cache/rendering path. |
| Composition and search | Text, attachments, voice, polls, contacts, locations and inline GIFs carry the topic destination. Attachment/schedule callbacks validate their captured context. Search and read markers use the selected topic. Destination-topic forwarding and explicit send-as selection are covered in the third update below. |
| Group/channel settings | Public usernames with availability checks, static photos/removal, slow mode, linked discussion groups, history visibility, post signatures, content protection, forum mode, join requirements/approval, member-list privacy and eligible anti-spam controls. Each setting has an explicit submit, fresh eligibility checks and conflict detection where applicable. |
| Legacy group upgrades | Explicit confirmation before upgrading a basic group; linking a discussion group requires a supergroup with visible old history. History is not exposed automatically when linking. |

Local verification: Pulse/Glimpse compilation; topic/draft identity fixtures; one isolated GTK regression covering same-chat topic transitions, separate draft restoration, wrong-topic message filtering and rejection of stale history. No live topics, chats, photos, permissions, memberships or messages were modified during checks. Live topic synchronization, comment delivery and administrative interoperability remain unverified.

## Release 3, third update: destinations, identities and administration history

| Feature | Status and behavior |
| --- | --- |
| Topic forwarding | Search/paginate forum destinations using the existing topic browser; choose one explicit topic per selected forum. Validate the topic again before sending. Track accepted source IDs separately for every chat/topic, lock a destination after partial acceptance, freeze controls during submission and uncheck completed recipients after a partial failure. Ambiguous results still require inspection before retry. |
| Send as | Choose from Telegram's available user/channel identities; show the current identity above the group/channel composer. Recheck identity availability, Premium eligibility and the prior selection before applying. Selection changes the chat's default for new messages; it sends no message and initiates no purchase. Live sender updates refresh the composer. |
| Invite-use history | Paginate people who joined through a selected invite link, with join dates, shared-folder attribution and approver names. Eligible subscription links offer expired-membership filtering. Member actions query fresh permissions. |
| Recent actions | Paginated administrator events with text/category/related-member filters; readable action names, before/after text and permission/status changes, fresh message navigation and existing moderation controls. Explicit reporting for incorrect anti-spam removals where Telegram permits it. Telegram exposes this log for the last 48 hours; some complex event payloads have summary-only details. |

Local verification: Pulse/Glimpse compilation and targeted fixtures for destination-specific forwarding identity, Premium sender eligibility, lossless 64-bit event cursors, event filters and before/after descriptions; isolated GTK regression for completed-recipient checkbox updates without reentrant state borrowing. No live forwards, identity changes, moderation actions, reports, messages or invitations were performed. Live interoperability remains unverified. APIs: [available senders](https://core.telegram.org/tdlib/docs/classtd_1_1td__api_1_1get_chat_available_message_senders.html), [administrator log](https://core.telegram.org/tdlib/docs/classtd_1_1td__api_1_1get_chat_event_log.html).

## Release 3, fourth update: topic artwork, ordering and member tags

| Feature | Status and behavior |
| --- | --- |
| Topic artwork | Render topic colors and custom emoji in the topic browser; default icon picker plus installed custom emoji packs for eligible Premium accounts. Previews use static vector frames or video-emoji thumbnails, bounded downloads and an account-scoped cache. Monochrome vector previews use the current text color. Closing a window prevents queued icon downloads from starting. |
| Topic editing | Choose a custom/default icon and creation color, preserve General-topic restrictions, detect concurrent name/icon changes and recheck eligibility for newly selected icons. An unchanged existing icon does not block renaming after Premium expires. |
| Pinned ordering | Load the complete topic list before arranging pinned topics, move them up/down, then explicitly save. Compare the current pinned order immediately before submission; a concurrent membership/order change requires reopening the dialog. |
| Member tags | Edit eligible member tags/administrator titles or own tags; preserve individual restrictions, enforce current rights and reject conflicting edits. Show tags in member lists/details and incoming message headers; bound long names/tags with ellipses. Telegram validates the 16-character/no-emoji rule. |

Local verification: topic-order bounds/membership fixtures, restricted/self/administrator tag-permission fixtures, and isolated GTK/native-renderer regression. Static previews release their renderer immediately; animated messages retain it until their widget is removed. No live icons, topic order, tags, memberships or messages were changed. Live Premium eligibility and cross-client behavior remain unverified. Custom emoji previews are intentionally static; full animation and some complex administrator-event details remain incomplete.

## Release 3, fifth update: video messages, live locations and poll controls

| Feature | Status and behavior |
| --- | --- |
| Video messages | Explicit camera/microphone selection, up to 60 seconds, or import the first minute of a video. Encode circular 384-pixel H.264 notes, validate output, preview with shared suite playback controls, then explicitly send to the captured chat/topic/reply. Camera and microphone start only on Record. Incoming video notes use the existing thumbnail/download/video viewer path. |
| Voice capture | Shared recording lifecycle handles automatic completion, graceful Stop, failed capture, preview, account changes and cleanup. Preserve files once a Telegram upload starts. |
| Live locations | Native live-location send, coordinate/map cards, manual position updates and confirmed stop, with fresh edit permission/expiry checks. Duration is explicit and updates preserve it. Automatic device tracking, proximity alerts and a consolidated active-sharing browser remain unfinished; the composer explains manual updating. |
| Poll controls | Description, revoting, member-only voting, shuffled answers, hidden results and public regular polls that permit added answers. Add answers only when Telegram reports eligibility, checked again before submission. Show voting restrictions and correct quiz answers when results are visible; keep quiz explanations hidden until then. Remap selected stable answer IDs against the current poll before voting, rejecting removed answers. |

Verification: two targeted payload/coordinate/stable-answer fixtures, one synthetic FFmpeg encode (no camera or microphone), and Pulse/Glimpse compilation. No live recordings, messages, votes or location changes were performed. Hardware capture and account interoperability remain unverified. Poll media, multiple-correct-answer quiz composition, answer deletion and advanced country restrictions remain incomplete.

## Release 3, sixth update: poll media and answer actions

| Feature | Status and behavior |
| --- | --- |
| Attachments | Optional photo, H.264 MP4 video or GIF attachments for poll answers; photos, video, GIFs, audio and documents for the poll and quiz explanation. Collapsible selectors keep optional controls compact. Validate image limits and video encoding off the UI thread; pending validation blocks submission. Local files remain in place for Telegram uploads. |
| Media display | Render poll, answer and visible quiz-explanation attachments with thumbnail previews and the existing media viewer/download path. Incoming link, location and venue attachments have link/map actions. Unsupported media types remain explicit. |
| Answer actions | Fetch current answer permissions before showing actions. Reply with formatted text to the exact answer in its original topic without discarding the composer draft; copy a direct answer link; confirm deletion and check permission again before removing an answer and its votes. Replies to another chat remain unfinished. |
| Quiz and voting | Compose quizzes with multiple correct answers and enable multiple-answer voting accordingly. Channel-only member and country restrictions, normalized country-code input, and eligibility-sensitive added-answer controls. Voter browsers resolve stable answer IDs again before each page, so answer reordering cannot select another answer's voters. |

Verification: two targeted media-schema/validation and country-code fixtures plus Pulse/Glimpse compilation. No live uploads, replies, deletions or votes were performed. Cross-client poll interoperability remains unverified. Location/link/sticker attachment composition, rich custom-emoji poll text, dynamic server option limits and replies to another chat remain incomplete.

## Combined implementation pass: account, desktop, bots, stories, calls and paid workflows

This pass connects a broad set of real TDLib workflows in one installation. These are implementations with local checks, not a claim of complete Telegram Desktop parity or live interoperability.

| Area | Implemented in this pass |
| --- | --- |
| Navigation | Tools entry in the sidebar and settings; conversation/message actions for bots, stories, group calls, boosts, statistics, translation, speech recognition, suggestions and export. Long tool labels wrap; existing bounded menus and modal styling are reused. |
| Account | Edit name, bio, username and static profile photo; search/add/remove contacts; active-session and connected-website revocation; account/session inactivity lifetimes. Profile edits detect concurrent changes before applying. |
| Privacy/security | Editors for twelve privacy scopes, user allow/deny exceptions, preservation of existing additional rules, conflict detection; two-step verification/password removal with an explicit choice, recovery-email verification/resend; registration terms, email login steps, login-code resend and password recovery. Earlier quick privacy buttons no longer discard all exceptions. |
| Notifications | Edit private/group/channel scope mute, previews, mention and pin alerts while preserving other fields; browse exception chats. |
| Downloads/storage/proxies | Paginated download search, pause/resume, source-message navigation and list removal; per-chat/all-cache statistics and confirmed cache removal; SOCKS5/MTProto/HTTP add/edit/enable/delete and latency checks. |
| Export | JSON Lines export of text/native metadata with history pagination, duplicate-boundary filtering, content-protection checks, secret-chat exclusion, a new destination file and cancellation. Attachments are not copied. |
| Bots | Render inline/reply keyboards; text commands, callbacks/password callbacks/game callbacks, copy/user/URL/login buttons, explicit phone/location/poll/user/chat sharing, command browser and paginated inline-bot results with native delivery. Callback data is rechecked against the current keyboard before use. Web apps have an explicit browser handoff; this does not provide the embedded Mini App bridge. |
| Stories | Active-story updates/browser, per-chat stories, own archive, photo/video viewing, 24-hour photo/H.264-video posting, captions, reactions, replies, privacy, profile visibility and confirmed deletion. Rights are rechecked for edits. |
| Group calls | Native NTgCalls group-audio creation/join payloads and connection, initially muted capture, mute/unmute, leave, group-call creation, participant updates, invite links and permission-checked ending. Group and one-to-one transports are mutually exclusive and stop on account switches. This is locally compiled, experimental audio support; no live call or microphone test was performed. |
| Rich messages | Checklist composition/completion/additions; story, invoice, paid-media, game, dice, animated-emoji and expired-media cards; readable service-event summaries; Telegram translation and speech-recognition actions. |
| Statistics/suggestions | Eligible chat/message statistics, asynchronous graph loading and bounded basic line plots; fresh-permission/terms checks before suggested-post approval or decline. |
| Stars/Premium/gifts | Balance and paginated transaction history, subscription cancellation/resumption, Premium offer review, explicit Stars invoice/subscription payment review, fixed-price gift sending, paid reactions and boost-slot assignment. Price/terms checks and explicit confirmations precede spending; ambiguous write outcomes disable blind retries. Card tokenization/checkout remains unavailable. |
| Business | Address, basic weekly opening hours, greeting/away automation with recipients, text start page, quick-reply creation/append/send/delete, chat-link creation/copy/delete, and connected-bot inspection/revocation. Existing advanced settings that this editor replaces are disclosed before saving. |
| Location | Device-coordinate fill and explicit automatic updates of outgoing live shares through GeoClue, active-share browser, disable/stop controls, expiry preservation, finite/range/age validation and account-switch cleanup. GeoClue is absent on this machine. System installation requires a sudo password unavailable to this session, so the installed application retains manual location sharing and reports provider failures. |
| State isolation | New requests use account-generation checks; per-account story/quick-reply/member/location state is reset. Existing custom-emoji downloads now recheck the generation after downloads, preventing stale account data from repopulating caches. |

Verification: Pulse/Glimpse compilation, 24 Pulse tests including the isolated GTK layout/media/draft and new checklist/keyboard regression, and 9 telegram-core tests. No account mutations, real messages, payments, location fixes, microphone capture or live calls were performed. Existing headless GTK binding/deprecation warnings remain in the prior regression path. Installed without opening or restarting Pulse.

## Latest combined update: extended workflows

This section supersedes the earlier remaining-work list. Implemented and installed together on 2026-10-20; this is not a claim that every Telegram Desktop feature or server/account combination has been verified.

| Area | Added |
| --- | --- |
| Account/security | Paginated profile-photo history with selection/deletion; birthday and verified phone-number changes; hardware FIDO2 passkey creation, sign-in and removal; session acceptance and call/secret-chat permissions. Passkey PIN/challenge data travels on stdin rather than process arguments. |
| Privacy/notifications | Named user/group exception pickers; Premium, bot and contact rule controls; per-chat inherited/custom notification settings, saved sounds and explicit indefinite mute. Existing settings are retained where applicable. |
| Desktop | Text scaling, stronger outlines, configurable shortcuts, separate conversation windows with independent history/info panes and shared native widgets, self-contained HTML/JSONL exports with optional copied media and cancellation. Reduced-motion preferences stop automatic animation playback. |
| Mini Apps | Isolated ephemeral WebKit window, origin-checked bridge, theme/viewport/buttons/popups/links, explicit phone/write/clipboard/location/device permissions, inline switching, invoices, native cloud/custom requests and confirmed file downloads. Native launches close on browser exit, crashes and account cleanup. |
| Bots | Managed-bot creation requests; inline destination/query prefill with destination-type checks; inline media previews; force-reply handling that preserves an existing draft; one-time keyboard consumption for text/composer and deferred phone/location/poll/user/chat sharing. Sharing dialogs use named selections, disclose requested bot access and recheck the request before submission. |
| Calls | Group camera/video reception, portal screen sharing, video source/tile cleanup, participant moderation, hand and volume controls; native timestamp/segment callbacks for RTMP/live-story playback and live-story joining. Call capture is never started by the installer or local checks. |
| Stories | Paginated interactions and albums, album creation/rename/order/add/delete, media replacement, stealth mode, interactive publication areas (links, locations, reactions, messages, weather and gifts), selectable lifetimes, album IDs, video-cover timestamp and conflict-checked editing of existing interactive areas. Telegram checks Premium eligibility and publication limits. |
| Payments | Regular provider checkout through HTTPS providers, Stripe Elements or SmartGlocal tokenization; order/shipping validation, saved credentials, tips, currency exponent formatting and final explicit payment review. Edited order details invalidate validation; refreshed invoice terms are compared before submission. Card details stay in the provider/helper process; Pulse receives payment tokens. Payment receipts, paid channel invitation creation/rename and paginated Star subscriptions are included. Refunds are a bot-only TDLib API and are not exposed as a user-account refund action. |
| Gifts | Paginated owned gifts and resale listings; visibility, conversion, upgrade, transfer and resale; gift collections; auction bids and Premium gifting with Stars. Spending/transfer operations require review and fresh eligibility/price checks. |
| Business | Full weekly multiple/overnight intervals; bot connection/disconnection and individual rights/recipient controls; custom away periods; preserved/replaced/removed start-page stickers; rich quick-reply creation, rename, media/message editing, deletion and shortcut ordering; Business link editing with conflict checks. |
| Messages/polls/location | Media replacement, rendered speech-recognition results, suggested-post offer composition, confirmed sender-wide moderation deletion; poll links/locations/venues/stickers, Markdown/custom-emoji text and replies to answers in another chat; live-location proximity distance controls. GeoClue 2.8.2 is installed. |

Verification: clean Pulse/Glimpse compilation, 27 Pulse tests and 9 telegram-core tests, Python helper compilation and an isolated local WebKit bridge round trip. The GTK regression covers layout/media/draft widgets and keyboard/shortcut behavior. The continuation passed two targeted GTK/poll-media checks, including bounded sticker validation, and another isolated WebKit bridge round trip. No live messages, account changes, payments, key operations, microphone/camera capture, location fixes or remote calls were performed. Existing headless GTK binding/deprecation warnings remain. The update installs the binary, native libraries, launcher, icon and Glimpse plugin without opening or restarting Pulse.

## Remaining limits

- Live cross-client/account verification is still required for group/broadcast transports, provider checkout, passkeys, story/gift/Business permissions and GeoClue fixes. Local checks do not prove those server-dependent paths.
- Complete Telegram Desktop parity remains broader than this update: conference-specific controls, live-stream production, QR/secure-device-storage Mini App APIs, advanced crop/paint editing, full complex administrator events/graphs, gift crafting/withdrawal/TON account flows and some less common native message types remain incomplete.
- Some advanced editors still use IDs or Unix timestamps. Detached windows currently target a conversation; separate forum-topic windows are not implemented. Mini Apps implement a subset of Telegram's bridge, and platform capabilities outside that subset may not work.
- Keyboard sharing, Mini App custom/cloud requests and story-area edits still need cross-client verification. Hardware biometric and motion-sensor requests explicitly report unavailable on this desktop.

Telegram remains authoritative for account eligibility, permissions, limits and prices. Ambiguous spending or message-write outcomes must not be blindly retried.

## UI polish and final reliability pass

- Matched info-header/body insets, wrapped long menu/form labels, associated field labels, bounded loading errors inside the tool scroller and added full-title/status tooltips.
- Added a visible composer focus boundary, compact selection controls and a responsive composer. Formatting and mentions remain available through the attachment menu when their toolbar buttons are hidden.
- Notification toggling in the info pane now changes only mute settings, preserving custom sounds, previews and other exceptions.
- Traced the previous GObject critical warnings to NTgCalls exporting its bundled GLib/GObject symbols into GTK's global linker scope. Replaced direct linking with cached, local/deep-bound loading of the pinned C ABI. Missing libraries/symbols produce a call-engine error. The library stays loaded while callbacks/handles may exist.
- Updated shared theme negotiation to use GTK's current color-scheme property when available, retaining compatibility with earlier GTK versions.

Validation: 27 Pulse tests, 9 telegram-core tests and a hardware-free native protocol/symbol-isolation test passed. The final 800-pixel chat/info/composer rendering check passed with `G_DEBUG=fatal-warnings`, including a fresh native-library load before GTK initialization. The synthetic preview is `/tmp/pulse-ui-audit.png`. No real chats/accounts or capture devices were used. This confirms the linker conflict and tested rendering paths are fixed; it does not establish that every live chat or server-dependent workflow is crash-free. Installed without opening or restarting Pulse.
