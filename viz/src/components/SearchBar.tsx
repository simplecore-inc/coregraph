import {
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import type { NodeDatum } from '../lib/types.ts'
import { searchNodes } from '../lib/fuzzy.ts'
import { nodeKindColor } from '../lib/palette.ts'

interface SearchBarProps {
  nodes: readonly NodeDatum[]
  inputRef: RefObject<HTMLInputElement | null>
  onPick: (node: NodeDatum) => void
  /** Action buttons rendered to the right of the input (share/export). */
  actions?: ReactNode
}

const MAX_HITS = 16

export function SearchBar({ nodes, inputRef, onPick, actions }: SearchBarProps) {
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const deferredQuery = useDeferredValue(query)
  const blurTimer = useRef<number | undefined>(undefined)

  // Clear the pending blur-close timer so re-focusing keeps the list open.
  const cancelBlurClose = (): void => {
    window.clearTimeout(blurTimer.current)
    blurTimer.current = undefined
  }
  useEffect(() => cancelBlurClose, [])

  const hits = useMemo(
    () => searchNodes(nodes, deferredQuery, MAX_HITS),
    [nodes, deferredQuery],
  )

  const pick = (node: NodeDatum): void => {
    onPick(node)
    setQuery('')
    setOpen(false)
    inputRef.current?.blur()
  }

  const showList = open && query.trim() !== ''

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    // Ignore navigation/commit keys while an IME composition is active, so
    // confirming a Hangul/Japanese candidate with Enter doesn't pick a result.
    if (event.nativeEvent.isComposing) return
    if (event.key === 'Escape') {
      setQuery('')
      setOpen(false)
      inputRef.current?.blur()
      return
    }
    // Only act on list navigation when the list is actually visible.
    if (!showList) return
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setActive((i) => Math.min(i + 1, hits.length - 1))
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      setActive((i) => Math.max(i - 1, 0))
    } else if (event.key === 'Enter') {
      const hit = hits[Math.min(active, hits.length - 1)]
      if (hit !== undefined) pick(hit.node)
    }
  }

  return (
    <div className="search" role="search">
      <div className="search-box">
        <input
          ref={inputRef}
          className="search-input"
          type="text"
          placeholder="search symbols…"
          spellCheck={false}
          value={query}
          role="combobox"
          aria-expanded={showList}
          aria-controls="search-listbox"
          aria-label="Search symbols"
          onChange={(event) => {
            cancelBlurClose()
            setQuery(event.target.value)
            setActive(0)
            setOpen(true)
          }}
          onFocus={() => {
            cancelBlurClose()
            setOpen(true)
          }}
          onBlur={() => {
            // Delay so result clicks land before the list unmounts; cancelled
            // if the input is re-focused or edited before the timer fires.
            blurTimer.current = window.setTimeout(() => setOpen(false), 120)
          }}
          onKeyDown={handleKeyDown}
        />
        <kbd className="search-kbd">/</kbd>
      </div>
      {actions !== undefined ? <div className="search-actions">{actions}</div> : null}
      {showList ? (
        <ul className="search-results" id="search-listbox" role="listbox">
          {hits.length === 0 ? (
            <li className="search-empty">no matches</li>
          ) : (
            hits.map((hit, index) => (
              <li
                key={hit.node.id}
                role="option"
                aria-selected={index === active}
                className={index === active ? 'hit active' : 'hit'}
                onMouseDown={(event) => {
                  event.preventDefault()
                  pick(hit.node)
                }}
                onMouseEnter={() => setActive(index)}
              >
                <i className="dot" style={{ background: nodeKindColor(hit.node.kind) }} />
                <span className="hit-name">{hit.node.name}</span>
                <span className="hit-kind">{hit.node.kind}</span>
                <span className="hit-file">{hit.node.rel}</span>
              </li>
            ))
          )}
        </ul>
      ) : null}
    </div>
  )
}
