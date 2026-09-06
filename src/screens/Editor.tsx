import { useEffect, useRef } from "react";
import { EditorContent, useEditor, type Editor as TiptapEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { hasText } from "../store/useCompose";
import "./editor.css";

/**
 * The body of a message, which is TipTap and StarterKit and nothing else.
 *
 * Paragraphs, bold, italic, links, lists, quotes and code, built the way margin builds its editor
 * in `src/editor/extensions.ts`: one configured StarterKit rather than a list of extensions
 * assembled by hand. What is deliberately absent is colour and type. A mail client that lets you
 * choose a typeface is a mail client that sends mail nobody can read, and the faces in settings are
 * the reader's choice rather than the writer's.
 *
 * There is no toolbar either. Every mark this editor can make has a key in docs/keyboard.md, the
 * keys are TipTap's own, and `src/keys/bindings.ts` declares them with a null command so the
 * dispatcher sees the frame and stands out of the way. `Cmd+K` is the one real collision in the
 * app, and inside here the link wins because the palette is one Escape away and a link is not.
 *
 * It writes HTML. Rust inlines the stylesheet and builds the plain text alternative on the way out,
 * which is why nothing here has an opinion about what the recipient's client can render.
 */

interface EditorProps {
  html: string;
  onChange: (html: string) => void;
  placeholder: string;
  label: string;
  /**
   * Takes the caret when it appears, which is what `r` is for.
   *
   * At the start of the document rather than the end, because a draft opens with a signature under
   * it and nobody writes underneath their own name.
   */
  autoFocus?: boolean;
  /** Handed the instance so the box around it can put the caret back. */
  onReady?: (editor: TiptapEditor | null) => void;
}

const EXTENSIONS = [
  StarterKit.configure({
    // Mail has no headings and no rules. The subject is the heading and the hairline between
    // messages is the pane's, so both would be a mark the reader meets somewhere it means nothing.
    heading: false,
    horizontalRule: false,
    link: { openOnClick: false, autolink: true, defaultProtocol: "https" },
  }),
];

export function Editor({ html, onChange, placeholder, label, autoFocus, onReady }: EditorProps) {
  const emitted = useRef(html);
  const ready = useRef(onReady);
  ready.current = onReady;

  const editor = useEditor({
    extensions: EXTENSIONS,
    content: html,
    autofocus: autoFocus ? "start" : false,
    editorProps: {
      attributes: { class: "editor-body", "aria-label": label },
    },
    onUpdate: ({ editor }) => {
      emitted.current = editor.getHTML();
      onChange(emitted.current);
    },
  });

  useEffect(() => {
    ready.current?.(editor ?? null);
    return () => ready.current?.(null);
  }, [editor]);

  // Written from outside: Instant intro puts a line at the top and pressing it again takes the line
  // away. Comparing against what this editor last emitted rather than against its own document is
  // what keeps that from fighting the caret on every keystroke.
  useEffect(() => {
    if (!editor || html === emitted.current) return;
    emitted.current = html;
    editor.commands.setContent(html, { emitUpdate: false });
  }, [editor, html]);

  return (
    <div className="editor" data-empty={hasText(html) ? undefined : ""}>
      {/* The placeholder is a sibling rather than the Placeholder extension, which is a dependency
          this app does not have and would be carrying for one line of grey text. */}
      <span className="editor-placeholder" aria-hidden="true">
        {placeholder}
      </span>
      <EditorContent editor={editor} />
    </div>
  );
}

export default Editor;
