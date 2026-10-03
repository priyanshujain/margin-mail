import { call, isTauri } from "../ipc";

export interface AppLockStatus {
  enabled: boolean;
  locked: boolean;
  available: boolean;
  unavailableReason: string | null;
}

export const appLockStatus = (): Promise<AppLockStatus> =>
  isTauri
    ? call<AppLockStatus>("app_lock_status")
    : Promise.resolve({ enabled: false, locked: false, available: false, unavailableReason: null });

export const appUnlock = () => call<void>("app_unlock");
