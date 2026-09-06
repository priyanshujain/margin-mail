// The two pure decisions the header and the toast make from a `SyncStatus`: which word the account
// chip prints, and whether a status is worth saying out loud. Both had the same bug, in that every
// failure that was not the network read as "Signed out" and was announced again on every pass.

import { describe, expect, it } from "vitest";
import type { SyncStatus } from "../ipc";
import { announce, arrival, filling, trouble } from "./useSync";

const status = (over: Partial<SyncStatus>): SyncStatus => ({
  accountId: "acct",
  phase: "idle",
  lastSyncMs: null,
  error: null,
  pendingWrites: 0,
  message: null,
  hydrated: 0,
  total: 0,
  oldestMs: null,
  ...over,
});

describe("trouble", () => {
  it("says nothing while nothing is wrong", () => {
    expect(trouble([status({ phase: "syncing", message: "Listing your mail" })], "acct")).toBeNull();
    expect(trouble([status({ phase: "caching", message: "Caching recent mail" })], null)).toBeNull();
  });

  it("prints the engine's word for a stopped account rather than guessing from the phase", () => {
    const refused = status({
      phase: "error",
      message: "Sync trouble",
      error: "Gmail label change failed (400): Invalid id value",
    });
    expect(trouble([refused], "acct")).toBe("Sync trouble");
    expect(trouble([status({ phase: "error", message: "Signed out", error: "signed out: x" })], "acct")).toBe(
      "Signed out",
    );
    expect(trouble([status({ phase: "error", message: "Rate limited" })], "acct")).toBe("Rate limited");
  });

  it("never says signed out on its own", () => {
    expect(trouble([status({ phase: "error", error: "something" })], "acct")).toBe("Sync trouble");
  });

  it("keeps offline and paused to one word whatever the engine's sentence was", () => {
    expect(trouble([status({ phase: "offline", message: "Offline", error: "network: x" })], "acct")).toBe(
      "Offline",
    );
    expect(
      trouble([status({ phase: "paused", message: "Paused after repeated failures. Sync now to try again." })], "acct"),
    ).toBe("Paused");
  });

  it("only looks at the account that is showing", () => {
    const other = status({ accountId: "other", phase: "error", message: "Signed out" });
    expect(trouble([other, status({})], "acct")).toBeNull();
    expect(trouble([other, status({})], null)).toBe("Signed out");
  });
});

describe("announce", () => {
  const failed = status({
    phase: "error",
    message: "Signed out",
    error: "signed out: Token has been expired or revoked",
  });

  it("keeps sync trouble to the chip: the next poll is the retry and a toast is not", () => {
    const trouble = status({
      phase: "error",
      message: "Sync trouble",
      error: "Gmail history failed (500): Backend Error",
    });
    expect(announce(trouble, null).toast).toBeNull();
    expect(announce(trouble, null).last).toBeNull();
  });

  it("says a failure a person can mend once, not once per pass", () => {
    const first = announce(failed, null);
    expect(first.toast).toBe("Signed out: signed out: Token has been expired or revoked");
    // The next pass begins, reports the same failure, and is not news.
    const start = announce(status({ phase: "syncing" }), first.last);
    expect(start.toast).toBeNull();
    expect(start.last).toBe(first.last);
    expect(announce(failed, start.last).toast).toBeNull();
  });

  it("forgets what it said once a pass ends clean, so the trouble coming back is news", () => {
    const { last } = announce(failed, null);
    const clean = announce(status({ phase: "idle" }), last);
    expect(clean.last).toBeNull();
    expect(announce(failed, clean.last).toast).not.toBeNull();
  });

  it("does not forget on the status a pass begins with", () => {
    const { last } = announce(failed, null);
    expect(announce(status({ phase: "syncing" }), last).last).toBe(last);
    expect(announce(status({ phase: "hydrating", message: "Fetching" }), last).last).toBe(last);
  });

  it("says a missing permission, which nothing else will mend", () => {
    const scope = status({ phase: "error", message: "Needs permission", error: "missing permission: contacts" });
    expect(announce(scope, null).toast).toBe("Needs permission: missing permission: contacts");
  });

  it("says nothing for offline or a rate limit: the chip has those", () => {
    expect(
      announce(status({ phase: "offline", message: "Offline", error: "network: error sending request" }), null)
        .toast,
    ).toBeNull();
    expect(
      announce(status({ phase: "error", message: "Rate limited", error: "rate limited, retry in 8000ms" }), null)
        .toast,
    ).toBeNull();
  });

  it("says a refused write as its own sentence, mid-pass, without calling the sync failed", () => {
    const dropped = status({
      phase: "syncing",
      error: "Archiving 1 message was refused and dropped: Gmail label change failed (400): Invalid id value",
    });
    expect(announce(dropped, null).toast).toBe(dropped.error);
  });

  it("says a pause in the engine's words, whichever failure caused it", () => {
    const paused = status({
      phase: "paused",
      message: "Paused after repeated failures. Sync now to try again.",
      error: "rate limited, retry in 64000ms",
    });
    expect(announce(paused, null).toast).toBe("Paused after repeated failures. Sync now to try again.");
    expect(announce(paused, "Paused after repeated failures. Sync now to try again.").toast).toBeNull();
  });
});

describe("filling", () => {
  it("is the engine's own sentence and count while a crawl or a backfill runs", () => {
    const crawl = status({ phase: "hydrating", message: "Fetching the newest mail first", hydrated: 450, total: 1395 });
    expect(filling([crawl], ["acct"])).toEqual({
      accountId: "acct",
      message: "Fetching the newest mail first",
      hydrated: 450,
      total: 1395,
    });
    const listing = status({ phase: "syncing", message: "Listing your mail" });
    expect(filling([listing], ["acct"])?.message).toBe("Listing your mail");
    const later = status({ phase: "backfilling", message: "Fetching older mail", hydrated: 10, total: 40 });
    expect(filling([later], ["acct"])?.total).toBe(40);
  });

  it("is not an ordinary poll, which runs under the same phase with nothing to say", () => {
    expect(filling([status({ phase: "syncing", lastSyncMs: 1 })], ["acct"])).toBeNull();
    expect(filling([status({ phase: "caching", message: "Caching recent mail", lastSyncMs: 1 })], ["acct"])).toBeNull();
  });

  it("is an account resting before it has ever finished a pass, and not one that has", () => {
    expect(filling([status({ phase: "idle", lastSyncMs: null })], ["acct"])?.message).toBe("Bringing in your mail");
    expect(filling([status({ phase: "idle", lastSyncMs: 1 })], ["acct"])).toBeNull();
  });

  it("is not an account the engine has no status for, which is one it does not hold", () => {
    expect(filling([], ["acct"])).toBeNull();
  });

  it("is never a stopped account, whose chip has the word for it", () => {
    expect(filling([status({ phase: "paused", message: "Paused", lastSyncMs: null })], ["acct"])).toBeNull();
    expect(filling([status({ phase: "offline", lastSyncMs: null })], ["acct"])).toBeNull();
    expect(filling([status({ phase: "error", message: "Signed out", error: "x", lastSyncMs: null })], ["acct"])).toBeNull();
  });

  it("looks at every account on screen under All accounts", () => {
    const settled = status({ accountId: "old", lastSyncMs: 1 });
    const arriving = status({ accountId: "new", phase: "hydrating", message: "Fetching the newest mail first", hydrated: 1, total: 9 });
    expect(filling([settled, arriving], ["old", "new"])?.accountId).toBe("new");
    expect(filling([settled, arriving], ["old"])).toBeNull();
  });

  it("clamps a count that has run past a total not yet raised", () => {
    const over = status({ phase: "hydrating", message: "Fetching the newest mail first", hydrated: 12, total: 10 });
    expect(filling([over], ["acct"])?.hydrated).toBe(10);
  });
});

describe("arrival", () => {
  it("is working until a pass has ended clean, whatever the statuses on the way say", () => {
    expect(arrival(null)).toBe("working");
    expect(arrival(status({ phase: "syncing", message: "Listing your mail" }))).toBe("working");
    expect(arrival(status({ phase: "hydrating", message: "Fetching the newest mail first", hydrated: 3, total: 9 }))).toBe(
      "working",
    );
    // A first pass that failed quietly ends idle with no stamp: the next poll picks it up.
    expect(arrival(status({ phase: "idle", lastSyncMs: null }))).toBe("working");
  });

  it("is done on a stamped pass resting in idle, or already caching bodies behind a full Inbox", () => {
    expect(arrival(status({ phase: "idle", lastSyncMs: 1 }))).toBe("done");
    expect(arrival(status({ phase: "caching", message: "Caching recent mail", lastSyncMs: 1, hydrated: 4, total: 40 }))).toBe(
      "done",
    );
    // A stamp from an earlier life of the mirror does not end a pass that is still running.
    expect(arrival(status({ phase: "syncing", message: "Listing your mail", lastSyncMs: 1 }))).toBe("working");
  });

  it("is stalled on a stopped account, whichever way it stopped", () => {
    expect(arrival(status({ phase: "error", message: "Signed out", error: "x" }))).toBe("stalled");
    expect(arrival(status({ phase: "offline" }))).toBe("stalled");
    expect(arrival(status({ phase: "paused", message: "Paused after repeated failures. Sync now to try again." }))).toBe(
      "stalled",
    );
  });
});
