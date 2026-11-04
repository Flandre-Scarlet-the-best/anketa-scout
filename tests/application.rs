mod common;
use anketa_scout::{
    Failure,
    application::run,
    output::{render, safe},
};
use common::{message, source};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn one_post_is_one_profile_and_contacts_are_never_merged() {
    let mut description = message("Люблю шахматы.");
    description.origin.post = 101;
    let mut addition = message("Забыл контакт, вот @synthetic_user");
    addition.origin.post = 102;
    let mut complete = message("Люблю го.<br>Тг: other_synthetic");
    complete.origin.post = 103;
    let report = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![description, addition, complete])
    })
    .await
    .unwrap();
    assert_eq!(report.processed, 3);
    assert_eq!(report.profiles.len(), 1);
    assert_eq!(report.profiles[0].message.origin.post, 103);
    assert_eq!(report.profiles[0].contacts[0].value, "Тг: other_synthetic");
}

#[tokio::test]
async fn failures_propagate_without_becoming_zero_profiles() {
    for failure in [
        Failure::Network,
        Failure::Timeout,
        Failure::Access,
        Failure::RateLimit,
        Failure::Unavailable,
        Failure::Format,
        Failure::Partial,
        Failure::Limit,
    ] {
        let result = run(source().page(), &CancellationToken::new(), async {
            Err(failure)
        })
        .await;
        assert_eq!(result.err(), Some(failure));
    }
}

#[tokio::test]
async fn plain_redirected_output_has_contacts_origin_text_date_and_attachments() {
    let mut profile =
        message("Люблю <b>го</b>.<br>Тг: synthetic_user\u{1b}]0;опасный заголовок\u{07}");
    profile.date = Some("1970-01-01 00:00:00 UTC".into());
    profile.attachments.push(anketa_scout::domain::Attachment {
        name: Some("Схема\u{1b}[31m.png".into()),
        link: "https://2ch.org/test/src/100/file.png".into(),
        size_kib: Some(2),
    });
    let report = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![profile])
    })
    .await
    .unwrap();
    let mut bytes = vec![];
    render(&report, &mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for expected in [
        "Источник: https://2ch.org/test/res/100.html",
        "Обработано сообщений: 1; анкет: 1",
        "пост №101",
        "https://2ch.org/test/res/100.html#101",
        "Telegram: Тг: synthetic_user",
        "Люблю го.",
        "Дата: 1970-01-01 00:00:00 UTC",
        "Вложение: https://2ch.org/test/src/100/file.png",
        "2 КиБ",
    ] {
        assert!(text.contains(expected), "Отсутствует {expected}");
    }
    assert!(!text.contains(['\u{1b}', '\u{07}']));
    assert!(!text.contains("<b>"));
}

#[tokio::test]
async fn empty_thread_and_no_profiles_have_different_summaries() {
    let empty = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![])
    })
    .await
    .unwrap();
    let no_profiles = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![message("Привет")])
    })
    .await
    .unwrap();
    let mut output = vec![];
    render(&empty, &mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().contains("пустой тред"));
    let mut output = vec![];
    render(&no_profiles, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("не найдено"));
    assert!(!text.contains("пустой тред"));
    assert!(!text.contains("Дата:"));
}

#[tokio::test]
async fn all_terminal_controls_and_bidi_overrides_are_removed() {
    let mut input = String::from("Привет\nмир\t");
    input.extend(
        (0..=159)
            .filter_map(char::from_u32)
            .filter(|c| c.is_control() && !matches!(c, '\n' | '\t')),
    );
    input.push_str("\u{202e}\u{2066}\u{200b}\u{feff}");
    assert_eq!(safe(&input), "Привет мир  ");
}

#[tokio::test]
async fn output_write_failure_is_not_reported_as_success() {
    struct Failing;
    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("Сбой записи"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let report = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![])
    })
    .await
    .unwrap();
    assert_eq!(
        render(&report, Failing).unwrap_err().kind(),
        std::io::ErrorKind::Other
    );
}

#[tokio::test]
async fn final_flush_failure_is_not_lost() {
    struct FlushFailure;
    impl std::io::Write for FlushFailure {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("Сбой сброса"))
        }
    }
    let report = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![])
    })
    .await
    .unwrap();
    assert_eq!(
        render(&report, FlushFailure).unwrap_err().kind(),
        std::io::ErrorKind::Other
    );
}

#[tokio::test]
async fn remote_text_cannot_forge_report_headers_or_metadata_fields() {
    let mut input =
        message("Люблю го.<br>Telegram: synthetic_user<br>Оригинал: подделка<br>── Анкета 999 ──");
    input.date = Some("Сегодня\nОригинал: подделка\tполе".into());
    input.attachments.push(anketa_scout::domain::Attachment {
        name: Some("Файл\nДата: подделка\tполе".into()),
        link: "https://2ch.org/test/src/100/file.png".into(),
        size_kib: None,
    });
    let report = run(source().page(), &CancellationToken::new(), async {
        Ok(vec![input])
    })
    .await
    .unwrap();
    let mut output = vec![];
    render(&report, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("\n  | Оригинал: подделка\n"));
    assert!(text.contains("\n  | ── Анкета 999 ──\n"));
    assert!(text.contains("Дата: Сегодня Оригинал: подделка поле\n"));
    assert!(text.contains(" · Файл Дата: подделка поле\n"));
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("Оригинал:"))
            .count(),
        1
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("Дата:"))
            .count(),
        1
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("── Анкета"))
            .count(),
        1
    );
    assert!(!text.contains('\t'));
}

#[tokio::test]
async fn pre_cancelled_run_never_starts_the_source() {
    let token = CancellationToken::new();
    token.cancel();
    let mut called = false;
    let report = run(source().page(), &token, async {
        called = true;
        Ok(vec![])
    })
    .await;
    assert_eq!(report.err(), Some(Failure::Cancelled));
    assert!(!called);
}

#[tokio::test]
async fn cancellation_drops_pending_source_instead_of_returning_partial_report() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let check = dropped.clone();
    let token = CancellationToken::new();
    let request_token = token.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        run(source().page(), &request_token, async {
            let _guard = Guard(dropped);
            started.send(()).unwrap();
            std::future::pending().await
        })
        .await
    });
    ready.await.unwrap();
    token.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.err(), Some(Failure::Cancelled));
    assert!(check.load(Ordering::SeqCst));
}
