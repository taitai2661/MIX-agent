import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { AssociationList, type AssociationEdge } from "./AssociationsView";

const OLD = "11111111-2222-3333-4444-555555555555";
const NEW = "aaaaaaa1-bbbb-cccc-dddd-eeeeeeeeeeee";

const edges: AssociationEdge[] = [
  // §4's own direction: the row carrying `superseded_by_id` is the target.
  {
    id: "link-1",
    source_memory_id: NEW,
    target_memory_id: OLD,
    kind: "supersedes",
    weight: 0.2,
    coactivation_count: 0,
  },
  {
    id: "link-2",
    source_memory_id: OLD,
    target_memory_id: NEW,
    kind: "relates_to",
    weight: 0.35,
    coactivation_count: 2,
  },
];

describe("AssociationList", () => {
  it("renders both directions with §4's ids, kind, weight and count", () => {
    const html = renderToStaticMarkup(
      <AssociationList memoryId={OLD} edges={edges} />,
    );
    // Both edges are listed, source → target, in the shape §4 returns.
    expect(html).toContain("<code>aaaaaaa1</code> → <code>11111111</code>");
    expect(html).toContain("<code>11111111</code> → <code>aaaaaaa1</code>");
    // Which end carries the §3 column, and which end this memory is.
    expect(html).toContain("列を持つ側");
    expect(html).toContain("列が指す側");
    expect(html).toContain("この Memory");
    // §2's vocabulary is shown as stored — never re-worded for the UI.
    expect(html).toContain("<code>supersedes</code>");
    expect(html).toContain("<code>relates_to</code>");
    expect(html).toContain("weight 0.20");
    expect(html).toContain("coactivation_count 2");
  });

  it("says so when the memory has no edge at all", () => {
    const html = renderToStaticMarkup(
      <AssociationList memoryId={OLD} edges={[]} />,
    );
    expect(html).toContain("この記憶に繋がりはまだありません。");
  });
});
