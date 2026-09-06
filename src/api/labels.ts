import { call, type LabelInfo, type Undo } from "../ipc";

export const labelsList = (accountId: string | null) =>
  call<LabelInfo[]>("labels_list", { accountId });

export const labelApply = (keys: string[], labelId: string, on: boolean) =>
  call<Undo>("label_apply", { keys, labelId, on });

/** Applies a label and archives, which is what "move" means to a provider with labels. */
export const labelMove = (keys: string[], labelId: string) =>
  call<Undo>("label_move", { keys, labelId });
