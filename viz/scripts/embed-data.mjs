#!/usr/bin/env node
// Reads a `coregraph export --format json-graph` document from stdin (or a
// file path argument), drops edge fields the viewer does not use, and writes
// a compact copy to src/data/graph.json for build-time embedding.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const target = join(here, '..', 'src', 'data', 'graph.json')

function fail(message) {
  console.error(`embed-data: ${message}`)
  process.exit(1)
}

const source = process.argv[2] ?? 0 // 0 = stdin
let text
try {
  text = readFileSync(source, 'utf8')
} catch (error) {
  fail(error instanceof Error ? error.message : String(error))
}

let raw
try {
  raw = JSON.parse(text)
} catch (error) {
  fail(`input is not valid JSON: ${error instanceof Error ? error.message : String(error)}`)
}

if (raw === null || typeof raw !== 'object' || !Array.isArray(raw.nodes) || !Array.isArray(raw.edges)) {
  fail('input is not a json-graph export (expected top-level "nodes" and "edges" arrays)')
}

const slim = {
  nodes: raw.nodes,
  edges: raw.edges.map((edge) => ({
    from: edge.from,
    to: edge.to,
    kind: edge.kind,
    origin: edge.origin ?? edge.trust ?? 'Unknown',
    confidence: edge.confidence ?? 1,
  })),
}

mkdirSync(dirname(target), { recursive: true })
const json = JSON.stringify(slim)
writeFileSync(target, json)
console.log(
  `embed-data: wrote ${target} — ${slim.nodes.length} nodes, ${slim.edges.length} edges, ${(json.length / 1024 / 1024).toFixed(1)} MB`,
)
