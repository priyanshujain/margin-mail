import { useEffect } from "react";
import type { MessageView, Surface } from "../ipc";
import { imageKey, useImages } from "../store/useImages";
import { Banner, icons } from "../ui";
import { MessageBody } from "./MessageBody";

interface MessageContentProps {
  accountId: string;
  message: MessageView;
  plain?: boolean;
  surface?: Surface;
  quoted?: boolean;
}

function useMessageImages(accountId: string, message: MessageView) {
  const entry = useImages((s) => s.entries[imageKey(accountId, message.id)]);
  const ensure = useImages((s) => s.ensure);
  useEffect(() => ensure(accountId, message), [accountId, message, ensure]);
  const allowed = entry?.allowed ?? message.imagesAllowed ?? false;
  const current = entry?.html === message.html && entry.quotedHtml === message.quotedHtml;
  const content = allowed && current && entry?.content ? entry.content : message;
  return { entry, allowed, current, content };
}

export function MessageImageBanner({ accountId, message }: Pick<MessageContentProps, "accountId" | "message">) {
  const { entry, allowed, current, content } = useMessageImages(accountId, message);
  const hasImages = message.blockedImages > 0 || /<img\b|\bbackground\s*=/i.test(message.html + (message.quotedHtml ?? ""));
  const trackers = content.trackers;
  const vendors = [...new Set(trackers.map((tracker) => tracker.vendor))];
  const failed = allowed && entry?.phase === "error";
  const loading = allowed && hasImages && !failed && (entry?.phase === "loading" || !current || !entry?.content);
  const missing = allowed && current && entry?.content ? content.blockedImages : 0;
  if (!(hasImages && !allowed) && !loading && !failed && missing === 0 && trackers.length === 0) return null;
  return <div className="thread-banner">
    <Banner
      icon={icons.SHIELD}
      action={hasImages && (!allowed || loading || failed || missing > 0) ? {
        label: allowed ? "Try again" : "Show images",
        busy: loading,
        busyLabel: "Loading images…",
        onClick: () => void (allowed ? useImages.getState().load(accountId, message) : useImages.getState().allow(accountId, message, true)),
      } : undefined}
    >
      {trackers.length > 0 ? <>Blocked <b>{`${trackers.length} tracker${trackers.length === 1 ? "" : "s"}`}</b>{vendors.length > 0 ? ` from ${vendors.join(", ")}. ` : ". "}</> : null}
      {hasImages && !allowed ? `${message.blockedImages > 0 ? "Remote" : "Embedded"} images are off for this email.` : failed ? "Could not load images." : missing > 0 ? `${missing} image${missing === 1 ? "" : "s"} did not load.` : null}
    </Banner>
  </div>;
}

export function MessageContent({ accountId, message, plain, surface, quoted }: MessageContentProps) {
  const { allowed, content } = useMessageImages(accountId, message);
  return <>
    <MessageBody html={content.html} plain={plain} surface={surface ?? message.surface} imagesAllowed={allowed} />
    {quoted && content.quotedHtml ? <MessageBody html={content.quotedHtml} plain={plain} surface={surface ?? message.surface} imagesAllowed={allowed} /> : null}
  </>;
}
