import type { Section } from "./types";

/**
 * The gate, and the three things anybody asks about it: how a suggestion is made, how a decision is
 * changed afterwards, and what a No actually does.
 */
export const SCREENER: Section = {
  id: "screener",
  title: "The Screener",
  articles: [
    {
      id: "screen-how",
      title: "How the Screener works",
      blocks: [
        {
          p: [
            "A first message from a sender you have no decision about is held. It is in no box, and the Inbox shows a pill saying how many senders are waiting.",
          ],
        },
        {
          p: [
            "The Screener",
            { cap: "place-screener" },
            " is one card per sender: who they are, what they sent, and the box the app suggests with the reason that made it.",
          ],
        },
        {
          figure: "screener-card",
          caption:
            "What you are deciding is where that sender goes, not what to do with one message.",
        },
        { keys: ["screen-yes", "screen-elsewhere", "screen-no", "screen-reply"] },
        {
          p: [
            "Yes and Elsewhere set the rule for that address, and Elsewhere can set it for everyone at the domain instead. No routes the sender's mail to Screened out from then on.",
          ],
        },
        {
          p: [
            "The suggestion is a rule rather than a guess. Written by a person, so Inbox. Carries an unsubscribe header, so Feed. Sent by a service on somebody's behalf, or a receipt, so Paper Trail, even when it carries an unsubscribe footer as well. The card says which rule fired.",
          ],
        },
        {
          p: [
            "Only senders whose first message arrives after the account was added wait here. A reply to a thread you are already in is never held.",
          ],
        },
        {
          note: ["Nothing is sent to the sender, whichever you press."],
        },
        {
          picture: "screener",
          alt: "The Screener, with a card per sender and the three choices on the right of each",
          caption: "Three senders waiting, each with its suggested box and the reason for it.",
        },
      ],
    },
    {
      id: "screen-where",
      title: "Say where a sender goes, and change it later",
      blocks: [
        {
          p: [
            "One rule per sender, changed in one place: the contact card",
            { cap: "contact-card" },
            ". Delivers to is the row that says where their mail goes.",
          ],
        },
        {
          picture: "contact-card",
          alt: "The contact card hanging from a sender's name, with Delivers to, Notify, a note and recent threads",
          caption: "Delivers to sets the box, and the switch under it sets the whole domain.",
        },
        {
          note: [
            "Changing it moves the threads that are already here, not only the ones still to come.",
          ],
        },
        {
          p: [
            "The card opens on the thread you are looking at, and a click on any name or avatar opens it too. Move",
            { cap: "move" },
            " does the same from the list without opening anything, and Contacts, in the palette",
            { cap: "command-palette" },
            ", lists everyone you have decided about with the same control on each row.",
          ],
        },
        {
          p: [
            "A rule is keyed on the address, or on the domain when you chose everyone at that domain. An address rule beats a domain rule. The shared domains everybody's mail comes from, gmail.com and the like, cannot carry a domain rule at all.",
          ],
        },
      ],
    },
    {
      id: "screen-back",
      title: "Bring back somebody you screened out",
      blocks: [
        {
          p: [
            "Set Delivers to on their contact card. Whatever they sent that is still on the device comes back with them, into the box you choose.",
          ],
        },
        {
          steps: [
            [
              "Open Screened out from the palette",
              { cap: "command-palette" },
              ", under Other.",
            ],
            ["Open a thread from the sender you want back."],
            ["Open their contact card", { cap: "contact-card" }, " and set Delivers to."],
          ],
        },
        {
          p: [
            "Nothing was sent when you screened them out, and nothing is sent when you let them back in. There is a question about exactly that: ",
            { see: "ask-nothing-sent" },
          ],
        },
        {
          p: [
            "Screened-out mail sits there for as long as this account keeps mail on the device, and falls off with everything else of its age. ",
            { see: "acct-window" },
            " is the setting that decides how long that is.",
          ],
        },
      ],
    },
    {
      id: "screen-clear",
      title: "Clear the whole queue",
      blocks: [
        {
          p: [
            "Clear all screens out every sender waiting, after one confirmation that says how many. It sits at the top right of the Screener, above the cards.",
          ],
        },
        {
          p: [
            "It is the answer to a queue that has run away from you rather than a decision about the people in it. Nothing is sent, their mail goes to Screened out from then on, and any of them can be let back in: ",
            { see: "screen-back" },
          ],
        },
        {
          p: [{ place: "screener", label: "Open the Screener" }, " to see what is waiting."],
        },
      ],
    },
  ],
};
