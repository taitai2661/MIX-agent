/**
 * Shared bounds for the paginated conversation list.
 *
 * `GET /conversations` is bounded (its own maximum is 500), so the client pages
 * with `limit`/`offset` instead of assuming the first response is the whole
 * history. Desktop uses the same pattern with its own (smaller) page size.
 */
export const CONVERSATION_PAGE_SIZE = 100;

/** Offset of the next page, or `undefined` when the last page was short. */
export function nextConversationOffset(
  lastPageLength: number,
  lastPageOffset: number,
): number | undefined {
  return lastPageLength === CONVERSATION_PAGE_SIZE
    ? lastPageOffset + lastPageLength
    : undefined;
}
