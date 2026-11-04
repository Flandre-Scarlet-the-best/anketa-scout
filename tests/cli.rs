use std::{fs, process::Command};

#[test]
fn help_and_version_need_no_config_or_network() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_anketa-scout"))
        .arg("--help")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("--config <ПУТЬ>"));
    assert!(text.contains("Использование:"));
    assert!(!text.contains('\u{1b}'));
    assert!(output.stderr.is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_anketa-scout"))
        .arg("--version")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("anketa-scout {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
}

#[test]
fn invalid_arguments_have_russian_safe_diagnostics_and_code_two() {
    for args in [
        vec!["--unknown=private"],
        vec!["--config"],
        vec!["https://example.invalid/?token=private"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_anketa-scout"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("Некорректные аргументы"));
        assert!(!error.contains("private"));
        assert!(!error.contains('\u{1b}'));
    }
}

#[test]
fn explicit_missing_or_invalid_config_fails_before_network_with_empty_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let missing = Command::new(env!("CARGO_BIN_EXE_anketa-scout"))
        .arg("--config")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert!(
        String::from_utf8(missing.stderr)
            .unwrap()
            .contains("отсутствует")
    );
    assert!(!path.exists());
    for content in [
        "sources = []",
        "sources = ['https://127.0.0.1/?token=private']",
        "bad TOML private",
    ] {
        fs::write(&path, content).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_anketa-scout"))
            .arg("--config")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("private")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }
}
