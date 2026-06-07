import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  flattenAndFilterDecls,
  formatTitle,
  formatCodeLensTooltip,
  isDeclKind,
  SYMBOL_KIND,
  type SymbolLike,
} from "../../src/providers/codelensFormat";
import type { ImpactEntry } from "../../src/ipc/types";

// Pure-logic tests for the CodeLens formatter helpers. The provider
// itself depends on the `vscode` namespace and is exercised via the
// extension host (integration tests, separate suite).

describe("codelensFormat – formatTitle", () => {
  it("renders known symbol with reach count + impact", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 11,
      edges: 25,
      confidence_weighted: 8.42,
    };
    assert.equal(formatTitle(entry), "reach 11 · impact 8.4");
  });

  it("renders Orphan when nodes === 0", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 0,
      edges: 0,
      confidence_weighted: 0,
    };
    assert.equal(formatTitle(entry), "Orphan — entry point or dead code");
  });

  it("renders not-in-graph message for null entry", () => {
    assert.equal(formatTitle(null), "CoreGraph: symbol not in graph");
  });
});

describe("codelensFormat – isDeclKind", () => {
  it("returns true for declaration kinds", () => {
    const declKinds = [
      SYMBOL_KIND.Function,
      SYMBOL_KIND.Method,
      SYMBOL_KIND.Class,
      SYMBOL_KIND.Interface,
      SYMBOL_KIND.Struct,
      SYMBOL_KIND.Enum,
      SYMBOL_KIND.Constructor,
    ];
    for (const kind of declKinds) {
      assert.equal(isDeclKind(kind), true, `kind ${kind}`);
    }
  });

  it("returns false for non-declaration kinds", () => {
    const otherKinds = [
      SYMBOL_KIND.Variable,
      SYMBOL_KIND.Field,
      SYMBOL_KIND.Property,
      SYMBOL_KIND.Constant,
      SYMBOL_KIND.Module,
      SYMBOL_KIND.File,
      SYMBOL_KIND.EnumMember,
    ];
    for (const kind of otherKinds) {
      assert.equal(isDeclKind(kind), false, `kind ${kind}`);
    }
  });
});

describe("codelensFormat – flattenAndFilterDecls", () => {
  // Use a simple opaque range type so the test doesn't depend on vscode.
  type Range = { line: number };
  const r = (line: number): Range => ({ line });

  const make = (
    name: string,
    kind: number,
    children?: SymbolLike<Range>[],
  ): SymbolLike<Range> => ({
    name,
    kind,
    selectionRange: r(0),
    children,
  });

  it("includes a single Function symbol", () => {
    const result = flattenAndFilterDecls<Range>([
      make("myFn", SYMBOL_KIND.Function),
    ]);
    assert.equal(result.length, 1);
    assert.equal(result[0].name, "myFn");
  });

  it("excludes noisy kinds (Variable, Field, Property)", () => {
    for (const kind of [
      SYMBOL_KIND.Variable,
      SYMBOL_KIND.Field,
      SYMBOL_KIND.Property,
    ]) {
      const result = flattenAndFilterDecls<Range>([make("x", kind)]);
      assert.equal(result.length, 0, `kind ${kind} should be excluded`);
    }
  });

  it("traverses nested children and collects qualifying kinds", () => {
    const cls = make("MyClass", SYMBOL_KIND.Class, [
      make("doThing", SYMBOL_KIND.Method),
      make("count", SYMBOL_KIND.Field),
    ]);
    const result = flattenAndFilterDecls<Range>([cls]);
    // cls (Class) + method (Method); field (Field) excluded.
    assert.equal(result.length, 2);
    const names = result.map((d) => d.name);
    assert.ok(names.includes("MyClass"));
    assert.ok(names.includes("doThing"));
    assert.ok(!names.includes("count"));
  });
});

describe("codelensFormat – formatCodeLensTooltip", () => {
  it("explains null entry as not-found with reindex hint", () => {
    const tip = formatCodeLensTooltip(null);
    assert.match(tip, /not found/i);
    assert.match(tip, /reindex/i);
  });

  it("explains zero-node entry as orphan / dead code", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 0,
      edges: 0,
      confidence_weighted: 0,
    };
    const tip = formatCodeLensTooltip(entry);
    assert.match(tip, /Orphan/i);
    assert.match(tip, /dead code/i);
  });

  it("explains reach and impact for a normal entry", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 7,
      edges: 14,
      confidence_weighted: 5.3,
    };
    const tip = formatCodeLensTooltip(entry);
    assert.match(tip, /reach 7/);
    assert.match(tip, /impact 5\.3/);
    assert.match(tip, /confidence/i);
  });

  it("formats impact to 1 decimal place", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 3,
      edges: 4,
      confidence_weighted: 2.0,
    };
    const tip = formatCodeLensTooltip(entry);
    assert.match(tip, /impact 2\.0/);
  });

  it("includes use-case sentence in normal entry tooltip", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 5,
      edges: 10,
      confidence_weighted: 4.0,
    };
    const tip = formatCodeLensTooltip(entry);
    assert.match(tip, /refactoring this symbol/i);
    assert.match(tip, /test\/review/i);
  });

  it("includes threshold line when threshold provided", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 5,
      edges: 10,
      confidence_weighted: 4.0,
    };
    const tip = formatCodeLensTooltip(entry, 20);
    assert.match(tip, /warn threshold: 20/);
    assert.match(tip, /change in settings/);
  });

  it("omits threshold line when threshold not provided", () => {
    const entry: ImpactEntry = {
      seeds: 1,
      seeds_used: 1,
      truncated: false,
      nodes: 5,
      edges: 10,
      confidence_weighted: 4.0,
    };
    const tip = formatCodeLensTooltip(entry);
    assert.doesNotMatch(tip, /warn threshold/);
  });
});
