import type { ClusterBy, ColorMode } from './GraphCanvas.tsx'
import { dirColor, edgeKindColor, nodeKindColor, ORIGIN_ORDER } from '../lib/palette.ts'

export interface KindRow {
  kind: string
  count: number
}

interface KindSectionProps {
  title: string
  rows: KindRow[]
  hidden: ReadonlySet<string>
  colorOf: (kind: string) => string
  onToggle: (kind: string) => void
  onOnly: (kind: string) => void
  onAll: () => void
  onReset: () => void
}

function KindSection({
  title,
  rows,
  hidden,
  colorOf,
  onToggle,
  onOnly,
  onAll,
  onReset,
}: KindSectionProps) {
  return (
    <details className="section" open>
      <summary>
        <span>{title}</span>
        <span className="section-actions">
          <button type="button" onClick={(e) => (e.preventDefault(), onAll())}>
            all
          </button>
          <button type="button" onClick={(e) => (e.preventDefault(), onReset())}>
            reset
          </button>
        </span>
      </summary>
      <ul className="kind-list">
        {rows.map((row) => {
          const off = hidden.has(row.kind)
          return (
            <li key={row.kind} className={off ? 'kind off' : 'kind'}>
              <label>
                <input type="checkbox" checked={!off} onChange={() => onToggle(row.kind)} />
                <i className="dot" style={{ background: colorOf(row.kind) }} />
                <span className="kind-name">{row.kind}</span>
                <span className="kind-count">{row.count.toLocaleString()}</span>
              </label>
              <button type="button" className="only" onClick={() => onOnly(row.kind)}>
                only
              </button>
            </li>
          )
        })}
      </ul>
    </details>
  )
}

interface FilterPanelProps {
  nodeKindRows: KindRow[]
  edgeKindRows: KindRow[]
  originRows: KindRow[]
  hiddenNodeKinds: ReadonlySet<string>
  hiddenEdgeKinds: ReadonlySet<string>
  hiddenOrigins: ReadonlySet<string>
  minConfidence: number
  focusDepth: number
  colorMode: ColorMode
  hideIsolated: boolean
  dirPalette: ReadonlyMap<string, string>
  onToggleNodeKind: (kind: string) => void
  onToggleEdgeKind: (kind: string) => void
  onToggleOrigin: (origin: string) => void
  onOnlyNodeKind: (kind: string) => void
  onOnlyEdgeKind: (kind: string) => void
  onAllNodeKinds: () => void
  onAllEdgeKinds: () => void
  onResetNodeKinds: () => void
  onResetEdgeKinds: () => void
  onMinConfidence: (value: number) => void
  onFocusDepth: (value: number) => void
  onColorMode: (mode: ColorMode) => void
  onHideIsolated: (value: boolean) => void
  // ── analysis & view extensions ──
  clusterBy: ClusterBy
  onClusterBy: (value: ClusterBy) => void
  minDegree: number
  maxDegree: number
  onMinDegree: (value: number) => void
  hideHubs: boolean
  onHideHubs: (value: boolean) => void
  /** Active directory restriction (restored from share links). */
  dirFilter: string | null
  onClearDirFilter: () => void
}

export function FilterPanel(props: FilterPanelProps) {
  const originOrder = new Map(ORIGIN_ORDER.map((origin, index) => [origin, index]))
  const sortedOrigins = [...props.originRows].sort(
    (a, b) => (originOrder.get(a.kind) ?? 99) - (originOrder.get(b.kind) ?? 99),
  )

  return (
    <aside className="panel filter-panel">
      <details className="section" open>
        <summary>
          <span>view</span>
        </summary>
        <div className="view-controls">
          <div className="seg" role="radiogroup" aria-label="Color mode">
            <button
              type="button"
              className={props.colorMode === 'kind' ? 'on' : ''}
              onClick={() => props.onColorMode('kind')}
            >
              by kind
            </button>
            <button
              type="button"
              className={props.colorMode === 'dir' ? 'on' : ''}
              onClick={() => props.onColorMode('dir')}
            >
              by dir
            </button>
            <button
              type="button"
              className={props.colorMode === 'unit' ? 'on' : ''}
              onClick={() => props.onColorMode('unit')}
            >
              by unit
            </button>
          </div>

          <label className="check">
            <input
              type="checkbox"
              checked={props.clusterBy === 'unit'}
              onChange={(event) => props.onClusterBy(event.target.checked ? 'unit' : 'none')}
            />
            <span title="Module/package/crate-level units derived from path structure">
              cluster by unit
            </span>
          </label>

          <label className="slider">
            <span>
              min confidence <b>{props.minConfidence.toFixed(2)}</b>
            </span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={props.minConfidence}
              onChange={(event) => props.onMinConfidence(Number(event.target.value))}
            />
          </label>

          <label className="slider">
            <span>
              isolate depth <b>{props.focusDepth}</b>
            </span>
            <input
              type="range"
              min={1}
              max={3}
              step={1}
              value={props.focusDepth}
              onChange={(event) => props.onFocusDepth(Number(event.target.value))}
            />
          </label>

          <label className="check">
            <input
              type="checkbox"
              checked={props.hideIsolated}
              onChange={(event) => props.onHideIsolated(event.target.checked)}
            />
            <span>hide unconnected nodes</span>
          </label>

          <label className="slider">
            <span>
              min degree <b>{props.minDegree}</b>
            </span>
            <input
              type="range"
              min={0}
              max={props.maxDegree}
              step={1}
              value={props.minDegree}
              onChange={(event) => props.onMinDegree(Number(event.target.value))}
            />
          </label>

          <label className="check">
            <input
              type="checkbox"
              checked={props.hideHubs}
              onChange={(event) => props.onHideHubs(event.target.checked)}
            />
            <span>hide top hubs (p99 degree)</span>
          </label>

          {props.dirFilter !== null ? (
            <button type="button" className="load-btn" onClick={props.onClearDirFilter}>
              dir: {props.dirFilter} ✕
            </button>
          ) : null}

          {props.colorMode === 'dir' ? (
            <ul className="dir-legend">
              {[...props.dirPalette.entries()].map(([dir, color], index) => (
                <li key={dir}>
                  <i className="dot" style={{ background: color || dirColor(index) }} />
                  <span>{dir}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </details>

      <KindSection
        title="symbol kinds"
        rows={props.nodeKindRows}
        hidden={props.hiddenNodeKinds}
        colorOf={nodeKindColor}
        onToggle={props.onToggleNodeKind}
        onOnly={props.onOnlyNodeKind}
        onAll={props.onAllNodeKinds}
        onReset={props.onResetNodeKinds}
      />

      <KindSection
        title="edge kinds"
        rows={props.edgeKindRows}
        hidden={props.hiddenEdgeKinds}
        colorOf={edgeKindColor}
        onToggle={props.onToggleEdgeKind}
        onOnly={props.onOnlyEdgeKind}
        onAll={props.onAllEdgeKinds}
        onReset={props.onResetEdgeKinds}
      />

      <details className="section" open>
        <summary>
          <span>analysis origin</span>
        </summary>
        <ul className="kind-list">
          {sortedOrigins.map((row) => {
            const off = props.hiddenOrigins.has(row.kind)
            return (
              <li key={row.kind} className={off ? 'kind off' : 'kind'}>
                <label>
                  <input
                    type="checkbox"
                    checked={!off}
                    onChange={() => props.onToggleOrigin(row.kind)}
                  />
                  <span className="kind-name">{row.kind}</span>
                  <span className="kind-count">{row.count.toLocaleString()}</span>
                </label>
              </li>
            )
          })}
        </ul>
      </details>
    </aside>
  )
}
