#!/usr/bin/env node
// coregraph atlas bridge — serves the built single-file viewer and proxies
// graph requests to the coregraph daemon over its local IPC socket.
//
//   node server.mjs [--port N] [--no-open] [--bin path/to/coregraph]
//
// Endpoints (same-origin only; the server binds to 127.0.0.1):
//   GET  /                  the built viewer (dist/index.html)
//   GET  /api/status        daemon liveness + loaded-project list
//   POST /api/daemon/start  spawn the daemon if it is not running
//   POST /api/graph         {project, minConfidence?} → json-graph document
//   GET  /api/resolve?path= validate/canonicalize a project path
import { spawn, spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { existsSync, readFileSync, realpathSync, statSync } from 'node:fs'
import { createServer } from 'node:http'
import { createConnection } from 'node:net'
import { homedir, userInfo } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const htmlPath = join(here, 'dist', 'index.html')

const args = process.argv.slice(2)
function argValue(flag) {
  const idx = args.indexOf(flag)
  return idx !== -1 && idx + 1 < args.length ? args[idx + 1] : null
}
const PORT = Number(argValue('--port') ?? process.env.COREGRAPH_VIZ_PORT ?? 7321)
const BIN = argValue('--bin') ?? process.env.COREGRAPH_BIN ?? 'coregraph'
const NO_OPEN = args.includes('--no-open')

/** Status requests are quick; graph extraction may index a whole repo. */
const STATUS_TIMEOUT_MS = 5_000
const GRAPH_TIMEOUT_MS = 10 * 60_000
const DAEMON_START_TIMEOUT_MS = 30_000

// CSRF / DNS-rebinding hardening: only loopback hosts are served, cross-origin
// requests are rejected, and every /api/* call must echo a per-process token
// that is injected into the served HTML (so only pages we served hold it).
const TOKEN = randomBytes(16).toString('hex')
const ALLOWED_HOSTS = new Set([`127.0.0.1:${PORT}`, `localhost:${PORT}`, `[::1]:${PORT}`])

function socketPath() {
  if (process.platform === 'win32') {
    return `\\\\.\\pipe\\coregraph-${process.env.USERNAME ?? userInfo().username}`
  }
  if (process.env.XDG_RUNTIME_DIR) {
    return join(process.env.XDG_RUNTIME_DIR, 'coregraph', 'server.sock')
  }
  return join(homedir(), '.coregraph', 'server.sock')
}

class DaemonDownError extends Error {
  constructor(cause) {
    super(`daemon not reachable: ${cause}`)
    this.name = 'DaemonDownError'
  }
}

class DaemonError extends Error {
  constructor(message) {
    super(message)
    this.name = 'DaemonError'
  }
}

/** Bad client request — mapped to HTTP 4xx (status carried on the error). */
class RequestError extends Error {
  constructor(message, status = 400) {
    super(message)
    this.name = 'RequestError'
    this.status = status
  }
}

/** One IPC round-trip: newline-delimited JSON over the local socket. */
function ipcRequest(method, params, project, timeoutMs) {
  return new Promise((resolvePromise, rejectPromise) => {
    const socket = createConnection(socketPath())
    // StringDecoder buffers partial multibyte UTF-8 code points across chunk
    // boundaries; a naive per-chunk toString('utf8') would corrupt non-ASCII
    // identifiers in large (multi-MB) responses split across ~100 chunks.
    socket.setEncoding('utf8')
    let buffer = ''
    let settled = false
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true
        socket.destroy()
        rejectPromise(new DaemonError(`${method} timed out after ${timeoutMs}ms`))
      }
    }, timeoutMs)
    const finish = (fn, value) => {
      if (!settled) {
        settled = true
        clearTimeout(timer)
        socket.destroy()
        fn(value)
      }
    }
    socket.on('connect', () => {
      socket.write(`${JSON.stringify({ method, params, project })}\n`)
    })
    socket.on('data', (chunk) => {
      buffer += chunk
      const nl = buffer.indexOf('\n')
      if (nl === -1) return
      try {
        const envelope = JSON.parse(buffer.slice(0, nl))
        if (envelope.ok) finish(resolvePromise, envelope.body)
        else finish(rejectPromise, new DaemonError(envelope.error ?? 'daemon error'))
      } catch (error) {
        finish(rejectPromise, new DaemonError(`bad daemon response: ${error.message}`))
      }
    })
    // A clean close before any newline-terminated line means the daemon died
    // mid-response (crash, `server stop`, or a concurrent restart). Without
    // this the promise would hang until the (10-minute) timeout fires.
    socket.on('close', () => {
      finish(rejectPromise, new DaemonDownError('connection closed before response'))
    })
    socket.on('error', (error) => {
      if (error.code === 'ENOENT' || error.code === 'ECONNREFUSED') {
        finish(rejectPromise, new DaemonDownError(error.code))
      } else {
        finish(rejectPromise, error)
      }
    })
  })
}

async function daemonStatus() {
  try {
    const body = await ipcRequest('status', null, '', STATUS_TIMEOUT_MS)
    let manager
    try {
      manager = JSON.parse(body)
    } catch (error) {
      // A malformed status body is a daemon fault, not a bad client request —
      // tag it DaemonError so the handler reports 502, not 400.
      throw new DaemonError(`malformed daemon status: ${error.message}`)
    }
    return { running: true, manager }
  } catch (error) {
    if (error instanceof DaemonDownError) return { running: false, manager: null }
    throw error
  }
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms))
}

/** In-flight daemon spawn, shared so concurrent requests start it only once. */
let daemonStarting = null

async function spawnDaemonAndPoll(project) {
  const child = spawn(BIN, ['server', 'start', '-C', project], {
    detached: true,
    stdio: 'ignore',
  })
  child.on('error', () => {
    // Surfaced via the poll timeout below; spawn errors (ENOENT on the
    // binary) leave the socket dead and the loop reports it.
  })
  child.unref()
  const deadline = Date.now() + DAEMON_START_TIMEOUT_MS
  while (Date.now() < deadline) {
    await sleep(400)
    const status = await daemonStatus()
    if (status.running) return { ...status, started: true }
  }
  throw new DaemonError(
    `daemon did not come up within ${DAEMON_START_TIMEOUT_MS / 1000}s — ` +
      `is "${BIN}" on PATH? (override with COREGRAPH_BIN)`,
  )
}

/**
 * Spawn `coregraph server start` detached and wait until the socket answers.
 * Duplicate concurrent calls share one spawn (single-flight): there is only
 * one daemon per user, so a second `server start` would be wasted work.
 */
async function ensureDaemon(project) {
  const first = await daemonStatus()
  if (first.running) return { ...first, started: false }
  if (daemonStarting === null) {
    daemonStarting = spawnDaemonAndPoll(project).finally(() => {
      daemonStarting = null
    })
  }
  return daemonStarting
}

/** In-flight daemon restart, shared so concurrent retries restart only once. */
let daemonRestarting = null

function spawnRestart(project) {
  return new Promise((resolvePromise, rejectPromise) => {
    // Async spawn (not spawnSync) so the up-to-30s restart never blocks the
    // event loop — other HTTP requests and status polls keep flowing.
    const child = spawn(BIN, ['server', 'restart', '-C', project], { stdio: 'ignore' })
    const timer = setTimeout(() => {
      child.kill('SIGKILL')
      rejectPromise(new DaemonError('daemon restart timed out'))
    }, DAEMON_START_TIMEOUT_MS)
    child.on('error', (error) => {
      clearTimeout(timer)
      rejectPromise(new DaemonError(`daemon restart failed: ${error.message}`))
    })
    child.on('exit', (code) => {
      clearTimeout(timer)
      if (code === 0) resolvePromise()
      else rejectPromise(new DaemonError(`daemon restart exited with code ${code}`))
    })
  })
}

/**
 * Restart the daemon when the running one predates the export_graph method.
 * Single-flighted: with two projects open, both /api/graph retries share one
 * restart instead of the second killing the daemon under the first's retry.
 */
function restartDaemon(project) {
  if (daemonRestarting === null) {
    daemonRestarting = spawnRestart(project).finally(() => {
      daemonRestarting = null
    })
  }
  return daemonRestarting
}

async function fetchGraphFresh(project, minConfidence) {
  await ensureDaemon(project)
  const params = { min_confidence: minConfidence }
  try {
    return await ipcRequest('export_graph', params, project, GRAPH_TIMEOUT_MS)
  } catch (error) {
    if (error instanceof DaemonError && error.message.includes('unknown method')) {
      // A daemon from an older binary is still serving the socket; restart it
      // with the current binary and retry once.
      await restartDaemon(project)
      await ensureDaemon(project)
      return await ipcRequest('export_graph', params, project, GRAPH_TIMEOUT_MS)
    }
    throw error
  }
}

/** In-flight graph extractions, keyed by project+confidence (single-flight). */
const inflightGraphs = new Map()

/**
 * Deduplicate concurrent identical /api/graph requests: a double-clicked
 * "open" or a re-sent request while a long first indexing runs joins the
 * existing extraction instead of racing a second one.
 */
function fetchGraph(project, minConfidence) {
  const key = `${project}\u0000${minConfidence}`
  const existing = inflightGraphs.get(key)
  if (existing !== undefined) return existing
  const promise = fetchGraphFresh(project, minConfidence).finally(() => {
    inflightGraphs.delete(key)
  })
  inflightGraphs.set(key, promise)
  return promise
}

function resolveProjectPath(input) {
  let p = input.trim()
  if (p === '') return { ok: false, error: 'empty path' }
  if (p === '~' || p.startsWith('~/')) p = join(homedir(), p.slice(1))
  p = resolve(p)
  if (!existsSync(p)) return { ok: false, error: `path does not exist: ${p}` }
  if (!statSync(p).isDirectory()) return { ok: false, error: `not a directory: ${p}` }
  return { ok: true, path: realpathSync(p) }
}

function sendJson(res, code, value) {
  const body = typeof value === 'string' ? value : JSON.stringify(value)
  res.writeHead(code, { 'Content-Type': 'application/json; charset=utf-8' })
  res.end(body)
}

function readBody(req, limit = 1_048_576) {
  return new Promise((resolvePromise, rejectPromise) => {
    let size = 0
    let aborted = false
    const chunks = []
    req.on('data', (chunk) => {
      if (aborted) return
      size += chunk.length
      if (size > limit) {
        // Stop consuming but leave the socket open so the outer handler can
        // still write a 413 response (destroying it here would race the reply).
        aborted = true
        req.pause()
        rejectPromise(new RequestError('request body too large', 413))
        return
      }
      chunks.push(chunk)
    })
    req.on('end', () => {
      if (!aborted) resolvePromise(Buffer.concat(chunks).toString('utf8'))
    })
    req.on('error', rejectPromise)
  })
}

/** Read + JSON-parse a request body, returning a plain object (4xx otherwise). */
async function readJsonObject(req) {
  const text = (await readBody(req)) || '{}'
  let value
  try {
    value = JSON.parse(text)
  } catch (error) {
    throw new RequestError(`invalid JSON body: ${error.message}`)
  }
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new RequestError('body must be a JSON object')
  }
  return value
}

const server = createServer(async (req, res) => {
  // Reject DNS-rebound hosts and cross-origin requests BEFORE parsing the URL:
  // an empty or space-containing Host header makes `new URL` throw, and a throw
  // here (outside the try below) would crash the whole process on one bad
  // request. The allowlist check needs only the raw header.
  if (!ALLOWED_HOSTS.has(req.headers.host ?? '')) {
    res.writeHead(403, { 'Content-Type': 'text/plain' })
    res.end('forbidden: bad host')
    return
  }
  const origin = req.headers.origin
  if (typeof origin === 'string') {
    let originHost = null
    try {
      originHost = new URL(origin).host
    } catch {
      // Malformed Origin header — treat as cross-origin below.
    }
    if (originHost !== req.headers.host) {
      res.writeHead(403, { 'Content-Type': 'text/plain' })
      res.end('forbidden: bad origin')
      return
    }
  }

  // Host is in the loopback allowlist, so the base is always a valid URL now.
  const url = new URL(req.url ?? '/', `http://${req.headers.host}`)

  if (url.pathname.startsWith('/api/') && req.headers['x-bridge-token'] !== TOKEN) {
    res.writeHead(403, { 'Content-Type': 'text/plain' })
    res.end('forbidden: missing bridge token')
    return
  }

  try {
    if (req.method === 'GET' && (url.pathname === '/' || url.pathname === '/index.html')) {
      if (!existsSync(htmlPath)) {
        res.writeHead(500, { 'Content-Type': 'text/plain' })
        res.end('viewer not built — run `npm run build` in viz/ first')
        return
      }
      // Inject the per-process bridge token so only HTML we served can call /api/*.
      const html = readFileSync(htmlPath, 'utf8').replace(
        '</head>',
        `<script>window.__BRIDGE_TOKEN__=${JSON.stringify(TOKEN)}</script></head>`,
      )
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' })
      res.end(html)
      return
    }

    if (req.method === 'GET' && url.pathname === '/api/status') {
      const status = await daemonStatus()
      sendJson(res, 200, { bridge: 'ok', socket: socketPath(), ...status })
      return
    }

    if (req.method === 'GET' && url.pathname === '/api/resolve') {
      sendJson(res, 200, resolveProjectPath(url.searchParams.get('path') ?? ''))
      return
    }

    if (req.method === 'POST' && url.pathname === '/api/daemon/start') {
      const body = await readJsonObject(req)
      const project =
        typeof body.project === 'string' && body.project !== '' ? body.project : homedir()
      const status = await ensureDaemon(project)
      sendJson(res, 200, status)
      return
    }

    // Analysis proxies. query/impact answer symbol-not-found as ok:true with a
    // PLAIN TEXT body even in JSON mode — surfaced as a 404 error envelope.
    const analysisRoutes = {
      '/api/impact': (body) => {
        if (typeof body.symbol !== 'string' || body.symbol.trim() === '') {
          throw new RequestError('missing "symbol"')
        }
        return {
          method: 'impact',
          params: {
            symbol: body.symbol,
            depth: typeof body.depth === 'number' ? body.depth : 5,
            risk: true,
            output_format: 'json',
          },
        }
      },
      '/api/orphans': (body) => ({
        method: 'orphans',
        params: {
          exclude_tests: body.excludeTests === true,
          public_only: body.publicOnly !== false,
          output_format: 'json',
        },
      }),
      '/api/inconsistencies': (body) => {
        const params = { exclude_tests: body.excludeTests === true, output_format: 'json' }
        if (typeof body.category === 'string' && body.category !== '') {
          params.category = body.category
        }
        return { method: 'inconsistencies', params }
      },
      '/api/diff': (body) => ({
        method: 'diff',
        params: { base_ref: typeof body.baseRef === 'string' ? body.baseRef : 'HEAD' },
      }),
    }
    if (req.method === 'POST' && analysisRoutes[url.pathname] !== undefined) {
      const body = await readJsonObject(req)
      if (typeof body.project !== 'string') {
        sendJson(res, 400, { error: 'missing "project"' })
        return
      }
      const resolved = resolveProjectPath(body.project)
      if (!resolved.ok) {
        sendJson(res, 400, { error: resolved.error })
        return
      }
      const { method, params } = analysisRoutes[url.pathname](body)
      await ensureDaemon(resolved.path)
      const out = await ipcRequest(method, params, resolved.path, GRAPH_TIMEOUT_MS)
      const trimmed = out.trimStart()
      if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) {
        sendJson(res, 404, { error: out.trim() })
        return
      }
      res.writeHead(200, { 'Content-Type': 'application/json; charset=utf-8' })
      res.end(out)
      return
    }

    if (req.method === 'POST' && url.pathname === '/api/source') {
      const body = await readJsonObject(req)
      if (typeof body.project !== 'string' || typeof body.file !== 'string' || body.file === '') {
        sendJson(res, 400, { error: 'missing "project" or "file"' })
        return
      }
      const resolved = resolveProjectPath(body.project)
      if (!resolved.ok) {
        sendJson(res, 400, { error: resolved.error })
        return
      }
      let canonical
      try {
        canonical = realpathSync(body.file)
      } catch {
        sendJson(res, 404, { error: `file does not exist: ${body.file}` })
        return
      }
      // Containment check keeps this from becoming a generic file reader.
      if (!canonical.startsWith(resolved.path + '/') && canonical !== resolved.path) {
        sendJson(res, 403, { error: 'file is outside the project root' })
        return
      }
      const stat = statSync(canonical)
      if (!stat.isFile()) {
        sendJson(res, 400, { error: 'not a regular file' })
        return
      }
      if (stat.size > 2 * 1024 * 1024) {
        sendJson(res, 413, { error: 'file too large for preview' })
        return
      }
      const text = readFileSync(canonical, 'utf8')
      const spanStart = Number.isFinite(body.spanStart) ? body.spanStart : 0
      const spanEnd = Math.max(Number.isFinite(body.spanEnd) ? body.spanEnd : 0, spanStart)
      const lineOf = (offset) => {
        let count = 0
        const limit = Math.min(offset, text.length)
        for (let i = 0; i < limit; i++) if (text[i] === '\n') count++
        return count
      }
      const startLine = lineOf(spanStart)
      const endLine = lineOf(spanEnd)
      const allLines = text.split('\n')
      const windowStart = Math.max(0, startLine - 4)
      const windowEnd = Math.min(allLines.length, Math.min(endLine + 5, windowStart + 240))
      sendJson(res, 200, {
        file: canonical,
        startLine,
        endLine,
        windowStart,
        lines: allLines.slice(windowStart, windowEnd),
        truncated: windowEnd < Math.min(endLine + 5, allLines.length),
      })
      return
    }

    if (req.method === 'POST' && url.pathname === '/api/reindex') {
      const body = await readJsonObject(req)
      if (typeof body.project !== 'string') {
        sendJson(res, 400, { error: 'missing "project"' })
        return
      }
      const resolved = resolveProjectPath(body.project)
      if (!resolved.ok) {
        sendJson(res, 400, { error: resolved.error })
        return
      }
      await ensureDaemon(resolved.path)
      // Forced full re-index: the daemon drops the cached graph and rebuilds
      // from source, bypassing the snapshot warm-load.
      const result = await ipcRequest('reload_project', null, resolved.path, GRAPH_TIMEOUT_MS)
      sendJson(res, 200, result)
      return
    }

    if (req.method === 'POST' && url.pathname === '/api/graph') {
      const body = await readJsonObject(req)
      if (typeof body.project !== 'string') {
        sendJson(res, 400, { error: 'missing "project"' })
        return
      }
      const resolved = resolveProjectPath(body.project)
      if (!resolved.ok) {
        sendJson(res, 400, { error: resolved.error })
        return
      }
      const minConfidence =
        typeof body.minConfidence === 'number' && Number.isFinite(body.minConfidence)
          ? body.minConfidence
          : 0
      const graphJson = await fetchGraph(resolved.path, minConfidence)
      res.writeHead(200, {
        'Content-Type': 'application/json; charset=utf-8',
        'X-Coregraph-Project': encodeURIComponent(resolved.path),
      })
      res.end(graphJson)
      return
    }

    res.writeHead(404, { 'Content-Type': 'text/plain' })
    res.end('not found')
  } catch (error) {
    if (error instanceof RequestError) {
      sendJson(res, error.status, { error: error.message })
    } else if (error instanceof DaemonDownError) {
      sendJson(res, 503, { error: error.message })
    } else if (error instanceof DaemonError) {
      sendJson(res, 502, { error: error.message })
    } else {
      console.error(`[bridge] ${req.method} ${url.pathname} failed:`, error)
      sendJson(res, 500, { error: error instanceof Error ? error.message : String(error) })
    }
  }
})

// A bridge-level safety net: a single malformed request must never take down
// the long-lived viewer process. Logged, not fatal.
process.on('uncaughtException', (error) => {
  console.error('[bridge] uncaught exception (ignored):', error)
})
process.on('unhandledRejection', (reason) => {
  console.error('[bridge] unhandled rejection (ignored):', reason)
})

server.listen(PORT, '127.0.0.1', () => {
  const url = `http://127.0.0.1:${PORT}/`
  console.log(`coregraph atlas bridge → ${url}`)
  console.log(`  daemon socket: ${socketPath()}`)
  console.log(`  coregraph bin: ${BIN}`)
  if (!NO_OPEN) {
    const opener =
      process.platform === 'darwin' ? 'open' : process.platform === 'win32' ? 'start' : 'xdg-open'
    const child = spawn(opener, [url], { stdio: 'ignore', detached: true, shell: process.platform === 'win32' })
    child.on('error', () => console.log(`  open ${url} in your browser`))
    child.unref()
  }
})

server.on('error', (error) => {
  console.error(`[bridge] cannot listen on 127.0.0.1:${PORT}: ${error.message}`)
  process.exit(1)
})
