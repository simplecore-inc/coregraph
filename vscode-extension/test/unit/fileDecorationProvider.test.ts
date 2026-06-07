import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  impactTier,
  themeColorId,
  badgeChar,
  formatTooltip,
  toDecorationMap,
} from "../../src/providers/fileDecorationFormat";
import type { DiffResult, DiffFileImpact } from "../../src/ipc/types";

describe("fileDecorationFormat – impactTier", () => {
  it("returns low for < 5", () => {
    assert.equal(impactTier(0), "low");
    assert.equal(impactTier(4.9), "low");
  });
  it("returns medium for 5-20 inclusive", () => {
    assert.equal(impactTier(5), "medium");
    assert.equal(impactTier(15), "medium");
    assert.equal(impactTier(20), "medium");
  });
  it("returns high for > 20", () => {
    assert.equal(impactTier(20.01), "high");
    assert.equal(impactTier(100), "high");
  });
});

describe("fileDecorationFormat – themeColorId + badgeChar", () => {
  it("maps tiers to stable ids + glyphs", () => {
    assert.equal(themeColorId("low"), "coregraph.impact.low");
    assert.equal(themeColorId("medium"), "coregraph.impact.medium");
    assert.equal(themeColorId("high"), "coregraph.impact.high");
    assert.equal(badgeChar("low"), "•");
    assert.equal(badgeChar("medium"), "!");
    assert.equal(badgeChar("high"), "‼");
  });
});

describe("fileDecorationFormat – formatTooltip", () => {
  it("includes impact, reach, seeds, and top_affected", () => {
    const f: DiffFileImpact = {
      file: "/r.rs",
      seed_symbols: ["save", "load"],
      reachable_count: 42,
      confidence_weighted: 18.5,
      top_affected: [
        { name: "controller.handle", file: "/c.rs", confidence: 0.99 },
        { name: "service.run", file: "/s.rs", confidence: 0.85 },
      ],
    };
    const tip = formatTooltip(f);
    assert.match(tip, /Impact: 18\.5/);
    assert.match(tip, /reach: 42 symbols/);
    assert.match(tip, /Seeds: save, load/);
    assert.match(tip, /controller\.handle/);
    assert.match(tip, /99%/);
  });

  it("includes threshold warning when threshold provided", () => {
    const f: DiffFileImpact = {
      file: "/r.rs",
      seed_symbols: [],
      reachable_count: 0,
      confidence_weighted: 0,
      top_affected: [],
    };
    const tip = formatTooltip(f, 20);
    assert.match(tip, /warn threshold: 20/);
    assert.match(tip, /commit above this level triggers warning/);
  });

  it("omits threshold line when threshold not provided", () => {
    const f: DiffFileImpact = {
      file: "/r.rs",
      seed_symbols: [],
      reachable_count: 0,
      confidence_weighted: 0,
      top_affected: [],
    };
    const tip = formatTooltip(f);
    assert.doesNotMatch(tip, /warn threshold/);
  });

  it("omits top_affected section when empty", () => {
    const f: DiffFileImpact = {
      file: "/r.rs",
      seed_symbols: [],
      reachable_count: 0,
      confidence_weighted: 0,
      top_affected: [],
    };
    const tip = formatTooltip(f);
    assert.doesNotMatch(tip, /Top affected/);
    assert.doesNotMatch(tip, /Seeds:/);
  });
});

describe("fileDecorationFormat – toDecorationMap", () => {
  it("groups changed_files by file path", () => {
    const diff: DiffResult = {
      base_ref: "HEAD",
      changed_files: [
        {
          file: "/a.rs", seed_symbols: [], reachable_count: 0,
          confidence_weighted: 0, top_affected: [],
        },
        {
          file: "/b.rs", seed_symbols: ["x"], reachable_count: 5,
          confidence_weighted: 3.1, top_affected: [],
        },
      ],
      total_reachable: 5,
      total_confidence_weighted: 3.1,
      inconsistencies_introduced: [],
      new_orphans: [],
      git_operation_in_progress: false,
    };
    const map = toDecorationMap(diff);
    assert.equal(map.size, 2);
    assert.equal(map.get("/b.rs")?.confidence_weighted, 3.1);
  });
});
