import type { Section } from "./types";

/** Triage: what the single keys do to a thread, and how each of them is undone. */
export const TRIAGE: Section = {
  id: "triage",
  title: "Triage",
  articles: [
    {
      id: "triage-archive",
      title: "Archive, trash and spam",
      blocks: [
        {
          p: [
            "Archive takes a thread out of the Inbox. It is not deleted and it is not hidden: it lives in Everything",
            { cap: "place-everything" },
            " with the rest of the mailbox.",
          ],
        },
        {
          keys: ["archive", "trash", "spam", "undo"],
        },
        {
          p: [
            "A new message in an archived thread brings it back to the top of the Inbox on its own.",
          ],
        },
        {
          p: [
            "Trash and spam are each the same key twice. Pressed again in Trash, a thread comes back, and it lands where it was rather than in the Inbox.",
          ],
        },
        {
          p: [
            "Pressed again in Spam, the spam mark comes off and the thread is routed like any other: to its sender's box, or to the Screener if you never decided about them.",
          ],
        },
        {
          p: [
            "There are two ways back and then a deadline. The toast that is still up, and Undo",
            { cap: "undo" },
            " while it is. The place and the same verb, a week later. After thirty days Gmail empties its own trash and the message goes with it.",
          ],
        },
        {
          p: [
            "Trash and Spam are in the palette under Other. There is no Empty button, and there is a question about why: ",
            { see: "ask-empty-trash" },
          ],
        },
      ],
    },
    {
      id: "triage-piles",
      title: "Reply later and Set aside",
      blocks: [
        {
          p: [
            "Two stacks of cards sit at the foot of the list, always in view. Reply later",
            { cap: "reply-later" },
            " is what you owe an answer to. Set aside",
            { cap: "set-aside" },
            " is what you need to hand: a ticket, an itinerary, a code.",
          ],
        },
        {
          figure: "piles",
          caption: "A pile takes a thread out of its list, and the same key puts it back.",
        },
        {
          p: [
            "Reply later",
            { cap: "place-reply-later" },
            " and Set aside",
            { cap: "place-set-aside" },
            " are places as well as piles, with the pane beside them. Sending a reply on a Reply later thread clears it from the pile, with an undo.",
          ],
        },
        {
          picture: "piles",
          alt: "The two piles at the foot of the list, Reply later on the left and Set aside on the right",
          caption: "Each stack carries its label, its key, and the thread on top of it.",
        },
        {
          p: [{ see: "triage-focus" }, " is the other half of the arrangement."],
        },
        {
          note: [
            "Nothing nags. A thread can sit in Set aside for a year and the app will never mention it.",
          ],
        },
      ],
    },
    {
      id: "triage-focus",
      title: "Focus & Reply",
      blocks: [
        {
          p: [
            "Focus & Reply lines up every thread in the Reply later pile on one page, each with its latest message on the left and a reply box on the right.",
          ],
        },
        {
          figure: "focus",
          caption: "One page, one item per thread you owe an answer to.",
        },
        {
          keys: ["focus-reply", "focus-next", "focus-prev", "send"],
        },
        {
          p: [
            "Sending collapses the item to a Sent to line and moves on. Escape leaves the page.",
          ],
        },
        {
          p: [
            "Items you skip stay in the pile. It is an hour spent answering rather than a queue you have to finish.",
          ],
        },
      ],
    },
    {
      id: "triage-snooze",
      title: "Snooze, and if no reply by",
      blocks: [
        {
          p: [
            "Snooze",
            { cap: "snooze" },
            " takes a thread away until later today, tomorrow, the weekend, next week, or a time you pick. It leaves its list and waits in Snoozed",
            { cap: "place-snoozed" },
            " with the time it is due back.",
          ],
        },
        {
          figure: "snooze",
          caption: "The choices are points on one line of time, from later today to next week.",
        },
        {
          p: [
            "If no reply by is the last choice in the same picker. That thread comes back only if nobody but you has written to it since; a reply cancels the reminder and lands as normal.",
          ],
        },
        {
          picture: "snooze",
          alt: "The snooze picker open on a thread, with its six choices and the key beside each",
          caption: "Six choices, each with its key, and a date picker behind the last two.",
        },
        {
          p: [
            "What comes back arrives under Back, at the top of the place it left, and stays there until it is opened.",
          ],
        },
        {
          p: [
            "The times behind later today, tomorrow, the weekend and next week are yours to set, in ",
            { settings: "Settings" },
            " under Piles and snooze.",
          ],
        },
        {
          note: [
            "Nothing runs in the background. Whichever of your devices next opens the app works out what is due, so a thread can come back late, and one that does says it was due yesterday.",
          ],
        },
      ],
    },
    {
      id: "triage-ignore",
      title: "Ignore a thread",
      blocks: [
        {
          p: [
            "Ignore",
            { cap: "ignore" },
            " is for the thread that will not end. It stops that thread reading as new, counting on the badge, or notifying you.",
          ],
        },
        {
          p: [
            "New messages still arrive and still append, and the thread still rises with them, because the list is in time order.",
          ],
        },
        {
          p: [
            "A banner on the thread says you are ignoring it and offers Stop ignoring. The same key stops it too.",
          ],
        },
      ],
    },
    {
      id: "triage-several",
      title: "Act on several at once",
      blocks: [
        {
          p: [
            "A verb acts on the whole selection when there is one, and the two piles give way to a bar carrying the verbs a selection can take.",
          ],
        },
        {
          figure: "selection",
          caption: "The piles' footprint, filled with the same verbs and the same keys.",
        },
        {
          p: [
            "Open",
            { cap: "open-selection" },
            " reads them one after another, and Merge",
            { cap: "merge" },
            " is the selection's own verb: it wants two threads or more. The keys that build a selection are in ",
            { see: "read-together" },
            ".",
          ],
        },
        {
          note: ["Every bulk action is one undo", { cap: "undo" }, ", not one per thread."],
        },
      ],
    },
  ],
};
