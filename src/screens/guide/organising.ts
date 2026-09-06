import type { Section } from "./types";

/**
 * The things kept beside the mail rather than in it, and the two ways of finding something again.
 * Every article here ends up saying the same thing in a different way: none of it is written into
 * the mailbox, so none of it is lost when the mailbox changes hands.
 */
export const ORGANISING: Section = {
  id: "organising",
  title: "Organising",
  articles: [
    {
      id: "org-notes",
      title: "Notes",
      blocks: [
        {
          p: [
            "Note",
            { cap: "note" },
            " adds a private note to a thread. In the thread it is a block after the message that was latest when you wrote it, dated. In the list it is one line under the row.",
          ],
        },
        {
          p: ["A note is kept against the thread rather than against a message in it."],
        },
        {
          note: ["Nothing is written to the mailbox, and nobody on the thread can see it."],
        },
      ],
    },
    {
      id: "org-rename",
      title: "Rename a thread",
      blocks: [
        {
          p: [
            "Click the subject in the reading pane, or run Rename from the palette",
            { cap: "command-palette" },
            ". The pane then shows the name you gave it, with what it was in small type beside it, and the list shows the new name.",
          ],
        },
        {
          p: [
            "Replies still carry the real subject, so the thread holds together at both ends.",
          ],
        },
        {
          p: ["The rename is yours alone, and Undo", { cap: "undo" }, " takes it back."],
        },
      ],
    },
    {
      id: "org-merge",
      title: "Merge threads",
      blocks: [
        {
          p: [
            "Select the threads",
            { cap: "select" },
            " and merge",
            { cap: "merge" },
            ": they read as one thread everywhere, named after the longest of them or after a name you type.",
          ],
        },
        {
          figure: "merge",
          caption: "Two threads become one thread in every list, and the banner is the way back.",
        },
        {
          p: [
            "It is for three people answering the same question in three separate threads.",
          ],
        },
        {
          p: [
            "A banner on the merged thread says where it came from and offers Unmerge. Replies and new messages in any of the threads underneath appear in the merged one.",
          ],
        },
      ],
    },
    {
      id: "org-clips",
      title: "Clips",
      blocks: [
        {
          p: [
            "Select text in any message and Save clip appears. Save the selection as a clip",
            { cap: "save-clip" },
            " does it from the keyboard, anywhere.",
          ],
        },
        {
          p: [
            "Clips, in the palette, lists every passage you have kept with its sender, its thread and its date, and each one links back to where it was said.",
          ],
        },
      ],
    },
    {
      id: "org-files",
      title: "All files",
      blocks: [
        {
          p: [
            "All files, in the palette, is every attachment on the device as a card: the name, the type, the size, who sent it and which thread it came in, newest first.",
          ],
        },
        {
          p: [
            "Filter it by type or by sender. Opening a card opens the thread the file arrived in.",
          ],
        },
        {
          note: [
            "The small images a signature drags along are left out, so it is a list of the files somebody meant to send you.",
          ],
        },
      ],
    },
    {
      id: "org-labels",
      title: "Labels and folders",
      blocks: [
        {
          p: [
            "The provider's labels and folders are places. They are listed under Labels in the palette",
            { cap: "command-palette" },
            ", one place each.",
          ],
        },
        {
          p: [
            "Label",
            { cap: "label" },
            " applies or removes one on what is selected, and in a label's own list Move",
            { cap: "move" },
            " moves a thread into another one.",
          ],
        },
        {
          p: ["They are the provider's, and they roam with the mailbox."],
        },
        {
          note: [
            "The Inbox, the Feed and the Paper Trail are made of your decisions about senders, not of labels.",
          ],
        },
      ],
    },
    {
      id: "org-search",
      title: "Search, and what it reaches",
      blocks: [
        {
          p: [
            "Search",
            { cap: "search" },
            " runs over what is on this device: subjects, participants, snippets and the text of the messages.",
          ],
        },
        {
          figure: "search-reach",
          caption:
            "This device answers first. The provider is asked at the foot of the results, and only then.",
        },
        {
          p: [
            "Results replace the list and the pane works as it always does. Escape gives you back the place you were in, where you were in it.",
          ],
        },
        {
          p: [
            "The operators are from:, to:, subject:, has:attachment, filename:, in:, before:, after: and label:. The one that reads oddly is in:, which narrows to a single place.",
          ],
        },
        {
          p: [
            "Because the device holds a window of the mailbox rather than all of it, a result list is a partial answer and says so. Every one of them ends with a button that runs the provider's own search and appends what it finds. ",
            { see: "acct-window" },
            " is how much of it is here to begin with.",
          ],
        },
        {
          note: [
            "Search reaches Spam, Trash and Screened out, and names the place on the row when it does. The message you most need to find is often the one something else decided you should not see.",
          ],
        },
      ],
    },
  ],
};
