import type { Section } from "./types";

/**
 * The questions this app raises by not working like the last one. The first sentence is the answer,
 * because somebody who is here is stuck or surprised and is owed the answer before the reasoning,
 * and every one of them names the article that has the rest.
 */
export const QUESTIONS: Section = {
  id: "questions",
  title: "Questions",
  articles: [
    {
      id: "ask-empty-inbox",
      title: "Why is my Inbox nearly empty?",
      blocks: [
        {
          p: ["Three other places have taken what used to land in one."],
        },
        {
          p: [
            "Newsletters go to the Feed, receipts and confirmations to the Paper Trail, and the first message from anyone you have not decided about waits in the Screener. Nothing was deleted, and Everything",
            { cap: "place-everything" },
            " is the one list that holds all of it: ",
            { see: "start-places" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-newsletter",
      title: "Where did that newsletter go?",
      blocks: [
        {
          p: [
            "The Feed",
            { cap: "place-feed" },
            ", almost certainly, which is where a sender carrying an unsubscribe header is routed.",
          ],
        },
        {
          p: [
            "If it is the first thing that sender has ever sent you, it is in the Screener",
            { cap: "place-screener" },
            " instead. Search",
            { cap: "search" },
            " reaches every place either way: ",
            { see: "org-search" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-nothing-sent",
      title: "Does a sender find out that I screened them out?",
      blocks: [
        {
          p: ["No. Nothing is ever sent."],
        },
        {
          p: [
            "Their mail keeps arriving as it always did and is routed to Screened out, where you can read it. ",
            { see: "screen-back" },
            " is how it is undone.",
          ],
        },
      ],
    },
    {
      id: "ask-labels",
      title: "Can I still get to my Gmail labels and folders?",
      blocks: [
        {
          p: [
            "Yes. Every label is a place, listed under Labels in the palette",
            { cap: "command-palette" },
            ".",
          ],
        },
        {
          p: [
            "Label",
            { cap: "label" },
            " applies or removes one. They are the provider's and they roam with the mailbox, which is why they are not what the Inbox, the Feed and the Paper Trail are made of: ",
            { see: "org-labels" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-last-year",
      title: "Why can I not find mail from last year?",
      blocks: [
        {
          p: [
            "This device keeps a window of the mailbox, the last 30 days unless you asked for more.",
          ],
        },
        {
          p: [
            "The rest is still with your provider, and every list of search results ends with a button that goes and gets it. To keep more of it here, widen the window: ",
            { see: "acct-window" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-empty-trash",
      title: "Is there an Empty Trash button?",
      blocks: [
        {
          p: ["No. Gmail empties Trash after 30 days, and the foot of the list says so."],
        },
        {
          p: [
            "Deleting for good through Gmail's API needs a permission that amounts to total access to the mailbox, and asking every account for that to destroy things thirty days early is a bad trade. The key that trashed a thread is the key that puts it back: ",
            { see: "triage-archive" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-ai",
      title: "Does this app read my mail with AI?",
      blocks: [
        {
          p: [
            "No. There is none in it, and nothing you have is sent anywhere to be summarised, drafted or classified.",
          ],
        },
        {
          p: [
            "Where a message goes is decided by its headers and by the rules you set, which is why every Screener card says in one line which rule fired: ",
            { see: "screen-how" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-where-kept",
      title: "Where is my mail kept, and what leaves this device?",
      blocks: [
        {
          p: [
            "On this computer: a copy of the window you chose, with your piles, notes, renames, clips and sender rules beside it.",
          ],
        },
        {
          p: [
            "What leaves is your mail, to and from your provider, and a backup if you turn one on, which holds ciphertext and file names and nothing else. ",
            { see: "acct-privacy" },
            " has the rest.",
          ],
        },
      ],
    },
    {
      id: "ask-notify",
      title: "Why did nothing notify me?",
      blocks: [
        {
          p: ["Because notifications are off everywhere until you turn one on."],
        },
        {
          figure: "notify",
          caption: "A thread, a person or a place can be turned on, and all of them start off.",
        },
        {
          p: [
            "Turn one on for a thread",
            { cap: "notify" },
            ", for a person from their contact card",
            { cap: "contact-card" },
            ", or for a whole place in ",
            { settings: "Settings" },
            ", where one switch covers the machine. Only mail that arrives while the app is running is announced, so a first sync or a week away says nothing about the backlog.",
          ],
        },
        {
          note: [
            "The dock badge is the exception and it is on. It is not a notification: it counts the Inbox threads waiting for you.",
          ],
        },
      ],
    },
    {
      id: "ask-archived",
      title: "How do I get back something I archived?",
      blocks: [
        {
          p: [
            "It is in Everything",
            { cap: "place-everything" },
            ", which holds every thread on the device.",
          ],
        },
        {
          p: [
            "Archiving takes a thread out of the Inbox and does nothing else to it, and a new message in an archived thread brings it back on its own. If it was the last thing you did, Undo",
            { cap: "undo" },
            " takes it back: ",
            { see: "triage-archive" },
            ".",
          ],
        },
      ],
    },
    {
      id: "ask-leave",
      title: "What happens if I stop using this app?",
      blocks: [
        {
          p: [
            "Nothing is done to your mailbox that your provider cannot already see. Archiving, trashing, spam, labels and sends are ordinary mailbox changes.",
          ],
        },
        {
          p: [
            "Everything this app invented is kept beside the mail and never written into it, and ",
            { settings: "Settings" },
            ", under Data, exports your mail as mbox with every decision as JSON alongside. There is more in ",
            { see: "acct-privacy" },
            ".",
          ],
        },
      ],
    },
  ],
};
