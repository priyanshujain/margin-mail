import type { Section } from "./types";

/** Writing: the card, the box in the thread, and the four things the send footer carries. */
export const WRITING: Section = {
  id: "writing",
  title: "Writing",
  articles: [
    {
      id: "write-compose",
      title: "Write, reply, reply all, forward",
      blocks: [
        {
          p: [
            "New message opens a card over the list, bottom right, with the list still usable behind it. A reply does not use the card: it opens a box under the last message in the thread, with the recipients as chips, where the mail you are answering already is.",
          ],
        },
        { keys: ["compose", "reply", "reply-all", "forward", "compose-expand"] },
        {
          picture: "compose",
          alt: "The compose card floating over the Inbox, with the From, To and Subject fields and the send footer",
          caption: "From, To, Subject, the body, and a footer with the send key and the undo delay.",
        },
        {
          p: [
            "The editor does paragraphs, bold, italic, links, lists, quotes and code. No colours and no fonts, because mail that arrives looking like the machine it was written on is mail that arrives looking wrong.",
          ],
        },
        {
          p: [
            "Whether the reply key means reply or reply all is yours to set, in ",
            { settings: "Settings" },
            " under Writing.",
          ],
        },
      ],
    },
    {
      id: "write-undo",
      title: "Undo a send",
      blocks: [
        {
          p: [
            "Every send is held for ten seconds before it goes. A toast at the foot of the window says who it went to and offers Undo, and taking it back reopens the draft where it was.",
          ],
        },
        {
          figure: "undo",
          caption: "Ten seconds between the key and the message leaving, and the toast is the way back.",
        },
        { keys: ["send", "send-now", "undo"] },
        {
          note: [
            "Nothing has left this machine while the toast is up. Undo puts the draft back rather than chasing a message that has already gone.",
          ],
        },
        {
          p: [
            "The delay is five, ten, twenty or thirty seconds, in ",
            { settings: "Settings" },
            " under Writing.",
          ],
        },
        {
          p: [
            "A send that fails, or one made offline, waits and retries. The thread says it is waiting to send until it goes, so a message never quietly does not exist.",
          ],
        },
      ],
    },
    {
      id: "write-attachments",
      title: "Attachments, and the size limit",
      blocks: [
        {
          p: ["Drag a file onto the message, paste it, or Attach", { cap: "attach" }, "."],
        },
        {
          p: [
            "The provider sets a limit on the whole encoded message, which for Gmail is 35 MB.",
          ],
        },
        {
          note: [
            "A file that would push a message past the limit is refused as you attach it, rather than after you have written the message and pressed send.",
          ],
        },
      ],
    },
    {
      id: "write-remind",
      title: "Remind me if no reply",
      blocks: [
        {
          p: [
            "Remind me if no reply",
            { cap: "remind-if-no-reply" },
            " is a toggle in the send footer with a date on it. If nobody but you has written by that date, the thread comes back to the top of the place it lives in, under Back.",
          ],
        },
        {
          p: [
            "A reply cancels it, and the reply lands as normal. The toggle applies to the thread once the send has gone.",
          ],
        },
        {
          p: [
            "It is the last choice in the snooze picker under another name, and it comes back the same way: ",
            { see: "triage-snooze" },
            ".",
          ],
        },
      ],
    },
    {
      id: "write-intro",
      title: "Instant intro",
      blocks: [
        {
          p: [
            "Instant intro",
            { cap: "instant-intro" },
            " in a reply moves the introducer to Bcc and puts a thank-you line at the top. Pressing it again reverts both.",
          ],
        },
        {
          p: [
            "It is for the mail that introduces you to somebody else, where the first thing you write is a thank-you to the introducer and a note that they can drop off the thread.",
          ],
        },
        {
          p: [
            "The line is a template, and it is yours to write once, in ",
            { settings: "Settings" },
            " under Writing.",
          ],
        },
      ],
    },
    {
      id: "write-drafts",
      title: "Drafts and signatures",
      blocks: [
        {
          p: [
            "A draft saves to this device as you type and to the provider every few seconds, so it is in your mailbox's drafts as well and it follows you to another machine.",
          ],
        },
        {
          p: [
            "Escape leaves the editor and keeps the draft. Discard",
            { cap: "discard-draft" },
            " throws it away.",
          ],
        },
        {
          p: [
            "The signature is the one the provider holds for the address you are sending from. It is editable in ",
            { settings: "Settings" },
            ", on the account card under Accounts and again under Writing, which is where somebody writing a signature looks for it.",
          ],
        },
        {
          p: [
            "The From field picks the account a message goes through, and an alias the provider has verified can be chosen there too: ",
            { see: "acct-switch" },
            ".",
          ],
        },
      ],
    },
  ],
};
