import { call, isTauri } from "../ipc";

export type UpdateChannel = "direct" | "appstore" | "none";

export function updateChannel(): Promise<UpdateChannel> {
  return isTauri ? call("update_channel") : Promise.resolve("none");
}

export function openAppStoreUpdates(): Promise<void> {
  return call("open_appstore_updates");
}
