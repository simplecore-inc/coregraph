import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import { buildDiffImpactHtml } from "../../src/panels/diffImpactHtml";
import type { DiffResult } from "../../src/ipc/types";

function makeDiff(partial: Partial<DiffResult> = {}): DiffResult {
  return {
    base_ref: "HEAD",
    changed_files: [],
    total_reachable: 0,
    total_confidence_weighted: 0,
    inconsistencies_introduced: [],
    new_orphans: [],
    git_operation_in_progress: false,
    ...partial,
  };
}

describe("diffImpactHtml – buildDiffImpactHtml", () => {
  const csp = "vscode-webview:";
  // Stub values for the two new parameters added in Phase 2.2+.
  const cytoscapeUriStub = "cytoscape-url-stub";
  const elementsStub = "[]";

  it("renders empty-state sections when nothing changed", () => {
    const html = buildDiffImpactHtml(makeDiff(), csp, cytoscapeUriStub, elementsStub);
    assert.match(html, /No files changed/);
    assert.match(html, /No inconsistencies introduced/);
    assert.match(html, /No new orphans introduced/);
  });

  it("renders per-file rows with impact + reach", () => {
    const html = buildDiffImpactHtml(
      makeDiff({
        changed_files: [
          {
            file: "/proj/a.rs",
            seed_symbols: ["foo"],
            reachable_count: 5,
            confidence_weighted: 2.5,
            top_affected: [
              { name: "bar", file: "/proj/b.rs", confidence: 0.9 },
            ],
          },
        ],
        total_reachable: 5,
        total_confidence_weighted: 2.5,
      }),
      csp,
      cytoscapeUriStub,
      elementsStub,
    );
    assert.match(html, /\/proj\/a\.rs/);
    assert.match(html, /impact 2\.5/);
    assert.match(html, /reach 5/);
    assert.match(html, /top:/);
    assert.match(html, /bar/);
  });

  it("renders inconsistency rows with jump links", () => {
    const html = buildDiffImpactHtml(
      makeDiff({
        inconsistencies_introduced: [
          {
            category: "EnumMismatch",
            shared_value: "ACTIVE",
            a: { name: "Foo", file: "/a.rs", line: 5 },
            b: { name: "Bar", file: "/b.rs", line: 9 },
          },
        ],
      }),
      csp,
      cytoscapeUriStub,
      elementsStub,
    );
    assert.match(html, /\[EnumMismatch\]/);
    assert.match(html, /data-file="\/a\.rs"/);
    assert.match(html, /data-line="5"/);
    assert.match(html, /data-file="\/b\.rs"/);
  });

  it("renders new orphans list", () => {
    const html = buildDiffImpactHtml(
      makeDiff({ new_orphans: ["helper_fn", "dead_code"] }),
      csp,
      cytoscapeUriStub,
      elementsStub,
    );
    assert.match(html, /helper_fn/);
    assert.match(html, /dead_code/);
  });

  it("escapes HTML in user-supplied data", () => {
    const html = buildDiffImpactHtml(
      makeDiff({
        changed_files: [
          {
            file: '/proj/<script>alert("x")</script>.rs',
            seed_symbols: [],
            reachable_count: 0,
            confidence_weighted: 0,
            top_affected: [],
          },
        ],
      }),
      csp,
      cytoscapeUriStub,
      elementsStub,
    );
    assert.doesNotMatch(html, /<script>alert/);
    assert.match(html, /&lt;script&gt;/);
  });

  it("includes CSP header with the provided cspSource", () => {
    const html = buildDiffImpactHtml(makeDiff(), "my-csp-value", cytoscapeUriStub, elementsStub);
    assert.match(html, /style-src my-csp-value/);
    assert.match(html, /script-src my-csp-value/);
  });

  it("surfaces git-operation warning when flag is set", () => {
    const html = buildDiffImpactHtml(
      makeDiff({ git_operation_in_progress: true }),
      csp,
      cytoscapeUriStub,
      elementsStub,
    );
    assert.match(html, /Git operation in progress/);
  });

  it("emits tab buttons for Text and Graph", () => {
    const html = buildDiffImpactHtml(makeDiff(), csp, cytoscapeUriStub, elementsStub);
    assert.match(html, /data-tab="text"/);
    assert.match(html, /data-tab="graph"/);
  });

  it("includes the cytoscapeUri in a script src", () => {
    const html = buildDiffImpactHtml(makeDiff(), csp, "https://example.com/cy.js", "[]");
    assert.match(html, /src="https:\/\/example\.com\/cy\.js"/);
  });
});
