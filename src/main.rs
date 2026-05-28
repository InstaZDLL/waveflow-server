//! waveflow-server — bootstrap entry point.
//!
//! Phase 1.b will replace this with an axum app exposing `/health`,
//! `/api/v1/*` CRUD over Postgres, and the JWKS-verified auth
//! middleware described in RFC-001 §6.4. For now the binary just
//! prints a banner so CI has something to compile, lint and test.

fn main() {
    println!("waveflow-server bootstrap — Phase 1.b not implemented yet.");
    println!(
        "See https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md"
    );
}
