import { call, type MessageView } from "../ipc";

/**
 * Re-renders one message with its remote images fetched. Rust does the fetching, without cookies or
 * a referrer, and inlines the results as `data:` URIs, so the webview never makes a request of its
 * own and the sender learns nothing but that somebody asked once.
 */
export const messageShowImages = (messageId: string) =>
  call<MessageView>("message_show_images", { messageId });

/** The bytes of an attachment as a `data:` URI, for the inline preview. Fetched on demand. */
export const attachmentDataUrl = (attachmentId: string) =>
  call<string>("attachment_data_url", { attachmentId });

/** Writes it to the downloads directory and returns the path. */
export const attachmentSave = (attachmentId: string) =>
  call<string>("attachment_save", { attachmentId });

/** Hands it to the OS to open with whatever owns the type. */
export const attachmentOpen = (attachmentId: string) =>
  call<void>("attachment_open", { attachmentId });
