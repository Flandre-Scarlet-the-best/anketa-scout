mod common;
use anketa_scout::{
    Failure,
    application::Cursor,
    domain::{Age, Gender, classify},
    source::normalize_after,
};
use common::{message, post, source};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn profile(text: &str) -> anketa_scout::domain::Profile {
    classify(message(&format!(
        "{text}<br>Люблю настольные игры.<br>ТГ: synthetic_user"
    )))
    .unwrap()
}

#[test]
fn explicit_metadata_and_lower_age_bounds_are_independent() {
    for (text, gender, age) in [
        ("м19", Some(Gender::Male), Some(Age::Exact(19))),
        ("д21", Some(Gender::Female), Some(Age::Exact(21))),
        ("кун 32", Some(Gender::Male), Some(Age::Exact(32))),
        (
            "Я девушка, 25+",
            Some(Gender::Female),
            Some(Age::AtLeast(25)),
        ),
        ("34 лвл", None, Some(Age::Exact(34))),
        ("25 лет", None, Some(Age::Exact(25))),
        ("20+", None, Some(Age::AtLeast(20))),
        ("м19+", Some(Gender::Male), Some(Age::AtLeast(19))),
    ] {
        let p = profile(text);
        assert_eq!(p.metadata.gender, gender, "{text}");
        assert_eq!(p.metadata.age, age, "{text}");
    }
    for text in [
        "ищу М 25+",
        "нужен парень",
        "Девушка нужна для общения",
        "кому за 30",
        "185/70",
        "150 см",
        "время 20:30",
        ">>12345",
        "Любила гулять",
        "ищу тян",
        "Не девушка",
    ] {
        let p = profile(text);
        assert_eq!(p.metadata.gender, None, "{text}");
        assert_eq!(p.metadata.age, None, "{text}");
    }
    let p = profile("м19<br>ж25");
    assert_eq!(p.metadata.gender, None);
    assert_eq!(p.metadata.age, None);
}

#[test]
fn location_online_negation_and_discord_are_not_guessed() {
    for (text, city) in [
        ("ДС1", "Москва"),
        ("ДС", "Москва"),
        ("МСК", "Москва"),
        ("Москва", "Москва"),
        ("ДС2", "Санкт-Петербург"),
        ("СПБ", "Санкт-Петербург"),
        ("Питер", "Санкт-Петербург"),
        ("Город: Бердск", "Бердск"),
    ] {
        assert_eq!(profile(text).metadata.location.as_deref(), Some(city));
    }
    for text in [
        "ДС: synthetic_name",
        "ДС Discord",
        "Часто бываю в Москве",
        "ищу девушку из Питера",
        "город неважен",
        "Город: неизвестно",
    ] {
        {
            assert_eq!(profile(text).metadata.location, None, "{text}");
        }
    }
    assert_eq!(profile("Онлайн").metadata.online, Some(true));
    assert_eq!(
        profile("онлайн-общение не интересует").metadata.online,
        Some(false)
    );
    assert_eq!(
        profile("Давно читал об онлайн-общении").metadata.online,
        None
    );
}

#[test]
fn display_cleanup_preserves_author_text_and_contact_order() {
    let p = classify(message("&gt;&gt;123<br>Люблю гулять по Москве, мне 25 лет.<br>ТГ: synthetic_one<br>Discord: synthetic_two<br>Почта: user@example.invalid")).unwrap();
    assert_eq!(p.description, "Люблю гулять по Москве, мне 25 лет.");
    assert_eq!(p.contacts.len(), 3);
    assert!(p.message.original().contains("synthetic_one"));
    let p = classify(message("Заметка.<br>@synthetic_one<br>VK: synthetic_two")).unwrap();
    assert!(p.contacts[0].assumed_platform);
    assert!(!p.contacts[1].assumed_platform);
    let p = classify(message("Люблю го. Мой ТГ: synthetic_one, пишите вечером.")).unwrap();
    assert!(p.description.contains("пишите вечером"));
    assert!(!p.description.contains("synthetic_one"));
}

#[test]
fn media_types_are_preserved_and_counted_without_downloads() {
    let mut p = post(101, "Люблю го. ТГ: synthetic_one");
    p["files"] = json!([
        {"path":"/test/src/a.jpg","type":1}, {"path":"/test/src/a.png","type":2},
        {"path":"/test/src/a.webm","type":6}, {"path":"/test/src/a.webp","type":9},
        {"path":"/test/src/a.mp4","type":10}, {"path":"/test/src/a.bin","type":99}
    ]);
    let messages = normalize_after(
        &source(),
        &serde_json::to_vec(&json!({"result":1,"posts":[p]})).unwrap(),
        100,
    )
    .unwrap();
    let p = classify(messages.into_iter().next().unwrap()).unwrap();
    assert_eq!((p.metadata.images, p.metadata.videos), (3, 2));
    assert_eq!(p.message.attachments[2].media_type, Some(6));
}

#[tokio::test]
async fn high_water_mark_includes_rejections_and_survives_invalid_batches() {
    let cancel = CancellationToken::new();
    let mut cursor = Cursor::default();
    let mut initial = message("Люблю го. ТГ: synthetic_one");
    initial.origin.post = 101;
    initial.origin.link = format!("{}#101", source().page());
    assert_eq!(
        cursor
            .process(source().page(), &cancel, vec![initial])
            .await
            .unwrap()
            .profiles
            .len(),
        1
    );
    let body = serde_json::to_vec(&json!({"result":1,"posts":[post(101,"Люблю го. ТГ: synthetic_one"),post(105,"Без контакта"),post(103,"Люблю го. ТГ: synthetic_two")]})).unwrap();
    let batch = normalize_after(&source(), &body, cursor.value()).unwrap();
    let report = cursor
        .process(source().page(), &cancel, batch)
        .await
        .unwrap();
    assert_eq!(report.processed, 2);
    assert_eq!(report.profiles.len(), 1);
    assert_eq!(cursor.value(), 105);
    assert!(normalize_after(&source(), br#"{"result":1,"posts":["#, cursor.value()).is_err());
    assert_eq!(cursor.value(), 105);
    let empty = normalize_after(&source(), br#"{"result":1,"posts":[]}"#, 105).unwrap();
    assert!(
        cursor
            .process(source().page(), &cancel, empty)
            .await
            .unwrap()
            .profiles
            .is_empty()
    );
    assert_eq!(cursor.value(), 105);
    let mut forged = message("Люблю го. ТГ: synthetic_three");
    forged.origin.post = 110;
    assert!(matches!(
        cursor.process(source().page(), &cancel, vec![forged]).await,
        Err(Failure::Format)
    ));
    assert_eq!(cursor.value(), 105);
}

#[test]
fn incremental_envelope_is_strict_and_boundary_is_deduplicated() {
    for posts in [vec![post(101, "a"), post(101, "a")], vec![post(99, "a")]] {
        assert!(
            normalize_after(
                &source(),
                &serde_json::to_vec(&json!({"result":1,"posts":posts})).unwrap(),
                100
            )
            .is_err()
        );
    }
    for body in [br#"{"result":0,"posts":[]}"#.as_slice(), br#"{}"#] {
        assert!(normalize_after(&source(), body, 100).is_err());
    }
}

#[test]
fn explicit_other_platform_wins_over_at_sign() {
    use anketa_scout::domain::ContactKind;
    for (label, kind) in [
        ("MAX", ContactKind::Max),
        ("Discord", ContactKind::Discord),
        ("VK", ContactKind::Vk),
    ] {
        let p = classify(message(&format!("Люблю го.<br>{label}: @synthetic_user"))).unwrap();
        assert_eq!(p.contacts[0].kind, kind);
        assert!(!p.contacts[0].assumed_platform);
    }
}

#[test]
fn timestamp_roundtrip_includes_os_offset_and_dst_dates() {
    use anketa_scout::source::normalize;
    for stamp in [0, 1711845000, 1729992600] {
        let mut p = post(101, "Люблю го. ТГ: synthetic_one");
        p["timestamp"] = json!(stamp);
        p["date"] = json!("не UTC и не локальная дата ОС");
        let messages = normalize(
            &source(),
            &serde_json::to_vec(&common::envelope(vec![post(100, "Начало"), p])).unwrap(),
        )
        .unwrap();
        let text = messages[1].date.as_ref().unwrap();
        let parsed = chrono::DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S %:z").unwrap();
        assert_eq!(parsed.timestamp(), stamp);
        assert_eq!(
            parsed.offset().local_minus_utc(),
            chrono::DateTime::from_timestamp(stamp, 0)
                .unwrap()
                .with_timezone(&chrono::Local)
                .offset()
                .local_minus_utc()
        );
        assert_eq!(messages[1].timestamp, Some(stamp));
        assert_eq!(
            messages[1].source_date.as_deref(),
            Some("не UTC и не локальная дата ОС")
        );
    }
}

#[test]
fn explicit_confirmation_replaces_assumption_without_reordering() {
    let p = classify(message("Люблю го.<br>@synthetic_one<br>ТГ: synthetic_one")).unwrap();
    assert_eq!(p.contacts.len(), 1);
    assert!(!p.contacts[0].assumed_platform);
    assert_eq!(profile("Онлайн<br>Онлайн: нет").metadata.online, None);
}

#[test]
fn appended_output_has_no_repeated_summary_and_empty_batch_is_silent() {
    use anketa_scout::{
        application::Report,
        output::{Options, render_batch},
    };
    let mut report = Report {
        source: source().page(),
        processed: 1,
        profiles: vec![profile("м19")],
    };
    let mut out = Vec::new();
    render_batch(&report, &mut out, Options::default(), false).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("Anketa Scout"));
    assert!(!text.contains("Сообщений"));
    assert!(text.contains("пост №101"));
    report.profiles.clear();
    let mut out = Vec::new();
    render_batch(&report, &mut out, Options::default(), false).unwrap();
    assert!(out.is_empty());
}

#[test]
fn restoration_cleanup_keeps_neighbouring_author_sentence() {
    let p = classify(message(
        "Люблю го.<br>ТГ: syn0thetic<br>(Убери нули). Люблю прогулки.",
    ))
    .unwrap();
    assert!(p.description.contains("Люблю прогулки"));
    let p = classify(message("Люблю го.<br>ТГ: syn0thetic<br>(Убери нули)")).unwrap();
    assert_eq!(p.description, "Люблю го.");
}

#[test]
fn unknown_optional_media_type_does_not_reject_a_profile() {
    for kind in [json!(999), json!("unknown"), json!(null)] {
        let mut p = post(101, "Люблю го. ТГ: synthetic_one");
        p["files"] = json!([{"path":"/test/src/a.png","type":kind}]);
        let messages = normalize_after(
            &source(),
            &serde_json::to_vec(&json!({"result":1,"posts":[p]})).unwrap(),
            100,
        )
        .unwrap();
        let p = classify(messages.into_iter().next().unwrap()).unwrap();
        assert_eq!(p.metadata.images, 1);
    }
}
#[cfg(unix)]
#[test]
fn local_time_zones_and_dst_are_checked_in_fresh_processes() {
    use anketa_scout::source::normalize;
    if let Ok(zone) = std::env::var("SCOUT_TZ_PROBE") {
        let expected = if zone == "UTC" {
            ["2024-03-10 06:30:00 +00:00", "2024-03-10 07:30:00 +00:00"]
        } else {
            ["2024-03-10 01:30:00 -05:00", "2024-03-10 03:30:00 -04:00"]
        };
        for (stamp, expected) in [1710052200i64, 1710055800].into_iter().zip(expected) {
            let mut p = post(101, "Люблю го. ТГ: synthetic_one");
            p["timestamp"] = json!(stamp);
            let messages = normalize(
                &source(),
                &serde_json::to_vec(&common::envelope(vec![post(100, "Начало"), p])).unwrap(),
            )
            .unwrap();
            assert_eq!(messages[1].date.as_deref(), Some(expected));
        }
        return;
    }
    for zone in ["UTC", "America/New_York"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "local_time_zones_and_dst_are_checked_in_fresh_processes",
            ])
            .env("TZ", zone)
            .env("SCOUT_TZ_PROBE", zone)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn explicit_platform_on_preceding_line_keeps_priority_and_is_not_description() {
    use anketa_scout::domain::ContactKind;
    for (label, kind) in [
        ("Discord", ContactKind::Discord),
        ("MAX", ContactKind::Max),
        ("VK", ContactKind::Vk),
        ("ТГ", ContactKind::Telegram),
    ] {
        let p = classify(message(&format!(
            "Люблю го.<br>{label}:<br>@synthetic_user"
        )))
        .unwrap();
        assert_eq!(p.contacts[0].kind, kind);
        assert!(!p.contacts[0].assumed_platform);
        assert_eq!(p.description, "Люблю го.");
        assert!(classify(message(&format!("{label}:<br>@synthetic_user"))).is_err());
    }
}
