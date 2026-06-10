import { useSyncExternalStore } from 'react'

function subscribe(onChange: () => void): () => void {
  window.addEventListener('resize', onChange)
  return () => window.removeEventListener('resize', onChange)
}

/** Viewport size; the snapshot is cached so identical sizes don't re-render. */
let cached = { width: 0, height: 0 }

function getSnapshot(): { width: number; height: number } {
  if (cached.width !== window.innerWidth || cached.height !== window.innerHeight) {
    cached = { width: window.innerWidth, height: window.innerHeight }
  }
  return cached
}

export function useWindowSize(): { width: number; height: number } {
  return useSyncExternalStore(subscribe, getSnapshot)
}
