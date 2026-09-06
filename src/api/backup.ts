import { call, type BackupSettings } from "../ipc";

export const backupStatus = () => call<BackupSettings>("backup_status");

/** `store` is "drive" or "r2". R2 takes the four S3 fields; Drive takes none. */
export const backupConfigure = (store: string, config: Record<string, string>) =>
  call<BackupSettings>("backup_configure", { store, config });

export const backupNow = () => call<BackupSettings>("backup_now");

/** Shown once, at setup, and never returned again. */
export const backupPhrase = () => call<string>("backup_phrase");

/** Attaches this device to an existing backup, which is also how a lost device is replaced. */
export const backupRestore = (phrase: string) => call<void>("backup_restore", { phrase });
