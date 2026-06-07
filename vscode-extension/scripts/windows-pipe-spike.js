// D1 spike: Node client for the Windows named-pipe round-trip check.
// Usage (Windows only):
//   1. `cargo run --example spike_pipe` (starts the Rust server)
//   2. `node vscode-extension/scripts/windows-pipe-spike.js`
// Expected output: "received: pong"
//
// On macOS/Linux this script fails with ENOENT -- the path format is
// Windows-specific. That is expected; the spike is for Windows
// validation only.
//
// NOTE: The Rust server (spike_pipe.rs) requires Task D2 to land first
// (adds `interprocess` to CLI dev-dependencies). Run D2 before this spike.

'use strict';

const net = require('node:net');

const PIPE = '\\\\.\\pipe\\coregraph-spike';

const sock = net.createConnection(PIPE);

sock.on('connect', () => {
    sock.write('ping\n');
});

sock.on('data', (buf) => {
    const text = buf.toString().trim();
    console.log('received:', text);
    if (text !== 'pong') {
        process.exit(1);
    }
    sock.end();
});

sock.on('error', (e) => {
    console.error('FAIL:', e.message);
    process.exit(1);
});
