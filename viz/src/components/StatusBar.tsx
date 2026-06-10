interface StatusBarProps {
  focusMode: boolean
  focusName: string | null
  focusDepth: number
  visibleNodes: number
  visibleLinks: number
  totalNodes: number
  totalEdges: number
  /** Live daemon counters for the open project (bridge mode). */
  daemonLive: { symbols: number; idle: number } | null
}

export function StatusBar({
  focusMode,
  focusName,
  focusDepth,
  visibleNodes,
  visibleLinks,
  totalNodes,
  totalEdges,
  daemonLive,
}: StatusBarProps) {
  return (
    <footer className="status-bar">
      {focusMode && focusName !== null ? (
        <span className="mode focus">◎ {focusName} ±{focusDepth}</span>
      ) : (
        <span className="mode">cosmos</span>
      )}
      <span className="counts">
        {visibleNodes.toLocaleString()}/{totalNodes.toLocaleString()} nodes ·{' '}
        {visibleLinks.toLocaleString()}/{totalEdges.toLocaleString()} edges
      </span>
      {daemonLive !== null ? (
        <span className="daemon-live" title="Daemon-side graph for this project">
          ● daemon {daemonLive.symbols.toLocaleString()} sym · idle {daemonLive.idle}s
        </span>
      ) : null}
      <span className="hints">/ search · click inspect · esc back · drag orbit · scroll zoom</span>
    </footer>
  )
}
