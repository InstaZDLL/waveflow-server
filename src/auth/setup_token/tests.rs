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
fn a_file_another_process_has_not_finished_writing_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(FILE_NAME);
    let token = security::generate_token(PREFIX);
    // Created and not yet written, then cut short: neither is a token, and
    // the first would otherwise admit a request that sends none.
    for partial in [
        String::new(),
        "\n".to_owned(),
        token[..PREFIX.len() + 4].to_owned(),
    ] {
        std::fs::write(&path, &partial).unwrap();
        let refused = written_by_another(&path).expect_err(&format!("{partial:?}"));
        assert!(refused.to_string().contains(&path.display().to_string()));
    }
    std::fs::write(&path, format!("{token}\n")).unwrap();
    assert_eq!(written_by_another(&path).unwrap(), token);
}

#[test]
fn an_operator_token_built_in_process_still_meets_the_floor() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = config(&temp);
    config.setup_token =
        Some(uuid::Uuid::new_v4().to_string()[..SETUP_TOKEN_MIN_CHARS - 1].to_owned());
    assert!(SetupGate::prepare(&config, true).is_err());
    // Spaces do not count toward it.
    config.setup_token = Some(format!(
        "  {}  ",
        &uuid::Uuid::new_v4().to_string()[..SETUP_TOKEN_MIN_CHARS - 1]
    ));
    assert!(SetupGate::prepare(&config, true).is_err());
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
