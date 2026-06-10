import type { DiffResult } from "../ipc/types";

/** A single Cytoscape element (node OR edge). The shape matches the
 * format Cytoscape.js's `cy.add(elements)` accepts. */
export interface CyElement {
  group: "nodes" | "edges";
  data: {
    id: string;
    label?: string;
    file?: string;
    weight?: number;
    source?: string;
    target?: string;
    confidence?: number;
  };
  classes?: string;
}

/** Convert a DiffResult into a Cytoscape elements array.
 *
 * Nodes: one per seed symbol (per changed file) + one per distinct
 * `top_affected` entry. Seed nodes get class `seed`; affected nodes
 * get class `affected`.
 *
 * Edges: seed → affected, one per (seed, affected) pair where the
 * affected symbol appears in that seed's file's `top_affected`.
 *
 * Node weight (used for layout sizing) = `confidence_weighted` of
 * the enclosing DiffFileImpact for seeds; confidence for affected. */
export function diffResultToElements(diff: DiffResult): CyElement[] {
  const out: CyElement[] = [];
  const seen = new Set<string>();

  for (const f of diff.changed_files) {
    // Emit one seed node per symbol declared as touched in this file.
    for (const name of f.seed_symbols) {
      const id = seedId(f.file, name);
      if (seen.has(id)) continue;
      seen.add(id);
      out.push({
        group: "nodes",
        data: {
          id,
          label: name,
          file: f.file,
          weight: f.confidence_weighted,
        },
        classes: "seed",
      });
    }

    // Emit affected nodes and edges from each seed in this file.
    for (const a of f.top_affected) {
      const aid = affectedId(a.file, a.name);
      if (!seen.has(aid)) {
        seen.add(aid);
        out.push({
          group: "nodes",
          data: {
            id: aid,
            label: a.name,
            file: a.file,
            weight: a.confidence,
          },
          classes: "affected",
        });
      }
      // One edge per (seed, affected) pair originating from this file.
      for (const seedName of f.seed_symbols) {
        const sid = seedId(f.file, seedName);
        const edgeId = `${sid}->${aid}`;
        out.push({
          group: "edges",
          data: {
            id: edgeId,
            source: sid,
            target: aid,
            confidence: a.confidence,
          },
        });
      }
    }
  }

  return out;
}

function seedId(file: string, name: string): string {
  return `seed::${file}::${name}`;
}

function affectedId(file: string, name: string): string {
  return `aff::${file}::${name}`;
}

/** Compute aggregate counts (total nodes/edges plus seed/affected splits)
 * from a Cytoscape elements array. Not currently wired into the rendered
 * graph tab header, which shows fixed explanatory text; only consumed by
 * unit tests today. */
export function graphSummary(elements: CyElement[]): {
  nodes: number;
  edges: number;
  seeds: number;
  affected: number;
} {
  let nodes = 0;
  let edges = 0;
  let seeds = 0;
  let affected = 0;
  for (const e of elements) {
    if (e.group === "nodes") {
      nodes += 1;
      if (e.classes === "seed") seeds += 1;
      else if (e.classes === "affected") affected += 1;
    } else {
      edges += 1;
    }
  }
  return { nodes, edges, seeds, affected };
}
