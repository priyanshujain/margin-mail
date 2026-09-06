import { describe, expect, it } from "vitest";
import { placesSentence, withPlace } from "./notifyPlaces";

describe("withPlace", () => {
  it("turns a place on in the section's order rather than at the end", () => {
    expect(withPlace([], "feed", true)).toEqual(["feed"]);
    expect(withPlace(["feed"], "inbox", true)).toEqual(["inbox", "feed"]);
    expect(withPlace(["inbox", "feed"], "paper-trail", true)).toEqual(["inbox", "feed", "paper-trail"]);
  });

  it("turns a place off and leaves the rest alone", () => {
    expect(withPlace(["inbox", "feed"], "inbox", false)).toEqual(["feed"]);
    expect(withPlace(["feed"], "inbox", false)).toEqual(["feed"]);
  });

  it("does not list a place twice", () => {
    expect(withPlace(["inbox"], "inbox", true)).toEqual(["inbox"]);
  });
});

describe("placesSentence", () => {
  it("says nothing is on when nothing is", () => {
    expect(placesSentence([])).toBe("Nothing notifies you yet.");
  });

  it("names one, two or three places as a sentence", () => {
    expect(placesSentence(["inbox"])).toBe("Notifications are on for the Inbox.");
    expect(placesSentence(["inbox", "feed"])).toBe("Notifications are on for the Inbox and the Feed.");
    expect(placesSentence(["feed", "inbox", "paper-trail"])).toBe(
      "Notifications are on for the Inbox, the Feed and the Paper Trail.",
    );
  });
});
