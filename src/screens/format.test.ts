// The pure half of what the screens print. These are the functions with a right answer rather than
// a look, so they are asserted here instead of in the browser suite.
//
// `format.ts` reaches for a DOMParser in `previewOf` and for the binding table in `cap`, and it
// does both at call time, so importing the module in node is safe and the rest of it can be tested
// without a document.

import { describe, expect, it } from "vitest";
import { displayName, participantLine } from "./format";

describe("participantLine", () => {
  it("says nothing about nobody", () => {
    expect(participantLine([], false)).toBe("");
  });

  it("names one person as themselves", () => {
    expect(participantLine(["City Power"], false)).toBe("City Power");
  });

  it("puts you last, and only when you wrote in the thread", () => {
    expect(participantLine(["Arun Kulkarni"], true)).toBe("Arun Kulkarni and you");
    expect(participantLine(["Arun Kulkarni"], false)).toBe("Arun Kulkarni");
  });

  it("joins up to four in full, because a count would be longer than the name", () => {
    expect(participantLine(["Arun", "Maya", "Karthik", "Priya"], false)).toBe(
      "Arun, Maya, Karthik and Priya",
    );
    expect(participantLine(["Arun", "Maya", "Karthik"], true)).toBe("Arun, Maya, Karthik and you");
  });

  it("names three of a crowd and counts the rest", () => {
    const forty = Array.from({ length: 40 }, (_, i) => `Person ${i + 1}`);
    expect(participantLine(forty, false)).toBe("Person 1, Person 2, Person 3 and 37 others");
  });

  it("counts you among the others rather than losing the count to you", () => {
    const forty = Array.from({ length: 40 }, (_, i) => `Person ${i + 1}`);
    expect(participantLine(forty, true)).toBe("Person 1, Person 2, Person 3 and 38 others");
  });

  it("never runs past a line, however many people are in the thread", () => {
    const many = Array.from({ length: 400 }, (_, i) => `Somebody With A Long Name ${i}`);
    expect(participantLine(many, true).length).toBeLessThan(120);
  });
});

describe("displayName", () => {
  it("reads a sorting key back the way it was written", () => {
    expect(displayName({ name: "Young, Russell", address: "r@example.com" })).toBe("Russell Young");
  });

  it("leaves a name with two commas alone, because it is not a sorting key", () => {
    expect(displayName({ name: "Beale, Jones and Fry, LLP", address: "hi@bjf.example" })).toBe(
      "Beale, Jones and Fry, LLP",
    );
  });

  it("falls back to the address when there is no name", () => {
    expect(displayName({ name: null, address: "no-reply@example.com" })).toBe(
      "no-reply@example.com",
    );
  });
});
