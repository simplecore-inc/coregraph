/** Normalize Windows separators so all path logic can assume "/". */
export function normalizeSlashes(path: string): string {
  return path.replace(/\\/g, '/')
}

/** Unix or Windows-drive absolute path (after normalizeSlashes). */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:\//.test(path)
}

/**
 * Longest common directory prefix of the given paths, ending with "/".
 * Returns "" when there is no shared directory (or no input).
 */
export function commonDirPrefix(paths: readonly string[]): string {
  if (paths.length === 0) return ''
  let prefix = normalizeSlashes(paths[0])
  // Trim to the containing directory of the first path.
  prefix = prefix.slice(0, prefix.lastIndexOf('/') + 1)
  for (let i = 1; i < paths.length && prefix !== ''; i++) {
    const p = normalizeSlashes(paths[i])
    while (prefix !== '' && !p.startsWith(prefix)) {
      if (prefix === '/') {
        // At the root: absolute paths share "/", anything else (relative or
        // empty) shares nothing. Either way this terminates the loop — the
        // bug that hung the viewer was slicing "/" to "/" forever here.
        prefix = p.startsWith('/') ? '/' : ''
        break
      }
      // Drop the last directory segment and retry.
      const cut = prefix.lastIndexOf('/', prefix.length - 2)
      prefix = cut < 0 ? '' : prefix.slice(0, cut + 1)
    }
  }
  return prefix
}

/**
 * First path segment, or "(root)" for files directly at the root. Tolerates a
 * leading "/" (when no common prefix was stripped) so absolute paths still
 * yield a real directory name rather than "".
 */
export function topSegment(rel: string): string {
  const s = rel.startsWith('/') ? rel.slice(1) : rel
  const idx = s.indexOf('/')
  if (s === '') return '(root)'
  return idx === -1 ? '(root)' : s.slice(0, idx)
}
