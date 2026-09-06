// The pure half of the notification toggles, shared by onboarding and the settings section.

import type { Place } from "../ipc";

/** The three places a device can be told about. The Screener never notifies. */
export const NOTIFY_PLACES: { id: Place; label: string; note: string }[] = [
  { id: "inbox", label: "Inbox", note: "Mail from somebody you have screened in." },
  { id: "feed", label: "Feed", note: "Newsletters and anything else that arrives on its own schedule." },
  { id: "paper-trail", label: "Paper Trail", note: "Receipts, statements and confirmations." },
];

/** The list with one place turned on or off, in the order the section lists them. */
export function withPlace(places: Place[], place: Place, on: boolean): Place[] {
  const kept = places.filter((one) => one !== place);
  if (!on) return kept;
  return NOTIFY_PLACES.map((one) => one.id).filter((id) => id === place || kept.includes(id));
}

/** "Nothing notifies you yet.", "Notifications are on for the Inbox and the Feed." */
export function placesSentence(places: Place[]): string {
  const names = NOTIFY_PLACES.filter((one) => places.includes(one.id)).map((one) => `the ${one.label}`);
  if (names.length === 0) return "Nothing notifies you yet.";
  if (names.length === 1) return `Notifications are on for ${names[0]}.`;
  return `Notifications are on for ${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}.`;
}
