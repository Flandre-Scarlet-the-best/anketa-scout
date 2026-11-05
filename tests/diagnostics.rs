mod common;
use anketa_scout::{Failure, application};
use std::process::Command;
use tokio_util::sync::CancellationToken;

#[test]
fn logging_levels_and_streams_never_expose_remote_content() {
    for (level, expected) in [
        ("ERROR", vec!["ERROR"]),
        ("WARN", vec!["ERROR", "WARN"]),
        ("INFO", vec!["ERROR", "WARN", "INFO"]),
        ("DEBUG", vec!["ERROR", "WARN", "INFO", "DEBUG"]),
        ("TRACE", vec!["ERROR", "WARN", "INFO", "DEBUG", "TRACE"]),
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "logging_probe", "--ignored", "--nocapture"])
            .env("SCOUT_LOG", level)
            .output()
            .unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        for secret in [
            "synthetic_secret",
            "private@example.invalid",
            "Секретный текст",
            "\u{1b}",
        ] {
            assert!(!stderr.contains(secret));
            assert!(!stdout.contains(secret));
        }
        let events = stderr
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        for allowed in &expected {
            assert!(events.iter().any(|v| v["level"] == *allowed));
        }
        assert!(
            events
                .iter()
                .all(|v| expected.iter().any(|allowed| v["level"] == *allowed))
        );
    }
}

#[tokio::test]
#[ignore = "Дочерний процесс проверки изоляции stderr и уровней"]
async fn logging_probe() {
    let cancellation = CancellationToken::new();
    let messages = vec![
        common::message("Секретный текст.<br>Тг: synthetic_secret<br>private@example.invalid"),
        common::message("Секретный текст.<br>Discord: x"),
    ];
    assert_eq!(
        application::run(common::source().page(), &cancellation, async {
            Ok(messages)
        })
        .await
        .unwrap()
        .profiles
        .len(),
        1
    );
    assert!(
        application::run(common::source().page(), &cancellation, async {
            Err(Failure::Network)
        })
        .await
        .is_err()
    );
}
