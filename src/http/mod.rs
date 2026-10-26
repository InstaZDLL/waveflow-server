//! HTTP pieces outside the two API surfaces: the security headers on every
//! response, the public share page, the `/reference` page and the embedded
//! web client.

pub mod public_share;
pub mod reference;
pub mod security_headers;
pub mod webui;
