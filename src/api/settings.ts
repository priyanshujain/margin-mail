import { call, type Settings, type StorageUsed } from "../ipc";

export const settingsGet = () => call<Settings>("settings_get");

/** A partial patch. Absent fields are unchanged. Written atomically. */
export const settingsSet = (patch: Partial<Settings>) => call<Settings>("settings_set", { patch });

/** The families installed on this machine, for the Appearance section's two font pickers. */
export const systemFonts = () => call<string[]>("system_fonts");

/** The path of the keymap file, so the Keyboard section can open it in the editor. */
export const keymapPath = () => call<string>("keymap_path");

export const keymapReset = () => call<void>("keymap_reset");

export const storageUsed = () => call<StorageUsed[]>("storage_used");

/** Throws the mirror away and syncs it again. The state database is untouched. */
export const mirrorClear = (accountId: string) => call<void>("mirror_clear", { accountId });

export const exportMbox = (accountId: string) => call<string>("export_mbox", { accountId });

export const exportState = () => call<string>("export_state");

export const importState = (path: string) => call<void>("import_state", { path });

/** The package manager that owns this install, when one does. */
export const packagedBy = () => call<string | null>("packaged_by");
