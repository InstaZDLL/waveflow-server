//! Home Assistant's ingress (RFC-030): the web client inside Home Assistant,
//! served on a listener of its own.
//!
//! Home Assistant proxies `/api/hassio_ingress/<token>/…` to the app with the
//! prefix stripped, and names it in `X-Ingress-Path`. Everything here exists so
//! that header is read **only** where only Home Assistant can send it:
//!
//! - [`IngressListener`] accepts connections from the configured peers alone
//!   (the Supervisor, `172.30.32.2`) and drops every other at once
//!   (decision 1);
//! - [`require_prefix`] is the layer of the router that listener serves, and
//!   only of that one: it checks the header and hands the prefix on as an
//!   [`IngressPath`] extension, or answers `400` (decision 2);
//! - [`Ingress`] is how a handler asks whether it is serving an ingress
//!   request. It reads the extension, never the header, so the main listener,
//!   which has no such layer, can never be talked into the mode by a client.

use std::{net::IpAddr, sync::Arc};

use axum::{
    extract::{FromRequestParts, Request},
    http::{request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use ipnet::IpNet;
use tokio::net::{TcpListener, TcpStream};

use crate::api::ErrorResponse;

/// The header Home Assistant's Core names the prefix in.
pub const INGRESS_PATH_HEADER: &str = "x-ingress-path";

/// What a prefix may look like: Home Assistant's own path, then one token of
/// URL-safe characters. Checked to the byte, because the value ends up in the
/// shell's HTML and in a cookie's `Path`.
const PREFIX_ROOT: &str = "/api/hassio_ingress/";
const TOKEN_MAX: usize = 128;

/// The prefix an ingress request was served under, without a trailing slash:
/// `/api/hassio_ingress/<token>`. Only [`require_prefix`] makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressPath(Arc<str>);

impl IngressPath {
    /// Checks a header value against Home Assistant's form.
    pub fn parse(value: &str) -> Option<Self> {
        let token = value.strip_prefix(PREFIX_ROOT)?;
        let valid = !token.is_empty()
            && token.len() <= TOKEN_MAX
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        valid.then(|| Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether the request being served came through ingress, and under which
/// prefix. `None` on the main listener, always.
#[derive(Clone, Debug, Default)]
pub struct Ingress(pub Option<IngressPath>);

impl Ingress {
    /// The root the request's paths start from: `""` on the main listener,
    /// the prefix under ingress. A path is this followed by `/…`.
    pub fn root(&self) -> &str {
        self.0.as_ref().map_or("", IngressPath::as_str)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Ingress {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(parts.extensions.get::<IngressPath>().cloned()))
    }
}

/// The ingress router's layer: the prefix is checked and carried on, or the
/// request is refused. A page served at the wrong root is the failure RFC-030
/// removes, and the listener has no other client, so there is no request
/// without a prefix to serve.
pub async fn require_prefix(mut request: Request, next: Next) -> Response {
    match prefix_of(request.headers()) {
        Some(prefix) => {
            request.extensions_mut().insert(prefix);
            next.run(request).await
        }
        None => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                code: "ingress_path_invalid",
                message: "An ingress request must name its prefix in X-Ingress-Path",
            }),
        )
            .into_response(),
    }
}

fn prefix_of(headers: &HeaderMap) -> Option<IngressPath> {
    let mut values = headers.get_all(INGRESS_PATH_HEADER).iter();
    let value = values.next()?;
    // Two of them is not a prefix: one would have to be chosen, and which one
    // is not a question the server should answer.
    if values.next().is_some() {
        return None;
    }
    IngressPath::parse(value.to_str().ok()?)
}

/// A listener that keeps the connections of its peers and drops the others as
/// they arrive, without a byte read or written: the Supervisor is the only
/// client an ingress port has, and a scanner learns nothing from a reset.
pub struct IngressListener {
    inner: TcpListener,
    peers: Vec<IpNet>,
}

impl IngressListener {
    pub fn new(inner: TcpListener, peers: Vec<IpNet>) -> Self {
        Self { inner, peers }
    }
}

/// Whether a peer may use the ingress listener.
pub fn peer_allowed(peers: &[IpNet], address: IpAddr) -> bool {
    let address = match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        IpAddr::V4(_) => address,
    };
    peers.iter().any(|net| net.contains(&address))
}

impl axum::serve::Listener for IngressListener {
    type Io = TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, address) = axum::serve::Listener::accept(&mut self.inner).await;
            if peer_allowed(&self.peers, address.ip()) {
                return (stream, address);
            }
            tracing::debug!(peer = %address.ip(), "ingress connection refused");
            drop(stream);
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

#[cfg(test)]
mod tests;
