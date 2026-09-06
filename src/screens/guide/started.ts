import type { Section } from "./types";

/**
 * The first section, and the one somebody reads once. Everything in it is about the shape of the
 * app rather than about a verb: what is different, what the window holds, where mail lands, what
 * the first hour is, and why every button prints a letter.
 */
export const STARTED: Section = {
  id: "started",
  title: "Getting started",
  articles: [
    {
      id: "start-different",
      title: "What is different here",
      blocks: [
        {
          p: [
            "Your mail is not one list. People, newsletters and receipts go to three separate places, and somebody writing to you for the first time waits at a gate before they reach any of them.",
          ],
        },
        {
          figure: "boxes",
          caption:
            "Mail from people, kept apart from newsletters and receipts by a decision you made.",
        },
        {
          p: [
            "Nothing guesses on your behalf. You say where a sender goes, once, and that decision covers everything they send from then on. ",
            { see: "start-places" },
            " is the places there are, and ",
            { see: "screen-how" },
            " is the gate.",
          ],
        },
        {
          p: [
            "Reply later",
            { cap: "reply-later" },
            " and Set aside",
            { cap: "set-aside" },
            " are piles at the foot of the list rather than flags on a row. A thread you pile leaves the list, and the same key brings it back.",
          ],
        },
        {
          p: ["Every verb is one key, and the button that runs it prints the key."],
        },
        {
          note: [
            "There is no counter, no streak and no assistant. The one number anywhere is the dock badge, and it counts unseen Inbox threads.",
          ],
        },
      ],
    },
    {
      id: "start-window",
      title: "The shape of the window",
      blocks: [
        {
          p: [
            "One header, then the stage. The header carries the account on the left, the three boxes in the middle with their number keys on them, and search, places and Write on the right.",
          ],
        },
        {
          figure: "window",
          caption: "There is nothing else to learn: no sidebar, no folder tree, no toolbar.",
        },
        {
          p: [
            "The list column has the place's name at its head, and in the Inbox the pill saying how many senders are waiting. The piles sit under the list and are always in view.",
          ],
        },
        {
          p: [
            "The reading pane can be hidden",
            { cap: "toggle-pane" },
            ". The list then takes the width and a thread opens in place, with Escape going back to the list.",
          ],
        },
        {
          p: [
            "Everything that is not one of the three boxes is in the palette",
            { cap: "command-palette" },
            ", which is the only menu in the app and the way every setting is reached.",
          ],
        },
        {
          picture: "inbox",
          alt: "The Inbox: the list on the left with the two piles under it, and a thread open in the reading pane",
          caption: "Every verb in the bar over the message prints the key it answers to.",
        },
      ],
    },
    {
      id: "start-places",
      title: "Where your mail goes",
      blocks: [
        {
          p: [
            "Every sender has exactly one destination. Three of them are boxes you read, and the fourth is Screened out, which is where the mail you never want to see goes.",
          ],
        },
        {
          figure: "routing",
          caption:
            "A first message waits at the gate. After that, everything from that sender goes straight to its box.",
        },
        {
          p: [
            "Inbox",
            { cap: "place-inbox" },
            " is people, and the few services you want to hear from as they arrive. It is one list in time order, and a reply pulls a thread back up it.",
          ],
        },
        {
          p: [
            "Feed",
            { cap: "place-feed" },
            " is newsletters and long reads. Every item is already open, newest first, with no read state and no count.",
          ],
        },
        {
          p: [
            "Paper Trail",
            { cap: "place-paper-trail" },
            " is receipts, confirmations and the mail a machine sent you: the things you file and search for later.",
          ],
        },
        {
          p: [
            "Screener",
            { cap: "place-screener" },
            " is the gate in front of the three. It holds the first message from anyone you have not decided about. ",
            { see: "screen-how" },
            " is the whole of it.",
          ],
        },
        {
          p: [
            "Everything",
            { cap: "place-everything" },
            " is the one list that holds all of it at once, archived, spam and screened out included. Nothing in this app is anywhere you cannot get to.",
          ],
        },
      ],
    },
    {
      id: "start-day",
      title: "Your first day",
      blocks: [
        {
          p: [
            "Empty the Screener, put right anything the Feed and the Paper Trail have in the wrong place, and pile what you cannot answer now. That is the first hour.",
          ],
        },
        {
          steps: [
            [
              { place: "screener", label: "Open the Screener" },
              " and empty it. Every card carries a suggestion and the reason for it. What each answer does is in ",
              { see: "screen-how" },
              ".",
            ],
            [
              "Look through the Feed",
              { cap: "place-feed" },
              " and the Paper Trail",
              { cap: "place-paper-trail" },
              ". Anything in the wrong one is one change on the sender's contact card",
              { cap: "contact-card" },
              ".",
            ],
            [
              "Pile rather than file. Reply later",
              { cap: "reply-later" },
              " is what you owe an answer to, and Set aside",
              { cap: "set-aside" },
              " is what you need to hand.",
            ],
            [
              "When you cannot remember a key, the palette",
              { cap: "command-palette" },
              " lists every place, every command and every setting, with the key beside it.",
            ],
          ],
        },
        {
          note: [
            "Everyone the account already knew was screened in when it was added, so what waits in the Screener is somebody genuinely new.",
          ],
        },
        {
          picture: "palette",
          alt: "The command palette with two letters typed in it, its rows grouped into Places, Other and Actions",
          caption: "Two letters narrow every group at once, and every row prints its key.",
        },
      ],
    },
    {
      id: "start-keys",
      title: "Every key is printed",
      blocks: [
        {
          p: [
            "One unmodified key per verb. Nothing is chorded, nothing is modal, and a key that would act on nothing does nothing.",
          ],
        },
        {
          figure: "keyboard",
          caption: "The verbs you use hourly, on the keys they answer to.",
        },
        {
          p: [
            "Every button carries the key its verb answers to, which is how the mouse teaches the keyboard. The palette",
            { cap: "command-palette" },
            " prints the same keys beside the same commands, and the whole table is behind",
            { cap: "shortcuts" },
            ".",
          ],
        },
        {
          p: [
            "Where Gmail and Superhuman agree on a letter, this app uses theirs. Where HEY has a verb they do not, it uses HEY's.",
          ],
        },
        {
          p: [
            "The keymap is a file in the app's data directory. ",
            { settings: "Settings" },
            ", under Keyboard, opens it and resets it, and the sheet is generated from that file, so a key you remap is the key the buttons print.",
          ],
        },
        {
          picture: "shortcuts",
          alt: "The keyboard shortcuts sheet, with the verbs grouped and a key printed beside each",
          caption: "The sheet is generated from the keymap, so it cannot drift from what the keys do.",
        },
      ],
    },
  ],
};
