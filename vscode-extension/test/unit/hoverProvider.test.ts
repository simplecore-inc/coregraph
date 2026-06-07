import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import { formatHoverMarkdown, formatAge } from "../../src/providers/hoverFormat";
import type { InspectResult, EdgeInfo } from "../../src/ipc/types";

// Pure-logic tests for the Hover Markdown formatter.

const baseFreshness = { last_rebuild_at_ms: 1234 };

function edge(
  partial: Partial<EdgeInfo> & Pick<EdgeInfo, "kind" | "origin" | "confidence" | "stale">,
): EdgeInfo {
  return { ...partial };
}

describe("hoverFormat – formatHoverMarkdown", () => {
  it("renders the null-symbol notice when no symbol resolved", () => {
    const result: InspectResult = {
      symbol: null,
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    assert.match(md, /No symbol resolved at this position/);
    assert.match(md, /Last rebuild: 1\.2 s ago/);
  });

  it("renders symbol header + file:line for a resolved symbol", () => {
    const result: InspectResult = {
      symbol: { name: "save", kind: "Method", file: "/proj/repo.rs", line: 42 },
      edges_in: [],
      edges_out: [],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    assert.match(md, /\*\*`save`\*\* — _Method_/);
    assert.match(md, /`\/proj\/repo\.rs:42`/);
    assert.match(md, /\*\*Incoming\*\* \(0\)/);
    assert.match(md, /\*\*Outgoing\*\* \(0\)/);
    assert.match(md, /No callers known/);
    assert.match(md, /No callees known/);
  });

  it("lists top neighbors sorted by confidence with stale suffix", () => {
    const result: InspectResult = {
      symbol: { name: "fn", kind: "Function", file: "f.rs", line: 1 },
      edges_in: [
        edge({ from: "alpha", kind: "Calls", origin: "NameResolved", confidence: 0.95, stale: 0 }),
        edge({ from: "beta", kind: "Calls", origin: "Syntactic", confidence: 0.99, stale: 2 }),
        edge({ from: "gamma", kind: "Calls", origin: "NameResolved", confidence: 0.42, stale: 0 }),
      ],
      edges_out: [],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    // Sorted: beta (0.99), alpha (0.95), gamma (0.42)
    const betaIdx = md.indexOf("`beta`");
    const alphaIdx = md.indexOf("`alpha`");
    const gammaIdx = md.indexOf("`gamma`");
    assert.ok(betaIdx >= 0 && alphaIdx >= 0 && gammaIdx >= 0);
    assert.ok(betaIdx < alphaIdx, "beta (0.99) before alpha (0.95)");
    assert.ok(alphaIdx < gammaIdx, "alpha (0.95) before gamma (0.42)");
    assert.match(md, /`beta` — 99% · Calls \(stale 2\)/);
    assert.match(md, /`alpha` — 95% · Calls$/m);
  });

  it("truncates neighbor list at 5 with '… N more' suffix", () => {
    const edges_in: EdgeInfo[] = [];
    for (let i = 0; i < 8; i++) {
      edges_in.push(
        edge({
          from: `caller${i}`,
          kind: "Calls",
          origin: "NameResolved",
          confidence: 0.9 - i * 0.01,
          stale: 0,
        }),
      );
    }
    const result: InspectResult = {
      symbol: { name: "fn", kind: "Function", file: "f.rs", line: 1 },
      edges_in,
      edges_out: [],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    assert.match(md, /_… 3 more_/);
  });

  it("shows stale-evidence warning when any edge has stale > 0", () => {
    const result: InspectResult = {
      symbol: { name: "fn", kind: "Function", file: "f.rs", line: 1 },
      edges_in: [
        edge({ from: "x", kind: "Calls", origin: "NameResolved", confidence: 0.5, stale: 3 }),
      ],
      edges_out: [],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    assert.match(md, /⚠ \*\*Stale evidence:\*\* 1 edge\(s\)/);
  });

  it("omits stale warning when no edge is stale", () => {
    const result: InspectResult = {
      symbol: { name: "fn", kind: "Function", file: "f.rs", line: 1 },
      edges_in: [
        edge({ from: "x", kind: "Calls", origin: "NameResolved", confidence: 0.99, stale: 0 }),
      ],
      edges_out: [],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    assert.doesNotMatch(md, /Stale evidence/);
  });

  it("summarizes origin distribution across both directions", () => {
    const result: InspectResult = {
      symbol: { name: "fn", kind: "Function", file: "f.rs", line: 1 },
      edges_in: [
        edge({ from: "a", kind: "Calls", origin: "NameResolved", confidence: 0.99, stale: 0 }),
        edge({ from: "b", kind: "Calls", origin: "NameResolved", confidence: 0.99, stale: 0 }),
      ],
      edges_out: [
        edge({ to: "c", kind: "Calls", origin: "Syntactic", confidence: 0.5, stale: 0 }),
      ],
      freshness: baseFreshness,
    };
    const md = formatHoverMarkdown(result);
    assert.match(md, /\*\*Sources:\*\* NameResolved × 2, Syntactic × 1/);
  });
});

describe("hoverFormat – formatAge", () => {
  it("renders sub-second as ms", () => {
    assert.equal(formatAge(42), "42 ms");
  });

  it("renders < 10 s with one decimal", () => {
    assert.equal(formatAge(1500), "1.5 s");
    assert.equal(formatAge(9900), "9.9 s");
  });

  it("renders < 60 s as integer seconds", () => {
    assert.equal(formatAge(15000), "15 s");
    assert.equal(formatAge(59000), "59 s");
  });

  it("renders < 60 min as integer minutes", () => {
    assert.equal(formatAge(60000), "1 min");
    assert.equal(formatAge(180000), "3 min");
  });

  it("renders >= 1 h with one decimal", () => {
    assert.equal(formatAge(3600000), "1.0 h");
    assert.equal(formatAge(5400000), "1.5 h");
  });
});
