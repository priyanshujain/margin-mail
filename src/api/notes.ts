import { call, type Clip, type FileCard, type Note, type Undo } from "../ipc";

export const noteAdd = (threadKey: string, body: string) =>
  call<Note>("note_add", { threadKey, body });

export const noteUpdate = (id: string, body: string) => call<void>("note_update", { id, body });

export const noteDelete = (id: string) => call<Undo>("note_delete", { id });

export const clipSave = (accountId: string, threadKey: string, messageId: string, text: string) =>
  call<Clip>("clip_save", { accountId, threadKey, messageId, text });

export const clipsList = (accountId: string | null) => call<Clip[]>("clips_list", { accountId });

export const clipDelete = (id: string) => call<Undo>("clip_delete", { id });

/** `category` filters the All files place: images, pdfs, documents and the rest, or "" for all. */
export const filesList = (accountId: string | null, category: string, sender: string) =>
  call<FileCard[]>("files_list", { accountId, category, sender });
