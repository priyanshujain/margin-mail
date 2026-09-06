import { useEffect, useRef } from "react";
import { Button, Icon, icons, Key, NO_AUTOFILL, Pill } from "../ui";
import { keyLabel } from "../keys/bindings";
import { registerCommands } from "../keys/commands";
import { useEscapeLayer } from "../escape";
import { useMail } from "../store/useMail";
import { useSearch } from "../store/useSearch";
import "./search.css";

/** Long enough that a fast typist sends one query rather than eight, short enough to feel live. */
const SETTLE_MS = 140;

/**
 * Search lives in the header, where it is on every screen and where `/` can always reach it.
 *
 * The field is a field and nothing more: the query goes to Rust, which parses the operators, and
 * the results land in the list column. Escape gives the place back that the results took.
 */
export function SearchBar() {
  const phase = useSearch((s) => s.phase);
  const query = useSearch((s) => s.query);
  const setQuery = useSearch((s) => s.setQuery);
  const run = useSearch((s) => s.run);
  const close = useSearch((s) => s.close);
  const openSearch = useSearch((s) => s.open);

  const field = useRef<HTMLInputElement | null>(null);
  const on = phase !== "off";

  useEffect(
    () =>
      registerCommands({
        search: () => {
          useSearch.getState().open();
          // Already open is not nothing: `/` a second time is how you get back to the query.
          field.current?.select();
        },
      }),
    [],
  );

  useEffect(() => {
    if (on) field.current?.focus();
  }, [on]);

  // One query per pause rather than one per keystroke. The phase is deliberately not a dependency:
  // it changes twice inside every run, and an effect that watched it would search forever.
  useEffect(() => {
    if (!on) return;
    const timer = window.setTimeout(() => void run(), SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [query, on, run]);

  useEscapeLayer(on, close);

  if (!on) {
    return (
      <Button
        variant="ghost"
        iconOnly
        icon={icons.SEARCH}
        title={`Search (${keyLabel("/")})`}
        onClick={openSearch}
      />
    );
  }

  return (
    <div className="search" data-phase={phase} aria-busy={phase === "searching"}>
      <Icon d={icons.SEARCH} size={14} />
      <input
        ref={field}
        className="search-input"
        type="text"
        value={query}
        aria-label="Search"
        placeholder="Search this mailbox"
        {...NO_AUTOFILL}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          // Down and Return hand the keyboard to the results, which is where the verbs are.
          if (e.key !== "ArrowDown" && e.key !== "Enter") return;
          const first = useMail.getState().threads[0];
          if (!first) return;
          e.preventDefault();
          useMail.getState().focus(first.key);
          field.current?.blur();
        }}
      />
      <Key size="sm">⎋</Key>
    </div>
  );
}

/**
 * The operators, drawn rather than understood.
 *
 * Rust parses the query and this recognises the shape of an operator so that `from:maya` reads as
 * something the app knows about rather than as a typo. It is not a parser and it must not become
 * one: if the two ever disagree, the answer in the list is the one that is right.
 */
const OPERATOR = /^[a-z]+:./i;

export function QueryTerms({ query }: { query: string }) {
  const terms = query.split(/\s+/).filter(Boolean);
  if (terms.length === 0) return null;
  return (
    <span className="search-terms">
      {terms.map((term, at) =>
        OPERATOR.test(term) ? (
          <Pill key={`${term}-${at}`} tone="wash">
            {term}
          </Pill>
        ) : (
          <span className="search-word" key={`${term}-${at}`}>
            {term}
          </span>
        ),
      )}
    </span>
  );
}

export default SearchBar;
