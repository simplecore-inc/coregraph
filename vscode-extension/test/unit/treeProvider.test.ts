import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  symbolDescription,
  directionLabel,
  edgeDescription,
  edgeTooltipMarkdown,
  symbolToDirectionNodes,
  edgesToNodes,
  summarizeSymbol,
  type TreeNode,
} from "../../src/providers/treeFormat";
import type { InspectResult, EdgeInfo } from "../../src/ipc/types";

const baseFreshness = { last_rebuild_at_ms: 100 };

function edge(
  partial: Partial<EdgeInfo> &
    Pick<EdgeInfo, "kind" | "origin" | "confidence" | "stale">,
): EdgeInfo {
  return { ...partial };
}

describe("treeFormat – symbolDescription", () => {
  it("renders just kind when edges absent", () => {
    const node: Extract<TreeNode, { kind: "symbol" }> = {
      kind: "symbol",
      file: "/f.rs",
      name: "save",
      symbolKind: "Method",
      line: 0,
      col: 0,
    };
    assert.equal(symbolDescription(node), "[Method]");
  });

  it("includes edges + stale when populated", () => {
    const node: Extract<TreeNode, { kind: "symbol" }> = {
      kind: "symbol",
      file: "/f.rs",
      name: "save",
      symbolKind: "Method",
      line: 0,
      col: 0,
      edges: 12,
      staleCount: 2,
    };
    assert.equal(symbolDescription(node), "[Method] · edges 12 · ⚠ 2 stale");
  });

  it("omits stale section when count is 0", () => {
    const node: Extract<TreeNode, { kind: "symbol" }> = {
      kind: "symbol",
      file: "/f.rs",
      name: "x",
      symbolKind: "Function",
      line: 0,
      col: 0,
      edges: 5,
      staleCount: 0,
    };
    assert.equal(symbolDescription(node), "[Function] · edges 5");
  });

  it("renders edges without stale when staleCount is undefined", () => {
    const node: Extract<TreeNode, { kind: "symbol" }> = {
      kind: "symbol",
      file: "/f.rs",
      name: "x",
      symbolKind: "Class",
      line: 0,
      col: 0,
      edges: 3,
    };
    assert.equal(symbolDescription(node), "[Class] · edges 3");
  });
});

describe("treeFormat – directionLabel", () => {
  it("renders Incoming/Outgoing with count", () => {
    assert.equal(
      directionLabel({
        kind: "direction",
        file: "/f",
        symbolName: "x",
        direction: "incoming",
        count: 3,
      }),
      "Incoming (3)",
    );
    assert.equal(
      directionLabel({
        kind: "direction",
        file: "/f",
        symbolName: "x",
        direction: "outgoing",
        count: 7,
      }),
      "Outgoing (7)",
    );
  });

  it("renders count 0", () => {
    assert.equal(
      directionLabel({
        kind: "direction",
        file: "/f",
        symbolName: "x",
        direction: "incoming",
        count: 0,
      }),
      "Incoming (0)",
    );
  });
});

describe("treeFormat – edgeDescription", () => {
  it("renders confidence % + kind", () => {
    const n: Extract<TreeNode, { kind: "edge" }> = {
      kind: "edge",
      otherName: "callee",
      edgeKind: "Calls",
      confidence: 0.99,
      stale: 0,
    };
    assert.equal(edgeDescription(n), "99% · Calls");
  });

  it("appends stale suffix when stale > 0", () => {
    const n: Extract<TreeNode, { kind: "edge" }> = {
      kind: "edge",
      otherName: "callee",
      edgeKind: "Calls",
      confidence: 0.85,
      stale: 3,
    };
    assert.equal(edgeDescription(n), "85% · Calls · stale 3");
  });

  it("formats confidence 1.0 as 100%", () => {
    const n: Extract<TreeNode, { kind: "edge" }> = {
      kind: "edge",
      otherName: "callee",
      edgeKind: "Uses",
      confidence: 1,
      stale: 0,
    };
    assert.equal(edgeDescription(n), "100% · Uses");
  });
});

describe("treeFormat – symbolToDirectionNodes", () => {
  it("emits both directions when both have edges", () => {
    const inspect: InspectResult = {
      symbol: { name: "x", kind: "F", file: "/f", line: 0 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
      ],
      edges_out: [
        edge({ to: "b", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
      ],
      freshness: baseFreshness,
    };
    const nodes = symbolToDirectionNodes("/f", "x", inspect);
    assert.equal(nodes.length, 2);
    assert.equal(nodes[0].direction, "incoming");
    assert.equal(nodes[0].count, 1);
    assert.equal(nodes[1].direction, "outgoing");
    assert.equal(nodes[1].count, 1);
  });

  it("omits direction when no edges", () => {
    const inspect: InspectResult = {
      symbol: { name: "x", kind: "F", file: "/f", line: 0 },
      edges_in: [],
      edges_out: [
        edge({ to: "b", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
      ],
      freshness: baseFreshness,
    };
    const nodes = symbolToDirectionNodes("/f", "x", inspect);
    assert.equal(nodes.length, 1);
    assert.equal(nodes[0].direction, "outgoing");
  });

  it("returns empty when both directions are empty", () => {
    const inspect: InspectResult = {
      symbol: { name: "x", kind: "F", file: "/f", line: 0 },
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const nodes = symbolToDirectionNodes("/f", "x", inspect);
    assert.equal(nodes.length, 0);
  });

  it("passes file and symbolName through to direction nodes", () => {
    const inspect: InspectResult = {
      symbol: { name: "myFn", kind: "F", file: "/src/mod.ts", line: 5 },
      edges_in: [
        edge({ from: "caller", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
      ],
      edges_out: [],
      freshness: baseFreshness,
    };
    const nodes = symbolToDirectionNodes("/src/mod.ts", "myFn", inspect);
    assert.equal(nodes[0].file, "/src/mod.ts");
    assert.equal(nodes[0].symbolName, "myFn");
  });
});

describe("treeFormat – edgesToNodes", () => {
  it("sorts by confidence descending", () => {
    const edges = [
      edge({ to: "lo", kind: "Calls", origin: "NameResolved", confidence: 0.5, stale: 0 }),
      edge({ to: "hi", kind: "Calls", origin: "NameResolved", confidence: 0.99, stale: 0 }),
      edge({ to: "mid", kind: "Calls", origin: "NameResolved", confidence: 0.7, stale: 0 }),
    ];
    const nodes = edgesToNodes(edges, "to");
    assert.deepEqual(nodes.map((n) => n.otherName), ["hi", "mid", "lo"]);
  });

  it("uses 'from' for incoming", () => {
    const edges = [
      edge({ from: "caller", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
    ];
    const nodes = edgesToNodes(edges, "from");
    assert.equal(nodes[0].otherName, "caller");
  });

  it("falls back to '<unknown>' when endpoint absent", () => {
    const edges = [
      edge({ kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
    ];
    const nodes = edgesToNodes(edges, "to");
    assert.equal(nodes[0].otherName, "<unknown>");
  });

  it("secondary sort by kind when confidence is equal", () => {
    const edges = [
      edge({ to: "b", kind: "Uses", origin: "NameResolved", confidence: 0.8, stale: 0 }),
      edge({ to: "a", kind: "Calls", origin: "NameResolved", confidence: 0.8, stale: 0 }),
    ];
    const nodes = edgesToNodes(edges, "to");
    // "Calls" < "Uses" alphabetically
    assert.equal(nodes[0].edgeKind, "Calls");
    assert.equal(nodes[1].edgeKind, "Uses");
  });

  it("preserves stale value on output nodes", () => {
    const edges = [
      edge({ to: "x", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 5 }),
    ];
    const nodes = edgesToNodes(edges, "to");
    assert.equal(nodes[0].stale, 5);
  });

  it("returns empty list for empty input", () => {
    const nodes = edgesToNodes([], "to");
    assert.equal(nodes.length, 0);
  });
});

describe("treeFormat – summarizeSymbol", () => {
  it("counts edges as in + out", () => {
    const inspect: InspectResult = {
      symbol: { name: "x", kind: "F", file: "/f", line: 0 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
        edge({ from: "b", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 1 }),
      ],
      edges_out: [
        edge({ to: "c", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 0 }),
      ],
      freshness: baseFreshness,
    };
    const s = summarizeSymbol(inspect);
    assert.equal(s.edges, 3);
    assert.equal(s.staleCount, 1);
  });

  it("counts stale across both in and out edges", () => {
    const inspect: InspectResult = {
      symbol: { name: "x", kind: "F", file: "/f", line: 0 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 3 }),
      ],
      edges_out: [
        edge({ to: "b", kind: "Calls", origin: "NameResolved", confidence: 1, stale: 2 }),
      ],
      freshness: baseFreshness,
    };
    const s = summarizeSymbol(inspect);
    // Each edge with stale > 0 counts as 1, regardless of stale value
    assert.equal(s.staleCount, 2);
  });

  it("returns zero edges and staleCount when no edges", () => {
    const inspect: InspectResult = {
      symbol: { name: "x", kind: "F", file: "/f", line: 0 },
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const s = summarizeSymbol(inspect);
    assert.equal(s.edges, 0);
    assert.equal(s.staleCount, 0);
  });
});

describe("treeFormat – edgeTooltipMarkdown", () => {
  const makeEdgeNode = (
    otherName: string,
    edgeKind: string,
    confidence: number,
    stale: number,
  ): Extract<TreeNode, { kind: "edge" }> => ({
    kind: "edge",
    otherName,
    edgeKind,
    confidence,
    stale,
  });

  it("includes the symbol name as a header", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("myCallee", "Calls", 0.99, 0));
    assert.match(md, /\*\*myCallee\*\*/);
  });

  it("includes the edge kind", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("x", "Uses", 0.75, 0));
    assert.match(md, /\*\*Uses\*\*/);
  });

  it("shows confidence as percentage", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("x", "Calls", 1.0, 0));
    assert.match(md, /\*\*100%\*\*/);
  });

  it("rounds confidence to integer percentage", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("x", "Calls", 0.955, 0));
    // toFixed(0) on 95.5 → "96"
    assert.match(md, /\*\*96%\*\*/);
  });

  it("omits stale section when stale is 0", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("x", "Calls", 0.99, 0));
    assert.doesNotMatch(md, /Stale/);
  });

  it("includes stale section when stale > 0", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("x", "Calls", 0.8, 3));
    assert.match(md, /⚠ \*\*Stale:\*\*/);
    assert.match(md, /3 reindex\(es\)/);
  });

  it("includes hover-to-reveal hint", () => {
    const md = edgeTooltipMarkdown(makeEdgeNode("x", "Calls", 0.99, 0));
    assert.match(md, /Hover to reveal the Go-to-symbol link/);
  });
});
