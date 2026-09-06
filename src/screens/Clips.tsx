import { useEffect } from "react";
import { Button, EmptyState, icons } from "../ui";
import type { Clip } from "../ipc";
import { useLibrary } from "../store/useLibrary";
import { useMail } from "../store/useMail";
import { useStage } from "../store/useStage";
import { displayName } from "./format";
import "./list.css";
import "./library.css";

/**
 * The Clips place: every passage saved out of a message, newest first.
 *
 * A clip is a quotation, so it is drawn as one: the words first and where they came from
 * underneath, rather than a row with the text as a snippet. Each one goes back to the thread it
 * was taken from, which is the only thing a clip is for.
 */
const clipDate = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

/** Back to the mail, on the thread the clip came out of. */
export function openThread(threadKey: string): void {
  useStage.getState().close();
  void useMail.getState().open(threadKey);
}

export function Clips() {
  const clips = useLibrary((s) => s.clips);
  const phase = useLibrary((s) => s.clipsPhase);
  const load = useLibrary((s) => s.loadClips);
  const remove = useLibrary((s) => s.removeClip);
  const accountId = useMail((s) => s.accountId);

  useEffect(() => {
    void load();
  }, [accountId, load]);

  return (
    <main className="stage library">
      <div className="list-head">
        <h1 className="list-title">Clips</h1>
      </div>

      {clips.length === 0 ? (
        <div className="list list-blank">
          {phase === "loading" ? null : <EmptyState>Nothing saved yet</EmptyState>}
        </div>
      ) : (
        <div className="list clips-list">
          {clips.map((clip) => (
            <Passage key={clip.id} clip={clip} onDelete={() => void remove(clip.id)} />
          ))}
        </div>
      )}
    </main>
  );
}

function Passage({ clip, onDelete }: { clip: Clip; onDelete: () => void }) {
  return (
    <article className="clip">
      <button type="button" className="clip-open" onClick={() => openThread(clip.threadKey)}>
        <blockquote className="clip-text">{clip.text}</blockquote>
        <div className="clip-meta">
          <span className="clip-sender">{displayName(clip.sender)}</span>
          <span aria-hidden="true">·</span>
          <span className="clip-subject">{clip.subject}</span>
          <span className="clip-date">{clipDate.format(clip.createdAtMs)}</span>
        </div>
      </button>
      <Button
        variant="ghost"
        iconOnly
        icon={icons.TRASH}
        title="Delete this clip"
        label="Delete this clip"
        onClick={onDelete}
      />
    </article>
  );
}

export default Clips;
