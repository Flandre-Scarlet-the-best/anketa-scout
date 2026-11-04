use anketa_scout::{
    Failure,
    config::{DEFAULT_CONFIG, MAX_CONFIG, load},
};
use std::fs;

#[test]
fn first_run_creates_default_and_second_run_never_overwrites() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings").join("config.toml");
    let config = load(None, &path).unwrap();
    assert_eq!(config.path, path);
    assert_eq!(config.source.page(), "https://2ch.org/soc/res/5959967.html");
    assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
    let own = "# Мои настройки\nsources = [\"https://2ch.org/test/res/100.html\"]\n";
    fs::write(&path, own).unwrap();
    assert_eq!(load(None, &path).unwrap().source.thread, 100);
    assert_eq!(fs::read_to_string(path).unwrap(), own);
}

#[test]
fn explicit_path_wins_and_does_not_create_default() {
    let directory = tempfile::tempdir().unwrap();
    let default = directory.path().join("default.toml");
    let explicit = directory.path().join("выбор.toml");
    fs::write(
        &explicit,
        "sources = [\"https://2ch.org/test/res/100.json\"]",
    )
    .unwrap();
    let config = load(Some(&explicit), &default).unwrap();
    assert_eq!(config.path, explicit);
    assert_eq!(config.source.thread, 100);
    assert!(!default.exists());
}

#[test]
fn missing_explicit_path_neither_creates_nor_falls_back() {
    let directory = tempfile::tempdir().unwrap();
    let explicit = directory.path().join("missing.toml");
    let default = directory.path().join("default.toml");
    fs::write(&default, DEFAULT_CONFIG).unwrap();
    assert!(matches!(
        load(Some(&explicit), &default),
        Err(Failure::Config(_))
    ));
    assert!(!explicit.exists());
    assert_eq!(fs::read_to_string(default).unwrap(), DEFAULT_CONFIG);
}

#[test]
fn invalid_toml_unknown_fields_and_wrong_types_are_errors_without_content_leaks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    for content in [
        "sources = [secret",
        "sources = 3",
        "sources = []\ntoken = 'private'",
        "other = []",
    ] {
        fs::write(&path, content).unwrap();
        let failure = load(Some(&path), &path).err().unwrap();
        assert_eq!(failure.code(), 2);
        assert!(!failure.to_string().contains("secret"));
        assert!(!failure.to_string().contains("private"));
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }
}

#[test]
fn empty_multiple_and_unsupported_sources_are_rejected_before_fetch() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    for content in [
        "sources = []",
        "sources = ['https://2ch.org/test/res/100.html', 'https://2ch.org/test/res/200.html']",
        "sources = ['https://example.invalid/?token=private']",
    ] {
        fs::write(&path, content).unwrap();
        let failure = load(Some(&path), &path).err().unwrap();
        assert_eq!(failure.code(), 2);
        assert!(!failure.to_string().contains("private"));
    }
}

#[test]
fn config_size_boundary_is_enforced_without_overwriting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let mut content = DEFAULT_CONFIG.to_owned();
    content.push('#');
    content.push_str(&"x".repeat(MAX_CONFIG as usize - content.len()));
    fs::write(&path, &content).unwrap();
    assert!(load(Some(&path), &path).is_ok());
    content.push('x');
    fs::write(&path, &content).unwrap();
    assert!(matches!(load(Some(&path), &path), Err(Failure::Config(_))));
    assert_eq!(fs::metadata(path).unwrap().len(), MAX_CONFIG + 1);
}

#[test]
fn creation_failure_and_unreadable_path_are_specific_config_errors() {
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("file");
    fs::write(&blocker, "Сохранить").unwrap();
    assert!(matches!(
        load(None, &blocker.join("config.toml")),
        Err(Failure::Config(_))
    ));
    assert!(matches!(
        load(Some(directory.path()), &blocker),
        Err(Failure::Config(_))
    ));
    assert_eq!(fs::read_to_string(blocker).unwrap(), "Сохранить");
}

#[test]
fn concurrent_first_runs_read_complete_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings").join("config.toml");
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || load(None, &path).map(|config| config.source.thread))
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap().unwrap(), 5959967);
    }
    assert_eq!(fs::read_to_string(path).unwrap(), DEFAULT_CONFIG);
}

#[test]
fn non_utf8_config_is_rejected_and_kept_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    let failure = load(Some(&path), &path).err().unwrap();
    assert_eq!(failure.code(), 2);
    assert!(failure.to_string().contains("UTF-8"));
    assert_eq!(fs::read(path).unwrap(), [0xff, 0xfe]);
}
