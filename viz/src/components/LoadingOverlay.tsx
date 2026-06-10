import { useEffect, useState } from 'react'

export type StageState = 'pending' | 'active' | 'done' | 'error'

export interface Stage {
  key: string
  label: string
  state: StageState
}

interface LoadingOverlayProps {
  project: string
  stages: Stage[]
  startedAt: number
  error: string | null
  onBack: () => void
}

/** Rotating sub-captions shown while the long analyze stage runs. */
const ANALYZE_CAPTIONS = [
  'scanning source files…',
  'extracting symbols with tree-sitter…',
  'resolving names across files (stack-graphs)…',
  'scoring edge confidence…',
  'assembling the constellation…',
]

function StageIcon({ state }: { state: StageState }) {
  if (state === 'done') return <i className="stage-icon done">✔</i>
  if (state === 'error') return <i className="stage-icon error">✖</i>
  if (state === 'active') return <i className="stage-icon active" aria-hidden="true" />
  return <i className="stage-icon pending">◌</i>
}

export function LoadingOverlay({ project, stages, startedAt, error, onBack }: LoadingOverlayProps) {
  const [now, setNow] = useState(() => Date.now())
  const [captionIndex, setCaptionIndex] = useState(0)

  useEffect(() => {
    const timer = window.setInterval(() => {
      setNow(Date.now())
      setCaptionIndex((index) => index + 1)
    }, 1800)
    return () => window.clearInterval(timer)
  }, [])

  const elapsed = Math.max(0, Math.floor((now - startedAt) / 1000))
  const analyzing = stages.some((stage) => stage.key === 'analyze' && stage.state === 'active')

  return (
    <div className="loading-overlay">
      <div className="loading-card">
        <div className="orbit" aria-hidden="true">
          <span className="orbit-ring r1" />
          <span className="orbit-ring r2" />
          <span className="orbit-dot d1" />
          <span className="orbit-dot d2" />
          <span className="orbit-dot d3" />
          <span className="orbit-core" />
        </div>

        <p className="loading-project" title={project}>
          {project}
        </p>

        <ul className="stage-list">
          {stages.map((stage) => (
            <li key={stage.key} className={`stage ${stage.state}`}>
              <StageIcon state={stage.state} />
              <span>{stage.label}</span>
            </li>
          ))}
        </ul>

        {error !== null ? (
          <p className="loading-error">✖ {error}</p>
        ) : analyzing ? (
          <p className="loading-caption">
            {ANALYZE_CAPTIONS[captionIndex % ANALYZE_CAPTIONS.length]}
          </p>
        ) : (
          <p className="loading-caption">&nbsp;</p>
        )}

        <p className="loading-foot">
          <span className="elapsed">{elapsed}s</span>
          {error !== null ? (
            <button type="button" className="linkish" onClick={onBack}>
              ◀ back
            </button>
          ) : (
            <span className="loading-hint">first analysis of a large repo can take a while</span>
          )}
        </p>
      </div>
    </div>
  )
}
