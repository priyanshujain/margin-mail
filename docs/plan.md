# Plan

How the app gets built, in the order it gets built, and what it is built out of. The product is
specified in [design.md](design.md) and [features.md](features.md); this document is about the
work rather than the result, and it exists so that somebody picking the repository up in six
months can see why the pieces landed in the order they did.

Work is cut into milestones, and a milestone into work packages. A package is a unit somebody can
finish: it owns a set of files, it ends with something that runs, and it never edits another
package's files. The package names below are the ones the source comments already use, so a
placeholder that says "the contract lands in F3" means the work package named here.

## The milestones

**M0, foundations.** F1 is the scaffold: the Tauri crate, the Vite front end, the icons, the
justfile and CI. F2 is the design system, which is `src/styles/tokens.css` and the primitives that
sit on it, reviewed through the Kit page. F3 is the contracts: `src-tauri/src/dto.rs` and its
mirror `src/ipc.ts`, frozen before anything implements them. F4 is the documentation, this file
among it. None of M0 sends a byte to Google, and that is the point: the shape is settled while it
is still cheap to change.

**M1, read.** The milestone that proves the two hard things. R1 is authentication and accounts:
the loopback flow, the sealed token, the granted scopes. R2 is the Gmail client behind the
`Provider` trait, with the quota arithmetic and the backoff in it. R3 is the mirror and the sync
loop, including the storage window and eviction. R4 is the message pipeline: the MIME parse, the
sanitiser, the tracker stripper. R5 is the shell, which is the header, the list column, the
reading pane and the keyboard. R6 is connect and onboarding, the first thing a new user meets and
the last thing built in this milestone, because it cannot be designed honestly until the sync it
narrates exists. At the end of M1 the app reads mail and does nothing else.

**M2, triage.** T1 is flags and undo: seen, star, archive, trash, spam, each optimistic and each
reversible. T2 is selection and bulk actions. T3 is search, local over the window with the
provider as the second pass. T4 is labels. At the end of M2 it is a fast Gmail client and nothing
more, which is worth having in the hands of one user for a week before the next milestone changes
what it is.

**M3, places.** P1 is the state database and its journal, the schema that everything after this
depends on. P2 is routing: sender rules, the suggestion function, the overrides. P3 is the
Screener and the first-run pass. P4 is the Feed and the Paper Trail. P5 is contacts, the contact
card and autocomplete. This is the milestone where it stops being a Gmail client.

**M4, piles.** L1 is Reply later, Set aside and Focus & Reply. L2 is snooze with lazy evaluation.
L3 is notes, rename and merge. L4 is the rest of what the state database makes possible: clips,
All files, ignore, per-thread notifications and unsubscribe.

**M5, writing.** W1 is the editor and drafts. W2 is the send pipeline: the outbox, the undo delay,
attachments, threading headers. W3 is instant intro, remind me if no reply, and calendar invites
including the re-authorization that RSVP needs.

**M6, accounts, settings and backup.** A1 is more than one account and the unified view. A2 is the
settings screen, specified in [settings.md](settings.md). A3 is the backup store, the encryption
and the recovery phrase, with the journal merge behind it.

**M7, ship.** S1 is the release pipeline: signing, notarisation, the updater. S2 is the
verification materials Google's restricted scope review wants, which is a privacy policy that is
true, a demo video, and a written justification for each scope. Then the platforms in order: S3
the iPhone, S4 Linux, S5 the IMAP provider. They are last because each one is a second copy of a
problem already solved once, and solving it twice before it is solved once is how a project stalls.

## What comes from the siblings

Very little here is new, and that is deliberate. Margin Mail sits beside margin and Margin
Calendar on disk and takes from both.

From the calendar: the OAuth loopback flow with PKCE and its Google-specific handling, the sealed
token store, the deep-link path that mobile needs instead of a loopback listener, the overlay
title bar and the macOS window behaviour, the `data-phone` and `data-touch` scheme with the boot
script that sets them before first paint, the escape-layer stack, the palette, the zustand store
idiom, the release workflow's shape, and the `build.rs` trick that embeds
`google-credentials.json` with the example file as a fallback.

From margin: the HTTP half of `gdrive.rs`, which is folder lookup, upload and download against
Drive's v3 API; the trick of putting heavy synchronous work behind `#[tauri::command(async)]`; and
`margin-shared`, which is a real dependency rather than a copy. The tokens, the icon strings and
the font catalogue come from that package through a relative path in `package.json`, which is why
CI checks out both repositories side by side.

From neither: the mirror, the state database, the journal, the sanitiser and everything to do with
mail. Those are this repository's own work.

## The libraries

Every version below was checked against crates.io and npm on 3 September 2026.

On the Rust side, `mail-parser` 0.11 with `full_encoding` parses bodies from the raw RFC 2822
bytes; the feature is not optional, because the charsets it adds are the ones that still turn up
in real mail every day. `mail-builder` 0.5 builds outgoing MIME. `css-inline` 0.21 folds the
editor's stylesheet into the markup before the message is built, so a client that drops `<style>`
still renders what was written. `ammonia` 4.1 is the sanitiser, and two of its hooks do the work
that matters: `attribute_filter` rewrites `img src`, which is where tracker stripping and `cid:`
substitution happen, and `filter_style_properties` narrows inline CSS to an allowlist of
properties that can never take a `url()`, which closes the last route a message has to fetch
something.

`rusqlite` 0.40 with `bundled`, so FTS5 is compiled in rather than depending on what the
platform's libsqlite3 happened to be built with, and so several accounts sharing one process share
one predictable SQLite. `reqwest` 0.13 with `gzip`, `json` and `http2`: Gmail's JSON compresses by
an order of magnitude and hydration is thousands of responses, and a batch of fifty shares a
connection with the poll loop. Note that its TLS feature is now called `rustls` rather than
`rustls-tls`; the old name is a build error, not a warning. `chacha20poly1305` 0.11 seals tokens
and backup segments, `argon2` 0.6 derives the backup key from the recovery phrase slowly enough to
matter, `bip39` 2.2 produces the phrase, and `chrono` and `fontdb` do the obvious.

Two deliberate omissions. There is no `oauth2` crate: the calendar's hand-rolled flow is already
tested against Google's actual behaviour, including the parts that do not match the spec, and
replacing working code with a dependency that has to be taught the same lessons is not a trade.
There is no `tokio-rusqlite`: it pins an older rusqlite than the one above, and the synchronous
command trick makes it unnecessary anyway.

On the front end, `@tiptap/react` 3.31 with StarterKit is the editor, as in margin. `react-virtuoso`
4.18 renders the grouped list, because a mailbox list is long, its rows are two heights, and it has
group headers, which is exactly the case hand-rolled virtualisation gets wrong. Beyond those,
React, zustand and the Tauri API, and nothing else.

One consequence worth writing down. Message bodies render in an iframe with `srcdoc`, which
inherits the app's content security policy rather than escaping it, so the frame cannot fetch
anything: inline `cid:` images are rewritten to `data:` URIs by the sanitiser, and a remote image
the user has chosen to allow is fetched by Rust, without cookies or referrer, and handed to the
frame the same way. Every byte a message displays has been through Rust first.

The sandbox attribute is `allow-same-origin` rather than empty, and the reason is worth keeping.
An empty sandbox gives the frame an opaque origin, and a document the parent cannot reach is a
document the parent cannot measure: there is then no way to size the frame to its content, so every
message carries its own scrollbar, and no way to catch a click on a link, so nothing opens. Keeping
`allow-same-origin` leaves every other restriction in place, forms and top navigation included, and
scripts are still blocked three times over: the sandbox disables them without `allow-scripts`, the
content security policy is `script-src 'self'` which stops inline scripts and inline event handlers
whatever the origin, and the sanitiser removed them before either got a say.

## The shape of the repository

The front end is `src`. Screens live in `src/screens`, the primitives they are built from in
`src/ui`, one zustand store per domain in `src/store`, the typed IPC wrappers in `src/api`, and
the stylesheets in `src/styles`, where `tokens.css` is the only file allowed to hold a raw colour.
`src/ipc.ts` is the frontend half of the contract and `src/screens/Kit.tsx` is the page that
renders every primitive in every state, which is how a restyle gets reviewed.

The backend is `src-tauri/src`. `dto.rs` is the other half of the contract and is frozen once M0
ends. `lib.rs` holds the app setup, the menu and `emit_store_changed`. The Google client, the
mirror, the state database and the sync loop each get a module, and the Gmail specifics stay
inside the Gmail one so that the second provider has somewhere to be.

Documentation is `docs`, with the mockups under `docs/mockups`: HTML sources in `src` beside a
shared `mail.css`, rendered to PNGs by `docs/mockups/render.sh`, which pulls the fonts from the
margin repository and Chromium from the calendar's `node_modules` so nothing binary is vendored
here. The prose gate is `scripts/docs-check.mjs`. Local builds and installs are the `justfile`;
CI and releases are the two workflows in `.github/workflows`.

## How the work is verified

Four gates, all of them runnable on a laptop before anything is handed over.

`just test` runs the Vitest suites and `cargo test`. Both must be green; there are no retries
anywhere in this repository, because a test that passes on the second attempt is lying about
something.

`just test-ui` runs Playwright against the real UI, with `src/ipc.ts` routed to the dev fixture so
no browser ever talks to Google. The viewport is pinned to 1440 by 900, the same size the mockups
were rendered at, because half of what the suite asserts is geometry: the 420 px list column, the
row height, whether the piles are on screen. It also writes screenshots as it goes, starting with
the Kit page in both palettes under `screenshots/`, so a visual regression shows up as a diff
rather than as a complaint two weeks later, and it greps the stylesheets for a hex literal outside
the token layer, which is the one rule a reviewer will not reliably catch by eye.

`just docs` is the prose gate: no em dashes, no directory trees, no relative link that goes
nowhere.

`pnpm build` is `tsc` then Vite, so the typecheck and the bundle are one step, and
`pnpm fonts:check` catches the vendored font copy under `public/fonts` drifting from the
`margin-shared` package. CI runs all of it, checking out this repository and margin side by side
because the shared package is reached by a relative path.

Beyond the gates, the mockups in [ui.md](ui.md) are the target rather than an impression of it.
A screen is finished when it looks like its render, not when it looks reasonable.

## What was learned from reading Mailspring

Mailspring is GPL-3.0 and was read, not copied. Five things in it are worth having and are in this
design because of it. Bodies live in their own table with a `fetched_at` column, prefetched inside
a window and evicted outside it, which is where the storage window in
[architecture.md](architecture.md) comes from. The engine hands the UI a typed delta stream rather
than telling it to refetch, which here is the `store-changed` event with a scope in it, so a note
landing does not make the list reload its bodies. The provider's thread id and the `References`
threading are kept as separate columns rather than collapsed into a hash of the headers, because
they disagree often enough to matter. An account that fails to sync repeatedly is paused by a
circuit breaker instead of being retried into a rate limit. And SQLite gets a busy timeout,
because several accounts share one process and one connection.

The list of what to avoid is just as useful. Nothing may depend on the app running at a particular
wall-clock time, which is why snooze is evaluated lazily. No metadata is held in a cloud service.
No tracking pixels, no rewritten links, and no contact lookup that sends a correspondent's address
to a server to find out who they are. And no header that never expires: everything cached has a
rule for when it goes.
