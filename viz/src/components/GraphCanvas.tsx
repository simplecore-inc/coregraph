import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import ForceGraph3D from 'react-force-graph-3d'
import { Mesh, MeshBasicMaterial, SphereGeometry } from 'three'
import SpriteText from 'three-spritetext'
import type { LinkDatum, NodeDatum } from '../lib/types.ts'
import { linkEndId } from '../lib/graph.ts'
import { edgeKindColor, nodeKindColor, withAlpha } from '../lib/palette.ts'
import { useWindowSize } from '../hooks/useWindowSize.ts'

export type ColorMode = 'kind' | 'dir' | 'unit'
export type ClusterBy = 'none' | 'unit'

interface GraphCanvasProps {
  nodes: NodeDatum[]
  links: LinkDatum[]
  /** Bumped when a new dataset is loaded, to re-fit the camera. */
  dataEpoch: number
  colorMode: ColorMode
  dirPalette: ReadonlyMap<string, string>
  /** node id → unit label (module/package/crate-level grouping). */
  unitOf: ReadonlyMap<number, string>
  unitPalette: ReadonlyMap<string, string>
  /** Spatially pull same-group nodes together (cluster force). */
  clusterBy: ClusterBy
  selectedId: number | null
  neighborSet: ReadonlySet<number>
  focusMode: boolean
  /** Analysis overlay: node id → color; everything else is dimmed hard. */
  highlight: ReadonlyMap<number, string> | null
  /** Extra synthetic links rendered on top (path segments, mismatch pairs). */
  extraLinks: LinkDatum[]
  onNodeClick: (id: number) => void
  onBackgroundClick: () => void
}

const BACKGROUND = '#05070d'
const SELECTED_COLOR = '#ffe9c7'
const DIM_NODE_ALPHA = 0.16
const DIM_LINK_ALPHA = 0.05
/** Hard dim for nodes outside an active analysis highlight. */
const ANALYSIS_DIM_ALPHA = 0.07
const PATH_COLOR = '#ffd166'
const PAIR_COLOR = '#ff6b9d'
/** Above this visible-node count, compute layout up front and render statically. */
const STATIC_LAYOUT_THRESHOLD = 2500
/** Label sprites are only affordable on small focused neighborhoods. */
const LABEL_LIMIT = 160
/** Cosmos mode: nodes closer to the camera than this get a proximity label. */
const NEAR_LABEL_DISTANCE = 420
/** Cosmos mode: at most this many nearest nodes are labeled at once. */
const NEAR_LABEL_LIMIT = 80
/** How often the proximity-label pass re-evaluates camera distances. */
const NEAR_LABEL_POLL_MS = 250
/** preserveDrawingBuffer lets the share/export button capture the canvas. */
const RENDERER_CONFIG = { preserveDrawingBuffer: true }

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** Shared label sprite styling (focus-mode labels and cosmos proximity labels). */
function makeLabelSprite(name: string, bold: boolean, textHeight = 3.2): SpriteText {
  const sprite = new SpriteText(name, textHeight, '#c9d6e2')
  sprite.fontFace = 'Martian Mono Variable, ui-monospace, monospace'
  sprite.fontWeight = bold ? '700' : '400'
  sprite.backgroundColor = 'rgba(5,7,13,0.7)'
  sprite.padding = 1.5
  sprite.borderRadius = 2
  // Always draw labels in front of the node spheres: depthTest off means
  // the sphere's depth never occludes the label, and a high renderOrder
  // makes labels paint after (on top of) the geometry.
  sprite.material.depthTest = false
  sprite.material.depthWrite = false
  sprite.renderOrder = 10
  sprite.position.set(0, 7, 0)
  return sprite
}

/** Detach a label sprite and free its canvas texture (GPU memory). */
function disposeSprite(sprite: SpriteText): void {
  sprite.parent?.remove(sprite)
  const material = sprite.material as unknown as {
    map?: { dispose: () => void }
    dispose: () => void
  }
  material.map?.dispose()
  material.dispose()
}

/** Engine-managed runtime fields on a node (positions + internal three group). */
type EngineNode = NodeDatum & {
  x?: number
  y?: number
  z?: number
  vx?: number
  vy?: number
  vz?: number
  __threeObj?: { add: (sprite: SpriteText) => void }
}

interface Anchor {
  x: number
  y: number
  z: number
}

/**
 * Golden-spiral points on a sphere: deterministic, evenly spread anchor
 * positions for the cluster groups.
 */
function sphereAnchors(count: number, radius: number): Anchor[] {
  const anchors: Anchor[] = []
  const golden = Math.PI * (3 - Math.sqrt(5))
  for (let i = 0; i < count; i++) {
    const y = count === 1 ? 0 : 1 - (i / (count - 1)) * 2
    const r = Math.sqrt(Math.max(0, 1 - y * y))
    const theta = golden * i
    anchors.push({
      x: Math.cos(theta) * r * radius,
      y: y * radius,
      z: Math.sin(theta) * r * radius,
    })
  }
  return anchors
}

/** One translucent boundary sphere + name label per cluster group. */
interface ClusterHull {
  mesh: Mesh<SphereGeometry, MeshBasicMaterial>
  label: SpriteText
}

function makeHull(color: string): ClusterHull {
  const material = new MeshBasicMaterial({
    color,
    transparent: true,
    opacity: 0.12,
    depthWrite: false,
  })
  // Unit sphere scaled per frame — geometry is shared-shape, never rebuilt.
  const mesh = new Mesh(new SphereGeometry(1, 24, 24), material)
  mesh.renderOrder = -1
  const label = makeLabelSprite('', false, 24)
  label.color = color
  label.backgroundColor = 'rgba(5,7,13,0.35)'
  return { mesh, label }
}

function disposeHull(hull: ClusterHull): void {
  hull.mesh.parent?.remove(hull.mesh)
  hull.mesh.geometry.dispose()
  hull.mesh.material.dispose()
  disposeSprite(hull.label)
}

/**
 * d3-force-compatible custom force pulling each node toward its group anchor.
 * Velocity-based like forceX/Y/Z, so it composes with link/charge forces.
 */
function makeClusterForce(
  anchorOf: (node: EngineNode) => Anchor | undefined,
  strength: number,
): { (alpha: number): void; initialize: (nodes: EngineNode[]) => void } {
  let nodes: EngineNode[] = []
  const force = (alpha: number): void => {
    const k = strength * alpha
    for (const node of nodes) {
      const anchor = anchorOf(node)
      if (anchor === undefined || node.x === undefined) continue
      node.vx = (node.vx ?? 0) + (anchor.x - node.x) * k
      node.vy = (node.vy ?? 0) + (anchor.y - (node.y ?? 0)) * k
      node.vz = (node.vz ?? 0) + (anchor.z - (node.z ?? 0)) * k
    }
  }
  force.initialize = (initialNodes: EngineNode[]): void => {
    nodes = initialNodes
  }
  return force
}

export function GraphCanvas({
  nodes,
  links,
  dataEpoch,
  colorMode,
  dirPalette,
  unitOf,
  unitPalette,
  clusterBy,
  selectedId,
  neighborSet,
  focusMode,
  highlight,
  extraLinks,
  onNodeClick,
  onBackgroundClick,
}: GraphCanvasProps) {
  // The library's ref API (cameraPosition, controls, zoomToFit) has no
  // published instance type that survives strict generics, so keep it loose.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const fgRef = useRef<any>(null)
  const { width, height } = useWindowSize()
  const [interacted, setInteracted] = useState(false)
  // Cluster toggles must animate even on static-layout graphs: this flag
  // temporarily re-enables the cooldown loop until the engine settles.
  const [clusterTransition, setClusterTransition] = useState(false)
  /** Live boundary spheres per cluster group (imperative three objects). */
  const hullsRef = useRef(new Map<string, ClusterHull>())
  /** Group accessor + palette for the hull updater; null = hulls off. */
  const hullSpec = useRef<{
    keyOf: (node: EngineNode) => string
    colorOf: (group: string) => string
  } | null>(null)
  const lastHullUpdate = useRef(0)

  /** Recompute hull centers/radii from current node positions. */
  const updateHulls = useCallback((force = false) => {
    const fg = fgRef.current
    const spec = hullSpec.current
    if (fg === null) return
    const scene = fg.scene() as { add: (obj: Mesh) => void }
    const hulls = hullsRef.current
    if (spec === null) {
      if (hulls.size > 0) {
        for (const hull of hulls.values()) disposeHull(hull)
        hulls.clear()
      }
      return
    }
    const now = Date.now()
    if (!force && now - lastHullUpdate.current < 120) return
    lastHullUpdate.current = now

    interface Acc {
      n: number
      x: number
      y: number
      z: number
    }
    const accumulator = new Map<string, Acc>()
    for (const node of nodes as EngineNode[]) {
      if (node.x === undefined) continue
      const key = spec.keyOf(node)
      let acc = accumulator.get(key)
      if (acc === undefined) accumulator.set(key, (acc = { n: 0, x: 0, y: 0, z: 0 }))
      acc.n++
      acc.x += node.x
      acc.y += node.y ?? 0
      acc.z += node.z ?? 0
    }
    // Second pass: radius = p90-ish via mean distance * 1.6 (cheap, stable).
    const radius = new Map<string, number>()
    for (const node of nodes as EngineNode[]) {
      if (node.x === undefined) continue
      const key = spec.keyOf(node)
      const acc = accumulator.get(key)
      if (acc === undefined || acc.n === 0) continue
      const cx = acc.x / acc.n
      const cy = acc.y / acc.n
      const cz = acc.z / acc.n
      const dist = Math.hypot(node.x - cx, (node.y ?? 0) - cy, (node.z ?? 0) - cz)
      radius.set(key, (radius.get(key) ?? 0) + dist)
    }
    for (const [key, hull] of hulls) {
      if (!accumulator.has(key)) {
        disposeHull(hull)
        hulls.delete(key)
      }
    }
    for (const [key, acc] of accumulator) {
      if (acc.n === 0) continue
      let hull = hulls.get(key)
      if (hull === undefined) {
        hull = makeHull(spec.colorOf(key))
        hull.label.text = key
        scene.add(hull.mesh)
        scene.add(hull.label as unknown as Mesh)
        hulls.set(key, hull)
      }
      const cx = acc.x / acc.n
      const cy = acc.y / acc.n
      const cz = acc.z / acc.n
      const r = Math.max(36, ((radius.get(key) ?? 0) / acc.n) * 1.7 + 22)
      hull.mesh.position.set(cx, cy, cz)
      hull.mesh.scale.setScalar(r)
      hull.label.position.set(cx, cy + r + 16, cz)
    }
  }, [nodes])
  const fittedEpoch = useRef(-1)
  // Set when focus is (re)entered so the next settled layout re-frames the
  // whole ego-network instead of leaving the camera parked on one node.
  const pendingFit = useRef(false)
  // Throttle marker + last fitted span for the follow-fit that tracks the
  // settling layout (span gate stops the fit once growth has converged).
  const lastFollowFit = useRef(0)
  const lastFollowSpan = useRef(0)

  const graphData = useMemo(
    () => ({ nodes, links: extraLinks.length === 0 ? links : [...links, ...extraLinks] }),
    [nodes, links, extraLinks],
  )

  const nodeById = useMemo(() => {
    const index = new Map<number, NodeDatum>()
    for (const node of nodes) index.set(node.id, node)
    return index
  }, [nodes])

  // A new dataset means the camera is ours to manage again (a dropped file
  // swaps the data without remounting this component).
  useEffect(() => {
    setInteracted(false)
    lastFollowSpan.current = 0
  }, [dataEpoch])

  const baseColor = useCallback(
    (node: NodeDatum): string => {
      if (colorMode === 'kind') return nodeKindColor(node.kind)
      if (colorMode === 'unit') {
        return unitPalette.get(unitOf.get(node.id) ?? node.dir) ?? '#8899aa'
      }
      return dirPalette.get(node.dir) ?? '#8899aa'
    },
    [colorMode, dirPalette, unitOf, unitPalette],
  )

  const nodeColor = useCallback(
    (node: NodeDatum): string => {
      if (node.id === selectedId) return SELECTED_COLOR
      // An active analysis paints its own palette; everything else recedes.
      if (highlight !== null) {
        return highlight.get(node.id) ?? withAlpha(baseColor(node), ANALYSIS_DIM_ALPHA)
      }
      if (selectedId === null) return baseColor(node)
      const base = baseColor(node)
      return neighborSet.has(node.id) ? base : withAlpha(base, DIM_NODE_ALPHA)
    },
    [baseColor, selectedId, neighborSet, highlight],
  )

  const isIncident = useCallback(
    (link: LinkDatum): boolean =>
      selectedId !== null &&
      (linkEndId(link.source) === selectedId || linkEndId(link.target) === selectedId),
    [selectedId],
  )

  const clusterKeyOf = useCallback(
    (id: number): string | null => {
      if (clusterBy === 'none') return null
      const node = nodeById.get(id)
      if (node === undefined) return null
      return unitOf.get(node.id) ?? node.dir
    },
    [clusterBy, unitOf, nodeById],
  )

  const linkColor = useCallback(
    (link: LinkDatum): string => {
      if (link.kind === '__path') return PATH_COLOR
      if (link.kind === '__pair') return PAIR_COLOR
      const base = edgeKindColor(link.kind)
      if (highlight !== null) {
        const lit =
          highlight.has(linkEndId(link.source)) && highlight.has(linkEndId(link.target))
        return lit ? base : withAlpha(base, ANALYSIS_DIM_ALPHA)
      }
      if (selectedId !== null) {
        return isIncident(link) ? base : withAlpha(base, DIM_LINK_ALPHA)
      }
      // Clustered view: fade cross-cluster edges so the group separation
      // reads clearly; intra-cluster edges keep their full color. Keep the
      // fade mild — cross links are most of the long lines on screen, so a
      // hard fade reads as "all edges got thinner".
      if (clusterBy !== 'none') {
        const a = clusterKeyOf(linkEndId(link.source))
        const b = clusterKeyOf(linkEndId(link.target))
        if (a !== null && b !== null && a !== b) return withAlpha(base, 0.25)
      }
      return base
    },
    [selectedId, isIncident, highlight, clusterBy, clusterKeyOf],
  )

  const linkWidth = useCallback(
    (link: LinkDatum): number => {
      if (link.kind === '__path') return 2.6
      if (link.kind === '__pair') return 2
      return isIncident(link) ? 1.4 : 0
    },
    [isIncident],
  )

  const linkParticles = useCallback(
    (link: LinkDatum): number => {
      if (link.kind === '__path') return 4
      if (link.kind === '__pair') return 2
      return isIncident(link) ? 2 : 0
    },
    [isIncident],
  )

  const nodeVal = useCallback((node: NodeDatum): number => 1 + Math.min(node.deg, 60) * 0.3, [])

  const nodeLabel = useCallback(
    (node: NodeDatum): string =>
      `<div class="fg-tip"><b>${escapeHtml(node.name)}</b>` +
      `<span>${escapeHtml(node.kind)} · ${node.degIn}↓ ${node.degOut}↑</span>` +
      `<i>${escapeHtml(node.rel)}</i></div>`,
    [],
  )

  // Label sprites: focus mode on small neighborhoods. The
  // accessor is attached conditionally so cosmos mode never pays the
  // per-node object cost.
  const labeled = focusMode && nodes.length <= LABEL_LIMIT
  const nodeThreeObject = useCallback(
    (node: NodeDatum) =>
      makeLabelSprite(node.name, node.id === selectedId),
    [selectedId],
  )

  // Cosmos mode: proximity labels. A periodic pass lazily attaches a label
  // sprite to the nearest nodes within camera range and detaches it (freeing
  // the canvas texture) once the camera moves away — zooming into a cluster
  // names what you are looking at without paying a sprite per node up front.
  useEffect(() => {
    if (focusMode) return
    const sprites = new Map<number, SpriteText>()
    const timer = window.setInterval(() => {
      const fg = fgRef.current
      if (fg === null) return
      const camera = fg.camera() as { position: { x: number; y: number; z: number } }
      const near: { node: EngineNode; dist: number }[] = []
      for (const node of nodes as EngineNode[]) {
        if (node.x === undefined || node.y === undefined || node.z === undefined) continue
        const dist = Math.hypot(
          node.x - camera.position.x,
          node.y - camera.position.y,
          node.z - camera.position.z,
        )
        if (dist < NEAR_LABEL_DISTANCE) near.push({ node, dist })
      }
      near.sort((a, b) => a.dist - b.dist)
      const keep = new Set<number>()
      for (const { node } of near.slice(0, NEAR_LABEL_LIMIT)) keep.add(node.id)
      for (const [id, sprite] of sprites) {
        if (!keep.has(id)) {
          disposeSprite(sprite)
          sprites.delete(id)
        }
      }
      for (const { node } of near.slice(0, NEAR_LABEL_LIMIT)) {
        if (sprites.has(node.id)) continue
        const group = node.__threeObj
        if (group === undefined) continue
        // Slightly larger than focus labels: cosmos labels are read from
        // farther away, where perspective shrinks them.
        const sprite = makeLabelSprite(node.name, false, 4.2)
        group.add(sprite)
        sprites.set(node.id, sprite)
      }
    }, NEAR_LABEL_POLL_MS)
    return () => {
      window.clearInterval(timer)
      for (const sprite of sprites.values()) disposeSprite(sprite)
    }
  }, [focusMode, nodes])

  // Widen the usable zoom range. react-force-graph defaults (camera.near 0.1,
  // controls.minDistance 0.1, zoomSpeed 1.2) make the multiplicative trackball
  // dolly feel capped on a tight ego-network: lower the near plane / min
  // distance so the camera can get close, and speed the wheel up. Idempotent —
  // controls are created once at mount but re-applied defensively.
  const tuneControls = useCallback(() => {
    const fg = fgRef.current
    if (fg === null) return
    const cam = fg.camera() as { near: number; updateProjectionMatrix: () => void }
    if (cam.near !== 0.05) {
      cam.near = 0.05
      cam.updateProjectionMatrix()
    }
    const controls = fg.controls() as { minDistance: number; zoomSpeed: number }
    controls.minDistance = 0.2
    controls.zoomSpeed = 2
  }, [])

  useEffect(() => {
    tuneControls()
  }, [tuneControls])

  // While the animated cosmos layout settles, keep the camera tracking the
  // expanding graph with throttled re-fits. Without this the view sits on the
  // initial camera for ~15s and then snaps to a full fit in one jump when the
  // engine stops — perceived as a sudden zoom-out after load. The fit is
  // gated on the graph's bounding span actually CHANGING: re-fitting on a
  // timer alone chases the end-of-settle jitter and makes the view breathe
  // (grow/shrink repeatedly) until the simulation dies down.
  const handleEngineTick = useCallback(() => {
    updateHulls()
    if (focusMode || interacted) return
    const fg = fgRef.current
    if (fg === null) return
    const now = Date.now()
    if (now - lastFollowFit.current < 400) return
    const bbox = fg.getGraphBbox() as
      | { x: [number, number]; y: [number, number]; z: [number, number] }
      | null
    if (bbox === null) return
    const span = Math.max(
      bbox.x[1] - bbox.x[0],
      bbox.y[1] - bbox.y[0],
      bbox.z[1] - bbox.z[0],
    )
    const previous = lastFollowSpan.current
    if (previous > 0 && Math.abs(span - previous) / previous < 0.08) return
    lastFollowFit.current = now
    lastFollowSpan.current = span
    fg.zoomToFit(350, 48)
  }, [focusMode, interacted, updateHulls])

  // After a layout settles: in focus mode, frame the whole ego-network (so the
  // camera sits at a distance matched to the cluster, not parked on one node);
  // in cosmos mode, fit once per dataset — but NEVER once the user has taken
  // the camera. The animated cosmos layout settles ~15s after load, and a
  // re-fit at that point would yank a camera the user already zoomed/orbited.
  const handleEngineStop = useCallback(() => {
    const fg = fgRef.current
    if (fg === null) return
    tuneControls()
    setClusterTransition(false)
    updateHulls(true)
    if (pendingFit.current) {
      pendingFit.current = false
      fg.zoomToFit(700, 70)
      return
    }
    if (focusMode) return
    if (fittedEpoch.current === dataEpoch) return
    fittedEpoch.current = dataEpoch
    if (!interacted) fg.zoomToFit(800, 48)
  }, [dataEpoch, focusMode, tuneControls, interacted, updateHulls])

  // Queue a re-fit whenever focus is entered, the focused symbol changes, or
  // the module overview toggles; the fit itself runs in handleEngineStop once
  // the new layout settles.
  useEffect(() => {
    if (focusMode) pendingFit.current = true
  }, [focusMode, selectedId])

  // Cluster force: pull each node toward its group's sphere anchor.
  // The effect must be a strict no-op until clustering is actually used:
  // touching the simulation (even a reheat) right after mount, before the
  // engine has applied graphData, freezes the initial layout.
  const clusterApplied = useRef(false)
  /** Original link-force strength accessor, restored when clustering ends. */
  const savedLinkStrength = useRef<unknown>(null)
  useEffect(() => {
    const fg = fgRef.current
    if (fg === null) return
    const linkForce = fg.d3Force('link') as {
      strength: (s?: unknown) => unknown
    } | null
    if (clusterBy === 'none') {
      hullSpec.current = null
      updateHulls(true)
      if (clusterApplied.current) {
        clusterApplied.current = false
        fg.d3Force('cluster', null)
        if (linkForce !== null && savedLinkStrength.current !== null) {
          linkForce.strength(savedLinkStrength.current)
          savedLinkStrength.current = null
        }
        setClusterTransition(true)
        fg.d3ReheatSimulation()
      }
      return
    }
    const keyOf = (node: NodeDatum): string => unitOf.get(node.id) ?? node.dir
    const groups = [...new Set(nodes.map(keyOf))].sort()
    // With cross-cluster links zeroed out, the anchor radius IS the
    // cluster spacing — size it so neighboring groups sit close without
    // overlapping (golden-spiral neighbor gap ~ r·√(4π/n)).
    const radius = Math.max(360, 95 * Math.sqrt(groups.length) + 60)
    const anchors = sphereAnchors(groups.length, radius)
    const anchorByGroup = new Map(groups.map((group, index) => [group, anchors[index]]))
    fg.d3Force(
      'cluster',
      makeClusterForce((node) => anchorByGroup.get(keyOf(node)), 0.32),
    )
    // Cross-cluster links are what fold the groups back into one ball — a
    // hub with hundreds of edges overpowers the anchor force even at a low
    // uniform strength. Zero out BETWEEN-group links only; within-group
    // links keep their original strength so each cluster stays cohesive.
    if (linkForce !== null) {
      if (savedLinkStrength.current === null) {
        savedLinkStrength.current = linkForce.strength()
      }
      const original = savedLinkStrength.current
      const unitOfEnd = (end: LinkDatum['source']): string | null => {
        const node = nodeById.get(linkEndId(end))
        return node === undefined ? null : (unitOf.get(node.id) ?? node.dir)
      }
      linkForce.strength((link: LinkDatum, index: number, links: LinkDatum[]) => {
        const a = unitOfEnd(link.source)
        const b = unitOfEnd(link.target)
        if (a !== null && b !== null && a !== b) return 0.001
        return typeof original === 'function'
          ? (original as (l: LinkDatum, i: number, ls: LinkDatum[]) => number)(
              link,
              index,
              links,
            )
          : Number(original)
      })
    }
    // Translucent boundary spheres + group labels, tracked every few ticks.
    const palette = unitPalette
    hullSpec.current = {
      keyOf,
      colorOf: (group) => palette.get(group) ?? '#8899aa',
    }
    updateHulls(true)
    clusterApplied.current = true
    setClusterTransition(true)
    fg.d3ReheatSimulation()
  }, [clusterBy, unitOf, nodes, nodeById, unitPalette, dirPalette, updateHulls])

  // Hulls follow the simulation while it runs, and are torn down on unmount.
  useEffect(() => {
    return () => {
      hullSpec.current = null
      for (const hull of hullsRef.current.values()) disposeHull(hull)
      hullsRef.current.clear()
    }
  }, [])

  // Slow idle orbit until the user takes over; off in focus mode.
  useEffect(() => {
    const fg = fgRef.current
    if (fg === null) return
    const controls = fg.controls() as { autoRotate: boolean; autoRotateSpeed: number }
    controls.autoRotate = !interacted && !focusMode
    controls.autoRotateSpeed = 0.4
  }, [interacted, focusMode])

  // Fly the camera to a node selected in cosmos mode. Focus mode is handled by
  // the ego-network zoomToFit above, so single-node parking is skipped there.
  useEffect(() => {
    const fg = fgRef.current
    if (fg === null || selectedId === null || focusMode) return
    const node = nodes.find((n) => n.id === selectedId) as
      | (NodeDatum & { x?: number; y?: number; z?: number })
      | undefined
    if (node?.x === undefined || node.y === undefined || node.z === undefined) return
    const distance = 320
    const len = Math.hypot(node.x, node.y, node.z) || 1
    const ratio = 1 + distance / len
    fg.cameraPosition(
      { x: node.x * ratio, y: node.y * ratio, z: node.z * ratio },
      { x: node.x, y: node.y, z: node.z },
      900,
    )
    // Position values are mutated in place by the engine; nodes identity is the trigger.
  }, [selectedId, focusMode, nodes])

  // Compute the layout up front and render it statically (warmup ticks, no
  // cooldown) for big graphs, focus mode and the module overview. In focus
  // mode this is what makes isolating a symbol settle instantly: otherwise
  // the simulation keeps animating on-screen for seconds (cooldownTicks
  // Infinity), the camera waits for it to stop before re-framing, and zoom
  // feels dead until then.
  const staticLayout = nodes.length > STATIC_LAYOUT_THRESHOLD || focusMode

  const handleNodeClick = useCallback(
    (node: NodeDatum) => onNodeClick(node.id),
    [onNodeClick],
  )


  return (
    <div
      className="canvas-layer"
      // Wheel zoom must count as taking the camera too — without it, the
      // settle-time auto-fit would stomp a wheel-only user's zoom level.
      onPointerDown={() => setInteracted(true)}
      onWheel={() => setInteracted(true)}
    >
      <ForceGraph3D
        ref={fgRef}
        width={width}
        height={height}
        backgroundColor={BACKGROUND}
        graphData={graphData}
        nodeId="id"
        nodeVal={nodeVal}
        nodeColor={nodeColor}
        nodeLabel={nodeLabel}
        nodeResolution={10}
        nodeOpacity={0.92}
        nodeThreeObject={labeled ? nodeThreeObject : undefined}
        nodeThreeObjectExtend={true}
        linkColor={linkColor}
        linkOpacity={0.55}
        linkWidth={linkWidth}
        linkDirectionalParticles={linkParticles}
        linkDirectionalParticleWidth={1.6}
        linkDirectionalParticleSpeed={0.006}
        linkDirectionalArrowLength={focusMode ? 3.2 : 0}
        linkDirectionalArrowRelPos={0.96}
        warmupTicks={staticLayout ? 80 : 0}
        cooldownTicks={staticLayout && !clusterTransition ? 0 : Infinity}
        onEngineTick={handleEngineTick}
        onEngineStop={handleEngineStop}
        rendererConfig={RENDERER_CONFIG}
        enableNodeDrag={false}
        showNavInfo={false}
        onNodeClick={handleNodeClick}
        onBackgroundClick={onBackgroundClick}
      />
    </div>
  )
}
