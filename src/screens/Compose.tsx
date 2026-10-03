import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type ClipboardEvent,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import type { Editor as TiptapEditor } from "@tiptap/react";
import { Avatar, Button, Icon, icons, Key, NO_AUTOFILL, Popover, Sheet } from "../ui";
import { useEscapeLayer } from "../escape";
import { registerCommands } from "../keys/commands";
import { useKeyContext } from "../keys/keymap";
import { contactsSuggest } from "../api/contacts";
import { draftAttachmentStore } from "../api/write";
import { notify } from "../store/useToast";
import { isTauri, type Draft, type DraftAttachment, type Person } from "../ipc";
import { useAccounts } from "../store/useAccounts";
import { storeAttachments, useCompose, type Composer, type ComposerAt } from "../store/useCompose";
import { useMail } from "../store/useMail";
import { useSettings } from "../store/useSettings";
import { Editor } from "./Editor";
import { accountHue, cap, displayName, fileKind, fileSize, isBrand } from "./format";
import "./compose.css";

/**
 * The compose card, and the parts the reply box in the thread is made of too.
 *
 * `c` opens a 600px card over the bottom right of the stage with the list still readable behind it,
 * and `Cmd+Shift+P` makes it the window. It is not a modal and it does not take the stage: writing
 * a message while looking something up in the list is the ordinary thing to want, and a composer
 * that blacked out the mailbox behind it would make you close it to check a name.
 *
 * The keyboard is taken only while the caret is somewhere inside a composer, which is what lets
 * `a` still mean reply all on a thread whose reply box is open but not being typed in. The frame
 * pushed is `editor`, which shadows the view wholesale and leaves `Cmd+K` to TipTap's link.
 */

const CARD_TITLE = "New message";

// -------------------------------------------------------------------------------------------
// The frame a composer holds while the caret is in it
// -------------------------------------------------------------------------------------------

interface Frame {
  /** Put this on the composer's outermost element. */
  ref: (el: HTMLElement | null) => void;
  /** Whether the caret is anywhere inside it. */
  inside: boolean;
}

/**
 * The keyboard, the commands and the paste and drop handlers a composer owns while it is being
 * written in.
 *
 * Registering the writing commands on focus rather than on open is what keeps two open composers
 * from fighting over `Cmd+Enter`: the registry runs the last handler registered, and "last" would
 * otherwise mean whichever box was opened most recently rather than the one being typed in.
 */
export function useComposerFrame(at: ComposerAt, onSend: (now: boolean) => void): Frame {
  const [root, setRoot] = useState<HTMLElement | null>(null);
  const [inside, setInside] = useState(false);

  useEffect(() => {
    if (!root) return;
    const enter = () => setInside(true);
    const leave = (e: FocusEvent) => {
      if (!root.contains(e.relatedTarget as Node | null)) setInside(false);
    };
    root.addEventListener("focusin", enter);
    root.addEventListener("focusout", leave);
    setInside(root.contains(document.activeElement));
    return () => {
      root.removeEventListener("focusin", enter);
      root.removeEventListener("focusout", leave);
    };
  }, [root]);

  useKeyContext("editor", inside);

  // A Tauri window answers a drop itself rather than letting the webview see it, which is the whole
  // reason drag and drop is the one route that carries a real path: a `File` from the webview's own
  // picker has a name and no path, and `DraftAttachment` wants a path. The size is not in the event,
  // so it goes in as zero and `DraftSaved.overLimit` is what actually decides whether it fits.
  useEffect(() => {
    if (!isTauri || !root) return;
    const stop = listen<{ paths: string[]; position: { x: number; y: number } }>(
      "tauri://drag-drop",
      ({ payload }) => {
        const rect = root.getBoundingClientRect();
        const x = payload.position.x / window.devicePixelRatio;
        const y = payload.position.y / window.devicePixelRatio;
        if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return;
        useCompose.getState().attach(
          at,
          payload.paths.map((path) => ({
            path,
            filename: path.split(/[\\/]/).pop() ?? path,
            mimeType: "application/octet-stream",
            size: 0,
          })),
        );
      },
    );
    return () => void stop.then((off) => off());
  }, [at, root]);

  useEffect(() => {
    if (!inside) return;
    const compose = useCompose.getState();
    return registerCommands({
      send: () => onSend(false),
      "send-now": () => onSend(true),
      attach: () => pickFiles(at),
      "instant-intro": () => compose.instantIntro(at),
      "remind-if-no-reply": () => {
        const composer = at === "card" ? useCompose.getState().card : useCompose.getState().reply;
        if (!composer) return;
        compose.remind(at, composer.draft.remindAtMs ? null : defaultReminder());
      },
      "discard-draft": () => void compose.discard(at),
      "compose-expand": () => compose.toggleExpanded(),
    });
  }, [at, inside, onSend]);

  return { ref: setRoot, inside };
}

/** Three days out at nine, which is when a message nobody answered stops being new. */
function defaultReminder(): number {
  const day = new Date();
  day.setDate(day.getDate() + 3);
  day.setHours(9, 0, 0, 0);
  return day.getTime();
}

// -------------------------------------------------------------------------------------------
// Files
// -------------------------------------------------------------------------------------------

export function pickFiles(at: ComposerAt): void {
  const input = document.createElement("input");
  input.type = "file";
  input.multiple = true;
  input.style.display = "none";
  input.addEventListener("change", () => {
    void attachFiles(at, [...(input.files ?? [])]);
    input.remove();
  });
  input.addEventListener("cancel", () => input.remove());
  document.body.append(input);
  input.click();
}

export const asAttachment = (file: File): DraftAttachment => ({
  filename: file.name,
  mimeType: file.type || "application/octet-stream",
  size: file.size,
});

function attachFiles(at: ComposerAt, files: File[]): Promise<void> {
  return storeAttachments(at, persistFiles(at, files)).catch((error) => {
    notify(`Could not attach files: ${error}`);
  });
}

async function persistFiles(at: ComposerAt, files: File[]): Promise<void> {
  const composer = at === "card" ? useCompose.getState().card : useCompose.getState().reply;
  if (!composer || files.length === 0) return;
  const existingSize = Math.max(composer.encodedSize,
    (composer.draft.attachments ?? []).reduce((size, file) => size + file.size * 4 / 3, 0));
  if (existingSize + files.reduce((size, file) => size + file.size * 4 / 3, 0) > 35 * 1024 * 1024) {
    throw new Error("These files would put this message over the 35 MB the provider takes");
  }
  const draftId = composer.draft.id || crypto.randomUUID();
  if (!composer.draft.id) useCompose.getState().edit(at, { id: draftId });
  const attachments: DraftAttachment[] = [];
  for (const file of files) {
    const descriptor = asAttachment(file);
    if (!isTauri) {
      attachments.push(descriptor);
      continue;
    }
    const data = await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(String(reader.result).split(",")[1]);
      reader.onerror = () => reject(reader.error ?? new Error(`${file.name} could not be read`));
      reader.readAsDataURL(file);
    });
    attachments.push(await draftAttachmentStore(composer.draft.accountId, descriptor, data));
  }
  const current = at === "card" ? useCompose.getState().card : useCompose.getState().reply;
  if (current?.draft.id === draftId) {
    useCompose.getState().attach(at, attachments);
  }
}

// -------------------------------------------------------------------------------------------
// The card
// -------------------------------------------------------------------------------------------

export function Compose() {
  const card = useCompose((s) => s.card);
  const expanded = useCompose((s) => s.expanded);
  const holding = useCompose((s) => s.holding);
  const compose = useCompose((s) => s.compose);
  const closeCard = useCompose((s) => s.closeCard);
  const toggleExpanded = useCompose((s) => s.toggleExpanded);
  const [closing, setClosing] = useState(false);
  const requestClose = () => card && !card.pristine ? setClosing(true) : closeCard();

  // `c` and the Write button, for as long as this is mounted, which is the whole life of the app:
  // this is mounted once at the top of the tree, so writing a message is something you can do from
  // the Feed and the Screener and not only from a list.
  useEffect(() => registerCommands({ compose }), [compose]);
  useEffect(() => { void useCompose.getState().loadDrafts(); }, []);

  // The signature, the undo delay, the reply-all default and the instant intro line are all
  // settings, and settings are only read when the settings screen asks for them. Writing is the
  // other thing that needs them, and it needs them before the first key rather than after it.
  useEffect(() => {
    if (!useSettings.getState().settings) void useSettings.getState().load();
  }, []);

  // `z` inside the delay means the send and not the last triage action, so it is taken over for
  // exactly as long as there is a send to take back.
  useEffect(() => {
    if (!holding) return;
    return registerCommands({ undo: () => void useCompose.getState().undoSend() });
  }, [holding]);

  useEscapeLayer(card !== null && !closing, requestClose);

  if (!card) return null;

  return (
    <>
    <CloseDraft at="card" open={closing} onClose={() => setClosing(false)} />
    <ComposeCard
      composer={card}
      expanded={expanded}
      onClose={requestClose}
      onExpand={toggleExpanded}
    />
    </>
  );
}

interface ComposeCardProps {
  composer: Composer;
  expanded: boolean;
  onClose: () => void;
  onExpand: () => void;
}

function ComposeCard({ composer, expanded, onClose, onExpand }: ComposeCardProps) {
  const send = useCallback((now: boolean) => void useCompose.getState().post("card", now), []);
  const frame = useComposerFrame("card", send);
  const draft = composer.draft;
  const edit = useCompose((s) => s.edit);
  const setShowCc = useCompose((s) => s.setShowCc);
  const signature = useSettings((s) => s.settings?.accounts.find((account) => account.accountId === draft.accountId)?.signature);

  return (
    <div
      className="compose"
      data-expanded={expanded ? "" : undefined}
      role="dialog"
      aria-label={draft.id ? "Draft" : CARD_TITLE}
      ref={frame.ref}
      onDragOver={allowDrop}
      onDrop={(e) => dropped(e, "card")}
      onPaste={(e) => pasted(e, "card")}
    >
      <div className="compose-head">
        <h2>{draft.id ? "Draft" : CARD_TITLE}</h2>
        <Button
          variant="ghost"
          iconOnly
          icon={expanded ? icons.CHEVRON_DOWN : icons.CHEVRON_UP}
          title={`${expanded ? "Back to the corner" : "Expand to the window"} (${cap("compose-expand")})`}
          label={expanded ? "Back to the corner" : "Expand to the window"}
          onClick={onExpand}
        />
        <Button
          variant="ghost"
          iconOnly
          icon={icons.CLOSE}
          title="Close, keeping the draft (⎋)"
          label="Close, keeping the draft"
          onClick={onClose}
        />
      </div>

      <FromField at="card" draft={draft} />

      <ChipField
        label="To"
        people={draft.to}
        accountId={draft.accountId}
        autoFocus
        onChange={(to) => edit("card", { to })}
        trailing={
          composer.showCc ? null : (
            <button type="button" className="more" onClick={() => setShowCc("card", true)}>
              Cc · Bcc
            </button>
          )
        }
      />

      {composer.showCc ? (
        <>
          <ChipField
            label="Cc"
            people={draft.cc ?? []}
            accountId={draft.accountId}
            onChange={(cc) => edit("card", { cc })}
          />
          <ChipField
            label="Bcc"
            people={draft.bcc ?? []}
            accountId={draft.accountId}
            onChange={(bcc) => edit("card", { bcc })}
          />
        </>
      ) : null}

      <div className="compose-field">
        <span className="lab">Subject</span>
        <input
          className="compose-subject"
          value={draft.subject}
          aria-label="Subject"
          {...NO_AUTOFILL}
          onChange={(e) => edit("card", { subject: e.target.value })}
        />
      </div>

      <div className="compose-body">
        <Editor
          html={draft.bodyHtml}
          label="Message"
          placeholder="Write your message"
          signature={signature}
          onChange={(bodyHtml) => edit("card", { bodyHtml })}
        />
        <Attachments at="card" files={draft.attachments ?? []} />
      </div>

      <ComposeFoot at="card" composer={composer} onSend={send} />
    </div>
  );
}

// -------------------------------------------------------------------------------------------
// The pieces the reply box in the thread shares
// -------------------------------------------------------------------------------------------

export const allowDrop = (e: DragEvent<HTMLElement>): void => {
  if (e.dataTransfer.types.includes("Files")) e.preventDefault();
};

export function dropped(e: DragEvent<HTMLElement>, at: ComposerAt): void {
  const files = [...(e.dataTransfer.files ?? [])];
  if (files.length === 0) return;
  e.preventDefault();
  void attachFiles(at, files);
}

export function pasted(e: ClipboardEvent<HTMLElement>, at: ComposerAt): void {
  const files = [...(e.clipboardData?.files ?? [])];
  if (files.length === 0) return;
  e.preventDefault();
  void attachFiles(at, files);
}

interface FromFieldProps {
  at: ComposerAt;
  draft: Composer["draft"];
}

/** The address this goes out from: the account, or one of the aliases its settings carry. */
function FromField({ at, draft }: FromFieldProps) {
  const accounts = useAccounts((s) => s.accounts);
  const settings = useSettings((s) => s.settings);
  const edit = useCompose((s) => s.edit);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [open, setOpen] = useState(false);

  const account = accounts.find((a) => a.id === draft.accountId) ?? accounts[0];
  const aliases = settings?.accounts.find((a) => a.accountId === draft.accountId)?.aliases ?? [];
  const address = draft.fromAlias ?? account?.email ?? "";
  const choices = account ? [account.email, ...aliases] : [];

  return (
    <div className="compose-field">
      <span className="lab">From</span>
      <div className="chips">
        <button
          type="button"
          className="chip"
          ref={setAnchor}
          aria-label={`Sending from ${address}`}
          onClick={() => setOpen((was) => !was)}
        >
          <Avatar
            name={account?.name ?? address}
            address={address}
            hue={accountHue(account?.color ?? "")}
            size="xs"
          />
          {address}
          <Icon d={icons.CHEVRON_DOWN} size={10} />
        </button>
      </div>
      <Popover
        open={open && choices.length > 1}
        anchor={anchor}
        width={280}
        label="Send from"
        onClose={() => setOpen(false)}
      >
        <ul className="from-list">
          {choices.map((choice) => (
            <li key={choice}>
              <button
                type="button"
                className="from-option"
                data-on={choice === address ? "" : undefined}
                onClick={() => {
                  edit(at, { fromAlias: choice === account?.email ? null : choice });
                  setOpen(false);
                }}
              >
                {choice}
              </button>
            </li>
          ))}
        </ul>
      </Popover>
    </div>
  );
}

interface ChipFieldProps {
  label: string;
  people: Person[];
  accountId: string;
  onChange: (people: Person[]) => void;
  autoFocus?: boolean;
  trailing?: ReactNode;
}

const ADDRESS = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** `Maya Raghunathan <maya@…>` and a bare address are both things people paste into a To field. */
function parsePerson(text: string): Person | null {
  const trimmed = text.trim().replace(/[,;]+$/, "").trim();
  const angled = /^(.*)<([^>]+)>$/.exec(trimmed);
  if (angled) {
    const address = angled[2].trim();
    if (!ADDRESS.test(address)) return null;
    const name = angled[1].trim().replace(/^["']|["']$/g, "");
    return { name: name || null, address };
  }
  return ADDRESS.test(trimmed) ? { name: null, address: trimmed } : null;
}

/**
 * Recipients as chips, with autocomplete over the mirror.
 *
 * `contactsSuggest` reads what is already on this device first and never sends an address anywhere
 * to be looked up, which is the whole reason a mail client is allowed to have an address book.
 */
export function ChipField({
  label,
  people,
  accountId,
  onChange,
  autoFocus,
  trailing,
}: ChipFieldProps) {
  const [typed, setTyped] = useState("");
  const [suggestions, setSuggestions] = useState<Person[]>([]);
  const [highlight, setHighlight] = useState(0);
  const input = useRef<HTMLInputElement | null>(null);

  const taken = useMemo(
    () => new Set(people.map((p) => p.address.toLowerCase())),
    [people],
  );

  useEffect(() => {
    const prefix = typed.trim();
    if (prefix.length < 1) {
      setSuggestions([]);
      return;
    }
    let live = true;
    const timer = window.setTimeout(() => {
      void contactsSuggest(accountId, prefix)
        .then((found) => {
          if (!live) return;
          setSuggestions(found.filter((p) => !taken.has(p.address.toLowerCase())).slice(0, 6));
          setHighlight(0);
        })
        .catch(() => setSuggestions([]));
    }, 90);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [typed, accountId, taken]);

  const add = (person: Person) => {
    if (taken.has(person.address.toLowerCase())) return;
    onChange([...people, person]);
    setTyped("");
    setSuggestions([]);
  };

  const commit = (): boolean => {
    const chosen = suggestions[highlight];
    if (chosen) {
      add(chosen);
      return true;
    }
    const typedPerson = parsePerson(typed);
    if (typedPerson) {
      add(typedPerson);
      return true;
    }
    return false;
  };

  return (
    <div className="compose-field">
      <span className="lab">{label}</span>
      <div className="chips" onClick={() => input.current?.focus()}>
        {people.map((person, index) => (
          <span className="chip" key={person.address}>
            <Avatar
              name={displayName(person)}
              address={person.address}
              brand={isBrand(person)}
              size="xs"
            />
            {displayName(person)}
            <button
              type="button"
              className="chip-off"
              title={`Take ${displayName(person)} off`}
              aria-label={`Take ${displayName(person)} off`}
              onClick={() => onChange(people.filter((_, i) => i !== index))}
            >
              <Icon d={icons.CLOSE} size={10} />
            </button>
          </span>
        ))}
        <input
          ref={input}
          className="chip-input"
          value={typed}
          aria-label={label}
          autoFocus={autoFocus}
          autoComplete="off"
          onChange={(e) => setTyped(e.target.value)}
          onBlur={() => {
            // What was typed and not what was highlighted: leaving the field is not choosing from
            // a list, it is finishing an address.
            const typedPerson = parsePerson(typed);
            if (typedPerson) add(typedPerson);
            setSuggestions([]);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" && suggestions.length > 0) {
              e.preventDefault();
              setHighlight((was) => Math.min(was + 1, suggestions.length - 1));
              return;
            }
            if (e.key === "ArrowUp" && suggestions.length > 0) {
              e.preventDefault();
              setHighlight((was) => Math.max(was - 1, 0));
              return;
            }
            if (e.key === "Enter" || e.key === "Tab" || e.key === ",") {
              // Tab with nothing typed is still Tab: moving on is what it means everywhere else.
              if (e.key === "Tab" && typed.trim() === "") return;
              if (commit()) e.preventDefault();
              return;
            }
            if (e.key === "Backspace" && typed === "" && people.length > 0) {
              e.preventDefault();
              onChange(people.slice(0, -1));
            }
          }}
        />
      </div>
      {trailing}
      {suggestions.length > 0 ? (
        <ul className="suggest" role="listbox" aria-label={`${label} suggestions`}>
          {suggestions.map((person, index) => (
            <li key={person.address}>
              <button
                type="button"
                className="suggest-row"
                role="option"
                aria-selected={index === highlight}
                data-on={index === highlight ? "" : undefined}
                // The blur that a click causes would clear the list before the click landed.
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => add(person)}
              >
                <Avatar
                  name={displayName(person)}
                  address={person.address}
                  brand={isBrand(person)}
                  size="xs"
                />
                <span className="suggest-name">{displayName(person)}</span>
                <span className="suggest-addr">{person.address}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

interface AttachmentsProps {
  at: ComposerAt;
  files: DraftAttachment[];
}

export function Attachments({ at, files }: AttachmentsProps) {
  const detach = useCompose((s) => s.detach);
  if (files.length === 0) return null;
  return (
    <div className="attachments">
      {files.map((file, index) => (
        <span className="attachment" key={`${file.filename}-${index}`}>
          <span className="ext">{fileKind(file.filename, file.mimeType)}</span>
          {file.filename}
          <span className="size">{fileSize(file.size)}</span>
          <button
            type="button"
            className="chip-off"
            title={`Take ${file.filename} off`}
            aria-label={`Take ${file.filename} off`}
            onClick={() => detach(at, index)}
          >
            <Icon d={icons.CLOSE} size={10} />
          </button>
        </span>
      ))}
    </div>
  );
}

interface ComposeFootProps {
  at: ComposerAt;
  composer: Composer;
  onSend: (now: boolean) => void;
  /** The one control the thread's reply box has that the card does not. */
  extra?: ReactNode;
}

const asDateInput = (ms: number): string => {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

/**
 * Send with its key, the reminder, the paperclip, and the delay said in words.
 *
 * The delay is printed rather than assumed because it is a setting: somebody who moved it to thirty
 * seconds has to be able to see that they did, and somebody who moved it to five has to know that
 * pressing Send means it.
 */
export function ComposeFoot({ at, composer, onSend, extra }: ComposeFootProps) {
  const settings = useSettings((s) => s.settings);
  const remind = useCompose((s) => s.remind);
  const delay = settings?.undoDelaySecs ?? 10;
  const remindAt = composer.draft.remindAtMs ?? null;

  return (
    <div className="compose-foot">
      <Button variant="primary" keycap={cap("send")} onClick={() => onSend(false)}>
        Send
      </Button>
      <Button
        variant="ghost"
        active={remindAt !== null}
        title={`Remind me if no reply (${cap("remind-if-no-reply")})`}
        onClick={() => remind(at, remindAt === null ? defaultReminder() : null)}
      >
        Remind me if no reply
      </Button>
      {remindAt !== null ? (
        <input
          className="compose-date"
          type="date"
          aria-label="Remind me on"
          {...NO_AUTOFILL}
          value={asDateInput(remindAt)}
          onChange={(e) => {
            const chosen = new Date(`${e.target.value}T09:00`);
            if (!Number.isNaN(chosen.getTime())) remind(at, chosen.getTime());
          }}
        />
      ) : null}
      <span className="spacer" />
      {composer.overLimit ? (
        <span className="compose-warn">Over the 35 MB the provider takes</span>
      ) : null}
      {extra}
      <Button
        variant="ghost"
        iconOnly
        icon={icons.PAPERCLIP}
        title={`Attach a file (${cap("attach")})`}
        label="Attach a file"
        onClick={() => pickFiles(at)}
      />
      <Button variant="ghost" icon={icons.TRASH} onClick={() => void useCompose.getState().discard(at)}>Discard draft</Button>
      <span className="compose-delay">{`Undo send: ${delay} s`}</span>
    </div>
  );
}

interface ReplyBoxProps {
  /** The person a plain reply goes to, which is what the head names. */
  to: Person;
  composer: Composer;
}

/**
 * The box under the last message of a thread. Not the compose card: the mockups are clear that a
 * reply is written in the thread, under what it answers, at the thread's own measure.
 */
export function ReplyBox({ to, composer }: ReplyBoxProps) {
  const send = useCallback((now: boolean) => void useCompose.getState().post("reply", now), []);
  const frame = useComposerFrame("reply", send);
  const edit = useCompose((s) => s.edit);
  const setAll = useCompose((s) => s.setAll);
  const setShowCc = useCompose((s) => s.setShowCc);
  const closeReply = useCompose((s) => s.closeReply);
  const [closing, setClosing] = useState(false);
  const editor = useRef<TiptapEditor | null>(null);
  const draft = composer.draft;
  const signature = useSettings((s) => s.settings?.accounts.find((account) => account.accountId === draft.accountId)?.signature);

  // Escape leaves the editor and keeps the draft, which is docs/keyboard.md's own wording. The
  // second Escape is not this box's: it belongs to whatever is under it.
  useEscapeLayer(frame.inside, () => {
    editor.current?.commands.blur();
    (document.activeElement as HTMLElement | null)?.blur();
  });

  const forwarding = composer.kind === "forward";

  return (
    <div
      className="reply"
      ref={frame.ref}
      onDragOver={allowDrop}
      onDrop={(e) => dropped(e, "reply")}
      onPaste={(e) => pasted(e, "reply")}
    >
      <CloseDraft at="reply" open={closing} onClose={() => setClosing(false)} />
      <div className="reply-head">
        <span className="reply-who">
          {forwarding ? "Forward" : "Reply to "}
          {forwarding ? null : <b>{displayName(to)}</b>}
        </span>
        {forwarding ? null : (
          <button
            type="button"
            className="reply-all"
            data-on={composer.all ? "" : undefined}
            onClick={() => setAll(!composer.all)}
          >
            <Key size="sm">{cap("reply-all") ?? "a"}</Key>
            reply all
          </button>
        )}
        <button
          type="button"
          className="reply-close"
          title="Close, keeping the draft"
          aria-label="Close, keeping the draft"
          onClick={() => composer.pristine ? closeReply() : setClosing(true)}
        >
          <Icon d={icons.CLOSE} size={12} />
        </button>
      </div>

      <ChipField
        label="To"
        people={draft.to}
        accountId={draft.accountId}
        autoFocus={forwarding}
        onChange={(to) => edit("reply", { to })}
        trailing={
          composer.showCc ? null : (
            <button type="button" className="more" onClick={() => setShowCc("reply", true)}>
              Cc · Bcc
            </button>
          )
        }
      />

      {composer.showCc ? (
        <>
          <ChipField
            label="Cc"
            people={draft.cc ?? []}
            accountId={draft.accountId}
            onChange={(cc) => edit("reply", { cc })}
          />
          <ChipField
            label="Bcc"
            people={draft.bcc ?? []}
            accountId={draft.accountId}
            onChange={(bcc) => edit("reply", { bcc })}
          />
        </>
      ) : null}

      <div className="reply-body">
        <Editor
          html={draft.bodyHtml}
          label="Reply"
          placeholder="Write a reply"
          signature={signature}
          autoFocus={!forwarding}
          onChange={(bodyHtml) => edit("reply", { bodyHtml })}
          onReady={(instance) => {
            editor.current = instance;
          }}
        />
        <Attachments at="reply" files={draft.attachments ?? []} />
      </div>

      <ComposeFoot at="reply" composer={composer} onSend={send} />
    </div>
  );
}

/**
 * The line a thread carries while something of its own is still in the outbox.
 *
 * `ThreadSummary.sending` is the fact and the summary is the list's, so the pane reads the row it
 * came from rather than being told twice.
 */
export function SendingLine({ threadKey }: { threadKey: string }) {
  const sending = useMail((s) => s.threads.find((t) => t.key === threadKey)?.sending ?? false);
  const flush = useCompose((s) => s.flush);
  const phase = useCompose((s) => s.flushPhase);
  if (!sending) return null;
  const busy = phase === "sending";
  return (
    <div className="sending" data-phase={phase}>
      <Icon d={icons.CLOCK} size={13} />
      <span>Waiting to send</span>
      <button type="button" className="sending-now" disabled={busy} onClick={() => void flush()}>
        {busy ? "Sending" : "Send now"}
      </button>
    </div>
  );
}

export default Compose;

function CloseDraft({ at, open, onClose }: { at: ComposerAt; open: boolean; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    const compose = useCompose.getState();
    if (at === "card") await compose.closeCard(); else await compose.closeReply();
    const current = at === "card" ? useCompose.getState().card : useCompose.getState().reply;
    setBusy(false);
    if (current?.phase === "error") return;
    onClose();
  };
  return <Sheet open={open} size="mini" title="Keep your draft?" busy={busy} onClose={onClose} foot={<>
    <Button disabled={busy} onClick={onClose}>Keep editing</Button>
    <Button variant="danger" disabled={busy} onClick={() => { onClose(); void useCompose.getState().discard(at); }}>Discard</Button>
    <Button variant="primary" disabled={busy} onClick={() => void save()}>{busy ? "Saving…" : "Save draft and close"}</Button>
  </>}><p>Save what you wrote before closing, or discard this draft.</p></Sheet>;
}

export function DraftCard({ draft }: { draft: Draft }) {
  return <div className="draft-card">
    <span><b>Draft</b> · {draft.subject || "(no subject)"} · {draft.to.map(displayName).join(", ")}</span>
    <Button size="sm" onClick={() => useCompose.getState().resumeDraft(draft)}>Resume draft</Button>
    <Button size="sm" variant="ghost" onClick={() => { if (draft.id) void useCompose.getState().deleteDraft(draft.id); }}>Discard draft</Button>
  </div>;
}
