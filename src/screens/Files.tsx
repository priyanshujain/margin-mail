import { useEffect, useMemo } from "react";
import { Button, EmptyState, Icon, icons } from "../ui";
import type { FileCard, Person } from "../ipc";
import { CATEGORIES, useLibrary } from "../store/useLibrary";
import { useMail } from "../store/useMail";
import { openThread } from "./Clips";
import { displayName, fileKind, fileSize } from "./format";
import "./list.css";
import "./library.css";

/**
 * The All files place: every attachment in the mirror as a card, newest first.
 *
 * A grid rather than a list, because what you are looking for here is a document you half
 * remember, and a filename and a type mark are what you recognise it by. Nothing is fetched: the
 * cards are the local index, and the file itself is downloaded when the thread it is in is opened.
 */
const fileDate = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

/** Everyone who has sent a file, for the second filter. Drawn from the cards rather than asked for. */
function sendersOf(files: FileCard[]): Person[] {
  const seen = new Map<string, Person>();
  for (const card of files) {
    if (!seen.has(card.sender.address)) seen.set(card.sender.address, card.sender);
  }
  return [...seen.values()].sort((a, b) => displayName(a).localeCompare(displayName(b)));
}

export function Files() {
  const files = useLibrary((s) => s.files);
  const phase = useLibrary((s) => s.filesPhase);
  const category = useLibrary((s) => s.category);
  const sender = useLibrary((s) => s.sender);
  const load = useLibrary((s) => s.loadFiles);
  const setCategory = useLibrary((s) => s.setCategory);
  const setSender = useLibrary((s) => s.setSender);
  const accountId = useMail((s) => s.accountId);

  useEffect(() => {
    void load();
  }, [accountId, load]);

  // The sender list is whoever is in the answer, which means it narrows with the type filter and
  // never offers a name that would give back nothing.
  const senders = useMemo(() => sendersOf(files), [files]);

  return (
    <main className="stage library">
      <div className="list-head">
        <h1 className="list-title">All files</h1>
      </div>

      <div className="files-filters">
        {CATEGORIES.map((option) => (
          <Button
            key={option.id || "all"}
            variant="ghost"
            size="sm"
            active={category === option.id}
            onClick={() => setCategory(option.id)}
          >
            {option.label}
          </Button>
        ))}
        <span className="files-gap" />
        <label className="files-sender">
          From
          {/* The platform's own dropdown chrome is dropped and the app's is put back, so the
              chevron beside it is the same drawing as every other chevron here. */}
          <span className="files-picker">
            <select value={sender} onChange={(e) => setSender(e.target.value)} aria-label="From">
              <option value="">Anyone</option>
              {senders.map((person) => (
                <option key={person.address} value={person.address}>
                  {displayName(person)}
                </option>
              ))}
              {/* The chosen sender survives a type filter that has nothing of theirs in it, or the
                  control would silently reset itself to Anyone and show more than was asked for. */}
              {sender && !senders.some((p) => p.address === sender) ? (
                <option value={sender}>{sender}</option>
              ) : null}
            </select>
            <Icon d={icons.CHEVRON_DOWN} size={12} />
          </span>
        </label>
      </div>

      {files.length === 0 ? (
        <div className="list list-blank">
          {phase === "loading" ? null : <EmptyState>Nothing here</EmptyState>}
        </div>
      ) : (
        <div className="list files-grid">
          {files.map((card) => (
            <button
              type="button"
              className="file-card"
              key={card.attachment.id}
              data-category={card.category}
              onClick={() => openThread(card.threadKey)}
            >
              <span className="file-mark">
                {fileKind(card.attachment.filename, card.attachment.mimeType)}
              </span>
              <span className="file-name">{card.attachment.filename}</span>
              <span className="file-meta">
                {`${displayName(card.sender)} · ${fileSize(card.attachment.size)}`}
              </span>
              <span className="file-meta">
                {`${card.subject} · ${fileDate.format(card.dateMs)}`}
              </span>
            </button>
          ))}
        </div>
      )}
    </main>
  );
}

export default Files;
