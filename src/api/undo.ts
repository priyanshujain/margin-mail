import { call } from "../ipc";

/** `z`. Reverses the most recent reversible action, including a send inside its delay. */
export const undoLast = () => call<string | null>("undo_last");

/** The toast's own Undo button, which names the action it belongs to rather than the latest one. */
export const undoToken = (token: string) => call<void>("undo_token", { token });
