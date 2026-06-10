import { describe, expect, it } from 'vitest'
import { commonDirPrefix, isAbsolutePath, normalizeSlashes, topSegment } from './paths.ts'

describe('normalizeSlashes', () => {
  it('converts backslashes', () => {
    expect(normalizeSlashes('C:\\repo\\src\\a.rs')).toBe('C:/repo/src/a.rs')
  })
})

describe('commonDirPrefix', () => {
  it('returns empty for no paths', () => {
    expect(commonDirPrefix([])).toBe('')
  })

  it('finds the shared directory', () => {
    expect(
      commonDirPrefix(['/repo/src/a.rs', '/repo/src/sub/b.rs', '/repo/tests/c.rs']),
    ).toBe('/repo/')
  })

  it('uses the containing directory for a single path', () => {
    expect(commonDirPrefix(['/repo/src/a.rs'])).toBe('/repo/src/')
  })

  it('does not treat partial segment names as shared', () => {
    expect(commonDirPrefix(['/repo/srcfoo/a.rs', '/repo/src/b.rs'])).toBe('/repo/')
  })

  it('returns empty when nothing is shared', () => {
    expect(commonDirPrefix(['/x/a.rs', 'rel/b.rs'])).toBe('')
  })

  it('handles windows-style inputs', () => {
    expect(commonDirPrefix(['C:\\r\\src\\a.rs', 'C:\\r\\lib\\b.rs'])).toBe('C:/r/')
  })

  it('terminates on absolute paths mixed with empty paths', () => {
    // Regression: the daemon graph contains nodes with an empty file path;
    // narrowing past the root prefix "/" must end at "", not spin forever.
    expect(commonDirPrefix(['/Users/a/b.rs', '/Users/a/c.rs', ''])).toBe('')
  })

  it('terminates on absolute paths mixed with relative paths', () => {
    expect(commonDirPrefix(['/a/b/c.rs', '/a/d.rs', 'rel/x.rs'])).toBe('')
  })

  it('keeps the root "/" when absolute paths share only it', () => {
    expect(commonDirPrefix(['/x/a.rs', '/y/b.rs'])).toBe('/')
  })

  it('keeps the root for files directly under it', () => {
    expect(commonDirPrefix(['/a.rs', '/b.rs'])).toBe('/')
    expect(commonDirPrefix(['/', '/a.rs'])).toBe('/')
  })
})

describe('isAbsolutePath', () => {
  it('detects unix and windows-drive absolute paths', () => {
    expect(isAbsolutePath('/repo/a.rs')).toBe(true)
    expect(isAbsolutePath('C:/repo/a.rs')).toBe(true)
    expect(isAbsolutePath('../repo/a.rs')).toBe(false)
    expect(isAbsolutePath('repo/a.rs')).toBe(false)
  })
})

describe('topSegment', () => {
  it('returns the first directory', () => {
    expect(topSegment('crates/cli/src/main.rs')).toBe('crates')
  })

  it('marks root-level files', () => {
    expect(topSegment('Cargo.toml')).toBe('(root)')
  })

  it('skips a leading slash so absolute paths still yield a directory', () => {
    expect(topSegment('/crates/cli/main.rs')).toBe('crates')
    expect(topSegment('/main.rs')).toBe('(root)')
  })

  it('treats an empty path as root', () => {
    expect(topSegment('')).toBe('(root)')
  })
})
