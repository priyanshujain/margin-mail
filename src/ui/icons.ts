// Feather-style 24 unit paths for a 1.6 stroke, passed to `<Icon d={...} />`.
//
// There is no icon set and there will not be one. A set is several hundred kilobytes for the
// handful of shapes a mail client needs, every one of these is a few dozen bytes, and a path is a
// design decision rather than a detail worth outsourcing.
//
// The four glyphs the family already agreed on come from margin-shared, so a search here and a
// search in the calendar are the same drawing. Everything below the re-exports is mail's own:
// the verbs, the piles and the chrome no other app in the suite has.
//
// Each constant is a single `d`, which means the circles are written as arc pairs rather than as
// `<circle>` children. That keeps `Icon` to one prop.

export { SEARCH, CLOSE, CHECK, MORE, MOON } from "margin-shared/icons";

/* Places and chrome */

export const INBOX = "M3 13h5l2 3h4l2-3h5M5 5h14l2 8v6H3v-6z";
export const PLACES = "M4 6h16M4 12h16M4 18h10";
export const PEN = "M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z";
export const PLUS = "M12 5v14M5 12h14";
export const BELL = "M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9M13.7 21a2 2 0 0 1-3.4 0";
export const SHIELD = "M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6zM9 12l2 2 4-4";
export const BOLT = "M13 2 4 14h7l-1 8 9-12h-7z";

/* Chevrons, in the four directions the app actually points */

export const CHEVRON_DOWN = "M6 9l6 6 6-6";
export const CHEVRON_UP = "M6 15l6-6 6 6";
export const CHEVRON_LEFT = "M15 18l-6-6 6-6";
export const CHEVRON_RIGHT = "M9 6l6 6-6 6";

/* The verbs */

export const ENVELOPE = "M4 6h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM3.3 6.7 12 13l8.7-6.3";
export const REPLY = "M9 17 4 12l5-5M20 18v-2a4 4 0 0 0-4-4H4";
export const REPLY_ALL = "M8 17 3 12l5-5M13 17l-5-5 5-5M21 18v-2a4 4 0 0 0-4-4H8";
export const FORWARD = "M15 17l5-5-5-5M4 18v-2a4 4 0 0 1 4-4h12";
export const ARCHIVE =
  "M4 4h16a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8M10 12h4";
export const TRASH =
  "M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13M10 11v6M14 11v6";
export const SPAM = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7.5v6M12 16.6v.01";

/** The clock, which is Reply later on a pile and the plain hour elsewhere. */
export const CLOCK = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2";
/** The clock with two lines above it, which is the alarm every mail client draws for snooze. */
export const SNOOZE = "M5 3 2 6M22 6l-3-3M12 5a8 8 0 1 0 0 16 8 8 0 0 0 0-16zM12 9v4l2 2";

export const NOTE = "M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6";
export const STAR = "M12 3.5l2.6 5.6 6 .8-4.4 4.2 1.1 6-5.3-3-5.3 3 1.1-6L3.4 9.9l6-.8z";
export const PAPERCLIP =
  "M21 11l-8.5 8.5a5 5 0 0 1-7-7L14 4a3.3 3.3 0 0 1 4.7 4.7L10.5 17a1.7 1.7 0 0 1-2.4-2.4L16 6.7";
export const MERGE = "M18 15a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM6 3a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM6 21V9a9 9 0 0 0 9 9";

/** Set aside: the pin that holds a thing where your hand can reach it. */
export const SET_ASIDE = "M12 17v4M8 3h8l-1 7 3 3H6l3-3z";

/** The two stacks at the foot of the list: a card with two edges showing behind it. */
export const PILE = "M7 4h10M5 7h14M3 10h18v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z";

/* Help */

/** The circled question mark: the launcher in the corner, and the help rows behind it. */
export const HELP =
  "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM9.7 9.3a2.4 2.4 0 0 1 4.7.7c0 1.6-2.4 1.9-2.4 3.5M12 17.3v.01";

/** The open book: the guide. */
export const BOOK =
  "M12 6.5A3.5 3.5 0 0 0 8.5 4H4v13h5a3 3 0 0 1 3 3M12 6.5A3.5 3.5 0 0 1 15.5 4H20v13h-5a3 3 0 0 0-3 3M12 6.5V20";

/** The needle in its circle: the tour, which is the one thing here that walks you somewhere. */
export const COMPASS = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM15.6 8.4l-2.1 5.1-5.1 2.1 2.1-5.1z";
