//! Guarantees the embedded web client always has something to embed.
//!
//! `rust_embed` reads `webapp/dist/` at compile time and fails outright when the
//! directory is missing, which would make `cargo build` depend on a prior
//! front-end build. Creating a placeholder keeps the Rust build self-contained;
//! a real `vite build` overwrites it.
use std::{fs, path::Path};

fn main() {
    let dist = Path::new("webapp/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        fs::create_dir_all(dist).expect("create webapp/dist placeholder directory");
        fs::write(&index, PLACEHOLDER).expect("write webapp/dist placeholder index");
    }
    println!("cargo:rerun-if-changed=webapp/dist");
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>WaveFlow Server</title>
  </head>
  <body>
    <main>
      <h1>WaveFlow Server</h1>
      <p>The web client is not built. Run <code>bun --cwd=webapp run build</code>.</p>
      <p>The API is available at <a href="/reference">/reference</a>.</p>
    </main>
  </body>
</html>
"#;
