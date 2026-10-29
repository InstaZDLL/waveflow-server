//! Laptop mode: the defaults of a server someone downloaded and
//! double-clicked, rather than one an operator installed
//! (`docs/plans/standalone-download-plan.md`, §5).
//!
//! Asked for, never guessed. `WAVEFLOW_LAPTOP` switches it on, and the
//! `laptop.env` a standalone archive ships beside the executable is what sets
//! it. Not the presence of FFmpeg beside the binary, and not "no variable is
//! set": a systemd unit that relies on today's defaults sets nothing either,
//! and must not wake up one day with its data directory moved.
//!
//! It changes defaults only. Every variable set explicitly still wins.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::env::{parse_bool_value, parse_positive_value};
use super::tools::executable_dir;

/// The file a standalone archive ships beside the executable.
pub const LAPTOP_ENV_FILE: &str = "laptop.env";

/// `WAVEFLOW_TRANSCODE_CACHE_MAX_BYTES` unless laptop mode is on: 10 GiB.
pub(super) const DEFAULT_TRANSCODE_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// `WAVEFLOW_TRANSCODE_CACHE_MAX_BYTES` in laptop mode: 2 GiB, because a
/// laptop disk is not a NAS.
pub(super) const LAPTOP_TRANSCODE_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Read `laptop.env` beside the running executable into the environment, and
/// say where it was when there was one.
///
/// Through `dotenvy`, which never overrides a variable already set: what the
/// environment says wins over the file, and so does the `.env` of the working
/// directory, which `main` reads first. No file is the ordinary case — every
/// install that is not a standalone archive — and is not an error. A file that
/// is there but cannot be read or parsed is: a server that quietly started
/// without the settings its user wrote is the failure this refuses.
pub fn load_laptop_env() -> anyhow::Result<Option<PathBuf>> {
    let Some(directory) = executable_dir() else {
        return Ok(None);
    };
    load_env_file(&directory.join(LAPTOP_ENV_FILE))
}

fn load_env_file(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    match dotenvy::from_path(path) {
        Ok(()) => Ok(Some(path.to_path_buf())),
        Err(error) if error.not_found() => Ok(None),
        Err(error) => Err(anyhow::anyhow!("cannot read {}: {error}", path.display())),
    }
}

/// The three families of systems whose data folder laptop mode knows.
///
/// A parameter rather than a `cfg!` inside [`resolve`], so the tests walk all
/// three on whichever host runs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Windows,
    MacOs,
    /// Linux and the other Unixes: the XDG base directories.
    Unix,
}

impl Platform {
    pub(super) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Unix
        }
    }
}

/// What laptop mode decides, once the variables an operator set have had
/// their say.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Resolved {
    pub laptop: bool,
    pub data_dir: PathBuf,
    pub transcode_cache_max_bytes: u64,
}

/// `WAVEFLOW_LAPTOP` and the settings whose default it moves, read through
/// `lookup` — `std::env::var_os` at boot, a table in the tests.
///
/// Without laptop mode this is exactly what `from_env` read before it existed:
/// `WAVEFLOW_DATA_DIR` as given, else `data`; the cache at 10 GiB.
pub(super) fn resolve(
    lookup: impl Fn(&str) -> Option<OsString>,
    platform: Platform,
) -> anyhow::Result<Resolved> {
    let text = |name: &str| lookup(name).and_then(|value| value.into_string().ok());
    let laptop = parse_bool_value("WAVEFLOW_LAPTOP", text("WAVEFLOW_LAPTOP").as_deref(), false)?;
    let explicit_data_dir = lookup("WAVEFLOW_DATA_DIR")
        // A key uncommented in `laptop.env` with nothing after it is the
        // template's own setting left alone, not a request for the working
        // directory. Outside laptop mode an empty value keeps meaning what it
        // always has.
        .filter(|value| !(laptop && value.is_empty()));
    let data_dir = match explicit_data_dir {
        Some(explicit) => PathBuf::from(explicit),
        None if laptop => os_data_dir(&lookup, platform).ok_or_else(|| {
            anyhow::anyhow!(
                "laptop mode cannot tell where this system keeps application data ({} is not set); set WAVEFLOW_DATA_DIR in {LAPTOP_ENV_FILE}",
                match platform {
                    Platform::Windows => "LOCALAPPDATA",
                    Platform::MacOs | Platform::Unix => "HOME",
                }
            )
        })?,
        None => PathBuf::from("data"),
    };
    let transcode_cache_max_bytes = parse_positive_value(
        "WAVEFLOW_TRANSCODE_CACHE_MAX_BYTES",
        text("WAVEFLOW_TRANSCODE_CACHE_MAX_BYTES"),
        if laptop {
            LAPTOP_TRANSCODE_CACHE_MAX_BYTES
        } else {
            DEFAULT_TRANSCODE_CACHE_MAX_BYTES
        },
    )?;
    Ok(Resolved {
        laptop,
        data_dir,
        transcode_cache_max_bytes,
    })
}

/// The folder this system keeps a user's application data in, with the
/// server's own name under it.
///
/// The *local* one on Windows, `%LOCALAPPDATA%` rather than `%APPDATA%`: a
/// roaming profile would copy the database and the transcode cache to every
/// machine its user signs in to. On Linux, `$XDG_DATA_HOME` when it names an
/// absolute path — the specification says a relative one is ignored — and
/// `~/.local/share` otherwise.
fn os_data_dir(lookup: &impl Fn(&str) -> Option<OsString>, platform: Platform) -> Option<PathBuf> {
    let set = |name: &str| {
        lookup(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    match platform {
        Platform::Windows => set("LOCALAPPDATA").map(|local| local.join("WaveFlow Server")),
        Platform::MacOs => set("HOME").map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("WaveFlow Server")
        }),
        Platform::Unix => set("XDG_DATA_HOME")
            // `has_root` rather than `is_absolute`: the same on Unix, and on
            // a Windows host running these tests it still reads `/home/…` as
            // the absolute path it is on the system it describes.
            .filter(|xdg| xdg.has_root())
            .or_else(|| set("HOME").map(|home| home.join(".local").join("share")))
            .map(|data| data.join("waveflow-server")),
    }
}
