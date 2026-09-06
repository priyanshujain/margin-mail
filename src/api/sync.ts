import { call, type SyncStatus } from "../ipc";

/** One account, or every account when the id is omitted. */
export const syncNow = (accountId?: string) => call<SyncStatus[]>("sync_now", { accountId });

export const syncStatus = () => call<SyncStatus[]>("sync_status");

/** Drains the outbox. The close-request hook races this against a timeout. */
export const syncFlush = () => call<void>("sync_flush");

/** Fills in the range a widened storage window newly covers, newest first. */
export const syncBackfill = (accountId: string) => call<void>("sync_backfill", { accountId });
