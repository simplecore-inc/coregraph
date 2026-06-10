import { useMemo, useState } from 'react'
import type { IncidentEdge } from '../lib/graph.ts'
import type { SourceSnippet } from '../lib/api.ts'
import type { NodeDatum } from '../lib/types.ts'
import { edgeKindColor, nodeKindColor } from '../lib/palette.ts'

interface DetailPanelProps {
  node: NodeDatum
  incident: IncidentEdge[]
  focusMode: boolean
  /**
   * Name of the panel this one temporarily replaced (e.g. an active
   * analysis). When set, the close affordance becomes a back button so the
   * return path is explicit.
   */
  backTo?: string
  /** Code preview (bridge mode only): null = unavailable, undefined = loading. */
  source?: SourceSnippet | null
  /** Run the impact analysis for this symbol (bridge mode only). */
  onImpact?: () => void
  /** Start path mode with this symbol as the origin. */
  onPathFrom?: () => void
  onJump: (id: number) => void
  onIsolate: () => void
  onExitIsolate: () => void
  onClear: () => void
}

interface EdgeGroup {
  key: string
  kind: string
  dir: 'out' | 'in'
  items: IncidentEdge[]
}

const GROUP_PREVIEW = 24

function groupIncident(incident: IncidentEdge[]): EdgeGroup[] {
  const byKey = new Map<string, EdgeGroup>()
  for (const item of incident) {
    const key = `${item.edge.kind}/${item.dir}`
    let group = byKey.get(key)
    if (group === undefined) {
      group = { key, kind: item.edge.kind, dir: item.dir, items: [] }
      byKey.set(key, group)
    }
    group.items.push(item)
  }
  const groups = [...byKey.values()]
  for (const group of groups) {
    group.items.sort((a, b) => b.edge.confidence - a.edge.confidence)
  }
  groups.sort((a, b) => b.items.length - a.items.length || a.key.localeCompare(b.key))
  return groups
}

function copyText(text: string): void {
  // Clipboard API requires a secure context; file:// counts as one in
  // practice, but fall back silently to a no-op selection trick if not.
  if (navigator.clipboard !== undefined) {
    void navigator.clipboard.writeText(text)
  }
}

export function DetailPanel({
  node,
  incident,
  focusMode,
  backTo,
  source,
  onImpact,
  onPathFrom,
  onJump,
  onIsolate,
  onExitIsolate,
  onClear,
}: DetailPanelProps) {
  const groups = useMemo(() => groupIncident(incident), [incident])
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set())

  return (
    <aside className="panel detail-panel" aria-label="Symbol details">
      {backTo !== undefined ? (
        <button
          type="button"
          className="back-btn"
          title={`Back to the ${backTo} results`}
          onClick={onClear}
        >
          ← back to {backTo}
        </button>
      ) : null}
      <header className="detail-head">
        <i className="dot big" style={{ background: nodeKindColor(node.kind) }} />
        <div className="detail-title">
          <h2 title={node.name}>{node.name}</h2>
          <p>
            <span className="chip">{node.kind}</span>
            <span className="degrees">
              in {node.degIn} · out {node.degOut}
            </span>
          </p>
        </div>
        {backTo === undefined ? (
          <button
            type="button"
            className="icon-btn"
            aria-label="Clear selection"
            onClick={onClear}
          >
            ✕
          </button>
        ) : null}
      </header>

      <button
        type="button"
        className="detail-path"
        title="Copy absolute path"
        onClick={() => copyText(node.file)}
      >
        {node.rel}
        <span className="path-span">
          bytes {node.spanStart}–{node.spanEnd}
        </span>
      </button>

      <div className="detail-actions">
        {focusMode ? (
          <button type="button" className="action" onClick={onExitIsolate}>
            ◀ full graph
          </button>
        ) : (
          <button type="button" className="action primary" onClick={onIsolate}>
            ◎ isolate neighborhood
          </button>
        )}
        {onImpact !== undefined || onPathFrom !== undefined ? (
          <div className="action-row">
            {onImpact !== undefined ? (
              <button type="button" className="action" onClick={onImpact}>
                ⊚ impact
              </button>
            ) : null}
            {onPathFrom !== undefined ? (
              <button
                type="button"
                className="action"
                title="Pick another symbol (click or search) to trace the shortest path"
                onClick={onPathFrom}
              >
                ⇢ path from here
              </button>
            ) : null}
          </div>
        ) : null}
      </div>

      {source !== null && source !== undefined ? (
        <details className="source-preview" open>
          <summary>source · L{source.startLine + 1}</summary>
          <pre>
            {source.lines.map((line, index) => {
              const lineNo = source.windowStart + index
              const inSpan = lineNo >= source.startLine && lineNo <= source.endLine
              return (
                <span key={lineNo} className={inSpan ? 'src-line hit' : 'src-line'}>
                  <i>{String(lineNo + 1).padStart(4, ' ')}</i> {line}
                  {'\n'}
                </span>
              )
            })}
          </pre>
        </details>
      ) : null}

      <div className="detail-groups">
        {groups.length === 0 ? (
          <p className="detail-empty">no edges under the current filters</p>
        ) : (
          groups.map((group) => {
            const isOpen = expanded.has(group.key)
            const shown = isOpen ? group.items : group.items.slice(0, GROUP_PREVIEW)
            return (
              <section key={group.key} className="edge-group">
                <h3>
                  <i className="dot" style={{ background: edgeKindColor(group.kind) }} />
                  {group.dir === 'out' ? `${group.kind} →` : `← ${group.kind}`}
                  <span className="group-count">{group.items.length}</span>
                </h3>
                <ul>
                  {shown.map((item, index) => (
                    <li key={`${item.other.id}-${index}`}>
                      <button type="button" onClick={() => onJump(item.other.id)}>
                        <i
                          className="dot"
                          style={{ background: nodeKindColor(item.other.kind) }}
                        />
                        <span className="edge-name">{item.other.name}</span>
                        <span
                          className="conf"
                          title={`confidence ${item.edge.confidence.toFixed(2)} · ${item.edge.origin}`}
                        >
                          {item.edge.confidence.toFixed(2)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
                {group.items.length > GROUP_PREVIEW && !isOpen ? (
                  <button
                    type="button"
                    className="more"
                    onClick={() =>
                      setExpanded((prev) => {
                        const next = new Set(prev)
                        next.add(group.key)
                        return next
                      })
                    }
                  >
                    +{group.items.length - GROUP_PREVIEW} more
                  </button>
                ) : null}
              </section>
            )
          })
        )}
      </div>
    </aside>
  )
}
