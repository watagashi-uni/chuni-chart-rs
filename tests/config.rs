use chuni_chart_rs::config::Config;
use std::fs;

#[test]
fn config_paths_are_relative_to_file_and_secrets_never_echo_in_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.json");
    fs::write(
        &path,
        r#"{"chart_dir":"c2s","listen":"127.0.0.1:9090","timeout_seconds":4}"#,
    )
    .unwrap();
    let config = Config::load(Some(&path)).unwrap();
    assert_eq!(config.chart_dir, dir.path().join("c2s"));
    assert_eq!(config.timeout_seconds, 4);
    config.validate().unwrap();
    fs::write(
        &path,
        r#"{"chart_dir":"","etag_salt":"secret-must-not-appear"}"#,
    )
    .unwrap();
    assert!(Config::load(Some(&path)).is_err());
    for text in [
        r#"{"unexpected":"secret-must-not-appear"}"#,
        r#"{"listen": {"secret-must-not-appear":0}}"#,
        r#"{"etag_salt":true,"secret-must-not-appear":0}"#,
    ] {
        fs::write(&path, text).unwrap();
        let error = Config::load(Some(&path)).err().unwrap();
        assert!(!error.contains("secret-must-not-appear"));
    }
    fs::write(&path, vec![b' '; 65537]).unwrap();
    assert!(Config::load(Some(&path)).is_err());
}

#[test]
fn init_config_is_private_unique_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.json");
    let second = dir.path().join("second.json");
    Config::initialize(&first).unwrap();
    Config::initialize(&second).unwrap();
    let original = fs::read(&first).unwrap();
    assert!(Config::initialize(&first).is_err());
    assert_eq!(fs::read(&first).unwrap(), original);
    let one = Config::load(Some(&first)).unwrap();
    let two = Config::load(Some(&second)).unwrap();
    one.validate().unwrap();
    assert_ne!(one.etag_salt, two.etag_salt);
    assert_eq!(one.etag_salt.unwrap().len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&first).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn invalid_service_settings_fail_closed() {
    for seconds in [0, 31, u64::MAX] {
        assert!(
            Config {
                timeout_seconds: seconds,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    for listen in ["example.com:3000", "127.0.0.1", "not-an-address"] {
        assert!(
            Config {
                listen: listen.into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    for salt in [
        String::new(),
        "private-short-secret".into(),
        "x".repeat(4097),
    ] {
        let config = Config {
            etag_salt: Some(salt.clone()),
            ..Default::default()
        };
        let error = config.validate().unwrap_err();
        if !salt.is_empty() {
            assert!(!error.contains(&salt));
        }
    }
}
