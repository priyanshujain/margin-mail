import { call, type SearchResult } from "../ipc";

/**
 * Local FTS over the mirror, with the operators from features.md. When the query reaches past the
 * storage window the provider is asked as a second pass and its hits are hydrated as transient rows.
 */
export const search = (accountId: string | null, query: string, cursor?: string | null) =>
  call<SearchResult>("search", { accountId, query, cursor: cursor ?? null });

/** The explicit "Search older mail on Gmail" at the foot of a result list. */
export const searchProvider = (accountId: string, query: string) =>
  call<SearchResult>("search_provider", { accountId, query });
