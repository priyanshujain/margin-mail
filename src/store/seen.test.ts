// What opening a thread does to the page of rows it came from, which is the half of "opened means
// seen" that lives on this side: the dot goes now, the row stays put until the next reload.

import { describe, expect, it } from "vitest";
import type { ThreadSummary } from "../ipc";
import { seenInPlace } from "./seen";

function row(key: string, unseen: boolean, group: string): ThreadSummary {
  return {
    key,
    accountId: "acct-1",
    accountColor: "hue-1",
    subject: "The lease",
    originalSubject: null,
    from: { name: "Ana", address: "ana@example.com" },
    participants: [],
    snippet: "",
    dateMs: 1_000,
    messageCount: 1,
    unseen,
    starred: false,
    trashed: false,
    spam: false,
    hasAttachment: false,
    hasDraft: false,
    pile: null,
    snoozedUntil: null,
    ignored: false,
    notify: false,
    merged: false,
    note: null,
    group,
    sending: false,
  };
}

describe("seenInPlace", () => {
  it("takes the dot off the opened row and touches nothing else", () => {
    const other = row("b", true, "new");
    const after = seenInPlace([row("a", true, "new"), other], "a");
    expect(after[0].unseen).toBe(false);
    expect(after[1]).toBe(other);
  });

  it("leaves the group alone, so the row does not jump into the seen band mid-read", () => {
    const after = seenInPlace([row("a", true, "new")], "a");
    expect(after[0].group).toBe("new");
    expect(after[0].key).toBe("a");
  });

  it("hands the same array back when there was nothing to do", () => {
    const seen = [row("a", false, "seen")];
    expect(seenInPlace(seen, "a")).toBe(seen);
    expect(seenInPlace(seen, "not-here")).toBe(seen);
    expect(seenInPlace([], "a")).toEqual([]);
  });
});
