import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { GraphCanvas, type ClusterBy, type ColorMode } from './components/GraphCanvas.tsx'
import { SearchBar } from './components/SearchBar.tsx'
import { FilterPanel, type KindRow } from './components/FilterPanel.tsx'
import { ProjectBar } from './components/ProjectBar.tsx'
import { AnalysisDock } from './components/AnalysisDock.tsx'
import { DetailPanel } from './components/DetailPanel.tsx'
import { AnalysisPanel, type Analysis } from './components/AnalysisPanel.tsx'
import { Dialog } from './components/Dialog.tsx'
import { StatusBar } from './components/StatusBar.tsx'
import { ConnectScreen } from './components/ConnectScreen.tsx'
import { LoadingOverlay, type Stage, type StageState } from './components/LoadingOverlay.tsx'
import { DragOverlay, EmptyState } from './components/DropZone.tsx'
import { loadEmbedded } from './data/embedded.ts'
import {
  BridgeError,
  fetchDiff,
  fetchGraph,
  fetchImpact,
  fetchInconsistencies,
  fetchOrphans,
  fetchSource,
  fetchStatus,
  maybeBridged,
  recentProjects,
  reindexProject,
  rememberProject,
  startDaemon,
  type BridgeStatus,
  type SourceSnippet,
} from './lib/api.ts'
import {
  buildAdjacency,
  buildLocator,
  hopDistances,
  pathOverlayLinks,
  resolveSymbol,
  shortestPath,
} from './lib/analysis.ts'
import { computeVisible, incidentEdges, linkEndId, neighborIds } from './lib/graph.ts'
import { GraphParseError, parseDataset } from './lib/parse.ts'
import {
  DEFAULT_HIDDEN_EDGE_KINDS,
  DEFAULT_HIDDEN_NODE_KINDS,
  dirColor,
} from './lib/palette.ts'
import { decodeShareState, encodeShareState, type ShareState } from './lib/share.ts'
import { deriveUnits } from './lib/units.ts'
import type { LinkDatum, NodeDatum, ParsedGraph, ViewFilters } from './lib/types.ts'

const EMPTY_SET: ReadonlySet<number> = new Set()

type Mode = 'bridge' | 'standalone'

type Phase =
  | { kind: 'connect' }
  | { kind: 'loading'; project: string; stages: Stage[]; startedAt: number; error: string | null }
  | { kind: 'view' }

function countBy<T>(items: readonly T[], key: (item: T) => string): KindRow[] {
  const counts = new Map<string, number>()
  for (const item of items) {
    const k = key(item)
    counts.set(k, (counts.get(k) ?? 0) + 1)
  }
  return [...counts.entries()]
    .map(([kind, count]) => ({ kind, count }))
    .sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind))
}

function defaultHidden(defaults: readonly string[]): ReadonlySet<string> {
  return new Set(defaults)
}

function toggled(set: ReadonlySet<string>, value: string): ReadonlySet<string> {
  const next = new Set(set)
  if (next.has(value)) next.delete(value)
  else next.add(value)
  return next
}

/**
 * Analyses that are list/table-shaped and don't need the graph behind them
 * open as a centered dialog; the rest stay as side panels with overlays.
 */
const DIALOG_ANALYSES: ReadonlySet<Analysis['kind']> = new Set(['inconsistencies', 'diff'])

const LOAD_STAGES: readonly { key: string; label: string }[] = [
  { key: 'daemon', label: 'connect to daemon' },
  { key: 'analyze', label: 'index & extract symbol graph' },
  { key: 'prepare', label: 'build constellation' },
]

const REINDEX_STAGES: readonly { key: string; label: string }[] = [
  { key: 'daemon', label: 'connect to daemon' },
  { key: 'analyze', label: 're-index from source' },
  { key: 'prepare', label: 'build constellation' },
]

export default function App() {
  const [mode, setMode] = useState<Mode>(() => (maybeBridged() ? 'bridge' : 'standalone'))
  const [phase, setPhase] = useState<Phase>(() =>
    maybeBridged() ? { kind: 'connect' } : { kind: 'view' },
  )
  const [bridgeStatus, setBridgeStatus] = useState<BridgeStatus | null>(null)
  const [probeError, setProbeError] = useState<string | null>(null)
  const [recents, setRecents] = useState<string[]>(recentProjects)
  /** Project currently shown in the viewer (bridge mode), for re-index. */
  const [currentProject, setCurrentProject] = useState<string | null>(null)

  const [graph, setGraph] = useState<ParsedGraph | null>(() =>
    maybeBridged() ? null : loadEmbedded(),
  )
  const [dataEpoch, setDataEpoch] = useState(0)
  const [hiddenNodeKinds, setHiddenNodeKinds] = useState<ReadonlySet<string>>(() =>
    defaultHidden(DEFAULT_HIDDEN_NODE_KINDS),
  )
  const [hiddenEdgeKinds, setHiddenEdgeKinds] = useState<ReadonlySet<string>>(() =>
    defaultHidden(DEFAULT_HIDDEN_EDGE_KINDS),
  )
  const [hiddenOrigins, setHiddenOrigins] = useState<ReadonlySet<string>>(() => new Set())
  const [minConfidence, setMinConfidence] = useState(0.7)
  const [focusDepth, setFocusDepth] = useState(1)
  const [colorMode, setColorMode] = useState<ColorMode>('kind')
  const [hideIsolated, setHideIsolated] = useState(true)
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [focusOn, setFocusOn] = useState(false)
  const [toast, setToast] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  // ── analysis & view extensions ──
  const [analysis, setAnalysis] = useState<Analysis | null>(null)
  const [analysisLoading, setAnalysisLoading] = useState<string | null>(null)
  /** Whether a dialog-shaped analysis result is currently shown. */
  const [analysisDialogOpen, setAnalysisDialogOpen] = useState(false)
  const [clusterBy, setClusterBy] = useState<ClusterBy>('none')
  const [minDegree, setMinDegree] = useState(0)
  const [hideHubs, setHideHubs] = useState(false)
  const [dirFilter, setDirFilter] = useState<string | null>(null)
  const [pathSource, setPathSource] = useState<NodeDatum | null>(null)
  const [orphanOptions, setOrphanOptions] = useState({ excludeTests: true, publicOnly: true })
  const [updateAvailable, setUpdateAvailable] = useState(false)
  const [sourceSnippet, setSourceSnippet] = useState<SourceSnippet | null>(null)
  /** Live daemon counters for the open project (filled by the change poller). */
  const [daemonLive, setDaemonLive] = useState<{ symbols: number; idle: number } | null>(null)

  const searchRef = useRef<HTMLInputElement | null>(null)
  const toastTimer = useRef<number | undefined>(undefined)
  const abortRef = useRef<AbortController | null>(null)
  /** Project currently being opened — duplicate requests for it are ignored. */
  const openingRef = useRef<string | null>(null)
  const analysisAbort = useRef<AbortController | null>(null)
  const sourceAbort = useRef<AbortController | null>(null)
  const sourceCache = useRef(new Map<number, SourceSnippet | null>())
  /** Daemon-side counts recorded by the change poller for staleness detection. */
  const pollBaseline = useRef<{ nodes: number; edges: number } | null>(null)
  /** Share-link state to re-apply once the linked project finishes loading. */
  const pendingShare = useRef<ShareState | null>(null)

  const showToast = useCallback((message: string) => {
    setToast(message)
    window.clearTimeout(toastTimer.current)
    toastTimer.current = window.setTimeout(() => setToast(null), 4200)
  }, [])

  /** Install a parsed dataset and reset all view state. */
  const applyDataset = useCallback((parsed: ParsedGraph) => {
    setGraph(parsed)
    setDataEpoch((epoch) => epoch + 1)
    setSelectedId(null)
    setFocusOn(false)
    setHiddenNodeKinds(defaultHidden(DEFAULT_HIDDEN_NODE_KINDS))
    setHiddenEdgeKinds(defaultHidden(DEFAULT_HIDDEN_EDGE_KINDS))
    setHiddenOrigins(new Set())
    setAnalysis(null)
    setAnalysisLoading(null)
    setAnalysisDialogOpen(false)
    setClusterBy('none')
    setMinDegree(0)
    setHideHubs(false)
    setDirFilter(null)
    setPathSource(null)
    setUpdateAvailable(false)
    setSourceSnippet(null)
    sourceCache.current.clear()
    pollBaseline.current = null
  }, [])

  const probe = useCallback(async () => {
    try {
      const status = await fetchStatus()
      setBridgeStatus(status)
      setProbeError(null)
    } catch (error) {
      // Demote to offline mode ONLY on a definitive "no bridge here" signal:
      // a 404/405 (some other server answers this origin) or a 200 whose body
      // is not the JSON we expect (SyntaxError → static hosting). A transient
      // network failure (TypeError from fetch) must stay recoverable so a
      // refresh can retry, instead of stranding the user offline.
      const definitelyNoBridge =
        (error instanceof BridgeError && (error.status === 404 || error.status === 405)) ||
        error instanceof SyntaxError
      if (definitelyNoBridge) {
        setMode('standalone')
        setGraph((prev) => prev ?? loadEmbedded())
        setPhase({ kind: 'view' })
      } else {
        setProbeError(error instanceof Error ? error.message : String(error))
      }
    }
  }, [])

  // Bridge probe on startup.
  useEffect(() => {
    if (mode === 'bridge') void probe()
    // The probe deliberately runs once on mount; later refreshes are explicit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const setStage = useCallback((key: string, state: StageState) => {
    setPhase((prev) =>
      prev.kind === 'loading'
        ? {
            ...prev,
            stages: prev.stages.map((stage) => (stage.key === key ? { ...stage, state } : stage)),
          }
        : prev,
    )
  }, [])

  const openProject = useCallback(
    async (project: string, fresh = false) => {
      // Double-clicks and repeated requests for the same project join the
      // in-flight load instead of restarting it; a different project takes
      // over by aborting the previous one.
      if (openingRef.current === project) return
      openingRef.current = project
      abortRef.current?.abort()
      const controller = new AbortController()
      abortRef.current = controller
      setPhase({
        kind: 'loading',
        project,
        stages: (fresh ? REINDEX_STAGES : LOAD_STAGES).map((stage, index) => ({
          ...stage,
          state: index === 0 ? 'active' : 'pending',
        })),
        startedAt: Date.now(),
        error: null,
      })
      let stage = 'daemon'
      try {
        const status = await fetchStatus(controller.signal)
        if (controller.signal.aborted) return
        if (!status.running) await startDaemon(project, controller.signal)
        if (controller.signal.aborted) return
        setStage('daemon', 'done')
        stage = 'analyze'
        setStage('analyze', 'active')
        if (fresh) {
          // Forced rebuild from source; the graph fetch below then exports
          // the freshly indexed graph straight from daemon memory.
          await reindexProject(project, controller.signal)
          if (controller.signal.aborted) return
        }
        const raw = await fetchGraph(project, 0, controller.signal)
        if (controller.signal.aborted) return
        setStage('analyze', 'done')
        stage = 'prepare'
        setStage('prepare', 'active')
        const parsed = parseDataset(raw)
        if (controller.signal.aborted) return
        applyDataset(parsed)
        setCurrentProject(project)
        rememberProject(project)
        setRecents(recentProjects())
        // A share link restores its filters/selection after the load resets.
        const share = pendingShare.current
        if (share !== null) {
          pendingShare.current = null
          if (share.minConfidence !== undefined) setMinConfidence(share.minConfidence)
          if (share.depth !== undefined) setFocusDepth(share.depth)
          if (share.colorMode !== undefined) setColorMode(share.colorMode)
          if (share.clusterBy !== undefined) setClusterBy(share.clusterBy)
          if (share.minDegree !== undefined) setMinDegree(share.minDegree)
          if (share.hideHubs !== undefined) setHideHubs(share.hideHubs)
          if (share.dir !== undefined) setDirFilter(share.dir)
          if (share.hiddenNodeKinds !== undefined) {
            setHiddenNodeKinds(new Set(share.hiddenNodeKinds))
          }
          if (share.hiddenEdgeKinds !== undefined) {
            setHiddenEdgeKinds(new Set(share.hiddenEdgeKinds))
          }
          if (share.symbol !== undefined) {
            const node = resolveSymbol(buildLocator(parsed), share.symbol)
            if (node !== undefined) {
              setSelectedId(node.id)
              if (share.focus === true) setFocusOn(true)
            }
          }
        }
        setStage('prepare', 'done')
        setPhase({ kind: 'view' })
        const note = parsed.warnings.length > 0 ? ` · ${parsed.warnings.join('; ')}` : ''
        showToast(
          `${parsed.nodes.length.toLocaleString()} symbols · ${parsed.edges.length.toLocaleString()} edges${note}`,
        )
      } catch (error) {
        if (controller.signal.aborted) return
        const message =
          error instanceof BridgeError || error instanceof GraphParseError
            ? error.message
            : error instanceof Error
              ? error.message
              : String(error)
        setStage(stage, 'error')
        setPhase((prev) => (prev.kind === 'loading' ? { ...prev, error: message } : prev))
      } finally {
        // A takeover by another project has already replaced the marker.
        if (openingRef.current === project) openingRef.current = null
      }
    },
    [applyDataset, setStage, showToast],
  )

  const backToConnect = useCallback(() => {
    abortRef.current?.abort()
    setPhase({ kind: 'connect' })
    void probe()
  }, [probe])

  // Share-link bootstrap: a #v=... hash naming a project skips the picker.
  useEffect(() => {
    if (mode !== 'bridge') return
    const share = decodeShareState(window.location.hash)
    if (share !== null && typeof share.project === 'string' && share.project !== '') {
      pendingShare.current = share
      void openProject(share.project)
    }
    // Boot-time only; later navigation goes through the picker.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const nodeKindRows = useMemo(
    () => (graph === null ? [] : countBy(graph.nodes, (n) => n.kind)),
    [graph],
  )
  const edgeKindRows = useMemo(
    () => (graph === null ? [] : countBy(graph.edges, (e) => e.kind)),
    [graph],
  )
  const originRows = useMemo(
    () => (graph === null ? [] : countBy(graph.edges, (e) => e.origin)),
    [graph],
  )

  // Degree distribution drives the hub-filter controls: slider ceiling and
  // the p99 cap used by "hide top hubs".
  const degreeStats = useMemo(() => {
    if (graph === null || graph.nodes.length === 0) {
      return { max: 0, hubCap: Number.MAX_SAFE_INTEGER }
    }
    const degrees = graph.nodes.map((n) => n.deg).sort((a, b) => a - b)
    const p99 = degrees[Math.min(degrees.length - 1, Math.floor(degrees.length * 0.99))]
    return { max: Math.min(50, degrees[degrees.length - 1]), hubCap: p99 }
  }, [graph])

  const filters: ViewFilters = useMemo(() => {
    const nodeKinds = new Set(
      nodeKindRows.map((r) => r.kind).filter((k) => !hiddenNodeKinds.has(k)),
    )
    const edgeKinds = new Set(
      edgeKindRows.map((r) => r.kind).filter((k) => !hiddenEdgeKinds.has(k)),
    )
    const origins = new Set(originRows.map((r) => r.kind).filter((k) => !hiddenOrigins.has(k)))
    return {
      nodeKinds,
      edgeKinds,
      origins,
      minConfidence,
      hideIsolated,
      minDegree,
      hideHubs,
      hubCap: degreeStats.hubCap,
      dirs: dirFilter === null ? null : new Set([dirFilter]),
    }
  }, [
    nodeKindRows,
    edgeKindRows,
    originRows,
    hiddenNodeKinds,
    hiddenEdgeKinds,
    hiddenOrigins,
    minConfidence,
    hideIsolated,
    minDegree,
    hideHubs,
    degreeStats,
    dirFilter,
  ])

  // Isolate the focus spec so it is a stable `null` whenever focus is off:
  // otherwise every node selection would change `visible`'s identity and force
  // the WebGL engine to re-run its full layout warmup on each click.
  const focusSpec = useMemo(
    () => (focusOn && selectedId !== null ? { id: selectedId, depth: focusDepth } : null),
    [focusOn, selectedId, focusDepth],
  )

  const visible = useMemo(() => {
    if (graph === null) return { nodes: [], links: [] }
    return computeVisible(graph, filters, focusSpec)
  }, [graph, filters, focusSpec])

  const locator = useMemo(() => (graph === null ? null : buildLocator(graph)), [graph])

  const resolveId = useCallback(
    (name: string, file?: string): number | undefined =>
      locator === null ? undefined : resolveSymbol(locator, name, file)?.id,
    [locator],
  )

  // Analysis overlay → node color map + synthetic links for the canvas.
  const analysisOverlay = useMemo((): {
    highlight: ReadonlyMap<number, string> | null
    extraLinks: LinkDatum[]
  } => {
    if (graph === null || analysis === null) {
      return { highlight: null, extraLinks: [] }
    }
    const highlight = new Map<number, string>()
    const extraLinks: LinkDatum[] = []
    if (analysis.kind === 'impact') {
      const member = new Set<number>()
      for (const node of analysis.data.nodes) member.add(node.id)
      const seed = resolveId(analysis.data.symbol)
      // Hop-distance gradient: the seed burns white-hot, the rim cools out.
      const ramp = ['#ffffff', '#ffb454', '#ff8a5c', '#e06c5a', '#b3675e', '#8a5a5e']
      if (seed !== undefined) {
        const distances = hopDistances(buildAdjacency(graph.edges), seed, member)
        for (const [id, hop] of distances) {
          highlight.set(id, ramp[Math.min(hop, ramp.length - 1)])
        }
      }
      // Reachable nodes the gradient missed (e.g. unresolved seed) still glow.
      for (const id of member) {
        if (!highlight.has(id)) highlight.set(id, ramp[ramp.length - 1])
      }
    } else if (analysis.kind === 'orphans') {
      for (const orphan of analysis.data.orphans) {
        const id = resolveId(orphan.name, orphan.file)
        if (id === undefined) continue
        highlight.set(id, orphan.external_api ? '#d7ba7d' : orphan.is_test ? '#6b7d90' : '#ff6b6b')
      }
    } else if (analysis.kind === 'inconsistencies') {
      for (const report of analysis.data.reports) {
        if (report.a === undefined || report.b === undefined) continue
        const a = resolveId(report.a.name, report.a.file)
        const b = resolveId(report.b.name, report.b.file)
        if (a !== undefined) highlight.set(a, '#ff6b9d')
        if (b !== undefined) highlight.set(b, '#ff6b9d')
        if (a !== undefined && b !== undefined) {
          extraLinks.push({ source: a, target: b, kind: '__pair', origin: 'overlay', confidence: 1 })
        }
      }
    } else if (analysis.kind === 'diff') {
      for (const file of analysis.data.changed_files) {
        for (const seed of file.seed_symbols) {
          const id = resolveId(seed, file.file)
          if (id !== undefined) highlight.set(id, '#ffd166')
        }
        for (const affected of file.top_affected) {
          const id = resolveId(affected.name, affected.file)
          if (id !== undefined && !highlight.has(id)) highlight.set(id, '#56d4dd')
        }
      }
    } else {
      // path
      for (const node of analysis.nodes) highlight.set(node.id, '#ffd166')
      extraLinks.push(...pathOverlayLinks(analysis.nodes.map((n) => n.id)))
    }
    return { highlight, extraLinks }
  }, [graph, analysis, resolveId])

  // The force engine throws on links whose endpoints are not in the node set
  // (e.g. a mismatch pair on a hidden kind) — drop those overlay links.
  const safeExtraLinks = useMemo(() => {
    if (analysisOverlay.extraLinks.length === 0) return analysisOverlay.extraLinks
    const present = new Set(visible.nodes.map((n) => n.id))
    return analysisOverlay.extraLinks.filter(
      (link) => present.has(linkEndId(link.source)) && present.has(linkEndId(link.target)),
    )
  }, [analysisOverlay, visible.nodes])

  const neighborSet = useMemo(
    () => (selectedId === null ? EMPTY_SET : neighborIds(visible.links, selectedId)),
    [visible.links, selectedId],
  )

  const dirPalette = useMemo(() => {
    const palette = new Map<string, string>()
    if (graph !== null) {
      graph.dirs.forEach((dir, index) => palette.set(dir, dirColor(index)))
    }
    return palette
  }, [graph])

  // Module/package/crate-level grouping (structure-derived units).
  const unitIndex = useMemo(
    () => (graph === null ? { unitOf: new Map<number, string>(), units: [] } : deriveUnits(graph)),
    [graph],
  )

  const unitPalette = useMemo(() => {
    const palette = new Map<string, string>()
    unitIndex.units.forEach((unit, index) => palette.set(unit, dirColor(index)))
    return palette
  }, [unitIndex])

  const selectedNode: NodeDatum | null =
    graph !== null && selectedId !== null ? (graph.nodeById.get(selectedId) ?? null) : null

  const incident = useMemo(
    () =>
      graph !== null && selectedId !== null ? incidentEdges(graph, filters, selectedId) : [],
    [graph, filters, selectedId],
  )

  const selectNode = useCallback(
    (node: NodeDatum, isolate: boolean) => {
      // Path mode: the second pick completes the trace instead of selecting.
      if (pathSource !== null && graph !== null && node.id !== pathSource.id) {
        const from = pathSource
        setPathSource(null)
        const ids = shortestPath(graph, filters, from.id, node.id)
        if (ids === null) {
          showToast(`✖ no path between ${from.name} and ${node.name} under current edge filters`)
          return
        }
        const nodes = ids
          .map((id) => graph.nodeById.get(id))
          .filter((n): n is NodeDatum => n !== undefined)
        setAnalysis({ kind: 'path', from, to: node, nodes })
        setSelectedId(null)
        return
      }
      setSelectedId(node.id)
      if (isolate) setFocusOn(true)
      // Jumping to a symbol means looking at the graph — close any
      // list-shaped analysis dialog that would cover it.
      setAnalysisDialogOpen(false)
      // A symbol picked through search may belong to a hidden kind.
      setHiddenNodeKinds((prev) => {
        if (!prev.has(node.kind)) return prev
        const next = new Set(prev)
        next.delete(node.kind)
        return next
      })
    },
    [pathSource, graph, filters, showToast],
  )

  const handleCanvasClick = useCallback(
    (id: number) => {
      if (graph === null) return
      const node = graph.nodeById.get(id)
      if (node !== undefined) selectNode(node, false)
    },
    [graph, selectNode],
  )

  const handleBackgroundClick = useCallback(() => {
    if (!focusOn) setSelectedId(null)
  }, [focusOn])

  const loadFile = useCallback(
    async (file: File) => {
      // Cancel any in-flight bridge load so its later-arriving result can't
      // overwrite the file the user just dropped.
      abortRef.current?.abort()
      openingRef.current = null
      setCurrentProject(null)
      try {
        const text = await file.text()
        const parsed = parseDataset(JSON.parse(text))
        applyDataset(parsed)
        setPhase({ kind: 'view' })
        const note = parsed.warnings.length > 0 ? ` (${parsed.warnings.join('; ')})` : ''
        showToast(
          `loaded ${parsed.nodes.length.toLocaleString()} symbols · ${parsed.edges.length.toLocaleString()} edges${note}`,
        )
      } catch (error) {
        if (error instanceof GraphParseError || error instanceof SyntaxError) {
          showToast(`✖ ${error.message}`)
        } else {
          showToast('✖ failed to read the file')
          console.error('unexpected error while loading dataset:', error)
        }
      }
    },
    [applyDataset, showToast],
  )

  const inViewer = phase.kind === 'view' && graph !== null

  /** Run one daemon analysis with single-flight + abort semantics. */
  const runAnalysis = useCallback(
    (key: string, run: (signal: AbortSignal) => Promise<Analysis>) => {
      if (currentProject === null || analysisLoading !== null) return
      analysisAbort.current?.abort()
      const controller = new AbortController()
      analysisAbort.current = controller
      setAnalysisLoading(key)
      void (async () => {
        try {
          const result = await run(controller.signal)
          if (controller.signal.aborted) return
          setAnalysis(result)
          setAnalysisDialogOpen(DIALOG_ANALYSES.has(result.kind))
          // Surface the results: selection would otherwise keep the detail
          // panel on top of the analysis panel (clicking a node brings it back).
          setSelectedId(null)
          setFocusOn(false)
        } catch (error) {
          if (controller.signal.aborted) return
          showToast(`✖ ${error instanceof Error ? error.message : String(error)}`)
        } finally {
          if (!controller.signal.aborted) setAnalysisLoading(null)
        }
      })()
    },
    [currentProject, analysisLoading, showToast],
  )

  const runImpact = useCallback(() => {
    if (selectedNode === null || currentProject === null) return
    const symbol = selectedNode.name
    runAnalysis('impact', async (signal) => ({
      kind: 'impact',
      data: await fetchImpact(currentProject, symbol, 5, signal),
    }))
  }, [selectedNode, currentProject, runAnalysis])

  const runOrphans = useCallback(() => {
    if (currentProject === null) return
    const options = orphanOptions
    runAnalysis('orphans', async (signal) => ({
      kind: 'orphans',
      data: await fetchOrphans(currentProject, options, signal),
      ...options,
    }))
  }, [currentProject, orphanOptions, runAnalysis])

  const runInconsistencies = useCallback(
    (category: string) => {
      if (currentProject === null) return
      runAnalysis('inconsistencies', async (signal) => ({
        kind: 'inconsistencies',
        data: await fetchInconsistencies(
          currentProject,
          { category: category === '' ? undefined : category, excludeTests: true },
          signal,
        ),
        category,
      }))
    },
    [currentProject, runAnalysis],
  )

  const runDiff = useCallback(
    (baseRef: string) => {
      if (currentProject === null) return
      runAnalysis('diff', async (signal) => ({
        kind: 'diff',
        data: await fetchDiff(currentProject, baseRef, signal),
      }))
    },
    [currentProject, runAnalysis],
  )

  const startPathFrom = useCallback(() => {
    if (selectedNode === null) return
    setPathSource(selectedNode)
    showToast(`path mode: pick the target symbol (click or search) — esc cancels`)
  }, [selectedNode, showToast])

  const handleShareLink = useCallback(() => {
    const state: ShareState = {
      project: currentProject ?? undefined,
      symbol: selectedNode?.name,
      focus: focusOn || undefined,
      depth: focusDepth,
      minConfidence,
      colorMode,
      clusterBy: clusterBy === 'none' ? undefined : clusterBy,
      minDegree: minDegree > 0 ? minDegree : undefined,
      hideHubs: hideHubs || undefined,
      dir: dirFilter ?? undefined,
      hiddenNodeKinds: [...hiddenNodeKinds],
      hiddenEdgeKinds: [...hiddenEdgeKinds],
    }
    window.history.replaceState(null, '', `#${encodeShareState(state)}`)
    if (navigator.clipboard !== undefined) {
      void navigator.clipboard.writeText(window.location.href)
    }
    showToast('share link copied to clipboard')
  }, [
    currentProject,
    selectedNode,
    focusOn,
    focusDepth,
    minConfidence,
    colorMode,
    clusterBy,
    minDegree,
    hideHubs,
    dirFilter,
    hiddenNodeKinds,
    hiddenEdgeKinds,
    showToast,
  ])

  const downloadBlob = useCallback((name: string, blob: Blob) => {
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = name
    anchor.click()
    URL.revokeObjectURL(url)
  }, [])

  const handleExportPng = useCallback(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('.canvas-layer canvas')
    if (canvas === null) return
    canvas.toBlob((blob) => {
      if (blob !== null) downloadBlob('coregraph-atlas.png', blob)
    })
  }, [downloadBlob])

  const handleExportJson = useCallback(() => {
    const payload = {
      nodes: visible.nodes.map(({ id, name, kind, file, spanStart, spanEnd }) => ({
        id,
        name,
        kind,
        file,
        span_start: spanStart,
        span_end: spanEnd,
      })),
      edges: visible.links
        .filter((link) => !link.kind.startsWith('__'))
        .map((link) => ({
          from: linkEndId(link.source),
          to: linkEndId(link.target),
          kind: link.kind,
          origin: link.origin,
          confidence: link.confidence,
        })),
    }
    downloadBlob(
      'coregraph-atlas-subgraph.json',
      new Blob([JSON.stringify(payload)], { type: 'application/json' }),
    )
  }, [visible, downloadBlob])

  // Change polling: every 5s compare the daemon's node/edge counts for the
  // open project against the first observation. The daemon keeps its graph
  // fresh on its own (file watcher + staleness checks); the browser only
  // needs to notice and offer a one-click reload — auto-applying would reset
  // the layout and camera mid-exploration.
  useEffect(() => {
    if (mode !== 'bridge' || !inViewer || currentProject === null) return
    pollBaseline.current = null
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const status = await fetchStatus()
          const entry = status.manager?.projects.find((p) => p.path === currentProject)
          if (entry === undefined || !entry.loaded) return
          setDaemonLive({ symbols: entry.node_count, idle: entry.idle_seconds })
          const counts = { nodes: entry.node_count, edges: entry.edge_count }
          if (pollBaseline.current === null) {
            pollBaseline.current = counts
            return
          }
          if (
            counts.nodes !== pollBaseline.current.nodes ||
            counts.edges !== pollBaseline.current.edges
          ) {
            setUpdateAvailable(true)
          }
        } catch {
          // Transient poll failures are uninteresting; the next tick retries.
        }
      })()
    }, 5000)
    return () => window.clearInterval(timer)
  }, [mode, inViewer, currentProject, dataEpoch])

  // Source preview for the selected symbol (bridge mode), cached per node.
  useEffect(() => {
    if (mode !== 'bridge' || currentProject === null || selectedNode === null) {
      setSourceSnippet(null)
      return
    }
    if (selectedNode.file === '') {
      setSourceSnippet(null)
      return
    }
    const cached = sourceCache.current.get(selectedNode.id)
    if (cached !== undefined) {
      setSourceSnippet(cached)
      return
    }
    sourceAbort.current?.abort()
    const controller = new AbortController()
    sourceAbort.current = controller
    setSourceSnippet(null)
    void (async () => {
      try {
        const snippet = await fetchSource(
          currentProject,
          selectedNode.file,
          selectedNode.spanStart,
          selectedNode.spanEnd,
          controller.signal,
        )
        if (controller.signal.aborted) return
        sourceCache.current.set(selectedNode.id, snippet)
        setSourceSnippet(snippet)
      } catch (error) {
        if (controller.signal.aborted) return
        // Preview is best-effort; a missing/oversized file just shows nothing.
        sourceCache.current.set(selectedNode.id, null)
        setSourceSnippet(null)
        if (!(error instanceof BridgeError)) {
          console.error('source preview failed:', error)
        }
      }
    })()
  }, [mode, currentProject, selectedNode])

  // Global keyboard shortcuts: "/" focuses search, Escape unwinds focus/selection.
  useEffect(() => {
    if (!inViewer) return
    const onKeyDown = (event: KeyboardEvent): void => {
      const target = event.target as HTMLElement | null
      const typing =
        target !== null && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')
      if (event.key === '/' && !typing) {
        event.preventDefault()
        searchRef.current?.focus()
      } else if (event.key === 'Escape' && !typing) {
        if (pathSource !== null) setPathSource(null)
        else if (focusOn) setFocusOn(false)
        else setSelectedId(null)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [inViewer, focusOn, pathSource])

  // Window-level drag & drop for loading a json-graph file in any phase.
  useEffect(() => {
    let depth = 0
    const hasFile = (event: DragEvent): boolean =>
      event.dataTransfer?.types.includes('Files') ?? false
    const onDragEnter = (event: DragEvent): void => {
      if (!hasFile(event)) return
      depth++
      setDragging(true)
    }
    const onDragOver = (event: DragEvent): void => {
      if (hasFile(event)) event.preventDefault()
    }
    const onDragLeave = (event: DragEvent): void => {
      if (!hasFile(event)) return
      depth = Math.max(0, depth - 1)
      if (depth === 0) setDragging(false)
    }
    const onDrop = (event: DragEvent): void => {
      if (!hasFile(event)) return
      event.preventDefault()
      depth = 0
      setDragging(false)
      const file = event.dataTransfer?.files[0]
      if (file !== undefined) void loadFile(file)
    }
    window.addEventListener('dragenter', onDragEnter)
    window.addEventListener('dragover', onDragOver)
    window.addEventListener('dragleave', onDragLeave)
    window.addEventListener('drop', onDrop)
    return () => {
      window.removeEventListener('dragenter', onDragEnter)
      window.removeEventListener('dragover', onDragOver)
      window.removeEventListener('dragleave', onDragLeave)
      window.removeEventListener('drop', onDrop)
    }
  }, [loadFile])

  const overlays = (
    <>
      {dragging ? <DragOverlay /> : null}
      {toast !== null ? <div className="toast">{toast}</div> : null}
    </>
  )

  if (!inViewer) {
    if (mode === 'bridge' && phase.kind === 'loading') {
      return (
        <>
          <LoadingOverlay
            project={phase.project}
            stages={phase.stages}
            startedAt={phase.startedAt}
            error={phase.error}
            onBack={backToConnect}
          />
          {overlays}
        </>
      )
    }
    if (mode === 'bridge') {
      return (
        <>
          <ConnectScreen
            status={bridgeStatus}
            probeError={probeError}
            recents={recents}
            onOpenProject={(path) => void openProject(path)}
            onRefresh={() => void probe()}
          />
          {overlays}
        </>
      )
    }
    return (
      <>
        <EmptyState onLoadFile={(file) => void loadFile(file)} />
        {overlays}
      </>
    )
  }

  return (
    <>
      <GraphCanvas
        nodes={visible.nodes}
        links={visible.links}
        dataEpoch={dataEpoch}
        colorMode={colorMode}
        dirPalette={dirPalette}
        unitOf={unitIndex.unitOf}
        unitPalette={unitPalette}
        clusterBy={clusterBy}
        selectedId={selectedId}
        neighborSet={neighborSet}
        focusMode={focusOn}
        highlight={analysisOverlay.highlight}
        extraLinks={safeExtraLinks}
        onNodeClick={handleCanvasClick}
        onBackgroundClick={handleBackgroundClick}
      />
      <div className="vignette" aria-hidden="true" />

      {updateAvailable && currentProject !== null ? (
        <div className="update-banner" role="status">
          <span>graph changed on daemon</span>
          <button type="button" onClick={() => void openProject(currentProject)}>
            ↻ reload
          </button>
          <button
            type="button"
            className="dismiss"
            aria-label="Dismiss"
            onClick={() => {
              setUpdateAvailable(false)
              pollBaseline.current = null
            }}
          >
            ✕
          </button>
        </div>
      ) : null}

      {pathSource !== null ? (
        <div className="update-banner path-banner" role="status">
          <span>path from {pathSource.name} — pick the target symbol</span>
          <button type="button" className="dismiss" onClick={() => setPathSource(null)}>
            ✕
          </button>
        </div>
      ) : null}

      <SearchBar
        nodes={graph.nodes}
        inputRef={searchRef}
        onPick={(node) => selectNode(node, true)}
        actions={
          <>
            <button
              type="button"
              className="search-action"
              title="Copy a shareable link to this view"
              onClick={handleShareLink}
            >
              ⧉
            </button>
            <button
              type="button"
              className="search-action"
              title="Download the canvas as PNG"
              onClick={handleExportPng}
            >
              ⬇
            </button>
            <button
              type="button"
              className="search-action"
              title="Download the visible subgraph as json-graph"
              onClick={handleExportJson}
            >
              {'{}'}
            </button>
          </>
        }
      />

      <div className="left-rail">
        <ProjectBar
          rootLabel={graph.root}
          nodeTotal={graph.nodes.length}
          edgeTotal={graph.edges.length}
          onLoadFile={(file) => void loadFile(file)}
          onSwitchProject={mode === 'bridge' ? backToConnect : undefined}
          onReindex={
            mode === 'bridge' && currentProject !== null
              ? () => void openProject(currentProject, true)
              : undefined
          }
        />
        <FilterPanel
          nodeKindRows={nodeKindRows}
          edgeKindRows={edgeKindRows}
          originRows={originRows}
          hiddenNodeKinds={hiddenNodeKinds}
          hiddenEdgeKinds={hiddenEdgeKinds}
          hiddenOrigins={hiddenOrigins}
          minConfidence={minConfidence}
          focusDepth={focusDepth}
          colorMode={colorMode}
          hideIsolated={hideIsolated}
          dirPalette={dirPalette}
          onToggleNodeKind={(kind) => setHiddenNodeKinds((prev) => toggled(prev, kind))}
          onToggleEdgeKind={(kind) => setHiddenEdgeKinds((prev) => toggled(prev, kind))}
          onToggleOrigin={(origin) => setHiddenOrigins((prev) => toggled(prev, origin))}
          onOnlyNodeKind={(kind) =>
            setHiddenNodeKinds(new Set(nodeKindRows.map((r) => r.kind).filter((k) => k !== kind)))
          }
          onOnlyEdgeKind={(kind) =>
            setHiddenEdgeKinds(new Set(edgeKindRows.map((r) => r.kind).filter((k) => k !== kind)))
          }
          onAllNodeKinds={() => setHiddenNodeKinds(new Set())}
          onAllEdgeKinds={() => setHiddenEdgeKinds(new Set())}
          onResetNodeKinds={() => setHiddenNodeKinds(defaultHidden(DEFAULT_HIDDEN_NODE_KINDS))}
          onResetEdgeKinds={() => setHiddenEdgeKinds(defaultHidden(DEFAULT_HIDDEN_EDGE_KINDS))}
          onMinConfidence={setMinConfidence}
          onFocusDepth={setFocusDepth}
          onColorMode={setColorMode}
          onHideIsolated={setHideIsolated}
          clusterBy={clusterBy}
          onClusterBy={setClusterBy}
          minDegree={minDegree}
          maxDegree={degreeStats.max}
          onMinDegree={setMinDegree}
          hideHubs={hideHubs}
          onHideHubs={setHideHubs}
          dirFilter={dirFilter}
          onClearDirFilter={() => setDirFilter(null)}
        />
        {mode === 'bridge' && currentProject !== null ? (
          <AnalysisDock
            loading={analysisLoading}
            daemon={{
              onImpact: runImpact,
              impactReady: selectedNode !== null,
              onOrphans: runOrphans,
              onInconsistencies: runInconsistencies,
              onDiff: runDiff,
            }}
            orphanOptions={orphanOptions}
            onOrphanOptions={setOrphanOptions}
          />
        ) : null}
      </div>

      {(() => {
        // Fixed placement by KIND, never by situation: symbol details and
        // graph-overlay analyses (impact / dead code / path) always live in
        // the side panel; list-shaped analyses (inconsistencies / diff)
        // always open as a dialog. When a symbol is selected it takes the
        // single side slot; closing it brings the analysis panel back.
        const analysisIsDialog = analysis !== null && DIALOG_ANALYSES.has(analysis.kind)
        // The detail panel temporarily takes the analysis panel's slot; name
        // the way back so closing reads as "return", not "dismiss".
        const backTo =
          analysis === null || analysisIsDialog
            ? undefined
            : analysis.kind === 'orphans'
              ? 'dead code'
              : analysis.kind
        const analysisJump = (id: number): void => {
          const node = graph.nodeById.get(id)
          if (node !== undefined) selectNode(node, false)
        }
        return (
          <>
            {selectedNode !== null ? (
              <DetailPanel
                key={selectedNode.id}
                node={selectedNode}
                incident={incident}
                focusMode={focusOn}
                backTo={backTo}
                source={sourceSnippet}
                onImpact={
                  mode === 'bridge' && currentProject !== null && analysisLoading === null
                    ? runImpact
                    : undefined
                }
                onPathFrom={startPathFrom}
                onJump={(id) => {
                  const node = graph.nodeById.get(id)
                  if (node !== undefined) selectNode(node, focusOn)
                }}
                onIsolate={() => setFocusOn(true)}
                onExitIsolate={() => setFocusOn(false)}
                onClear={() => {
                  setFocusOn(false)
                  setSelectedId(null)
                }}
              />
            ) : analysis !== null && !analysisIsDialog ? (
              <AnalysisPanel
                analysis={analysis}
                resolveId={resolveId}
                onJump={analysisJump}
                onClose={() => setAnalysis(null)}
              />
            ) : null}
            {analysisIsDialog && analysisDialogOpen && analysis !== null ? (
              <Dialog
                wide
                onClose={() => {
                  setAnalysisDialogOpen(false)
                  setAnalysis(null)
                }}
              >
                <AnalysisPanel
                  analysis={analysis}
                  variant="dialog"
                  resolveId={resolveId}
                  onJump={analysisJump}
                  onClose={() => {
                    setAnalysisDialogOpen(false)
                    setAnalysis(null)
                  }}
                />
              </Dialog>
            ) : null}
          </>
        )
      })()}

      <StatusBar
        focusMode={focusOn && selectedNode !== null}
        focusName={selectedNode?.name ?? null}
        focusDepth={focusDepth}
        visibleNodes={visible.nodes.length}
        visibleLinks={visible.links.length}
        totalNodes={graph.nodes.length}
        totalEdges={graph.edges.length}
        daemonLive={daemonLive}
      />

      {overlays}
    </>
  )
}
