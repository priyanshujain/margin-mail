// The table's own invariants. Two bindings on one combo in one context would silently shadow each
// other, and a group the sheet does not render would silently hide a key, so both are asserted here
// rather than discovered later.
//
// This imports the table alone, which is why the labels live on the bindings: `commands.ts` reaches
// for the stores and for Tauri the moment it is loaded.

import { describe, expect, it } from "vitest";
import {
  ACCOUNT_KEYS,
  BINDINGS,
  GROUPS,
  keyLabel,
  keysFor,
  normalizeCombo,
} from "./bindings";

describe("the binding table", () => {
  it("never binds one combo twice in the same context", () => {
    const seen = new Set<string>();
    for (const binding of BINDINGS) {
      for (const key of binding.keys) {
        const slot = `${binding.context}:${normalizeCombo(key)}`;
        expect(seen.has(slot), `${slot} is bound twice`).toBe(false);
        seen.add(slot);
      }
    }
  });

  it("puts every binding in a group the sheet renders", () => {
    for (const binding of BINDINGS) expect(GROUPS).toContain(binding.group);
  });

  it("gives every binding a label, including the ones it does not own", () => {
    for (const binding of BINDINGS) expect(binding.label).not.toBe("");
  });

  it("documents Escape without claiming to handle it", () => {
    const escape = BINDINGS.find((b) => b.keys.includes("Escape"));
    expect(escape?.command).toBeNull();
  });

  it("leaves the editor's own keys to the editor", () => {
    for (const combo of ["cmd+b", "cmd+i", "cmd+k"]) {
      const inEditor = BINDINGS.find((b) => b.context === "editor" && b.keys.includes(combo));
      expect(inEditor?.command, `${combo} in the editor`).toBeNull();
    }
  });

  it("keeps the palette reachable from a text field", () => {
    const palette = BINDINGS.find((b) => b.command === "command-palette");
    expect(palette?.allowInInput).toBe(true);
    expect(palette?.context).toBe("global");
  });

  it("reuses y, v and n only on the cards docs/keyboard.md allows", () => {
    for (const key of ["y", "v", "n"]) {
      const contexts = BINDINGS.filter((b) => b.keys.includes(key)).map((b) => b.context);
      for (const context of contexts) {
        expect(["view", "screener", "invite"]).toContain(context);
      }
    }
  });

  it("does not let the account keys collide with a place key", () => {
    for (const combo of ACCOUNT_KEYS) {
      const clash = BINDINGS.find((b) => b.keys.map(normalizeCombo).includes(combo));
      expect(clash, `${combo} is both an account and a binding`).toBeUndefined();
    }
  });

  it("keeps Cmd+A and Cmd+Shift+A apart", () => {
    expect(normalizeCombo("cmd+a")).not.toBe(normalizeCombo("cmd+shift+a"));
    expect(keysFor("select-all").map(normalizeCombo)).toContain("cmd+a");
    expect(keysFor("attach").map(normalizeCombo)).toContain("cmd+shift+a");
  });

  it("reads a combo the same way the dispatcher builds one", () => {
    expect(normalizeCombo("Cmd+K")).toBe("cmd+k");
    expect(normalizeCombo("cmd+k")).toBe("cmd+k");
    expect(normalizeCombo("H")).toBe("shift+h");
    expect(normalizeCombo("shift+h")).toBe("shift+h");
    expect(normalizeCombo("/")).toBe("/");
    expect(normalizeCombo("shift+Tab")).toBe("shift+Tab");
    expect(normalizeCombo("shift+ ")).toBe("shift+ ");
  });

  it("prints a key the way a keycap does", () => {
    expect(keyLabel("shift+h")).toBe("⇧H");
    expect(keyLabel("h")).toBe("h".toUpperCase());
    expect(keyLabel("Enter")).toBe("↩");
    expect(keyLabel("Escape")).toBe("⎋");
    expect(keyLabel(" ")).toBe("Space");
  });
});
