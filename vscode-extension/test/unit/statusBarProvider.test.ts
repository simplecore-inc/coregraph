import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import { formatStatusBar } from "../../src/providers/statusBarFormat";
import type {
  InspectResult,
  ImpactEntry,
  EdgeInfo,
} from "../../src/ipc/types";

const baseFreshness = { last_rebuild_at_ms: 1000 };

function edge(
  partial: Partial<EdgeInfo> & Pick<EdgeInfo, "kind" | "origin" | "confidence" | "stale">,
): EdgeInfo {
  return { ...partial };
}

describe("statusBarFormat – formatStatusBar", () => {
  it("hides when no symbol resolved", () => {
    const inspect: InspectResult = {
      symbol: null,
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const r = formatStatusBar(inspect, null);
    assert.equal(r.visible, false);
    assert.equal(r.text, "");
  });

  it("shows symbol name only when impact is null", () => {
    const inspect: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/r.rs", line: 5 },
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const r = formatStatusBar(inspect, null);
    assert.equal(r.visible, true);
    assert.equal(r.text, "save");
  });

  it("appends impact value when present", () => {
    const inspect: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/r.rs", line: 5 },
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const impact: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 42,
      edges: 80,
      confidence_weighted: 42.13,
    };
    const r = formatStatusBar(inspect, impact);
    assert.equal(r.text, "save · impact 42.1");
  });

  it("appends stale-warning when any edge has stale > 0", () => {
    const inspect: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/r.rs", line: 5 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 1 }),
        edge({ from: "b", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 0 }),
      ],
      edges_out: [
        edge({ to: "c", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 2 }),
      ],
      freshness: baseFreshness,
    };
    const r = formatStatusBar(inspect, null);
    // 2 edges have stale > 0 (not summed by stale value, just counted).
    assert.equal(r.text, "save · ⚠ 2 stale");
  });

  it("renders all three parts in order: name · impact · stale", () => {
    const inspect: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/r.rs", line: 5 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 5 }),
      ],
      edges_out: [],
      freshness: baseFreshness,
    };
    const impact: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 10,
      edges: 12,
      confidence_weighted: 8.4,
    };
    const r = formatStatusBar(inspect, impact);
    assert.equal(r.text, "save · impact 8.4 · ⚠ 1 stale");
  });

  it("accessibleText is empty string when no symbol", () => {
    const inspect: InspectResult = {
      symbol: null,
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const r = formatStatusBar(inspect, null);
    assert.equal(r.accessibleText, "");
  });

  it("accessibleText is plain english with no codicons", () => {
    const inspect: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/r.rs", line: 5 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 5 }),
        edge({ from: "b", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 5 }),
      ],
      edges_out: [],
      freshness: baseFreshness,
    };
    const impact: ImpactEntry = {
      seeds: 1, seeds_used: 1, truncated: false, nodes: 10, edges: 12, confidence_weighted: 8.4,
    };
    const r = formatStatusBar(inspect, impact);
    assert.doesNotMatch(r.accessibleText, /\$\(/);
    assert.doesNotMatch(r.accessibleText, /⚠/);
    assert.match(r.accessibleText, /save/);
    assert.match(r.accessibleText, /impact 8\.4/);
    assert.match(r.accessibleText, /2 stale edges/);
  });

  it("tooltip includes file:line, in/out counts, impact summary, stale", () => {
    const inspect: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/r.rs", line: 42 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 1 }),
      ],
      edges_out: [
        edge({ to: "x", kind: "Calls", origin: "NameResolved", confidence: 0.9, stale: 0 }),
      ],
      freshness: baseFreshness,
    };
    const impact: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 5,
      edges: 8,
      confidence_weighted: 3.2,
    };
    const r = formatStatusBar(inspect, impact);
    assert.match(r.tooltip, /save\s+\[Method\]/);
    assert.match(r.tooltip, /\/r\.rs:42/);
    assert.match(r.tooltip, /In: 1/);
    assert.match(r.tooltip, /Out: 1/);
    assert.match(r.tooltip, /reach 5/);
    assert.match(r.tooltip, /impact 3\.2/);
    assert.match(r.tooltip, /1 caller\/callee edge\(s\) may be outdated/);
  });
});
