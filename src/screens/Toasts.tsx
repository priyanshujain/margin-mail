import { useEffect } from "react";
import { Toast } from "../ui";
import { useToast } from "../store/useToast";

/** Long enough to read it and reach for Undo, short enough not to sit over the list. */
const DISMISS_MS = 6000;

/**
 * The acknowledgement at the foot of the window, and the only feedback a triage key gives: rows do
 * not animate when they leave for a pile or an archive, because a list that rearranges itself under
 * the hand is a list you stop trusting.
 */
export function Toasts() {
  const message = useToast((s) => s.message);
  const action = useToast((s) => s.action);
  const seq = useToast((s) => s.seq);
  const dismiss = useToast((s) => s.dismiss);

  useEffect(() => {
    if (!message) return;
    const timer = window.setTimeout(dismiss, DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [message, seq, dismiss]);

  if (!message) return null;

  return (
    <Toast
      action={
        action
          ? {
              label: action.label,
              keycap: action.keycap,
              onClick: () => {
                action.run();
                dismiss();
              },
            }
          : undefined
      }
    >
      {message}
    </Toast>
  );
}

export default Toasts;
