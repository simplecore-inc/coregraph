import type { ChangeEvent } from 'react'

interface ProjectBarProps {
  rootLabel: string
  nodeTotal: number
  edgeTotal: number
  onLoadFile: (file: File) => void
  /** Present only in bridge mode: returns to the daemon project picker. */
  onSwitchProject?: () => void
  /** Present only in bridge mode with a live project: forced full re-index. */
  onReindex?: () => void
}

/** Top rail card: identity + dataset + project-level actions. */
export function ProjectBar(props: ProjectBarProps) {
  const handleFile = (event: ChangeEvent<HTMLInputElement>): void => {
    const file = event.target.files?.[0]
    if (file !== undefined) props.onLoadFile(file)
    event.target.value = ''
  }

  return (
    <section className="panel rail-card project-bar">
      <header className="wordmark">
        <h1>
          coregraph<em>atlas</em>
        </h1>
        <p className="tagline">symbol graph observatory</p>
      </header>
      <p className="dataset-root" title={props.rootLabel}>
        {props.rootLabel || '(no common root)'}
      </p>
      <p className="dataset-counts">
        {props.nodeTotal.toLocaleString()} symbols · {props.edgeTotal.toLocaleString()} edges
      </p>
      <div className="dataset-actions">
        {props.onSwitchProject !== undefined ? (
          <button type="button" className="load-btn" onClick={props.onSwitchProject}>
            ⇄ switch project
          </button>
        ) : null}
        {props.onReindex !== undefined ? (
          <button
            type="button"
            className="load-btn"
            title="Drop the cached graph and re-index the project from source"
            onClick={props.onReindex}
          >
            ⟳ reindex
          </button>
        ) : null}
        <label className="load-btn">
          load json…
          <input type="file" accept=".json,application/json" onChange={handleFile} />
        </label>
      </div>
    </section>
  )
}
