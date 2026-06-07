import type { DiffResult } from "../ipc/types";

/** Build the full webview HTML document for a DiffResult.
 *
 * Pure function — no `vscode` dependency. Produces self-contained HTML
 * with inline CSS so it renders correctly under VSCode's webview
 * `default-src 'none'` CSP.
 *
 * Click handlers for "jump-to-file" are emitted as `data-file` / `data-line`
 * attributes; the panel wires `onDidReceiveMessage` to open the editor at
 * the target location.
 *
 * @param diff - the git-enriched DiffResult from ipc.diff.
 * @param cspSource - webview.cspSource from the VSCode API (passed so tests
 *   can call with a fake value without touching vscode).
 * @param cytoscapeUri - webview URI string for the bundled cytoscape.min.js,
 *   resolved via `panel.webview.asWebviewUri(...)`.
 * @param elementsJson - JSON-serialized CyElement[] array for the graph.
 */
export function buildDiffImpactHtml(
  diff: DiffResult,
  cspSource: string,
  cytoscapeUri: string,
  elementsJson: string,
): string {
  const esc = (s: string): string =>
    s
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");

  const summary = `
    <section class="summary">
      <h2>Summary</h2>
      <ul>
        <li><strong>${diff.changed_files.length}</strong> files changed (base: <code>${esc(diff.base_ref)}</code>)</li>
        <li><strong>${diff.total_reachable}</strong> symbols reachable (depth 3)</li>
        <li>Impact: <strong>${diff.total_confidence_weighted.toFixed(1)}</strong></li>
        <li>${diff.inconsistencies_introduced.length} inconsistencies introduced</li>
        <li>${diff.new_orphans.length} new orphans</li>
        ${
          diff.git_operation_in_progress
            ? '<li class="warn">&#x26A0; Git operation in progress (merge / rebase / cherry-pick)</li>'
            : ""
        }
        ${diff.note ? `<li class="note">${esc(diff.note)}</li>` : ""}
      </ul>
    </section>
  `;

  const filesRows =
    diff.changed_files.length === 0
      ? '<p class="empty">No files changed relative to base_ref.</p>'
      : diff.changed_files
          .map((f) => {
            const topLine =
              f.top_affected.length > 0
                ? `<div class="top">top: ${f.top_affected
                    .map(
                      (t) =>
                        `<span class="sym" data-file="${esc(t.file)}">${esc(t.name)}</span> (${Math.round(t.confidence * 100) + "%"})`,
                    )
                    .join(", ")}</div>`
                : "";
            return `
            <div class="file-row">
              <a class="file-link" data-file="${esc(f.file)}">${esc(f.file)}</a>
              <span class="impact">impact ${f.confidence_weighted.toFixed(1)} &middot; reach ${f.reachable_count}</span>
              ${topLine}
            </div>
          `;
          })
          .join("");

  const incRows =
    diff.inconsistencies_introduced.length === 0
      ? '<p class="empty">No inconsistencies introduced.</p>'
      : diff.inconsistencies_introduced
          .map(
            (r) => `
          <div class="inc-row">
            <div class="inc-category">[${esc(r.category)}]</div>
            <div class="inc-bodies">
              <code>${esc(r.a.name)}</code> (<a data-file="${esc(r.a.file)}" data-line="${r.a.line}">${esc(r.a.file)}:${r.a.line}</a>)
              <span class="inc-sep">&#x2194;</span>
              <code>${esc(r.b.name)}</code> (<a data-file="${esc(r.b.file)}" data-line="${r.b.line}">${esc(r.b.file)}:${r.b.line}</a>)
              <div class="inc-shared">shared: <code>${esc(r.shared_value)}</code></div>
            </div>
          </div>
        `,
          )
          .join("");

  const orphanRows =
    diff.new_orphans.length === 0
      ? '<p class="empty">No new orphans introduced.</p>'
      : `<ul class="orphans">${diff.new_orphans
          .map((name) => `<li><code>${esc(name)}</code></li>`)
          .join("")}</ul>`;

  // Minimal CSS — works across light/dark themes via VSCode's CSS vars.
  // CSP: default-src 'none' means we MUST inline everything and not
  // reference external assets. Cytoscape is loaded via asWebviewUri so
  // it satisfies script-src ${cspSource}.
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy"
  content="default-src 'none'; style-src ${cspSource} 'unsafe-inline'; script-src ${cspSource} 'unsafe-inline';">
<title>CoreGraph Diff Impact</title>
<style>
  body {
    font-family: var(--vscode-font-family);
    color: var(--vscode-foreground);
    background: var(--vscode-editor-background);
    padding: 1rem 2rem;
    line-height: 1.5;
  }
  h1 { font-size: 1.2rem; margin-bottom: 0.5rem; }
  h2 { font-size: 1.0rem; margin-top: 1.5rem; margin-bottom: 0.3rem; }
  code {
    font-family: var(--vscode-editor-font-family);
    background: var(--vscode-textCodeBlock-background);
    padding: 0 0.3rem;
    border-radius: 3px;
  }
  .file-row, .inc-row { margin: 0.5rem 0; }
  .file-link, .sym, a[data-file] {
    color: var(--vscode-textLink-foreground);
    cursor: pointer;
    text-decoration: none;
  }
  .file-link:hover, .sym:hover, a[data-file]:hover {
    text-decoration: underline;
  }
  .impact {
    margin-left: 1em;
    color: var(--vscode-descriptionForeground);
  }
  .top { font-size: 0.9em; margin-top: 0.2em; padding-left: 1em; }
  .empty { color: var(--vscode-descriptionForeground); font-style: italic; }
  .warn { color: var(--vscode-editorWarning-foreground); }
  .note { color: var(--vscode-descriptionForeground); font-size: 0.9em; }
  .inc-category { font-weight: bold; }
  .inc-bodies { padding-left: 1em; }
  .inc-sep { margin: 0 0.5em; color: var(--vscode-descriptionForeground); }
  .inc-shared { font-size: 0.9em; color: var(--vscode-descriptionForeground); }
  /* Tab switcher */
  .tabs {
    display: flex;
    gap: 0;
    margin: 1em 0 0;
    border-bottom: 1px solid var(--vscode-panel-border);
  }
  .tab-btn {
    background: transparent;
    color: var(--vscode-foreground);
    border: none;
    padding: 0.5em 1em;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    font-family: var(--vscode-font-family);
    font-size: inherit;
  }
  .tab-btn.active {
    border-bottom-color: var(--vscode-textLink-foreground);
  }
  .tab-panel { display: none; }
  .tab-panel.active { display: block; }
  /* Cytoscape container */
  #cy {
    width: 100%;
    height: 500px;
    background: var(--vscode-editor-background);
    border: 1px solid var(--vscode-panel-border);
    border-radius: 4px;
    margin-top: 0.5em;
  }
  .graph-header {
    font-size: 0.85em;
    color: var(--vscode-descriptionForeground);
    margin-top: 1em;
    margin-bottom: 0.5em;
  }
</style>
</head>
<body>
  <h1>Diff Impact &mdash; ${esc(diff.base_ref)}..working</h1>
  ${summary}
  <div class="tabs">
    <button class="tab-btn active" data-tab="text">Text</button>
    <button class="tab-btn" data-tab="graph">Graph</button>
  </div>
  <div id="tab-text" class="tab-panel active">
    <section class="files">
      <h2>Changed files</h2>
      ${filesRows}
    </section>
    <section class="inc">
      <h2>Inconsistencies introduced</h2>
      ${incRows}
    </section>
    <section class="orphans-section">
      <h2>New orphans</h2>
      ${orphanRows}
    </section>
  </div>
  <div id="tab-graph" class="tab-panel">
    <div class="graph-header">
      Seeds (yellow) are symbols touched by the diff; affected neighbors shown with
      edges weighted by confidence. Click a node to jump to its file.
    </div>
    <div id="cy"></div>
  </div>
  <script src="${cytoscapeUri}"></script>
  <script>
    const vscode = acquireVsCodeApi();

    // Tab switching — lazy-init Cytoscape only on first Graph tab activation.
    const tabButtons = document.querySelectorAll(".tab-btn");
    tabButtons.forEach(function(btn) {
      btn.addEventListener("click", function() {
        tabButtons.forEach(function(b) { b.classList.remove("active"); });
        btn.classList.add("active");
        const target = btn.getAttribute("data-tab");
        document.querySelectorAll(".tab-panel").forEach(function(p) {
          p.classList.remove("active");
        });
        document.getElementById("tab-" + target).classList.add("active");
        if (target === "graph") { initCytoscape(); }
      });
    });

    // Jump-to-file click handler used by both tabs.
    document.body.addEventListener("click", function(ev) {
      const t = ev.target.closest("[data-file]");
      if (!t) { return; }
      vscode.postMessage({
        type: "openFile",
        file: t.getAttribute("data-file"),
        line: t.getAttribute("data-line") ? parseInt(t.getAttribute("data-line"), 10) : 0,
      });
    });

    // Cytoscape lazy init — executed at most once on first Graph tab click.
    let cyInitialized = false;
    const elements = ${elementsJson};

    function initCytoscape() {
      if (cyInitialized) { return; }
      cyInitialized = true;

      const cyContainer = document.getElementById("cy");
      if (!window.cytoscape) {
        cyContainer.innerText = "Cytoscape failed to load.";
        return;
      }

      // Resolve VSCode CSS custom properties to concrete hex values.
      // Cytoscape has its own style engine — it does NOT run through the
      // browser CSS cascade, so var(--vscode-*) strings would be silently
      // ignored. We read computed values here where they ARE available.
      const cs = getComputedStyle(document.documentElement);
      const linkColor = cs.getPropertyValue("--vscode-textLink-foreground").trim() || "#3794ff";
      const activeLinkColor = cs.getPropertyValue("--vscode-textLink-activeForeground").trim() || "#4daafc";
      const warnColor = cs.getPropertyValue("--vscode-editorWarning-foreground").trim() || "#cca700";
      const warnBorder = cs.getPropertyValue("--vscode-editorWarning-border").trim() || warnColor;
      const fgColor = cs.getPropertyValue("--vscode-foreground").trim() || "#cccccc";
      const descColor = cs.getPropertyValue("--vscode-descriptionForeground").trim() || "#999999";
      const editorBg = cs.getPropertyValue("--vscode-editor-background").trim() || "#1e1e1e";

      const cy = window.cytoscape({
        container: cyContainer,
        elements: elements,
        style: [
          {
            selector: "node",
            style: {
              "background-color": linkColor,
              "label": "data(label)",
              "color": fgColor,
              "font-size": "10px",
              "text-valign": "center",
              "text-halign": "right",
              "text-margin-x": "6px",
              "width": "mapData(weight, 0, 50, 10, 40)",
              "height": "mapData(weight, 0, 50, 10, 40)",
            },
          },
          {
            selector: "node.seed",
            style: {
              "background-color": warnColor,
              "border-width": 2,
              "border-color": warnBorder,
            },
          },
          {
            selector: "node.affected",
            style: {
              "background-color": activeLinkColor,
            },
          },
          {
            selector: "edge",
            style: {
              "width": "mapData(confidence, 0, 1, 1, 4)",
              "line-color": descColor,
              "curve-style": "bezier",
              "target-arrow-shape": "triangle",
              "target-arrow-color": descColor,
              "opacity": 0.7,
            },
          },
        ],
        layout: { name: "cose", animate: false, fit: true, padding: 30 },
      });

      // Click a node → open its file in the editor.
      cy.on("tap", "node", function(evt) {
        const data = evt.target.data();
        if (data.file) {
          vscode.postMessage({ type: "openFile", file: data.file, line: 0 });
        }
      });
    }
  </script>
</body>
</html>`;
}
