import type { Section } from "./types";

/** Reading: the pane, the Feed, and the four things a message can carry that are not prose. */
export const READING: Section = {
  id: "reading",
  title: "Reading",
  articles: [
    {
      id: "read-thread",
      title: "Open a thread and move through it",
      blocks: [
        {
          p: [
            "The list is on the left and the thread is in the pane beside it, so triage happens without leaving the list. Open",
            { cap: "open-selection" },
            " opens whatever is focused.",
          ],
        },
        {
          figure: "thread",
          caption:
            "The latest message is open and the ones before it are collapsed to a line each.",
        },
        {
          keys: [
            "select-next",
            "select-prev",
            "open-selection",
            "message-next",
            "message-prev",
            "message-toggle",
            "message-expand-all",
            "toggle-pane",
          ],
        },
        {
          p: [
            "Quoted text is behind a pill, so a long thread reads as the conversation rather than as the same paragraph six times.",
          ],
        },
        {
          p: [
            "Opening a thread marks it seen. Mark seen or unseen",
            { cap: "toggle-seen" },
            " puts that back.",
          ],
        },
        {
          note: [
            "Weight is the only thing that says a thread is new. There is no unread count on a place, on a group or on a row.",
          ],
        },
      ],
    },
    {
      id: "read-together",
      title: "Read several threads together",
      blocks: [
        {
          p: [
            "Select the threads and Open",
            { cap: "open-selection" },
            " shows them one after another in the pane, each under its own heading. It is how a morning of five short threads is read once rather than five times.",
          ],
        },
        {
          keys: [
            "select",
            "select-extend-down",
            "select-extend-up",
            "select-all",
            "open-selection",
          ],
        },
        {
          p: [
            "Escape clears the selection. While there is one, the piles at the foot of the list give way to a bar of the same verbs, and what they do is ",
            { see: "triage-several" },
            ".",
          ],
        },
      ],
    },
    {
      id: "read-feed",
      title: "The Feed reads differently",
      blocks: [
        {
          p: [
            "The Feed",
            { cap: "place-feed" },
            " takes the whole window rather than a list beside a pane, because every card is already open. Newest first, no read state, no counts, and nothing telling you how far behind you are.",
          ],
        },
        {
          picture: "feed",
          alt: "The Feed, one column of cards with each newsletter drawn open",
          caption: "One column of cards, each already open, under the line that counts the trackers.",
        },
        {
          p: [
            "Next",
            { cap: "select-next" },
            " and previous",
            { cap: "select-prev" },
            " move between cards. A card longer than a screen fades out with Read more, and Open",
            { cap: "open-selection" },
            " expands it in place and closes it again.",
          ],
        },
        {
          p: ["A hairline reading you left off here marks where you stopped last time."],
        },
        {
          p: [
            "The foot of a card carries Save clip, Unsubscribe and Move, and the top of the column says how many trackers were stripped today.",
          ],
        },
      ],
    },
    {
      id: "read-images",
      title: "Images and trackers",
      blocks: [
        {
          p: [
            "Remote images do not load until you ask. A banner at the top of the message says how many trackers were stripped and names the vendor, and Show images loads them for that message.",
          ],
        },
        {
          figure: "trackers",
          caption: "The images are held and the trackers are counted before you see any of it.",
        },
        {
          p: [
            "Loading a remote image tells the server that hosts it that you opened the message, from your IP address, at that moment. That is the whole reason the images wait.",
          ],
        },
        {
          p: [
            "The contact card",
            { cap: "contact-card" },
            " allows a sender's images always. ",
            { settings: "Settings" },
            ", under Privacy, sets the rule for everything: never, ask per message, or always. The senders you have allowed are listed there too.",
          ],
        },
        {
          p: [
            "Message bodies render with scripts, forms and external styles removed. A link shows its real destination on hover and opens with the tracking parameters taken off.",
          ],
        },
        {
          note: ["Nothing you send carries a tracker, and nothing reports when it was opened."],
        },
      ],
    },
    {
      id: "read-attachments",
      title: "Attachments",
      blocks: [
        {
          p: [
            "Attachments are chips at the foot of the message that carried them, each with a mark for its type. Pressing one opens the file with whatever owns that type on this machine.",
          ],
        },
        {
          p: [
            "They are fetched when you open the thread rather than during sync, and kept after that, so a mailbox full of attachments does not become a disk full of them.",
          ],
        },
        {
          p: [
            { see: "org-files" },
            ", in the palette, is every one of them on the device in a single grid, filtered by type and by sender.",
          ],
        },
      ],
    },
    {
      id: "read-invites",
      title: "Calendar invitations",
      blocks: [
        {
          p: [
            "An invitation renders as a card in the thread: the date in a box, the title, the time and place, the organiser, and the three answers. Open in Margin Calendar is beside them.",
          ],
        },
        {
          keys: ["invite-accept", "invite-maybe", "invite-decline"],
        },
        {
          p: [
            "Answering writes to the calendar the invitation was sent to. When the event is not there yet, it is put there first.",
          ],
        },
        {
          p: [
            "Calendar is not one of the permissions asked for when an account is added, so the first invitation you answer asks for it and runs the consent page again. Until then the card is read-only, and ",
            { see: "acct-permissions" },
            " says what else an account granted.",
          ],
        },
      ],
    },
  ],
};
