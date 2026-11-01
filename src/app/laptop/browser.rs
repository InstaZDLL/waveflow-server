//! Opening the default browser on the server's address
//! (`docs/plans/standalone-download-plan.md`, §5): what someone who
//! double-clicked the executable expects to see, rather than a console window
//! and an address to type.
//!
//! On a first start the setup token rides along in the URL **fragment**. A
//! browser never sends the fragment to the server, so the token is in no
//! request line, no server trace and no proxy log; the sign-in screen moves it
//! into its token field and removes it from the address bar. The token file
//! stays the source: the fragment is a hand-over, and a user who closes the tab can
//! still read the file.
//!
//! Only the copy that drew the token hands it over. A second copy that finds
//! WaveFlow on the port opens the bare address: all it knows of what answered
//! is a `/health` body any local program can imitate, and the `setup-token`
//! file outlives a first copy that stopped before setup, so a copy that read
//! the file and passed it on could be handing it to a stranger holding the
//! port.
//!
//! The copy that drew it hands it over only where nothing else on the machine
//! can pick it up on the way ([`may_hand_over`]). The URL is on the command
//! line of the opener, and often of the browser it starts, for as long as that
//! browser runs: Windows and macOS keep a process's arguments from other
//! accounts, but Linux and the BSDs show them to anyone, while the token file
//! is readable by its owner alone. And the browser opens the loopback address,
//! which a server listening on every interface does not own: Windows and the
//! BSDs let another program bind `127.0.0.1` on the same port, more specific,
//! and take the browser's connection. Elsewhere the browser opens the bare
//! address, and the token is in the file the log names. What this does not
//! cover is a machine that logs every command line it starts (Windows' event
//! 4688, Sysmon, an endpoint agent): its log readers see the token. The
//! operations guide documents that case and the variable that turns the
//! opening off.
//!
//! Through the opener each platform ships rather than a crate: `rundll32`'s
//! URL handler on Windows, `open` on macOS, `xdg-open` elsewhere. A failure to
//! open — a Linux machine with no desktop, no opener installed — is a log
//! line, never a reason to stop serving: the banner still says where to go.

use std::{net::SocketAddr, time::Duration};

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

use crate::auth::setup_token::TokenSource;

/// The name of the fragment parameter the sign-in screen reads
/// (`webapp/src/lib/setup-token-fragment.ts`).
pub const FRAGMENT_KEY: &str = "setup-token";

/// Everything but RFC 3986's unreserved characters is escaped, so the token
/// reads back unchanged through `URLSearchParams`, and the URL carries
/// nothing a command line could split or a shell could expand. A drawn token
/// is unreserved already; one an operator chose may not be.
const FRAGMENT_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// How long a second copy waits for the opener before it exits anyway. The
/// opener keeps running if it outlives the wait: `xdg-open` can stay in the
/// foreground for as long as the browser it started does.
const OPENER_WAIT: Duration = Duration::from_secs(5);

/// Whether another account of this machine is kept from reading the command
/// line of this account's processes: on Windows it needs rights over the
/// process, on macOS it needs root (xnu's `sysctl_procargsx`, which `ps`
/// reads, answers `EINVAL` to another uid that is not the superuser, and
/// `open` hands the URL to the browser through Launch Services rather than on
/// its command line). Linux shows every command line in `/proc`
/// unless it is mounted with `hidepid`, and the BSDs show them through `ps`.
pub const COMMAND_LINES_ARE_PRIVATE: bool = cfg!(any(windows, target_os = "macos"));

/// Whether the setup token may ride along when the browser opens a server
/// listening on `listening`: only where command lines are private, and only on
/// an address the listener owns. A wildcard listener is opened on the loopback
/// address, which another program may hold more specifically.
pub fn may_hand_over(listening: SocketAddr, command_lines_private: bool) -> bool {
    command_lines_private && !listening.ip().is_unspecified()
}

/// The address to open: `base` — the server's root, with its trailing slash —
/// or, when there is a setup token to hand over, its sign-in screen with the
/// token in the fragment.
///
/// The sign-in screen itself rather than the root: the root sends a visitor
/// without a session to `/login` through a client-side redirect, and that
/// redirect drops the fragment. A server whose setup is open has no account,
/// so nobody opening it has a session to land anywhere else.
pub fn handover_url(base: &str, setup_token: Option<&str>) -> String {
    match setup_token.map(str::trim).filter(|token| !token.is_empty()) {
        Some(token) => format!(
            "{base}login#{FRAGMENT_KEY}={}",
            utf8_percent_encode(token, FRAGMENT_VALUE)
        ),
        None => base.to_owned(),
    }
}

/// The setup token to hand over, while setup is open: the content of the file
/// the boot wrote, or the value an operator set. `None` once an account
/// exists, and when the file cannot be read — the browser then opens on the
/// sign-in screen with an empty field, which the file still fills.
pub fn handover_token(source: Option<&TokenSource>, configured: Option<&str>) -> Option<String> {
    let token = match source? {
        TokenSource::File(path) => std::fs::read_to_string(path).ok()?,
        TokenSource::Operator => configured?.to_owned(),
    };
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// The command that asks this platform to open `url` in the default browser.
///
/// On Windows, `rundll32`'s URL handler rather than `cmd /c start`: `cmd`
/// would read the URL as part of a command line, where `&` ends it and `%`
/// expands a variable. `rundll32` hands the rest of its line to the shell's
/// URL handler as it is.
fn opener(url: &str) -> tokio::process::Command {
    let mut command = if cfg!(windows) {
        let mut command = tokio::process::Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else if cfg!(target_os = "macos") {
        tokio::process::Command::new("open")
    } else {
        tokio::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

/// Open `url` in the default browser, and wait at most a few seconds to hear
/// how it went.
///
/// `address` is what the log names: `url` without its fragment, so a failure
/// is reported without the token it carried. A `url` that carries one is
/// passed only where [`may_hand_over`] said so: it sits on the opener's
/// command line.
pub async fn open(url: String, address: &str) {
    let mut child = match opener(&url).spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(
                %address,
                %error,
                "could not open a browser; open the address yourself"
            );
            return;
        }
    };
    match tokio::time::timeout(OPENER_WAIT, child.wait()).await {
        Ok(Ok(status)) if status.success() => {}
        // `rundll32` answers 0 whether or not a browser came up; `xdg-open`
        // and `open` say when nothing could handle the address, as on a
        // machine with no desktop.
        Ok(Ok(status)) => tracing::warn!(
            %address,
            %status,
            "the browser did not open; open the address yourself"
        ),
        Ok(Err(error)) => tracing::warn!(
            %address,
            %error,
            "could not open a browser; open the address yourself"
        ),
        // Still running: a browser started in the foreground. Left to it.
        Err(_) => {}
    }
}
