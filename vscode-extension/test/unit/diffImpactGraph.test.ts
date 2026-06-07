import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  diffResultToElements,
  graphSummary,
  type CyElement,
} from "../../src/panels/diffImpactGraph";
import type { DiffResult } from "../../src/ipc/types";

function diff(p: Partial<DiffResult> = {}): DiffResult {
  return {
    base_ref: "HEAD",
    changed_files: [],
    total_reachable: 0,
    total_confidence_weighted: 0,
    inconsistencies_introduced: [],
    new_orphans: [],
    git_operation_in_progress: false,
    ...p,
  };
}

describe("diffImpactGraph – diffResultToElements", () => {
  it("returns empty when no changed files", () => {
    assert.equal(diffResultToElements(diff()).length, 0);
  });

  it("emits seed nodes per file", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: ["foo", "bar"],
          reachable_count: 2,
          confidence_weighted: 3.0,
          top_affected: [],
        },
      ],
    });
    const els = diffResultToElements(d);
    const seeds = els.filter((e) => e.classes === "seed");
    assert.equal(seeds.length, 2);
    assert.ok(seeds.some((e) => e.data.label === "foo"));
    assert.ok(seeds.some((e) => e.data.label === "bar"));
  });

  it("emits affected nodes + edges from seeds", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: ["foo"],
          reachable_count: 1,
          confidence_weighted: 2.0,
          top_affected: [{ name: "callee", file: "/b.rs", confidence: 0.9 }],
        },
      ],
    });
    const els = diffResultToElements(d);
    const seeds = els.filter((e) => e.classes === "seed");
    const affected = els.filter((e) => e.classes === "affected");
    const edges = els.filter((e) => e.group === "edges");
    assert.equal(seeds.length, 1);
    assert.equal(affected.length, 1);
    assert.equal(edges.length, 1);
    const edge = edges[0];
    assert.ok(seeds[0] !== undefined);
    assert.ok(affected[0] !== undefined);
    assert.equal(edge.data.source, seeds[0]!.data.id);
    assert.equal(edge.data.target, affected[0]!.data.id);
    assert.equal(edge.data.confidence, 0.9);
  });

  it("dedups affected nodes across multiple seeds — only one node, two edges", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: ["foo", "bar"],
          reachable_count: 2,
          confidence_weighted: 5.0,
          top_affected: [{ name: "shared", file: "/c.rs", confidence: 0.8 }],
        },
      ],
    });
    const els = diffResultToElements(d);
    const affected = els.filter((e) => e.classes === "affected");
    // Only one distinct affected node, even though two seeds both point at it.
    assert.equal(affected.length, 1);
    // Two edges: one per seed.
    const edges = els.filter((e) => e.group === "edges");
    assert.equal(edges.length, 2);
  });

  it("dedups seed nodes across multiple changed files referencing the same symbol", () => {
    // Two changed files each listing the same seed symbol. The node should only
    // appear once in the output since the id is keyed on (file, name).
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: ["foo"],
          reachable_count: 1,
          confidence_weighted: 1.0,
          top_affected: [],
        },
        {
          file: "/a.rs",
          seed_symbols: ["foo"],
          reachable_count: 1,
          confidence_weighted: 1.0,
          top_affected: [],
        },
      ],
    });
    const els = diffResultToElements(d);
    const seeds = els.filter((e) => e.classes === "seed");
    assert.equal(seeds.length, 1);
  });

  it("attaches file and weight to seed nodes", () => {
    const d = diff({
      changed_files: [
        {
          file: "/proj/mod.rs",
          seed_symbols: ["init"],
          reachable_count: 0,
          confidence_weighted: 7.5,
          top_affected: [],
        },
      ],
    });
    const els = diffResultToElements(d);
    const seed = els.find((e) => e.classes === "seed");
    assert.ok(seed !== undefined);
    assert.equal(seed!.data.file, "/proj/mod.rs");
    assert.equal(seed!.data.weight, 7.5);
    assert.equal(seed!.data.label, "init");
  });

  it("attaches confidence as weight to affected nodes", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: ["s"],
          reachable_count: 1,
          confidence_weighted: 1.0,
          top_affected: [{ name: "dep", file: "/b.rs", confidence: 0.65 }],
        },
      ],
    });
    const els = diffResultToElements(d);
    const aff = els.find((e) => e.classes === "affected");
    assert.ok(aff !== undefined);
    assert.equal(aff!.data.weight, 0.65);
  });

  it("handles multiple changed files independently", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: ["fn_a"],
          reachable_count: 1,
          confidence_weighted: 2.0,
          top_affected: [{ name: "dep_a", file: "/dep.rs", confidence: 0.5 }],
        },
        {
          file: "/b.rs",
          seed_symbols: ["fn_b"],
          reachable_count: 1,
          confidence_weighted: 3.0,
          top_affected: [{ name: "dep_b", file: "/dep.rs", confidence: 0.7 }],
        },
      ],
    });
    const els = diffResultToElements(d);
    const seeds = els.filter((e) => e.classes === "seed");
    const affected = els.filter((e) => e.classes === "affected");
    const edges = els.filter((e) => e.group === "edges");
    assert.equal(seeds.length, 2);
    // dep_a and dep_b are distinct (different names → different ids).
    assert.equal(affected.length, 2);
    assert.equal(edges.length, 2);
  });
});

describe("diffImpactGraph – graphSummary", () => {
  it("counts nodes, edges, seeds, affected", () => {
    const els: CyElement[] = [
      { group: "nodes", data: { id: "s1" }, classes: "seed" },
      { group: "nodes", data: { id: "s2" }, classes: "seed" },
      { group: "nodes", data: { id: "a1" }, classes: "affected" },
      { group: "edges", data: { id: "e1", source: "s1", target: "a1" } },
    ];
    const s = graphSummary(els);
    assert.equal(s.nodes, 3);
    assert.equal(s.edges, 1);
    assert.equal(s.seeds, 2);
    assert.equal(s.affected, 1);
  });

  it("returns zeros for empty elements", () => {
    const s = graphSummary([]);
    assert.equal(s.nodes, 0);
    assert.equal(s.edges, 0);
    assert.equal(s.seeds, 0);
    assert.equal(s.affected, 0);
  });

  it("handles all-edges input", () => {
    const els: CyElement[] = [
      { group: "edges", data: { id: "e1", source: "s1", target: "a1" } },
      { group: "edges", data: { id: "e2", source: "s2", target: "a1" } },
    ];
    const s = graphSummary(els);
    assert.equal(s.nodes, 0);
    assert.equal(s.edges, 2);
  });
});
