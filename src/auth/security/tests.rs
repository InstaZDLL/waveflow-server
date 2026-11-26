use super::*;

#[test]
fn password_round_trip() {
    let hash = hash_password("correct horse battery staple").unwrap();
    assert!(verify_password("correct horse battery staple", &hash).unwrap());
    assert!(!verify_password("incorrect password", &hash).unwrap());
    assert!(!hash.contains("correct horse"));
}

/// A hash written before argon2 0.6, by `SaltString` and `hash_password`,
/// still verifies: every account created until then signs in with it.
#[test]
fn a_hash_written_by_argon2_0_5_still_verifies() {
    let written = "$argon2id$v=19$m=19456,t=2,p=1$BwcHBwcHBwcHBwcHBwcHBw$eZ8SueF3EIJEgtgpg1rLafWpNVv3dMTwc0KCOxG5CSg";
    assert!(verify_password("correct horse battery staple", written).unwrap());
    assert!(!verify_password("incorrect password", written).unwrap());
    let fresh = hash_password("correct horse battery staple").unwrap();
    assert!(fresh.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
}

#[test]
fn encrypted_secret_round_trip() {
    let secret_box = SecretBox::from_key_bytes(&[7; 32]).unwrap();
    let encrypted = secret_box.encrypt(b"subsonic-only-password").unwrap();
    assert_ne!(encrypted.ciphertext, b"subsonic-only-password");
    assert_eq!(
        secret_box
            .decrypt(&encrypted.nonce, &encrypted.ciphertext)
            .unwrap(),
        b"subsonic-only-password"
    );
}

#[test]
fn share_tokens_are_stable_and_bound_to_the_instance_and_share() {
    let secret = SecretBox::from_key_bytes(&[7; 32]).unwrap();
    let other_secret = SecretBox::from_key_bytes(&[8; 32]).unwrap();
    let share = uuid::Uuid::new_v4();

    assert_eq!(
        secret.derive_share_token(share),
        secret.derive_share_token(share)
    );
    assert_ne!(
        secret.derive_share_token(share),
        secret.derive_share_token(uuid::Uuid::new_v4())
    );
    assert_ne!(
        secret.derive_share_token(share),
        other_secret.derive_share_token(share)
    );
}
