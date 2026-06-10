interface EmptyStateProps {
  onLoadFile: (file: File) => void
}

/** Full-screen landing shown when no dataset is embedded or loaded yet. */
export function EmptyState({ onLoadFile }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-card">
        <h1>
          coregraph<em>atlas</em>
        </h1>
        <p className="tagline">symbol graph observatory</p>
        <p className="empty-lead">
          Drop a <code>json-graph</code> export anywhere on this page, or pick a file.
        </p>
        <pre className="empty-cmd">
          coregraph -C &lt;project&gt; export --format json-graph &gt; graph.json
        </pre>
        <label className="load-btn big">
          choose graph.json…
          <input
            type="file"
            accept=".json,application/json"
            onChange={(event) => {
              const file = event.target.files?.[0]
              if (file !== undefined) onLoadFile(file)
              event.target.value = ''
            }}
          />
        </label>
      </div>
    </div>
  )
}

/** Translucent overlay while a file is being dragged over the window. */
export function DragOverlay() {
  return (
    <div className="drag-overlay">
      <div className="drag-frame">
        <p>release to load json-graph</p>
      </div>
    </div>
  )
}
