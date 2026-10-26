use super::*;

fn secret() -> SecretBox {
    SecretBox::from_key_bytes(&[7u8; 32]).unwrap()
}

/// Seals a payload the public API cannot express, so the test can present a
/// ticket that is genuine in every respect except its kind.
fn seal(secret: &SecretBox, payload: &[u8]) -> String {
    let sealed = secret.encrypt(payload).unwrap();
    let mut raw = Vec::with_capacity(NONCE_LEN + sealed.ciphertext.len());
    raw.extend_from_slice(&sealed.nonce);
    raw.extend_from_slice(&sealed.ciphertext);
    URL_SAFE_NO_PAD.encode(raw)
}

fn payload_with_kind(byte: u8, user: Uuid, track: Uuid, expires_at: i64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_LEN);
    payload.push(byte);
    payload.extend_from_slice(user.as_bytes());
    payload.extend_from_slice(track.as_bytes());
    payload.extend_from_slice(&expires_at.to_be_bytes());
    payload
}

#[test]
fn round_trip_returns_the_authorised_triple() {
    let secret = secret();
    for kind in [
        TicketKind::Audio,
        TicketKind::Canvas,
        TicketKind::ShareVisit,
    ] {
        let user = Uuid::new_v4();
        let track = Uuid::new_v4();
        let ticket = mint(&secret, kind, user, track, 1_000).unwrap();
        assert_eq!(verify(&secret, &ticket, 999), Some((kind, user, track)));
    }
}

#[test]
fn the_kinds_do_not_answer_for_each_other() {
    let secret = secret();
    let user = Uuid::new_v4();
    let track = Uuid::new_v4();
    let audio = mint(&secret, TicketKind::Audio, user, track, 1_000).unwrap();
    let canvas = mint(&secret, TicketKind::Canvas, user, track, 1_000).unwrap();
    let visit = mint(&secret, TicketKind::ShareVisit, user, track, 1_000).unwrap();
    assert_ne!(audio, canvas, "the kind is inside the sealed payload");
    assert_eq!(
        verify(&secret, &audio, 999).map(|(kind, ..)| kind),
        Some(TicketKind::Audio)
    );
    assert_eq!(
        verify(&secret, &canvas, 999).map(|(kind, ..)| kind),
        Some(TicketKind::Canvas)
    );
    assert_eq!(
        verify(&secret, &visit, 999).map(|(kind, ..)| kind),
        Some(TicketKind::ShareVisit)
    );
}

#[test]
fn an_unknown_kind_is_refused_even_under_the_right_key() {
    let secret = secret();
    let user = Uuid::new_v4();
    let track = Uuid::new_v4();
    // Sealed under the real key, correct length, live expiry: the kind byte
    // is the only thing wrong with it. A forged ticket would prove nothing
    // about this contract, since it already fails to decrypt.
    for byte in [0x00, 0x04, 0xff] {
        let ticket = seal(&secret, &payload_with_kind(byte, user, track, 1_000));
        assert_eq!(
            verify(&secret, &ticket, 999),
            None,
            "kind {byte:#04x} is not one of the kinds"
        );
    }
}

#[test]
fn a_payload_of_the_previous_length_is_refused() {
    let secret = secret();
    // What `mint` produced before the kind byte existed: user || track ||
    // expiry, forty bytes, sealed under the current key.
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(Uuid::new_v4().as_bytes());
    payload.extend_from_slice(Uuid::new_v4().as_bytes());
    payload.extend_from_slice(&1_000i64.to_be_bytes());
    let ticket = seal(&secret, &payload);
    assert_eq!(verify(&secret, &ticket, 999), None);
}

#[test]
fn expired_forged_and_malformed_tickets_are_all_rejected() {
    let secret = secret();
    let ticket = mint(
        &secret,
        TicketKind::Audio,
        Uuid::new_v4(),
        Uuid::new_v4(),
        1_000,
    )
    .unwrap();
    assert_eq!(verify(&secret, &ticket, 1_000), None, "expiry is exclusive");
    assert_eq!(verify(&secret, &ticket, 1_001), None);

    let other = SecretBox::from_key_bytes(&[9u8; 32]).unwrap();
    assert_eq!(
        verify(&other, &ticket, 0),
        None,
        "a ticket minted elsewhere must not validate"
    );

    assert_eq!(verify(&secret, "not-base64!", 0), None);
    assert_eq!(verify(&secret, "", 0), None);
    let mut tampered = URL_SAFE_NO_PAD.decode(&ticket).unwrap();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    assert_eq!(
        verify(&secret, &URL_SAFE_NO_PAD.encode(tampered), 0),
        None,
        "AEAD must reject a flipped bit"
    );
}
