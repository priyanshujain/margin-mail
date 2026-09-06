# Conventions

This project is a sibling to margin and Margin Calendar and follows their conventions deliberately
rather than inventing new ones. When something here is unclear, the answer is almost always "do
what the calendar does", and the file to look at is named below.

## Rust

`Result<T, String>` everywhere. No `anyhow`, no custom error enum except
`provider::ProviderError`, which exists only because the sync engine has to branch: a revoked
token, a missing scope, a 429 to back off from, and a 404 from `history.list` that means the
change log expired and a full list is the answer rather than a failure.

DTOs crossing the IPC boundary live in `src-tauri/src/dto.rs` and are marked
`#[serde(rename_all = "camelCase")]`. That file is the contract and is frozen: implementation
modules add bodies, not fields. Its mirror is `src/ipc.ts`.

Read Google's responses through the ported `read_json`, which takes the body to a `String` first
so the error payload survives into the message rather than becoming "expected value at line 1".
It is the calendar's function and it lives with the Gmail client.

Heavy synchronous work goes behind `#[tauri::command(async)]` on a synchronous fn, which is
margin's trick in `pdf.rs` for getting off the main thread without hand-writing `spawn_blocking`.
Every command that touches SQLite qualifies.

Provider-specific behaviour stays inside the provider's module. Nothing above
`src-tauri/src/provider/` may know what a Gmail label id looks like, and nothing outside the
mirror may know that a thread has a provider id at all. The sync engine talks to the trait, which
is what lets `provider::fake` drive the whole engine in `cargo test` without credentials.

Comments are rare and explain why, never what. Match the density in `lib.rs`.

## TypeScript

One zustand store per domain in `src/store/`. No middleware. One selector call per field
(`useThing((s) => s.field)`, never a destructured object), actions as inline arrow properties, and
`set((s) => ...)` returning `{}` to no-op.

Async actions use a string phase union (`"idle" | "syncing" | "error"`), never boolean loading
flags. Errors stringify with `String(e)` and surface as a toast.

Side effects that touch disk, the DOM or Tauri live in a sibling module, never inside the store.

The OAuth connect flow in `src/store/useAccounts.ts` reuses margin's
promise-holding-its-own-resolver pattern from `useBackup.ts`: `connect()` returns a promise whose
`resolve` is stashed in state for a later Tauri event to settle.

Typed IPC wrappers live in `src/api/`, one module per domain, one thin function per command. They
are written once against the frozen contract; add bodies to Rust, not new wrappers.

## The design system

Three layers, and the rule is that each may only reach down.

Tokens are `src/styles/tokens.css`, which is a seam rather than a list: it imports the set
margin-shared holds for all three apps, then `src/styles/mail.css` for what mail adds on top.
Every colour, radius, size, duration and font stack is in one of those two, and nothing else in
the app may declare a token.

Primitives are `src/ui/`: the button, the keycap, the row, the avatar, the panel, the popover, the
toast. They read tokens and nothing else, and every one of them appears in every state on the Kit
page at `#/kit`, which is how a restyle gets reviewed.

Screens are `src/screens/`. A screen composes primitives and may never write a colour, a radius or
a size. If a screen needs a value that is not available to it, the answer is a new token or a new
primitive, not a literal.

## Places and stages

A `Place` in the frozen contract is a query over threads, and three of the palette's entries are
not that: Contacts lists people, Clips lists passages, All files lists attachments. Focus & Reply is
a fourth, a page over the Reply later pile rather than somewhere you can be. None of them belongs in
`Place`, and none is an overlay either, because an overlay is something you dismiss to get back to
what you were doing and these are somewhere you go.

So they are a stage, in `src/store/useStage.ts`, and the rule is that a stage wins over a place:
opening Contacts leaves the Inbox where it was and Escape puts you back on it. The current place
and the current stage are both on the root element as `data-place` and `data-stage`, which is what
a test reads and what a stylesheet keys off, so neither has to ask the app what it thinks it is
showing.

## CSS

Flat kebab-case class names, not BEM. State is a `data-*` attribute, never an `is-` class.

Every colour, radius and size goes through a token. If a value is not in the token layer, add it
to `src/styles/mail.css` rather than writing a literal.

Dark mode is `data-theme` on `<html>`, with both palettes defining an identical variable set.
Never a media query for theme.

Transitions name explicit properties and use `var(--ease)`. Never `transition: all`.

Responsiveness is JS-driven. `usePhone()` and `useTouch()` in `src/useMedia.ts` write `data-phone`
and `data-touch` on the root, and styles read those attributes rather than adding media queries.
They answer different questions. `data-phone` is a window too narrow for the desktop chrome and it governs
layout; `data-touch` is a coarse pointer and it governs interaction. A tablet is touch and not a
phone, a narrow desktop window is a phone and not touch, and treating either as a proxy for the
other is how a hover-only control ends up unreachable. Both are also set by the boot script in
`index.html`, so the first paint is already the right shape.

A rule that reads "you cannot hover here" belongs on `data-touch`. A rule that reads "there is no
room for this" belongs on `data-phone`.

There is no container query in this repository. The calendar has exactly one, on the event block,
and it earned it: what decides how many lines of a title fit is the block's own width and not the
window's. Nothing here has met that bar yet, and nothing may reach for one without the same kind
of reason.

Overlays follow margin's `.overlay` and `.panel` idiom, which is in `src/styles/app.css`, and
every one of them registers with `useEscapeLayer` from `src/escape.ts` so Escape unwinds the
layers in order.

## Icons

Feather-style 24x24 stroke `d` strings, named in `src/ui/icons.ts` and passed to
`<Icon d={...} />`. The handful of glyphs the whole suite shares are re-exported from
`margin-shared/icons` so a search here and a search in the calendar are the same drawing; the
verbs and the piles are mail's own. There is no icon set and no registry, and there will not be
one. An icon-only button always carries a `title` with its shortcut written in real glyphs.

## Storage keys

Anything in `localStorage` is prefixed `marginmail-`, following margin's convention: the theme is
`marginmail-theme` in `src/theme.ts`, and the fonts, the text size and the reading pane follow the
same shape. Keys read before first paint are restored by the blocking IIFE in `index.html`, which
is why they are flat strings rather than one blob.

## Work packages

A work package owns a set of files and never edits another package's files. When a package needs
something that lives in another one, the answer is to agree the interface up front (which is what
`dto.rs` and `src/ipc.ts` are for) and stub behind it, not to reach across. A package that has to
edit somebody else's file was cut in the wrong place.

## Never

No CSS framework, no component library, no router, no zustand middleware, no directory trees in
any document, and no em dashes anywhere including code comments.
