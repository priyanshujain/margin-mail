import { call, type NotifyPermission, type NotifyTarget } from "../ipc";

/** Posts one sample notification. */
export const notifyTest = () => call<void>("notify_test");

/**
 * Whether the system will show this app's notifications: what its settings say, and "prompt"
 * until the app has asked once. Where there is no permission to ask for the answer is "granted".
 */
export const notifyPermission = () => call<NotifyPermission>("notify_permission");

/** Asks, when the system has not been asked yet, and says what the answer is. */
export const askForNotifications = () => call<NotifyPermission>("notify_request");

/** Opens the system's own notification settings for this app, where a refusal is undone. */
export const openNotificationSettings = () => call<void>("notify_open_settings");

/**
 * Where the last click on a notification pointed, once: null after the first read, and always for
 * the sample from the settings screen.
 */
export const notifyTake = () => call<NotifyTarget | null>("notify_take");

