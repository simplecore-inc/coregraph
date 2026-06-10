import { useEffect, type ReactNode } from 'react'

interface DialogProps {
  onClose: () => void
  children: ReactNode
  /** Wider layout for table-like content (diff / inconsistencies). */
  wide?: boolean
}

/** Centered modal used for content that doesn't need the graph behind it. */
export function Dialog({ onClose, children, wide = false }: DialogProps) {
  // Escape closes the topmost dialog before the global chain sees the key.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        event.stopImmediatePropagation()
        onClose()
      }
    }
    // Capture phase so this wins over the app-level Escape handler.
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [onClose])

  return (
    <div
      className="dialog-overlay"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div className={wide ? 'dialog-card wide' : 'dialog-card'} role="dialog" aria-modal="true">
        {children}
      </div>
    </div>
  )
}
