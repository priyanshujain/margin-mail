import { useEffect, useRef } from "react";
import { useEscapeLayer } from "../escape";
import { NO_AUTOFILL } from "./autofill";
import { Key } from "./Key";
import "./Palette.css";

export interface PaletteItem {
  id: string;
  label: string;
  /** The half sentence after the label: "and 3 more", "currently 10 seconds". */
  hint?: string;
  /** The keys this command answers to, printed as caps. */
  keys?: string[];
}

export interface PaletteGroup {
  id: string;
  label: string;
  items: PaletteItem[];
}

export interface PaletteProps {
  open: boolean;
  query: string;
  onQuery: (query: string) => void;
  groups: PaletteGroup[];
  /** The row the arrow keys are on. Whoever owns the keymap moves it. */
  activeId?: string;
  onChoose: (id: string) => void;
  onClose: () => void;
  placeholder?: string;
  /** Shown in place of the list when the query matches nothing. */
  empty?: string;
}

/**
 * The command palette's shell: a scrim, a panel near the top, one field, and a grouped list whose
 * rows print their keys.
 *
 * The shell only. What the groups contain, how a query filters them and which row is active are
 * all the caller's, because the palette is the whole settings and discovery surface of the app and
 * that data has no business being in the design system.
 */
export function Palette({
  open,
  query,
  onQuery,
  groups,
  activeId,
  onChoose,
  onClose,
  placeholder = "Go to a place, run a command, or find a person",
  empty = "Nothing matches",
}: PaletteProps) {
  useEscapeLayer(open, onClose);
  const input = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (open) input.current?.focus();
  }, [open]);

  if (!open) return null;

  const anything = groups.some((g) => g.items.length > 0);

  return (
    <div
      className="overlay"
      data-align="top"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="panel palette" role="dialog" aria-modal="true" aria-label="Command palette">
        <input
          ref={input}
          className="palette-input"
          type="text"
          value={query}
          placeholder={placeholder}
          aria-label={placeholder}
          {...NO_AUTOFILL}
          onChange={(e) => onQuery(e.target.value)}
        />
        {anything ? (
          <ul className="palette-list" role="listbox">
            {groups
              .filter((g) => g.items.length > 0)
              .map((g) => (
                <li key={g.id} role="presentation">
                  <div className="palette-group">{g.label}</div>
                  <ul className="palette-items" role="group" aria-label={g.label}>
                    {g.items.map((it) => (
                      <li
                        key={it.id}
                        className="palette-row"
                        role="option"
                        aria-selected={it.id === activeId}
                        data-active={it.id === activeId ? "" : undefined}
                        onClick={() => onChoose(it.id)}
                      >
                        <span className="palette-label">
                          {it.label}
                          {it.hint ? <span className="hint">{it.hint}</span> : null}
                        </span>
                        {it.keys?.length ? (
                          <span className="palette-keys">
                            {it.keys.map((k) => (
                              <Key key={k} size="md">
                                {k}
                              </Key>
                            ))}
                          </span>
                        ) : null}
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
          </ul>
        ) : (
          <p className="palette-empty">{empty}</p>
        )}
      </div>
    </div>
  );
}
