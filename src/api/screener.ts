import { call, type Destination, type ScreenerCard, type Undo } from "../ipc";

export const screenerList = (accountId: string | null) =>
  call<ScreenerCard[]>("screener_list", { accountId });

/** Sets the rule for one sender. `wholeDomain` keys it on the domain instead of the address. */
export const screenerDecide = (
  accountId: string,
  address: string,
  destination: Destination,
  wholeDomain: boolean,
) => call<Undo>("screener_decide", { accountId, address, destination, wholeDomain });

export const screenerClearAll = (accountId: string | null) =>
  call<Undo>("screener_clear_all", { accountId });

/**
 * The first run pass: screen in everyone already known from the window, the People API and Sent,
 * routing each by the same suggestion function. Returns how many were screened in.
 */
/**
 * Screens in everyone the account already knows, once its first sync has brought the mailbox in.
 * Null until then: the seed reads what the crawl writes, and it is asked again when the account's
 * sync reports idle.
 */
export const screenerSeed = (accountId: string) =>
  call<number | null>("screener_seed", { accountId });
