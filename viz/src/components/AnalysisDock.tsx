import { useState } from 'react'

export interface DaemonAnalyses {
  onImpact: () => void
  impactReady: boolean
  onOrphans: () => void
  onInconsistencies: (category: string) => void
  onDiff: (baseRef: string) => void
}

interface AnalysisDockProps {
  loading: string | null
  daemon: DaemonAnalyses
  orphanOptions: { excludeTests: boolean; publicOnly: boolean }
  onOrphanOptions: (options: { excludeTests: boolean; publicOnly: boolean }) => void
}

const INCONSISTENCY_CATEGORIES = ['', 'enum-mismatch', 'api-path', 'config-key', 'doc-drift']

/** Bottom rail card: analysis COMMANDS, kept apart from the view filters. */
export function AnalysisDock({ loading, daemon, orphanOptions, onOrphanOptions }: AnalysisDockProps) {
  const [category, setCategory] = useState('')
  const [baseRef, setBaseRef] = useState('HEAD')
  const busy = (key: string): string => (loading === key ? '…' : '')

  return (
    <section className="panel rail-card analysis-dock">
      <h2 className="dock-title">analysis</h2>
      <button
        type="button"
        className="load-btn wide"
        disabled={!daemon.impactReady || loading !== null}
        title={
          daemon.impactReady
            ? 'Blast radius + risk for the selected symbol'
            : 'Select a symbol first'
        }
        onClick={daemon.onImpact}
      >
        ⊚ impact of selected {busy('impact')}
      </button>

      <button
        type="button"
        className="load-btn wide"
        disabled={loading !== null}
        onClick={daemon.onOrphans}
      >
        ☠ dead code {busy('orphans')}
      </button>
      <div className="inline-opts">
        <label className="check">
          <input
            type="checkbox"
            checked={orphanOptions.excludeTests}
            onChange={(event) =>
              onOrphanOptions({ ...orphanOptions, excludeTests: event.target.checked })
            }
          />
          <span>excl. tests</span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={orphanOptions.publicOnly}
            onChange={(event) =>
              onOrphanOptions({ ...orphanOptions, publicOnly: event.target.checked })
            }
          />
          <span>public only</span>
        </label>
      </div>

      <div className="combo-row">
        <select
          value={category}
          aria-label="Inconsistency category"
          onChange={(event) => setCategory(event.target.value)}
        >
          {INCONSISTENCY_CATEGORIES.map((value) => (
            <option key={value} value={value}>
              {value === '' ? 'all categories' : value}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="load-btn"
          disabled={loading !== null}
          onClick={() => daemon.onInconsistencies(category)}
        >
          ≠ check {busy('inconsistencies')}
        </button>
      </div>

      <div className="combo-row">
        <input
          type="text"
          value={baseRef}
          spellCheck={false}
          aria-label="Diff base ref"
          onChange={(event) => setBaseRef(event.target.value)}
        />
        <button
          type="button"
          className="load-btn"
          disabled={loading !== null}
          onClick={() => daemon.onDiff(baseRef.trim() === '' ? 'HEAD' : baseRef.trim())}
        >
          Δ diff {busy('diff')}
        </button>
      </div>
    </section>
  )
}
