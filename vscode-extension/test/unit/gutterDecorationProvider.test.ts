import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  toGutterHits,
  formatGutterTooltip,
  type GutterHit,
} from "../../src/providers/gutterDecorationFormat";
import type { DiffResult } from "../../src/ipc/types";

/** Build a minimal DiffResult with optional overrides. */
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

describe("gutterDecorationFormat – toGutterHits", () => {
  it("returns empty when no per_line_impact present", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: [],
          reachable_count: 0,
          confidence_weighted: 0,
          top_affected: [],
        },
      ],
    });
    assert.deepEqual(toGutterHits(d), []);
  });

  it("flattens per_line_impact entries into GutterHit[]", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: [],
          reachable_count: 0,
          confidence_weighted: 0,
          top_affected: [],
          per_line_impact: {
            "5": { symbols: ["foo"], confidence_weighted: 2.0 },
            "10": { symbols: ["bar", "baz"], confidence_weighted: 5.5 },
          },
        },
      ],
    });
    const hits = toGutterHits(d);
    assert.equal(hits.length, 2);
    const byLine = new Map(hits.map((h) => [h.line, h]));
    assert.deepEqual(byLine.get(5)?.symbols, ["foo"]);
    assert.equal(byLine.get(5)?.confidence_weighted, 2.0);
    assert.deepEqual(byLine.get(10)?.symbols, ["bar", "baz"]);
    assert.equal(byLine.get(10)?.confidence_weighted, 5.5);
  });

  it("skips entries with non-numeric keys", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: [],
          reachable_count: 0,
          confidence_weighted: 0,
          top_affected: [],
          per_line_impact: {
            "5": { symbols: ["foo"], confidence_weighted: 1.0 },
            garbage: { symbols: ["bogus"], confidence_weighted: 0 },
          },
        },
      ],
    });
    const hits = toGutterHits(d);
    assert.equal(hits.length, 1);
    assert.equal(hits[0].symbols[0], "foo");
  });

  it("aggregates hits from multiple files", () => {
    const d = diff({
      changed_files: [
        {
          file: "/a.rs",
          seed_symbols: [],
          reachable_count: 0,
          confidence_weighted: 0,
          top_affected: [],
          per_line_impact: {
            "0": { symbols: ["a"], confidence_weighted: 1.0 },
          },
        },
        {
          file: "/b.rs",
          seed_symbols: [],
          reachable_count: 0,
          confidence_weighted: 0,
          top_affected: [],
          per_line_impact: {
            "3": { symbols: ["b"], confidence_weighted: 2.0 },
          },
        },
      ],
    });
    const hits = toGutterHits(d);
    assert.equal(hits.length, 2);
    const files = hits.map((h) => h.file);
    assert.ok(files.includes("/a.rs"));
    assert.ok(files.includes("/b.rs"));
  });
});

describe("gutterDecorationFormat – formatGutterTooltip", () => {
  it("renders impact value and symbol list as Markdown", () => {
    const h: GutterHit = {
      file: "/a.rs",
      line: 5,
      symbols: ["foo", "bar"],
      confidence_weighted: 3.14,
    };
    const t = formatGutterTooltip(h);
    assert.match(t, /\*\*Impact: 3\.1\*\*/);
    assert.match(t, /- `foo`/);
    assert.match(t, /- `bar`/);
  });

  it("handles a single symbol", () => {
    const h: GutterHit = {
      file: "/b.ts",
      line: 0,
      symbols: ["onlySym"],
      confidence_weighted: 0.0,
    };
    const t = formatGutterTooltip(h);
    assert.match(t, /- `onlySym`/);
    assert.match(t, /Impact: 0\.0/);
  });

  it("includes threshold line when provided", () => {
    const h: GutterHit = {
      file: "/a.rs",
      line: 5,
      symbols: ["foo"],
      confidence_weighted: 3.14,
    };
    const t = formatGutterTooltip(h, 20);
    assert.match(t, /warn threshold: 20/);
    assert.match(t, /commit above this level triggers warning/);
  });

  it("omits threshold line when not provided", () => {
    const h: GutterHit = {
      file: "/a.rs",
      line: 5,
      symbols: ["foo"],
      confidence_weighted: 3.14,
    };
    const t = formatGutterTooltip(h);
    assert.doesNotMatch(t, /warn threshold/);
  });
});
