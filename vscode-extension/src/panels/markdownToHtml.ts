/** Convert a focused subset of Markdown (what `buildReviewMarkdown`
 * emits) to HTML.
 *
 * Intentionally narrow — we don't support setext headings, nested
 * lists, images, links-with-titles, HTML blocks, etc. Input comes
 * from our own formatter, so feature scope tracks that formatter.
 *
 * All user-supplied content (symbol names, file paths) is escaped
 * before being wrapped in tags. The only raw-HTML sink is the
 * `<sub>` footer tag which we recognize explicitly. */
export function markdownToHtml(md: string): string {
  const lines = md.split(/\r?\n/);
  const out: string[] = [];
  let i = 0;

  const esc = (s: string): string =>
    s.replace(/&/g, "&amp;")
     .replace(/</g, "&lt;")
     .replace(/>/g, "&gt;");

  /** Inline-format a single line: bold, italic, code. Escape first. */
  const inline = (s: string): string => {
    // Preserve known raw tags by swap-to-placeholder, then re-insert at end.
    const subTag = /(<\/?sub>)/g;
    const placeholders: string[] = [];
    const placeheld = s.replace(subTag, (m) => {
      placeholders.push(m);
      return `\u0001${placeholders.length - 1}\u0001`;
    });
    let html = esc(placeheld);
    html = html
      .replace(/`([^`]+)`/g, (_, g: string) => `<code>${g}</code>`)
      .replace(/\*\*([^*]+)\*\*/g, (_, g: string) => `<strong>${g}</strong>`)
      .replace(/_([^_]+)_/g, (_, g: string) => `<em>${g}</em>`);
    html = html.replace(/\u0001(\d+)\u0001/g, (_, idx: string) => placeholders[Number(idx)]);
    return html;
  };

  while (i < lines.length) {
    const line = lines[i];

    // Blank line — paragraph break.
    if (line.trim() === "") {
      i += 1;
      continue;
    }

    // Heading.
    const h = /^(#{2,6})\s+(.*)$/.exec(line);
    if (h) {
      const level = h[1].length;
      out.push(`<h${level}>${inline(h[2])}</h${level}>`);
      i += 1;
      continue;
    }

    // Blockquote (single line).
    if (line.startsWith("> ")) {
      out.push(`<blockquote>${inline(line.slice(2))}</blockquote>`);
      i += 1;
      continue;
    }

    // List block.
    if (line.startsWith("- ")) {
      const items: string[] = [];
      while (i < lines.length && lines[i].startsWith("- ")) {
        items.push(`<li>${inline(lines[i].slice(2))}</li>`);
        i += 1;
      }
      out.push(`<ul>${items.join("")}</ul>`);
      continue;
    }

    // Table. Need a header row followed by a separator row like |---|---|.
    if (
      line.startsWith("|") &&
      i + 1 < lines.length &&
      /^\|[\s:|-]+\|$/.test(lines[i + 1])
    ) {
      const header = parseRow(line).map((c) => `<th>${inline(c)}</th>`).join("");
      i += 2; // skip separator row
      const bodyRows: string[] = [];
      while (i < lines.length && lines[i].startsWith("|")) {
        const cells = parseRow(lines[i]).map((c) => `<td>${inline(c)}</td>`).join("");
        bodyRows.push(`<tr>${cells}</tr>`);
        i += 1;
      }
      out.push(
        `<table><thead><tr>${header}</tr></thead><tbody>${bodyRows.join("")}</tbody></table>`,
      );
      continue;
    }

    // Plain paragraph (collect until next block boundary).
    const paraLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("#") &&
      !lines[i].startsWith("> ") &&
      !lines[i].startsWith("- ") &&
      !lines[i].startsWith("|")
    ) {
      paraLines.push(lines[i]);
      i += 1;
    }
    if (paraLines.length > 0) {
      out.push(`<p>${inline(paraLines.join(" "))}</p>`);
    }
  }

  return out.join("\n");
}

function parseRow(line: string): string[] {
  // "| a | b |" -> ["a", "b"]
  const inner = line.replace(/^\|/, "").replace(/\|$/, "");
  return inner.split("|").map((c) => c.trim());
}
