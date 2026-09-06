import { call, type Account } from "../ipc";

export const accountsList = () => call<Account[]>("accounts_list");

/**
 * Returns the consent URL. Completion arrives as the `auth` event.
 *
 * `loginHint` is the address typed on the connect screen, so the consent page opens on that account
 * rather than on a chooser.
 *
 * `extraScopes` is how a feature asks for a scope it needs and does not have. Google does not
 * support incremental authorization for installed apps, so this re-runs the whole consent with the
 * base list plus the extras and replaces the stored token; it is never a second, narrower grant.
 */
export const accountConnect = (extraScopes?: string[], loginHint?: string) =>
  call<string>("account_connect", { extraScopes: extraScopes ?? [], loginHint: loginHint ?? null });

/**
 * Sets the window an account just added is to hold, hands it to the sync engine and starts its
 * first pass. Until this is called the account is in the registry and nowhere else.
 */
export const accountStart = (accountId: string, windowDays: number) =>
  call<void>("account_start", { accountId, windowDays });

/** Re-runs consent for an account that is already linked, to pick up a scope that was withheld. */
export const accountGrant = (accountId: string, extraScopes?: string[]) =>
  call<string>("account_grant", { accountId, extraScopes: extraScopes ?? [] });

/**
 * The one way an account leaves this device. Margin's access is revoked at Google first, for every
 * Margin app on every machine because the three share one OAuth client, then the token and the
 * account are forgotten here. `keepData` leaves the mirror and the decisions on disk, set aside for
 * the day the account is added again.
 */
export const accountRemove = (accountId: string, keepData: boolean) =>
  call<void>("account_remove", { accountId, keepData });

export const accountSetColor = (accountId: string, color: string) =>
  call<void>("account_set_color", { accountId, color });

export const accountSetName = (accountId: string, name: string) =>
  call<void>("account_set_name", { accountId, name });
