import type { ThreadSummary } from "../ipc";

/**
 * The rows after one of them was opened: its weight goes and nothing else moves.
 *
 * `group` is left alone on purpose. Rows arrive already grouped and ordered, and the group is the
 * view's word; this only clears `unseen`, so the row loses its weight where it stands and stays
 * there until the next page arrives. The same array comes back when there was nothing to do, so
 * opening a seen thread is not a render of every row.
 *
 * Out here rather than in `useMail` so it can be asserted in node: the store reads the document
 * the moment it is created, and this is the one rule in it with a right answer.
 */
export function seenInPlace(threads: ThreadSummary[], key: string): ThreadSummary[] {
  if (!threads.some((t) => t.key === key && t.unseen)) return threads;
  return threads.map((t) => (t.key === key ? { ...t, unseen: false } : t));
}
