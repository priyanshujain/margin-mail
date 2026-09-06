import { Fragment } from "react";
import { Key } from "../../ui";
import { labelFor, type CommandId } from "../../keys/bindings";
import type { Place } from "../../ipc";
import { useMail } from "../../store/useMail";
import { useOverlays } from "../../store/useOverlays";
import { useSettings } from "../../store/useSettings";
import { cap } from "../format";
import { sectionOf, titleOf } from "./content";
import { Figure } from "./Figures";
import { PICTURES, type Article, type Block, type Piece } from "./types";

/**
 * One article: a heading, prose, sometimes numbered steps, sometimes one picture.
 *
 * The markup is here and the words are in the section files, which is what lets the filter search
 * what an article says without rendering it.
 */

/**
 * A keycap, always the binding table's answer for that verb rather than a letter typed into the
 * copy, so a remapped key teaches the key it was remapped to. `cap` is what every button in the app
 * prints: the bare letter when it is unmodified, real glyphs when it is not.
 *
 * The space in front of it is inside the span rather than at the end of the sentence before it,
 * because on a phone the cap is not drawn: a space left behind there prints as "the Paper Trail .
 * Anything in the wrong one", which is a sentence with a hole in it.
 */
function Cap({ of }: { of: CommandId }) {
  const key = cap(of);
  if (!key) return null;
  return (
    <span className="guide-cap">
      {" "}
      <Key>{key}</Key>
    </span>
  );
}

/**
 * A link out of the guide closes the guide behind it, because the thing it points at is underneath
 * the panel and a guide left open over it would be one you have to dismiss twice. Coming back is
 * the question mark in the corner.
 */
function goToPlace(place: Place): void {
  useOverlays.getState().close();
  useMail.getState().goTo(place);
}

function openSettings(): void {
  useOverlays.getState().close();
  useSettings.getState().show();
}

function Pieces({ pieces, onOpen }: { pieces: readonly Piece[]; onOpen: (id: string) => void }) {
  return (
    <>
      {pieces.map((piece, at) => {
        if (typeof piece === "string") return <Fragment key={at}>{piece}</Fragment>;
        if ("cap" in piece) return <Cap key={at} of={piece.cap} />;
        if ("see" in piece) {
          const id = piece.see;
          return (
            <button key={at} type="button" className="guide-link" onClick={() => onOpen(id)}>
              {titleOf(id)}
            </button>
          );
        }
        if ("place" in piece) {
          const place = piece.place;
          return (
            <button key={at} type="button" className="guide-link" onClick={() => goToPlace(place)}>
              {piece.label}
            </button>
          );
        }
        return (
          <button key={at} type="button" className="guide-link" onClick={openSettings}>
            {piece.settings}
          </button>
        );
      })}
    </>
  );
}

function BlockView({ block, onOpen }: { block: Block; onOpen: (id: string) => void }) {
  if ("p" in block) {
    return (
      <p className="guide-p">
        <Pieces pieces={block.p} onOpen={onOpen} />
      </p>
    );
  }

  if ("steps" in block) {
    return (
      <ol className="guide-steps">
        {block.steps.map((step, at) => (
          <li key={at}>
            <Pieces pieces={step} onOpen={onOpen} />
          </li>
        ))}
      </ol>
    );
  }

  if ("note" in block) {
    return (
      <p className="guide-note">
        <Pieces pieces={block.note} onOpen={onOpen} />
      </p>
    );
  }

  // Nothing in a key table is written by the article: the cap is what the keymap answers to and the
  // words beside it are the binding's own label, which is what the palette and the shortcut sheet
  // print for the same verb. A remap moves all three together.
  if ("keys" in block) {
    return (
      <dl className="guide-keys">
        {block.keys.map((id) => {
          const key = cap(id);
          return (
            <div className="guide-key" key={id}>
              <dt>{key ? <Key>{key}</Key> : null}</dt>
              <dd>{labelFor(id)}</dd>
            </div>
          );
        })}
      </dl>
    );
  }

  if ("figure" in block) {
    return (
      <figure className="guide-figure" data-drawn="">
        <Figure of={block.figure} />
        <figcaption>{block.caption}</figcaption>
      </figure>
    );
  }

  // The size the picture was, which is half the pixels it was captured at, on the element rather
  // than in the stylesheet: it is a fact about that one file, the browser holds the room for it
  // before it arrives, and the stylesheet is left with the one thing that is a layout decision,
  // which is what happens when the column is narrower than the picture.
  const size = PICTURES[block.picture];
  return (
    <figure className="guide-figure">
      <img
        src={`/guide/${block.picture}.png`}
        alt={block.alt}
        width={size.width}
        height={size.height}
      />
      <figcaption>{block.caption}</figcaption>
    </figure>
  );
}

export function ArticleView({
  article,
  onOpen,
}: {
  article: Article;
  onOpen: (id: string) => void;
}) {
  const section = sectionOf(article.id);
  return (
    <article className="guide-article">
      {section ? <p className="guide-eyebrow">{section.title}</p> : null}
      <h2 className="guide-title">{article.title}</h2>
      {article.blocks.map((block, at) => (
        <BlockView key={at} block={block} onOpen={onOpen} />
      ))}
    </article>
  );
}
