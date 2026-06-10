import { useState, type FormEvent } from 'react'
import type { BridgeStatus, ProjectStatus } from '../lib/api.ts'
import { resolveProjectPath } from '../lib/api.ts'

interface ConnectScreenProps {
  status: BridgeStatus | null
  probeError: string | null
  recents: string[]
  onOpenProject: (path: string) => void
  onRefresh: () => void
}

function splitPath(path: string): { parent: string; name: string } {
  const normalized = path.replace(/\/+$/, '')
  const idx = normalized.lastIndexOf('/')
  return idx <= 0
    ? { parent: '', name: normalized }
    : { parent: normalized.slice(0, idx + 1), name: normalized.slice(idx + 1) }
}

function ProjectRow({
  project,
  onOpen,
}: {
  project: ProjectStatus
  onOpen: (path: string) => void
}) {
  const { parent, name } = splitPath(project.path)
  return (
    <li>
      <button type="button" className="project-row" onClick={() => onOpen(project.path)}>
        <span className="project-path">
          <b>{name}</b>
          <i>{parent}</i>
        </span>
        {project.loading ? (
          <span className="project-meta loading">indexing…</span>
        ) : project.loaded ? (
          <span className="project-meta">
            {project.node_count.toLocaleString()} symbols · {project.edge_count.toLocaleString()}{' '}
            edges · idle {project.idle_seconds}s
          </span>
        ) : (
          <span className="project-meta">on disk — will load on open</span>
        )}
        <span className="project-go">▶</span>
      </button>
    </li>
  )
}

export function ConnectScreen({
  status,
  probeError,
  recents,
  onOpenProject,
  onRefresh,
}: ConnectScreenProps) {
  const [path, setPath] = useState('')
  const [pathError, setPathError] = useState<string | null>(null)
  const [validating, setValidating] = useState(false)

  const running = status?.running === true
  const projects = status?.manager?.projects ?? []
  const loadedProjects = new Set(projects.map((p) => p.path))
  const recentOnly = recents.filter((r) => !loadedProjects.has(r))

  const submit = async (event: FormEvent): Promise<void> => {
    event.preventDefault()
    if (path.trim() === '' || validating) return
    setValidating(true)
    setPathError(null)
    try {
      const resolved = await resolveProjectPath(path)
      if (resolved.ok) {
        onOpenProject(resolved.path)
      } else {
        setPathError(resolved.error)
      }
    } catch (error) {
      setPathError(error instanceof Error ? error.message : String(error))
    } finally {
      setValidating(false)
    }
  }

  return (
    <div className="empty-state">
      <div className="empty-card connect-card">
        <h1>
          coregraph<em>atlas</em>
        </h1>
        <p className="tagline">symbol graph observatory</p>

        {probeError !== null ? (
          <p className="daemon-chip down" role="status">
            ✖ bridge error: {probeError}
          </p>
        ) : status === null ? (
          <p className="daemon-chip" role="status">
            probing daemon…
          </p>
        ) : running ? (
          <p className="daemon-chip up" role="status">
            ● daemon running · {projects.length}/{status.manager?.max_loaded ?? '?'} projects in
            memory · up {Math.floor((status.manager?.uptime_seconds ?? 0) / 60)}m
          </p>
        ) : (
          <p className="daemon-chip down" role="status">
            ○ daemon stopped — it will start automatically when you open a project
          </p>
        )}

        {projects.length > 0 ? (
          <section className="connect-section">
            <h2>loaded in daemon memory</h2>
            <ul className="project-list">
              {projects.map((project) => (
                <ProjectRow key={project.path} project={project} onOpen={onOpenProject} />
              ))}
            </ul>
          </section>
        ) : null}

        {recentOnly.length > 0 ? (
          <section className="connect-section">
            <h2>recent</h2>
            <ul className="project-list">
              {recentOnly.map((recent) => {
                const { parent, name } = splitPath(recent)
                return (
                  <li key={recent}>
                    <button
                      type="button"
                      className="project-row"
                      onClick={() => onOpenProject(recent)}
                    >
                      <span className="project-path">
                        <b>{name}</b>
                        <i>{parent}</i>
                      </span>
                      <span className="project-go">▶</span>
                    </button>
                  </li>
                )
              })}
            </ul>
          </section>
        ) : null}

        <section className="connect-section">
          <h2>analyze a project</h2>
          <form className="path-form" onSubmit={(event) => void submit(event)}>
            <input
              type="text"
              value={path}
              spellCheck={false}
              placeholder="/path/to/project"
              aria-label="Project path"
              onChange={(event) => {
                setPath(event.target.value)
                setPathError(null)
              }}
            />
            <button type="submit" disabled={validating || path.trim() === ''}>
              {validating ? '…' : 'open ▶'}
            </button>
          </form>
          {pathError !== null ? <p className="path-error">✖ {pathError}</p> : null}
        </section>

        <p className="connect-foot">
          <button type="button" className="linkish" onClick={onRefresh}>
            ↺ refresh
          </button>
          <span> · drop a json-graph export anywhere to view it offline</span>
        </p>
      </div>
    </div>
  )
}
