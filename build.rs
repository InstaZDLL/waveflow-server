//! Stages the embedded web client for `rust_embed`.
//!
//! `rust_embed` reads its folder at compile time and fails outright when it is
//! missing, which would make `cargo build` depend on a prior front-end build.
//! Rather than create that folder in the source tree — a build script must not
//! write to the sources it is compiling, which breaks read-only checkouts and
//! leaves untracked files behind — the client is staged under `OUT_DIR`: the
//! real `webapp/dist` when one exists, a placeholder page otherwise.
use std::{env, fs, path::Path};

fn main() {
    let out_dir = env::var_os("OUT_DIR").expect("cargo sets OUT_DIR for build scripts");
    let staged = Path::new(&out_dir).join("webapp-dist");
    if staged.exists() {
        fs::remove_dir_all(&staged).expect("clear staged web client");
    }
    fs::create_dir_all(&staged).expect("create staged web client directory");

    let dist = Path::new("webapp/dist");
    if dist.join("index.html").exists() {
        copy_dir(dist, &staged);
    } else {
        fs::write(staged.join("index.html"), PLACEHOLDER).expect("write placeholder index");
    }

    println!("cargo:rerun-if-changed=webapp/dist");
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).expect("read web client directory") {
        let entry = entry.expect("read web client entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("stat web client entry").is_dir() {
            fs::create_dir_all(&target).expect("create staged subdirectory");
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy web client file");
        }
    }
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
