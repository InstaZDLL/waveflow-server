//! Where the server finds `ffmpeg` and `ffprobe`.
//!
//! A setting names a tool, a standalone download carries one beside the
//! executable, and anything else is looked up on `PATH` — in that order.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The program the server runs for `name`: what `explicit` names when the
/// operator set it, else `<beside>/<name>` when that is a file, else the bare
/// name for the system to find on `PATH`.
///
/// `beside` is the directory of the running executable. A standalone archive
/// ships FFmpeg there, the way a Tauri or Bun sidecar does, so a double-clicked
/// server finds it without a `PATH` the user would have to edit
/// (`docs/plans/standalone-download-plan.md`). The setting still wins: an
/// operator who points at a build of their own means that build, sidecar or
/// not, and the Docker image names `/usr/bin/ffmpeg` that way.
///
/// Only a regular file counts: a directory named `ffmpeg` beside the binary is
/// a coincidence, not a sidecar. A sidecar that is there but cannot run is
/// still chosen, and the boot's version check refuses it by its full path,
/// rather than falling back to whatever `PATH` holds and leaving the user
/// unaware the archive's own copy was never used.
pub(super) fn resolve_tool(
    explicit: Option<OsString>,
    name: &str,
    beside: Option<&Path>,
) -> PathBuf {
    if let Some(explicit) = explicit {
        return PathBuf::from(explicit);
    }
    let file_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    beside
        .map(|directory| directory.join(&file_name))
        .filter(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// The directory of the running executable, when the platform can say.
///
/// Resolved through symbolic links on Unix, so a binary linked into
/// `/usr/local/bin` still finds the sidecar beside the file it points at.
/// Not on Windows, where canonicalising turns a path into its `\\?\` form for
/// no gain: a shortcut there is a `.lnk`, never a link `current_exe` returns.
pub(super) fn executable_dir() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    #[cfg(unix)]
    let executable = std::fs::canonicalize(&executable).unwrap_or(executable);
    executable.parent().map(Path::to_path_buf)
}
