import { create } from "zustand";
import { messageImagesSet, messageShowImages } from "../api/messages";
import type { MessageView } from "../ipc";
import { notify } from "./useToast";

type ImageContent = Pick<MessageView, "html" | "quotedHtml" | "trackers" | "blockedImages">;

interface ImageEntry {
  allowed: boolean;
  phase: "idle" | "loading" | "error";
  html: string;
  quotedHtml: string | null;
  content: ImageContent | null;
}

interface ImageState {
  entries: Record<string, ImageEntry>;
  ensure: (accountId: string, message: MessageView) => void;
  load: (accountId: string, message: MessageView) => Promise<void>;
  allow: (accountId: string, message: MessageView, allowed: boolean) => Promise<void>;
}

export const imageKey = (accountId: string, messageId: string): string => `${accountId}:${messageId}`;
const preferenceWrites = new Map<string, Promise<unknown>>();

export const useImages = create<ImageState>((set, get) => ({
  entries: {},
  ensure: (accountId, message) => {
    const key = imageKey(accountId, message.id);
    const prior = get().entries[key];
    if (prior?.html === message.html && prior.quotedHtml === message.quotedHtml) return;
    const entry: ImageEntry = {
      allowed: prior?.allowed ?? message.imagesAllowed ?? false,
      phase: "idle",
      html: message.html,
      quotedHtml: message.quotedHtml,
      content: null,
    };
    set((s) => ({ entries: { ...s.entries, [key]: entry } }));
    if (entry.allowed && !message.bodyPending) void get().load(accountId, message);
  },
  load: async (accountId, message) => {
    const key = imageKey(accountId, message.id);
    const pending = preferenceWrites.get(key);
    if (pending) {
      try {
        await pending;
      } catch {
        return;
      }
      if (preferenceWrites.has(key) && preferenceWrites.get(key) !== pending) return;
    }
    const entry = get().entries[key];
    if (!entry || !entry.allowed || entry.phase === "loading" || message.bodyPending) return;
    const loading = { ...entry, phase: "loading" as const };
    set((s) => ({ entries: { ...s.entries, [key]: loading } }));
    try {
      const view = await messageShowImages(accountId, message.id);
      if (get().entries[key] !== loading) return;
      const content = { html: view.html, quotedHtml: view.quotedHtml, trackers: view.trackers, blockedImages: view.blockedImages };
      set((s) => ({ entries: { ...s.entries, [key]: { ...loading, phase: "idle", content } } }));
    } catch (e) {
      if (get().entries[key] !== loading) return;
      set((s) => ({ entries: { ...s.entries, [key]: { ...loading, phase: "error" } } }));
      notify(`Could not load images: ${e}`);
    }
  },
  allow: async (accountId, message, allowed) => {
    const key = imageKey(accountId, message.id);
    const prior = get().entries[key] ?? {
      allowed: message.imagesAllowed ?? false,
      phase: "idle" as const,
      html: message.html,
      quotedHtml: message.quotedHtml,
      content: null,
    };
    const next = { ...prior, allowed, phase: "idle" as const };
    set((s) => ({ entries: { ...s.entries, [key]: next } }));
    const write = (preferenceWrites.get(key) ?? Promise.resolve())
      .catch(() => {})
      .then(() => messageImagesSet(accountId, message.id, allowed));
    preferenceWrites.set(key, write);
    try {
      await write;
      if (get().entries[key] !== next) return;
      if (allowed && !next.content) await get().load(accountId, message);
    } catch (e) {
      if (get().entries[key] === next) set((s) => ({ entries: { ...s.entries, [key]: { ...prior, phase: prior.allowed && !prior.content ? "error" : "idle" } } }));
      notify(`Could not save the image preference: ${e}`);
    } finally {
      if (preferenceWrites.get(key) === write) preferenceWrites.delete(key);
    }
  },
}));
