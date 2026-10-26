//! Who is calling: accounts and sessions, the password guard, native-client
//! OAuth, the first-run setup token, the instance key's cryptography and the
//! stream tickets browser playback is authorised with.

pub mod authentication;
pub mod oauth;
pub mod security;
pub mod setup_token;
pub mod sign_in;
pub mod stream_ticket;
