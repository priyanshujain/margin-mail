import { useEffect, useRef, useState, type ReactNode } from "react";
import { EditorContent, useEditor, useEditorState, type Editor as TiptapEditor } from "@tiptap/react";
import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import StarterKit from "@tiptap/starter-kit";
import { hasText } from "../store/useCompose";
import { useSettings } from "../store/useSettings";
import { isMacDesktop } from "../ipc";
import { proofText, type ProofIssue as NativeProofIssue } from "../api/proofing";
import { Icon, icons } from "../ui";
import { useEscapeLayer } from "../escape";
import { signatureHtml } from "../signature";
import { writingToolsAvailable, runWritingTool } from "../api/writingtools";
import "./editor.css";

interface EditorProps {
  html: string;
  onChange: (html: string) => void;
  placeholder: string;
  label: string;
  autoFocus?: boolean;
  onReady?: (editor: TiptapEditor | null) => void;
  signature?: string;
}

interface ProofIssue extends NativeProofIssue {
  from: number;
  to: number;
  word: string;
}

const proofKey = new PluginKey<DecorationSet>("mail-proofing");
const Proofing = Extension.create({
  name: "mailProofing",
  addProseMirrorPlugins() {
    return [new Plugin({
      key: proofKey,
      state: {
        init: () => DecorationSet.empty,
        apply: (transaction, decorations) => {
          const issues = transaction.getMeta(proofKey) as ProofIssue[] | undefined;
          if (issues) return DecorationSet.create(transaction.doc, issues.map((issue) =>
            Decoration.inline(issue.from, issue.to, { class: `editor-proof-${issue.kind}` })));
          return transaction.docChanged ? DecorationSet.empty : decorations;
        },
      },
      props: { decorations: (state) => proofKey.getState(state) },
    })];
  },
});

const EXTENSIONS = [StarterKit.configure({
  heading: false,
  horizontalRule: false,
  link: { openOnClick: false, autolink: true, defaultProtocol: "https" },
}), Proofing];

export function Editor({ html, onChange, placeholder, label, autoFocus, onReady, signature }: EditorProps) {
  const emitted = useRef(html);
  const ready = useRef(onReady);
  ready.current = onReady;
  const spelling = useSettings((state) => state.settings?.spellingEnabled ?? true);
  const grammar = useSettings((state) => state.settings?.grammarEnabled ?? false);
  const writingTools = useSettings((state) => state.settings?.writingToolsEnabled ?? false);
  const [appleToolsAvailable, setAppleToolsAvailable] = useState(false);
  const [writingToolsError, setWritingToolsError] = useState("");
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkValue, setLinkValue] = useState("");
  const [linkError, setLinkError] = useState("");
  const [issues, setIssues] = useState<ProofIssue[]>([]);
  const [proofOpen, setProofOpen] = useState(false);
  const [proofError, setProofError] = useState("");

  const editor = useEditor({
    extensions: EXTENSIONS,
    content: html,
    autofocus: autoFocus ? "start" : false,
    editorProps: { attributes: { class: "editor-body", "aria-label": label, spellcheck: String(!isMacDesktop && spelling) } },
    onUpdate: ({ editor }) => {
      emitted.current = editor.getHTML();
      onChange(emitted.current);
    },
  });

  useEditorState({ editor, selector: ({ editor }) => editor?.state });

  useEffect(() => {
    let live = true;
    setAppleToolsAvailable(false);
    if (writingTools && isMacDesktop) {
      void writingToolsAvailable().then((available) => { if (live) setAppleToolsAvailable(available); })
        .catch((error) => { if (live) setWritingToolsError(String(error)); });
    }
    return () => { live = false; };
  }, [writingTools]);

  useEffect(() => {
    editor?.view.dom.setAttribute("spellcheck", String(!isMacDesktop && spelling));
  }, [editor, spelling]);

  useEffect(() => {
    ready.current?.(editor ?? null);
    return () => ready.current?.(null);
  }, [editor]);

  useEffect(() => {
    if (!editor || html === emitted.current) return;
    emitted.current = html;
    editor.commands.setContent(html, { emitUpdate: false });
  }, [editor, html]);

  useEffect(() => {
    if (!editor) return;
    let version = 0;
    let timer: ReturnType<typeof setTimeout>;
    const proof = () => {
      const current = ++version;
      clearTimeout(timer);
      setIssues([]);
      editor.view.dispatch(editor.state.tr.setMeta(proofKey, []));
      if (!isMacDesktop || (!spelling && !grammar)) return;
      timer = setTimeout(() => {
        const doc = editor.state.doc;
        const segments: { offset: number; position: number; text: string }[] = [];
        let text = "";
        doc.descendants((node, position) => {
          if (!node.isTextblock) return true;
          if (text) text += "\n\n";
          node.forEach((child, offset) => {
            if (child.type.name === "hardBreak") { text += "\n"; return; }
            if (!child.isText) return;
            segments.push({ offset: text.length, position: position + 1 + offset, text: child.text ?? "" });
            text += child.text ?? "";
          });
          return false;
        });
        if (!text.trim()) return;
        const positionAt = (offset: number) => {
          const segment = segments.find((segment) => offset >= segment.offset && offset <= segment.offset + segment.text.length);
          return segment ? segment.position + offset - segment.offset : null;
        };
        void proofText(text, spelling, grammar).then((found) => {
          if (current !== version || editor.isDestroyed || !editor.state.doc.eq(doc)) return;
          const mapped = found.flatMap((issue) => {
            const from = positionAt(issue.start);
            const to = positionAt(issue.end);
            return from !== null && to !== null && to > from ? [{ ...issue, from, to, word: text.slice(issue.start, issue.end) }] : [];
          });
          setIssues(mapped);
          setProofError("");
          editor.view.dispatch(editor.state.tr.setMeta(proofKey, mapped));
        }).catch((error) => {
          if (current === version) setProofError(String(error));
        });
      }, 650);
    };
    proof();
    editor.on("update", proof);
    return () => { version++; clearTimeout(timer); editor.off("update", proof); };
  }, [editor, spelling, grammar]);

  useEscapeLayer(linkOpen || proofOpen, () => { setLinkOpen(false); setProofOpen(false); editor?.commands.focus(); });

  const openLink = () => {
    setLinkValue((editor?.getAttributes("link").href as string) ?? "");
    setLinkError("");
    setLinkOpen(true);
    setProofOpen(false);
  };

  useEffect(() => {
    if (!editor) return;
    const key = (event: KeyboardEvent) => {
      if (!editor.isFocused || event.key.toLowerCase() !== "k" || !(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      event.preventDefault();
      event.stopPropagation();
      openLink();
    };
    editor.view.dom.addEventListener("keydown", key);
    return () => editor.view.dom.removeEventListener("keydown", key);
  }, [editor]);

  const applyLink = () => {
    if (!editor) return;
    const value = linkValue.trim();
    if (!value) editor.chain().focus().extendMarkRange("link").unsetLink().run();
    else {
      const href = /^[a-z][a-z\d+.-]*:/i.test(value) ? value : `https://${value}`;
      try {
        const url = new URL(href);
        if (!["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) throw new Error();
      } catch { setLinkError("Use a web, email, or telephone link."); return; }
      if (editor.state.selection.empty && !editor.isActive("link")) {
        editor.chain().focus().insertContent({ type: "text", text: value, marks: [{ type: "link", attrs: { href } }] }).run();
      } else editor.chain().focus().extendMarkRange("link").setLink({ href }).run();
    }
    setLinkOpen(false);
  };

  const tool = (title: string, content: ReactNode, action: () => void, active = false, disabled = false) => (
    <button type="button" className="editor-tool" title={title} aria-label={title} aria-pressed={active}
      disabled={disabled} onMouseDown={(event) => event.preventDefault()} onClick={action}>{content}</button>
  );

  const insertSignature = () => {
    if (!editor || !signature) return;
    editor.chain().focus().insertContent(signatureHtml(signature)).run();
  };

  const runAppleTool = (tool: "Proofread" | "Rewrite") => {
    if (!editor) return;
    setWritingToolsError("");
    if (editor.state.selection.empty) editor.chain().focus().selectAll().run();
    else editor.commands.focus();
    requestAnimationFrame(() => {
      void runWritingTool(tool).catch((error) => setWritingToolsError(String(error)));
    });
  };

  return (
    <div className="editor">
      {editor ? <>
        <div className="editor-toolbar" role="toolbar" aria-label="Writing tools">
          {tool("Bold (⌘B)", <b>B</b>, () => editor.chain().focus().toggleBold().run(), editor.isActive("bold"))}
          {tool("Italic (⌘I)", <i>I</i>, () => editor.chain().focus().toggleItalic().run(), editor.isActive("italic"))}
          {tool("Underline (⌘U)", <u>U</u>, () => editor.chain().focus().toggleUnderline().run(), editor.isActive("underline"))}
          {tool("Strikethrough", <s>S</s>, () => editor.chain().focus().toggleStrike().run(), editor.isActive("strike"))}
          <span className="editor-tool-separator" />
          {tool("Bulleted list", <Icon d="M8 6h12M8 12h12M8 18h12M3 6h.01M3 12h.01M3 18h.01" />, () => editor.chain().focus().toggleBulletList().run(), editor.isActive("bulletList"))}
          {tool("Numbered list", <Icon d="M9 6h12M9 12h12M9 18h12M3 4h1v4M3 11c3-2 3 1 0 3h3M3 17h3l-2 2 2 1H3" />, () => editor.chain().focus().toggleOrderedList().run(), editor.isActive("orderedList"))}
          {tool("Quote", <Icon d="M7 8h4v4a4 4 0 0 1-4 4M14 8h4v4a4 4 0 0 1-4 4" />, () => editor.chain().focus().toggleBlockquote().run(), editor.isActive("blockquote"))}
          {tool("Link (⌘K)", <Icon d="M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1" />, openLink, editor.isActive("link") || linkOpen)}
          {tool("Clear formatting", <Icon d="M4 4h14M11 4l-4 15M14 14l6 6M20 14l-6 6" />, () => editor.chain().focus().unsetAllMarks().clearNodes().run())}
          <span className="editor-tool-separator" />
          {tool("Undo (⌘Z)", <Icon d={icons.REPLY} />, () => editor.chain().focus().undo().run(), false, !editor.can().undo())}
          {tool("Redo (⌘⇧Z)", <Icon d={icons.FORWARD} />, () => editor.chain().focus().redo().run(), false, !editor.can().redo())}
          {signature?.trim() ? tool("Insert signature", <Icon d={icons.PEN} />, insertSignature) : null}
          {writingTools && appleToolsAvailable ? <>
            {tool("Proofread with Apple Writing Tools", "Proofread", () => runAppleTool("Proofread"), false, !hasText(html))}
            {tool("Rewrite with Apple Writing Tools", "Rewrite", () => runAppleTool("Rewrite"), false, !hasText(html))}
          </> : null}
          {isMacDesktop && (spelling || grammar) ? tool(`Spelling and grammar${issues.length ? ` (${issues.length})` : ""}`, <><Icon d="M3 17 8 5l5 12M5 12h6M14 14l3 3 5-6" />{issues.length ? <span>{issues.length}</span> : null}</>, () => { setProofOpen(!proofOpen); setLinkOpen(false); }, proofOpen) : null}
        </div>
        {writingTools && writingToolsError ? <p className="editor-writing-error" role="alert">{writingToolsError}</p> : null}
        {linkOpen ? <form className="editor-link" onSubmit={(event) => { event.preventDefault(); applyLink(); }}>
          <input aria-label="Link URL" value={linkValue} placeholder="https://" autoFocus spellCheck={false} onChange={(event) => setLinkValue(event.target.value)} />
          <button type="submit">Apply</button>
          {editor.isActive("link") ? <button type="button" onClick={() => { editor.chain().focus().extendMarkRange("link").unsetLink().run(); setLinkOpen(false); }}>Remove</button> : null}
          <button type="button" aria-label="Close link editor" onClick={() => { setLinkOpen(false); editor.commands.focus(); }}><Icon d={icons.CLOSE} /></button>
          {linkError ? <span role="alert">{linkError}</span> : null}
        </form> : null}
        {proofOpen ? <div className="editor-proof-panel" aria-label="Spelling and grammar suggestions">
          {proofError ? <p role="alert">Writing check unavailable: {proofError}</p> : issues.length ? issues.map((issue, index) => <div className="editor-proof-issue" key={`${issue.from}:${issue.to}:${index}`}>
            <button type="button" className="editor-proof-word" onClick={() => editor.chain().focus().setTextSelection({ from: issue.from, to: issue.to }).run()}>{issue.word}</button>
            <span>{issue.message}</span>
            {issue.suggestions.map((suggestion, suggestionIndex) => <button type="button" key={suggestionIndex} onClick={() => {
              const range = { from: issue.from, to: issue.to };
              if (suggestion) editor.chain().focus().insertContentAt(range, { type: "text", text: suggestion }).run();
              else editor.chain().focus().deleteRange(range).run();
            }}>{suggestion || "Delete"}</button>)}
          </div>) : <p>No suggestions. Checks use your Mac’s available languages.</p>}
        </div> : null}
      </> : null}
      <div className="editor-content" data-empty={hasText(html) ? undefined : ""}>
        <span className="editor-placeholder" aria-hidden="true">{placeholder}</span>
        <EditorContent editor={editor} />
      </div>
    </div>
  );
}

export default Editor;
