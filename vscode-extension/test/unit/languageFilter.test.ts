import { strict as assert } from "node:assert";
import { describe, it } from "mocha";
import {
  SUPPORTED_LANGUAGES,
  isSupportedLanguage,
  isSupportedDoc,
  type DocLike,
} from "../../src/util/languageFilter";

// Covers the dogfood regression where settings.json / Output buffers
// / files under ~/Library/Application Support/Code were triggering
// reindex of unrelated directories. The filter keeps the daemon from
// indexing anything outside the user's workspace in a supported
// language.

function doc(partial: Partial<{ scheme: string; languageId: string }> = {}): DocLike {
  return {
    uri: { scheme: partial.scheme ?? "file" },
    languageId: partial.languageId ?? "rust",
  };
}

describe("languageFilter — SUPPORTED_LANGUAGES", () => {
  it("contains the exact 9 extractor targets", () => {
    // Tests live alongside the constant so drift is caught loudly:
    // any change here must match the LSP documentSelector in
    // extension.ts and the activationEvents in package.json.
    const expected = [
      "rust",
      "java",
      "typescript",
      "typescriptreact",
      "javascript",
      "javascriptreact",
      "python",
      "go",
      "kotlin",
    ].sort();
    const actual = [...SUPPORTED_LANGUAGES].sort();
    assert.deepEqual(actual, expected);
  });
});

describe("languageFilter — isSupportedLanguage", () => {
  it("returns true for every supported language id", () => {
    for (const id of SUPPORTED_LANGUAGES) {
      assert.equal(isSupportedLanguage(id), true, `expected ${id} supported`);
    }
  });

  it("returns false for unsupported ids", () => {
    for (const id of ["json", "jsonc", "plaintext", "markdown", "yaml", "log", "xml"]) {
      assert.equal(isSupportedLanguage(id), false, `expected ${id} unsupported`);
    }
  });

  it("is case-sensitive (VSCode emits lowercase ids)", () => {
    assert.equal(isSupportedLanguage("Rust"), false);
    assert.equal(isSupportedLanguage("TYPESCRIPT"), false);
  });
});

describe("languageFilter — isSupportedDoc", () => {
  it("accepts supported language under workspace root with file:// scheme", () => {
    assert.equal(isSupportedDoc(doc({ languageId: "typescript" }), true), true);
    assert.equal(isSupportedDoc(doc({ languageId: "rust" }), true), true);
  });

  it("rejects unsupported languages even under workspace", () => {
    // The dogfood regression: user saves settings.json, reindex fires
    // because scheme is file:// and workspace-folder lookup succeeded,
    // but the language filter must still bar it.
    assert.equal(isSupportedDoc(doc({ languageId: "json" }), true), false);
    assert.equal(isSupportedDoc(doc({ languageId: "plaintext" }), true), false);
    assert.equal(isSupportedDoc(doc({ languageId: "log" }), true), false);
  });

  it("rejects supported language outside any workspace folder", () => {
    // Files the user opens ad-hoc (drag-and-drop, external editor)
    // have no workspace folder. Reindexing would spawn the daemon
    // against an unrelated project root.
    assert.equal(isSupportedDoc(doc({ languageId: "typescript" }), false), false);
    assert.equal(isSupportedDoc(doc({ languageId: "rust" }), false), false);
  });

  it("rejects non-file schemes (output buffer, git diff, untitled)", () => {
    // Git diff view, Output panel, Untitled docs — all non-file schemes.
    // Even if languageId happens to be "typescript" (syntax preview),
    // there's no on-disk file to reindex.
    assert.equal(isSupportedDoc(doc({ scheme: "output", languageId: "typescript" }), true), false);
    assert.equal(isSupportedDoc(doc({ scheme: "git", languageId: "rust" }), true), false);
    assert.equal(isSupportedDoc(doc({ scheme: "untitled", languageId: "python" }), true), false);
    assert.equal(isSupportedDoc(doc({ scheme: "vscode-userdata", languageId: "json" }), true), false);
  });

  it("rejects when all three conditions fail simultaneously", () => {
    assert.equal(
      isSupportedDoc(doc({ scheme: "output", languageId: "json" }), false),
      false,
    );
  });
});
