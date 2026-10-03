import { call, isMacDesktop } from "../ipc";

export const writingToolsAvailable = () =>
  isMacDesktop ? call<boolean>("writing_tools_available") : Promise.resolve(false);

export const runWritingTool = (tool: "Proofread" | "Rewrite") =>
  call<void>("run_writing_tool", { tool });
