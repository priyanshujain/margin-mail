# App Store screenshots

Run `node scripts/appstore-screenshots.mjs` from the repository root to render the promotional
HTML frames in `src/`. Playwright Chromium must already be installed.

The four PNGs show the Inbox, Screener, Feed and private context in the dark palette at 2560 x 1600.
Like the sibling Margin listing, each frame places HTML app markup below a headline and caption.
The markup uses sample mail and the app's established mockup styles, with local bundled fonts.
Edit the HTML and shared CSS, rerender, and inspect every PNG. No app, server or mailbox is opened.

Review the depicted UI against the signed sandboxed app before upload.
