import { call, type ContactCard, type ContactPatch, type Person } from "../ipc";

export const contactCard = (accountId: string, address: string) =>
  call<ContactCard>("contact_card", { accountId, address });

export const contactUpdate = (accountId: string, address: string, patch: ContactPatch) =>
  call<void>("contact_update", { accountId, address, patch });

export const contactsList = (accountId: string | null, query: string) =>
  call<ContactCard[]>("contacts_list", { accountId, query });

/**
 * Autocomplete. The mirror first, ranked by recency and frequency, then the provider's contacts.
 * Correspondents' addresses never leave the device to be looked up.
 */
export const contactsSuggest = (accountId: string, prefix: string) =>
  call<Person[]>("contacts_suggest", { accountId, prefix });
