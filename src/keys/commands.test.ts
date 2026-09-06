import { describe, expect, it } from "vitest";
import { commandMatches, isRegistered, registerCommands, runCommand } from "./commands";

describe("the command registry", () => {
  it("does nothing when nobody owns a command", () => {
    expect(() => runCommand("archive")).not.toThrow();
    expect(isRegistered("archive")).toBe(false);
  });

  it("runs the handler that is mounted", () => {
    let ran = 0;
    const stop = registerCommands({ archive: () => (ran += 1) });
    runCommand("archive");
    expect(ran).toBe(1);
    stop();
    runCommand("archive");
    expect(ran).toBe(1);
  });

  it("lets a screen take a verb over and hand it back", () => {
    const order: string[] = [];
    const stopList = registerCommands({ reply: () => order.push("list") });
    const stopFocus = registerCommands({ reply: () => order.push("focus") });
    runCommand("reply");
    stopFocus();
    runCommand("reply");
    stopList();
    expect(order).toEqual(["focus", "list"]);
  });

  it("matches a label the way a palette does", () => {
    expect(commandMatches("Paper Trail", "pt")).toBe(true);
    expect(commandMatches("Archive", "arc")).toBe(true);
    expect(commandMatches("Archive", "")).toBe(true);
    expect(commandMatches("Archive", "zz")).toBe(false);
  });
});
