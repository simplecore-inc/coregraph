import type {
  DiffResult,
  ImpactResult,
  InconsistenciesResult,
  OrphansResult,
} from '../lib/api.ts'
import type { NodeDatum } from '../lib/types.ts'

/** Active analysis overlay state, owned by App. */
export type Analysis =
  | { kind: 'impact'; data: ImpactResult }
  | { kind: 'orphans'; data: OrphansResult; excludeTests: boolean; publicOnly: boolean }
  | { kind: 'inconsistencies'; data: InconsistenciesResult; category: string }
  | { kind: 'diff'; data: DiffResult }
  | { kind: 'path'; from: NodeDatum; to: NodeDatum; nodes: NodeDatum[] }

interface AnalysisPanelProps {
  analysis: Analysis
  /** 'dialog' renders the same content inside a modal card. */
  variant?: 'panel' | 'dialog'
  /** Resolve a (name, file) pair to a node id, if it exists in the dataset. */
  resolveId: (name: string, file?: string) => number | undefined
  onJump: (id: number) => void
  onClose: () => void
}

function riskTone(level: string): string {
  const normalized = level.toLowerCase()
  if (normalized.includes('critical')) return 'risk-critical'
  if (normalized.includes('high')) return 'risk-high'
  return 'risk-normal'
}

function JumpRow({
  id,
  primary,
  secondary,
  onJump,
}: {
  id: number | undefined
  primary: string
  secondary: string
  onJump: (id: number) => void
}) {
  return (
    <li>
      <button
        type="button"
        className="analysis-row"
        disabled={id === undefined}
        title={id === undefined ? 'not present in the loaded graph' : 'fly to symbol'}
        onClick={() => {
          if (id !== undefined) onJump(id)
        }}
      >
        <span className="analysis-row-name">{primary}</span>
        <span className="analysis-row-meta">{secondary}</span>
      </button>
    </li>
  )
}

function shortPath(file: string): string {
  const parts = file.split('/')
  return parts.length <= 3 ? file : `…/${parts.slice(-3).join('/')}`
}

export function AnalysisPanel({
  analysis,
  variant = 'panel',
  resolveId,
  onJump,
  onClose,
}: AnalysisPanelProps) {
  return (
    <aside
      className={
        variant === 'dialog'
          ? 'detail-panel analysis-panel as-dialog'
          : 'panel detail-panel analysis-panel'
      }
      aria-label="Analysis results"
    >
      <header className="detail-head">
        <div className="detail-title">
          <h2>
            {analysis.kind === 'impact' && `impact · ${analysis.data.symbol}`}
            {analysis.kind === 'orphans' && 'dead code'}
            {analysis.kind === 'inconsistencies' && 'inconsistencies'}
            {analysis.kind === 'diff' && `diff · ${analysis.data.base_ref}`}
            {analysis.kind === 'path' && 'path'}
          </h2>
        </div>
        <button type="button" className="icon-btn" aria-label="Close analysis" onClick={onClose}>
          ✕
        </button>
      </header>

      <div className="detail-groups">
        {analysis.kind === 'impact' ? (
          <>
            <div className="metric-grid">
              <div className="metric">
                <b>{analysis.data.reachable.toLocaleString()}</b>
                <span>reachable</span>
              </div>
              <div className="metric">
                <b>{analysis.data.risk?.caller_count.toLocaleString() ?? '–'}</b>
                <span>callers</span>
              </div>
              <div className="metric">
                <b>{analysis.data.risk?.module_count ?? '–'}</b>
                <span>modules</span>
              </div>
              <div className="metric">
                <b>{analysis.data.risk?.affected_tests.length ?? '–'}</b>
                <span>tests hit</span>
              </div>
            </div>
            {analysis.data.risk !== undefined ? (
              <section className="edge-group">
                <h3>
                  risk
                  <span className={`group-count ${riskTone(analysis.data.risk.level)}`}>
                    {analysis.data.risk.score.toFixed(2)} · {analysis.data.risk.level}
                  </span>
                </h3>
                <div className="risk-bar" aria-hidden="true">
                  <i
                    className={riskTone(analysis.data.risk.level)}
                    style={{ width: `${Math.min(100, analysis.data.risk.score * 100)}%` }}
                  />
                </div>
                <h3 style={{ marginTop: '12px' }}>
                  affected tests
                  <span className="group-count">{analysis.data.risk.affected_tests.length}</span>
                </h3>
                <ul>
                  {analysis.data.risk.affected_tests.slice(0, 40).map((test, index) => (
                    <JumpRow
                      key={`${test.name}-${index}`}
                      id={test.id ?? resolveId(test.name, test.file)}
                      primary={test.name}
                      secondary={`±${test.distance}`}
                      onJump={onJump}
                    />
                  ))}
                </ul>
              </section>
            ) : null}
          </>
        ) : null}

        {analysis.kind === 'orphans' ? (
          <>
            <div className="metric-grid">
              <div className="metric">
                <b>{analysis.data.likely_dead}</b>
                <span>likely dead</span>
              </div>
              <div className="metric">
                <b>{analysis.data.library_api_surface}</b>
                <span>library api</span>
              </div>
              <div className="metric">
                <b>{analysis.data.test_code}</b>
                <span>test code</span>
              </div>
            </div>
            <section className="edge-group">
              <h3>
                symbols<span className="group-count">{analysis.data.orphans.length}</span>
              </h3>
              <ul>
                {analysis.data.orphans.slice(0, 120).map((orphan, index) => (
                  <JumpRow
                    key={`${orphan.name}-${index}`}
                    id={resolveId(orphan.name, orphan.file)}
                    primary={orphan.name}
                    secondary={
                      orphan.external_api ? 'api' : orphan.is_test ? 'test' : 'dead'
                    }
                    onJump={onJump}
                  />
                ))}
              </ul>
            </section>
          </>
        ) : null}

        {analysis.kind === 'inconsistencies' ? (
          <section className="edge-group">
            <h3>
              findings<span className="group-count">{analysis.data.count}</span>
            </h3>
            {analysis.data.reports.slice(0, 80).map((report, index) => (
              <div key={index} className="pair-card">
                <p className="pair-head">
                  <span className="chip">{report.category}</span>
                  {report.shared_value !== undefined ? (
                    <span className="pair-value">{report.shared_value}</span>
                  ) : null}
                </p>
                {report.a !== undefined && report.b !== undefined ? (
                  <ul>
                    <JumpRow
                      id={resolveId(report.a.name, report.a.file)}
                      primary={report.a.name}
                      secondary={shortPath(report.a.file)}
                      onJump={onJump}
                    />
                    <JumpRow
                      id={resolveId(report.b.name, report.b.file)}
                      primary={report.b.name}
                      secondary={shortPath(report.b.file)}
                      onJump={onJump}
                    />
                  </ul>
                ) : (
                  <p className="pair-detail">
                    {report.symbol} — {report.detail}
                  </p>
                )}
              </div>
            ))}
          </section>
        ) : null}

        {analysis.kind === 'diff' ? (
          <>
            <div className="metric-grid">
              <div className="metric">
                <b>{analysis.data.changed_files.length}</b>
                <span>files</span>
              </div>
              <div className="metric">
                <b>{analysis.data.total_reachable.toLocaleString()}</b>
                <span>reachable</span>
              </div>
              <div className="metric">
                <b>{analysis.data.new_orphans.length}</b>
                <span>new orphans</span>
              </div>
              <div className="metric">
                <b>{analysis.data.inconsistencies_introduced.length}</b>
                <span>new inconsist.</span>
              </div>
            </div>
            {analysis.data.note !== undefined ? (
              <p className="detail-empty">{analysis.data.note}</p>
            ) : null}
            {analysis.data.changed_files.map((file) => (
              <section key={file.file} className="edge-group">
                <h3 title={file.file}>
                  {shortPath(file.file)}
                  <span className="group-count">{file.reachable_count}</span>
                </h3>
                <ul>
                  {file.seed_symbols.slice(0, 20).map((seed, index) => (
                    <JumpRow
                      key={`s-${seed}-${index}`}
                      id={resolveId(seed, file.file)}
                      primary={seed}
                      secondary="seed"
                      onJump={onJump}
                    />
                  ))}
                  {file.top_affected.slice(0, 12).map((affected, index) => (
                    <JumpRow
                      key={`a-${affected.name}-${index}`}
                      id={resolveId(affected.name, affected.file)}
                      primary={affected.name}
                      secondary={affected.confidence.toFixed(2)}
                      onJump={onJump}
                    />
                  ))}
                </ul>
              </section>
            ))}
            {analysis.data.new_orphans.length > 0 ? (
              <section className="edge-group">
                <h3>
                  new orphans
                  <span className="group-count">{analysis.data.new_orphans.length}</span>
                </h3>
                <ul>
                  {analysis.data.new_orphans.slice(0, 30).map((name, index) => (
                    <JumpRow
                      key={`${name}-${index}`}
                      id={resolveId(name)}
                      primary={name}
                      secondary="orphan"
                      onJump={onJump}
                    />
                  ))}
                </ul>
              </section>
            ) : null}
          </>
        ) : null}

        {analysis.kind === 'path' ? (
          <section className="edge-group">
            <h3>
              {analysis.from.name} → {analysis.to.name}
              <span className="group-count">{analysis.nodes.length - 1} hops</span>
            </h3>
            <ul>
              {analysis.nodes.map((node, index) => (
                <JumpRow
                  key={node.id}
                  id={node.id}
                  primary={`${index}. ${node.name}`}
                  secondary={node.kind}
                  onJump={onJump}
                />
              ))}
            </ul>
          </section>
        ) : null}
      </div>
    </aside>
  )
}
