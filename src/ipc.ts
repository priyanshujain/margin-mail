// Placeholder. The contract lands in F3.
import { invoke } from "@tauri-apps/api/core";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
export const isMacDesktop =
  isTauri && typeof navigator !== "undefined" && /mac/i.test(navigator.userAgent);

export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}
