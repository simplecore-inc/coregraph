import { parseDataset } from '../lib/parse.ts'
import type { ParsedGraph } from '../lib/types.ts'

// Optional build-time dataset: `npm run data` writes src/data/graph.json and the
// glob inlines it; without the file the viewer starts on the drop-zone screen.
const modules = import.meta.glob<{ default: unknown }>('./graph.json', { eager: true })

export function loadEmbedded(): ParsedGraph | null {
  const module_ = modules['./graph.json']
  if (module_ === undefined) return null
  try {
    return parseDataset(module_.default)
  } catch (error) {
    // An invalid embedded payload is a build-time mistake; surface it in the
    // console and fall back to the manual-load screen instead of crashing.
    console.error('embedded graph.json is not a valid json-graph export:', error)
    return null
  }
}
