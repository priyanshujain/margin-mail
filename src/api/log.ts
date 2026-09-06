import { call } from "../ipc";

/**
 * One line into the app's log file, beside what the engine writes there. For the failures only
 * the webview sees: an uncaught error, a promise nobody caught, an open that took too long. A
 * command that answers with an error is already written down by `call` itself.
 */
export const logNote = (who: string, line: string) =>
  call<void>("log_note", { who, line }).catch(() => {});
