import { useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isTauri, type Surface } from "../ipc";
import { useTheme } from "../store/useTheme";
import { notify } from "../store/useToast";
import { Pill } from "../ui";

/**
 * One message's body, in a sandboxed iframe.
 *
 * The frame is here for style isolation. Mail HTML sets global styles aggressively, and rendering a
 * newsletter in the app's own document lets it restyle the list beside it.
 *
 * Scripts cannot run in it. `sandbox` without `allow-scripts` disables them; the content security
 * policy in src-tauri/tauri.conf.json is `script-src 'self'`, which independently blocks inline
 * scripts and inline event handlers; and the sanitiser in Rust has already removed them. Three
 * layers, and the sanitiser is only the first.
 *
 * `sandbox=""` would be tighter and it is what an earlier draft called for, but it makes the
 * frame's origin opaque, and a document the parent cannot reach is a document the parent cannot
 * measure. There is then no way to size the frame to its content and no way to catch a click on a
 * link. `allow-same-origin` keeps every other restriction, forms and top navigation included.
 *
 * So the parent does the two jobs the frame cannot. A ResizeObserver on the frame's
 * documentElement sets the height, so the pane scrolls as one page rather than each message
 * carrying its own scrollbar. A click listener on the frame's document catches anchors, calls
 * preventDefault and hands the URL to the system browser: the href it opens is the one Rust
 * already cleaned.
 *
 * The token stylesheet has to be injected as well, or a message renders in Times. That stylesheet
 * is `public/message.css`, and the values it reads are set as custom properties on the frame's root
 * from the app's own computed tokens, so a body follows the theme and the text size without either
 * side holding a colour of its own.
 *
 * It is fetched once and written into the document rather than linked from it, because a linked
 * sheet arrives after the frame has already painted and a page of prose that resets its line breaks
 * a moment after you started reading is worse than one that arrives a beat late. Same reasoning as
 * `font-display: block` in the shared fonts sheet.
 */
export interface MessageBodyProps {
  html: string;
  /** Transactional mail, which reads in the interface face because it is data rather than prose. */
  plain?: boolean;
  /**
   * Which surface this body reads on, decided in Rust from what the sender painted.
   *
   * `paper` pins the light palette in both themes, for a message that laid out a page of its own:
   * a newsletter's wash behind a card, a receipt's tinted wrapper. `theme` is everything else, and
   * everything else is most mail. Rust has already taken the author colours that would be
   * unreadable on our own paper off a `theme` body, so what is left inherits ours.
   */
  surface?: Surface;
}

/**
 * A body the mirror does not have yet, in the shape of the paragraph it is going to be.
 *
 * `thread_view` is a local read and never waits on the network, so a message whose body has not
 * been fetched comes back with `bodyPending` set and nothing to render. Empty paper reads as a
 * message with nothing in it; three bars that breathe read as a message on its way, which is the
 * one thing Mailspring does that this app was asked to do too.
 */
export function BodySkeleton() {
  return (
    <div className="msg-pending" role="status" aria-label="Fetching this message">
      <span />
      <span />
      <span />
    </div>
  );
}

/**
 * The same slot once the fetch has come back with nothing in it: one line, and the way to ask
 * again. The alternative was the bars above breathing until the thread was closed, because a
 * provider refusing every body is a count of zero to the command and not a failure.
 */
export function BodyMissing({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="msg-pending" data-state="error" role="status">
      Could not fetch this message.
      <Pill tone="quiet" onClick={onRetry}>
        Try again
      </Pill>
    </div>
  );
}

/** The small set a message body needs. Anything else is the sender's business, not ours. */
const TOKENS: Record<string, string> = {
  "--m-font-text": "--font-heading",
  "--m-font-ui": "--font-ui",
  "--m-size": "--body-size",
  "--m-size-plain": "--t-3",
  "--m-ink": "--ink",
  "--m-faint": "--ink-faint",
  "--m-line": "--line",
  "--m-wash": "--accent-wash",
  "--m-measure": "--measure",
  "--m-paper": "--paper",
};

/** The same set for a message that painted its own page, pinned to the light palette in both. */
const PAPER_TOKENS: Record<string, string> = {
  ...TOKENS,
  "--m-ink": "--message-ink",
  "--m-faint": "--message-faint",
  "--m-line": "--message-line",
  "--m-wash": "--message-wash",
  "--m-paper": "--message-paper",
};

/** Fetched once for the whole app. The browser would cache it anyway; this caches the parse too. */
let sheet: Promise<string> | null = null;

/**
 * The same text once it has arrived, readable without waiting.
 *
 * A promise, however warm, is still a render with nothing to put in the frame, and a frame with
 * nothing in it is a navigation: every message body after the first would load an empty document
 * and then load itself over the top of it. One thread of eight messages is eight of those.
 */
let sheetText: string | null = null;

function messageCss(): Promise<string> {
  sheet ??= fetch("/message.css")
    .then((response) => response.text())
    .catch(() => "");
  return sheet.then((text) => {
    sheetText = text;
    return text;
  });
}

/**
 * The app's values, as a rule the frame's own stylesheet can read.
 *
 * A rule rather than a style attribute, because a font stack is `"Literata", Georgia, serif` and
 * the first of those quotes ends an attribute and silently takes every property after it with it.
 *
 * `--m-color-scheme` is the one value here that is not a token lookup, because it is not a colour:
 * it is what the document inside the frame tells the browser about itself. On a pinned page it is
 * `only light`, and that half is the one nobody ships. Without it a body we have painted white
 * still gets the user agent's dark canvas and dark form controls under it whenever the machine is
 * in dark mode, and white text on white is the result. On the theme it is simply ours.
 */
function rootRule(surface: Surface, theme: string): string {
  const computed = getComputedStyle(document.documentElement);
  const pinned = surface === "paper";
  const values = Object.entries(pinned ? PAPER_TOKENS : TOKENS)
    .map(([name, token]) => `${name}:${computed.getPropertyValue(token).trim()}`)
    .join(";");
  return `:root{${values};--m-color-scheme:${pinned ? "only light" : theme}}`;
}

export function MessageBody({ html, plain, surface = "theme" }: MessageBodyProps) {
  const frame = useRef<HTMLIFrameElement | null>(null);
  const theme = useTheme((s) => s.theme);
  const [css, setCss] = useState<string | null>(sheetText);

  useEffect(() => {
    if (sheetText !== null) return;
    let live = true;
    void messageCss().then((text) => {
      if (live) setCss(text);
    });
    return () => {
      live = false;
    };
  }, []);

  // Read once per palette rather than once per property, and again when the palette changes under
  // an open thread.
  const tokens = useMemo(() => rootRule(surface, theme), [theme, surface]);

  const srcdoc = useMemo(
    () =>
      css === null
        ? ""
        : `<!doctype html><html><head><meta charset="utf-8"><style>${tokens}\n${css}</style></head><body${plain ? " data-plain" : ""}>${html}</body></html>`,
    [css, html, plain, tokens],
  );

  useEffect(() => {
    const el = frame.current;
    if (!el || !srcdoc) return;
    let observer: ResizeObserver | null = null;
    // Which document the listener went on. Changing `srcdoc` leaves the old one in place for a
    // moment, so taking it off again means remembering the one it was added to.
    let attached: Document | null = null;

    const onClick = (event: MouseEvent) => {
      const target = event.target as HTMLElement | null;
      const anchor = target?.closest?.("a[href]") as HTMLAnchorElement | null;
      if (!anchor) return;
      event.preventDefault();
      const href = anchor.getAttribute("href");
      if (!href || href.startsWith("#")) return;
      if (isTauri) void openUrl(href).catch((e) => notify(`Could not open that link: ${String(e)}`));
      else window.open(href, "_blank", "noopener,noreferrer");
    };

    /** Whether there was a document there to take. */
    const attach = (): boolean => {
      const doc = el.contentDocument;
      // A document whose parser has not reached the body yet has nothing to measure and nothing to
      // observe, and `ResizeObserver.observe(null)` throws hard enough to take the screen with it.
      if (!doc?.body) return false;
      observer?.disconnect();
      // The height is written on the element rather than held in state: a body that reflows while
      // its images decode would otherwise re-render the whole thread on every frame.
      const size = () => {
        // The body rather than the documentElement. An iframe's root element never reports less
        // than the frame's own height, so measuring it floors every short message at whatever the
        // frame happened to be and leaves a band of blank paper under a two line note.
        //
        // Zero is not a height, it is a document that has not laid out yet, and writing it back
        // used to end the conversation: the root element is bounded by the frame, `html` here is
        // `overflow: hidden`, and a frame set to a pixel can never change size again, so the
        // observer was never asked a second time and the message stayed a sliver. It only ever
        // happened on a loaded machine, which is what made it look like a flaky test.
        const height = doc.body.scrollHeight;
        if (height > 0) el.style.height = `${height}px`;
      };
      observer = new ResizeObserver(size);
      observer.observe(doc.documentElement);
      // And the body, because that is the box that grows when the content finally arrives.
      observer.observe(doc.body);
      size();
      attached?.removeEventListener("click", onClick);
      doc.addEventListener("click", onClick);
      attached = doc;
      return true;
    };

    // The load is the backstop and not the moment. It waits on the two faces the sheet declares,
    // which are `font-display: block` and can be a whole second on a cold cache, and a message that
    // is not sized until then is a message that jumps under the reader. The body is laid out long
    // before that, so the frame is taken the moment it has one and on the frame after this one if
    // it does not yet. Reading a readyState instead would not help: every document a frame passes
    // through on the way to this one reports itself complete as readily as this one does.
    let waiting = 0;
    const take = () => {
      if (!attach()) waiting = requestAnimationFrame(take);
    };

    el.addEventListener("load", attach);
    take();

    return () => {
      cancelAnimationFrame(waiting);
      el.removeEventListener("load", attach);
      observer?.disconnect();
      attached?.removeEventListener("click", onClick);
    };
  }, [srcdoc]);

  return (
    <iframe
      className="msg-frame"
      ref={frame}
      title="Message"
      // Also on the element, because what a child document is told about `prefers-color-scheme` is
      // decided by whatever embeds it and not by its own root. pane.css is where that is written.
      data-surface={surface}
      sandbox="allow-same-origin"
      srcDoc={srcdoc}
    />
  );
}

export default MessageBody;
