//! The one-time token that guards first-run setup (RFC-019 decision 1).
//!
//! `POST /api/v2/setup` makes whoever calls it the administrator, and until
//! this module it asked only for a plausible `Origin` header — something any
//! client other than a browser writes for itself. On an instance published on
//! every interface, as the compose file does, the first stranger to find a
//! fresh server owned it.
//!
//! So while no account exists, setup also asks for a token only the operator
//! can read:
//!
//! - **drawn at boot** when the operator chose none, and written to
//!   `<data dir>/setup-token`, owner-only on Unix. The log names the *file*,
//!   never the token: a log line is read by more people, and kept for longer,
//!   than the data directory it describes;
//! - **or chosen by the operator** in `WAVEFLOW_SETUP_TOKEN`, for a deployment
//!   whose data directory is awkward to read. No file is written then;
//! - **compared in constant time**, as digests, so neither the content nor the
//!   length of a wrong guess changes how long the refusal takes;
//! - **forgotten once setup succeeds**: the gate closes in memory and the file
//!   is deleted. A boot that finds an account deletes a file left behind.
//!
//! The decision is made at boot. A database that loses its last account while
//! the server runs is not reopened for setup until the next start, which is
//! the direction to be wrong in.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::config::{Config, SETUP_TOKEN_MIN_CHARS};
use crate::security;

/// The file a drawn token is written to, inside the data directory.
pub const FILE_NAME: &str = "setup-token";

/// The prefix a drawn token carries, so a stray copy says what it is.
const PREFIX: &str = "wfsetup_";

/// Where the token an operator has to present comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    /// `WAVEFLOW_SETUP_TOKEN`.
    Operator,
    /// Drawn by this server and written to this file.
    File(PathBuf),
}

/// What the gate makes of a setup request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The token is the right one; setup may go ahead.
    Admitted,
    /// Setup is open and the token is missing or wrong.
    Rejected,
    /// Setup is not open: an account existed at boot, or setup has succeeded.
    Closed,
}

struct Open {
    digest: [u8; 32],
    source: TokenSource,
}

/// The gate itself, held on `AppState`.
pub struct SetupGate {
    open: Mutex<Option<Open>>,
}

impl SetupGate {
    /// A gate that admits nobody.
    pub fn closed() -> Self {
        Self {
            open: Mutex::new(None),
        }
    }

    /// Opens the gate when setup is still required, and makes sure the token
    /// file says the same thing the gate does.
    pub fn prepare(config: &Config, setup_required: bool) -> anyhow::Result<Self> {
        let path = config.data_dir.join(FILE_NAME);
        if !setup_required {
            remove_if_present(&path)?;
            return Ok(Self::closed());
        }
        let (token, source) = match &config.setup_token {
            Some(token) => {
                // One source of truth: a file drawn by an earlier boot would
                // otherwise name a token this boot no longer accepts.
                remove_if_present(&path)?;
                (token.clone(), TokenSource::Operator)
            }
            None => (read_or_draw(&path)?, TokenSource::File(path)),
        };
        Ok(Self {
            open: Mutex::new(Some(Open {
                digest: security::token_hash(&token),
                source,
            })),
        })
    }

    /// Where the token comes from, or `None` when setup is closed. What the
    /// boot tells the operator; never the token itself.
    pub fn source(&self) -> Option<TokenSource> {
        self.lock().as_ref().map(|open| open.source.clone())
    }

    /// Whether a request presenting `presented` may run setup.
    pub fn admit(&self, presented: Option<&str>) -> Admission {
        let open = self.lock();
        let Some(open) = open.as_ref() else {
            return Admission::Closed;
        };
        // An absent token is compared like a wrong one, so the two refusals
        // cannot be told apart by their timing either.
        if security::token_matches(presented.unwrap_or_default().trim(), &open.digest) {
            Admission::Admitted
        } else {
            Admission::Rejected
        }
    }

    /// Closes the gate for good and deletes the token file.
    pub fn close(&self) {
        let Some(open) = self.lock().take() else {
            return;
        };
        if let TokenSource::File(path) = open.source {
            if let Err(error) = remove_if_present(&path) {
                // The path only: the file holds a token, and the token is
                // already useless, but the rule against logging one has no
                // exception for a token that no longer opens anything.
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "the setup token file could not be deleted; delete it by hand"
                );
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Open>> {
        self.open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The token an earlier boot drew, or a fresh one written in its place.
///
/// Kept across restarts, because a container that restarts before its
/// operator has read the file must not have changed the answer underneath
/// them. A file too short to be one this server wrote is replaced.
fn read_or_draw(path: &Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let token = content.trim();
            if token.chars().count() >= SETUP_TOKEN_MIN_CHARS {
                return Ok(token.to_owned());
            }
            remove_if_present(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow::anyhow!(
                "could not read the setup token file {}: {error}",
                path.display()
            ))
        }
    }
    let token = security::generate_token(PREFIX);
    let written = security::write_new_secret_file(path, format!("{token}\n").as_bytes()).map_err(
        |error| {
            anyhow::anyhow!(
                "could not write the setup token file {}: {error}",
                path.display()
            )
        },
    )?;
    if written {
        return Ok(token);
    }
    // Another process — a CLI command beside the server — wrote it first. Its
    // token is the one on disk, so it is the one to accept.
    let content = std::fs::read_to_string(path)?;
    Ok(content.trim().to_owned())
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(temp: &tempfile::TempDir) -> Config {
        Config::for_data_dir(temp.path().join("data"))
    }

    fn written_token(config: &Config) -> String {
        std::fs::read_to_string(config.data_dir.join(FILE_NAME))
            .expect("the token file")
            .trim()
            .to_owned()
    }

    #[test]
    fn a_drawn_token_admits_only_itself_and_is_forgotten_on_close() {
        let temp = tempfile::tempdir().unwrap();
        let config = config(&temp);
        let gate = SetupGate::prepare(&config, true).unwrap();
        let token = written_token(&config);
        assert!(token.starts_with(PREFIX));
        assert_eq!(
            gate.source(),
            Some(TokenSource::File(config.data_dir.join(FILE_NAME)))
        );

        assert_eq!(gate.admit(None), Admission::Rejected);
        assert_eq!(gate.admit(Some("")), Admission::Rejected);
        let wrong = security::generate_token(PREFIX);
        assert_eq!(gate.admit(Some(&wrong)), Admission::Rejected);
        assert_eq!(gate.admit(Some(&token)), Admission::Admitted);

        gate.close();
        assert_eq!(gate.admit(Some(&token)), Admission::Closed);
        assert!(!config.data_dir.join(FILE_NAME).exists());
        assert_eq!(gate.source(), None);
    }

    #[test]
    fn a_restart_keeps_the_token_already_written() {
        let temp = tempfile::tempdir().unwrap();
        let config = config(&temp);
        SetupGate::prepare(&config, true).unwrap();
        let first = written_token(&config);
        let again = SetupGate::prepare(&config, true).unwrap();
        assert_eq!(written_token(&config), first);
        assert_eq!(again.admit(Some(&first)), Admission::Admitted);
    }

    #[test]
    fn an_operator_token_writes_no_file_and_replaces_a_drawn_one() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = config(&temp);
        SetupGate::prepare(&config, true).unwrap();
        let drawn = written_token(&config);

        let chosen = uuid::Uuid::new_v4().to_string();
        config.setup_token = Some(chosen.clone());
        let gate = SetupGate::prepare(&config, true).unwrap();
        assert!(!config.data_dir.join(FILE_NAME).exists());
        assert_eq!(gate.source(), Some(TokenSource::Operator));
        assert_eq!(gate.admit(Some(&drawn)), Admission::Rejected);
        assert_eq!(gate.admit(Some(&chosen)), Admission::Admitted);
    }

    #[test]
    fn a_boot_with_an_account_closes_setup_and_deletes_a_stale_file() {
        let temp = tempfile::tempdir().unwrap();
        let config = config(&temp);
        SetupGate::prepare(&config, true).unwrap();
        let drawn = written_token(&config);

        let gate = SetupGate::prepare(&config, false).unwrap();
        assert!(!config.data_dir.join(FILE_NAME).exists());
        assert_eq!(gate.admit(Some(&drawn)), Admission::Closed);
    }

    #[cfg(unix)]
    #[test]
    fn the_token_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let config = config(&temp);
        SetupGate::prepare(&config, true).unwrap();
        let mode = std::fs::metadata(config.data_dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "mode {mode:o}");
    }
}
