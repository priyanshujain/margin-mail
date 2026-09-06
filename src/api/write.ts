import {
  call,
  type Draft,
  type DraftSaved,
  type InviteResponse,
  type Outgoing,
  type Undo,
} from "../ipc";

/** Saves locally on every keystroke's debounce and to the provider every few seconds. */
export const draftSave = (draft: Draft) => call<DraftSaved>("draft_save", { draft });

export const draftGet = (id: string) => call<Draft>("draft_get", { id });

export const draftDelete = (id: string) => call<void>("draft_delete", { id });

/** Queues the send, held for the undo delay. The `Undo` it returns is what the toast counts down. */
export const send = (draft: Draft) => call<Undo>("send", { draft });

/** Skips the remaining hold. */
export const sendNow = (outgoingId: string) => call<void>("send_now", { outgoingId });

export const outboxList = () => call<Outgoing[]>("outbox_list");

/** Accept, maybe or decline, through the Calendar API on the invited calendar. */
export const inviteRespond = (messageId: string, response: InviteResponse) =>
  call<void>("invite_respond", { messageId, response });

/**
 * One click where the sender supports RFC 8058, the mailto where they do not, the link otherwise.
 * `alsoTrash` and `alsoScreenOut` are the two follow-ups the confirmation offers.
 */
export const unsubscribe = (
  accountId: string,
  address: string,
  alsoTrash: boolean,
  alsoScreenOut: boolean,
) => call<Undo>("unsubscribe", { accountId, address, alsoTrash, alsoScreenOut });
