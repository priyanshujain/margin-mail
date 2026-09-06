import type { Section } from "./types";

/**
 * The accounts and the machine they sit on, which is one subject in five questions: how another
 * mailbox is added, how several share one window, what each account was allowed to do, how much of
 * it is here, and what of it ever leaves.
 */
export const ACCOUNTS: Section = {
  id: "accounts",
  title: "Accounts",
  articles: [
    {
      id: "acct-add",
      title: "Add another account",
      blocks: [
        {
          p: [
            "Add account is at the foot of the Accounts section in ",
            { settings: "Settings" },
            ". It asks for the address and nothing else, and works the rest out from the domain.",
          ],
        },
        {
          steps: [
            [
              "Type the address. A Google address hands over to your browser, where you sign in with Google; any other address gets a sign-in step with the servers it found and a password field.",
            ],
            [
              "Choose how far back this device should keep for that account. The first sync reads the answer, so a year of a busy mailbox is a wait you agreed to.",
            ],
            [
              "Wait for the first pass. It says what it is doing and counts the messages in, then opens that account's Inbox with the senders it screened in.",
            ],
          ],
        },
        {
          picture: "settings",
          alt: "Settings with its rail of twelve sections down the left and one section, Appearance, on the right",
          caption: "Settings is a place with a rail of sections, and Accounts is the first of them.",
        },
        {
          p: [
            "Remove an account is in the same section. It takes that account's mail and decisions off this computer and leaves the mailbox at your provider as it was.",
          ],
        },
        {
          note: [
            "Removing a Google account revokes access for all three Margin apps on every machine, because they share one sign-in.",
          ],
        },
      ],
    },
    {
      id: "acct-switch",
      title: "Switch accounts, and see them all at once",
      blocks: [
        {
          p: [
            "The account chip at the left of the header is the switcher. It lists your accounts and All accounts",
            { cap: "accounts" },
            ", and prints the key beside each of them.",
          ],
        },
        {
          figure: "accounts",
          caption: "Each account keeps its own everything, and All accounts is all of them in one list.",
        },
        {
          p: [
            "An account has its own places, sender rules, piles, Screener, storage window and permissions, so a work account can keep a year while a personal one keeps a month.",
          ],
        },
        {
          p: [
            "All accounts merges every account's version of the place you are in. Each row carries a coloured edge for the account it came from, the colour on that account's card in ",
            { settings: "Settings" },
            ".",
          ],
        },
        {
          p: [
            "Writing picks the account from the thread you are replying to, or from the one you are looking at. The From field changes it.",
          ],
        },
      ],
    },
    {
      id: "acct-permissions",
      title: "Permissions",
      blocks: [
        {
          p: [
            "A Google account's card in ",
            { settings: "Settings" },
            ", under Accounts, lists what it granted, one line each: your mail, your mail settings, your contacts, the people you have written to, backup, and calendar.",
          ],
        },
        {
          note: [
            "Sign-in happens in your browser with Google. This app never sees your password, and you can revoke the key it was given from your Google account at any time.",
          ],
        },
        {
          p: [
            "Any of them can be missing, because consent is granular. A line that is missing says what it costs in plain terms rather than naming a scope, and carries a Grant button.",
          ],
        },
        {
          steps: [
            ["Open the account's card in Settings."],
            ["Press Grant on the line that is missing."],
            ["The consent page opens for the whole list, and the line reads as granted when you come back."],
          ],
        },
        {
          p: [
            "Calendar is the one deliberately not asked for at the start. Answering your first invitation asks for it: ",
            { see: "read-invites" },
            ".",
          ],
        },
        {
          p: [
            "An account that is not Google granted nothing, so there is no list on its card. It shows the two servers it is using, the username it logs in with, and where its password is kept.",
          ],
        },
      ],
    },
    {
      id: "acct-window",
      title: "How far back this device keeps",
      blocks: [
        {
          p: [
            "This device holds a window of the mailbox rather than all of it: the last 30 days unless you say otherwise, or 90 days, 180 days, a year, or everything. It is set per account in ",
            { settings: "Settings" },
            ", under Mail.",
          ],
        },
        {
          figure: "storage",
          caption:
            "What is inside the window is on this device, and the rest of the mailbox is still at your provider.",
        },
        {
          note: [
            "A thread you have done something to is kept whatever its age. A pile, a note, a snooze or any other decision is enough.",
          ],
        },
        {
          p: [
            "Widening the window starts a backfill and shows the same bar the first sync used. Narrowing it says how many threads will go before it does it.",
          ],
        },
        {
          p: [
            "Only Everything",
            { cap: "place-everything" },
            " says any of this out loud, in one line at the foot of its list. What is older is on the provider, and search offers to go and get it: ",
            { see: "org-search" },
            ".",
          ],
        },
      ],
    },
    {
      id: "acct-privacy",
      title: "What leaves this device",
      blocks: [
        {
          p: ["Your mail, to and from your provider. Nothing else goes anywhere unless you ask for it."],
        },
        {
          figure: "privacy",
          caption: "What leaves is the mail itself, and a backup only if you ask for one.",
        },
        {
          p: [
            "A copy of the window you chose is on this computer, and so is everything this app invented: your piles, notes, renames, clips and sender rules, kept beside the mail rather than written into it.",
          ],
        },
        {
          p: [
            "Backup is off until you choose a store in ",
            { settings: "Settings" },
            ", under Backup, and what it holds is ciphertext and file names and nothing else.",
          ],
        },
        {
          p: [
            "No remote image loads until you ask for it, and nothing you send carries a tracker: ",
            { see: "read-images" },
            ".",
          ],
        },
        {
          p: [
            { settings: "Settings" },
            ", under Data, exports your mail as mbox and every decision as JSON. There is a question about leaving: ",
            { see: "ask-leave" },
          ],
        },
      ],
    },
  ],
};
