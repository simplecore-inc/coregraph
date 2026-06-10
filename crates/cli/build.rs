//! Embeds the built atlas viewer (viz/dist/index.html) into the binary for
//! the `coregraph viz` subcommand. When the viewer has not been built (plain
//! `cargo build` without the npm step), a self-describing placeholder page is
//! embedded instead so the Rust build never depends on the Node toolchain.

use std::env;
use std::fs;
use std::path::Path;

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>coregraph atlas — viewer not bundled</title></head>
<body style="font-family:monospace;background:#05070d;color:#c9d6e2;padding:3rem">
<h1>coregraph atlas</h1>
<p>This binary was built without the bundled viewer.</p>
<p>Build it first, then rebuild the CLI:</p>
<pre>cd viz &amp;&amp; npm install &amp;&amp; npm run build
cargo build -p coregraph</pre>
<p>Or point at a built file directly: <code>coregraph viz --html viz/dist/index.html</code></p>
</body>
</html>
"#;

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let src = Path::new(&manifest_dir).join("../../viz/dist/index.html");
    let dest = Path::new(&out_dir).join("atlas.html");

    // A missing source path makes cargo re-run this script every build, which
    // is exactly right: the embed picks the file up as soon as it appears.
    println!("cargo:rerun-if-changed={}", src.display());

    if src.exists() {
        fs::copy(&src, &dest).expect("copying viz/dist/index.html into OUT_DIR");
    } else {
        fs::write(&dest, PLACEHOLDER).expect("writing placeholder atlas.html");
    }
}
