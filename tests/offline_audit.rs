use anketa_scout::{
    application,
    domain::classify,
    source::{ThreadUrl, normalize},
};
use std::collections::BTreeMap;

/// Явно включаемая проверка локального снимка; содержимое не выводится.
#[tokio::test]
#[ignore = "Нужен локальный снимок в SCOUT_AUDIT_JSON"]
async fn offline_snapshot_classification() {
    let bytes = std::fs::read(std::env::var_os("SCOUT_AUDIT_JSON").expect("Путь снимка"))
        .expect("Чтение снимка");
    let source = ThreadUrl::parse("https://2ch.org/soc/res/5959967.json").unwrap();
    let messages = normalize(&source, &bytes).expect("Полный корректный снимок");
    let mut counts = BTreeMap::new();
    let mut platforms = BTreeMap::new();
    let mut accepted = Vec::new();
    let mut instructions = 0;
    let instruction_pattern =
        regex::Regex::new("(?i)убери|убрать|без пробел|без нижнего|замени").unwrap();
    for message in messages.clone() {
        let post = message.origin.post;
        let instruction = instruction_pattern.is_match(message.text());
        instructions += usize::from(instruction);
        let reason = match classify(message) {
            Ok(profile) => {
                accepted.push(post);
                for contact in &profile.contacts {
                    *platforms
                        .entry(format!("{:?}", contact.kind))
                        .or_insert(0usize) += 1;
                }
                format!("Принят:{}", profile.contacts.len())
            }
            Err(reason) => format!("{reason:?}"),
        };
        println!("{post} {reason}");
        *counts.entry(reason).or_insert(0usize) += 1;
    }
    println!("Итоги: {counts:?}");
    println!("Платформы: {platforms:?}; инструкции: {instructions}");
    let report = application::run(
        source.page(),
        &tokio_util::sync::CancellationToken::new(),
        async { Ok(messages) },
    )
    .await
    .unwrap();
    assert_eq!(
        report
            .profiles
            .iter()
            .map(|p| p.message.origin.post)
            .collect::<Vec<_>>(),
        accepted
    );
}

/// Доказательства локального живого пакета и ширин без раскрытия содержимого.
#[test]
#[ignore = "Нужен локальный каталог SCOUT_AUDIT_DIR с живыми ответами"]
fn live_captures_normalize_and_render_without_exposing_content() {
    let root = std::path::PathBuf::from(std::env::var_os("SCOUT_AUDIT_DIR").expect("Каталог"));
    let source = ThreadUrl::parse("https://2ch.org/soc/res/5959967.json").unwrap();
    let messages = normalize(&source, &std::fs::read(root.join("thread.json")).unwrap()).unwrap();
    let last = messages.iter().map(|m| m.origin.post).max().unwrap();
    let boundary = anketa_scout::source::normalize_after(
        &source,
        &std::fs::read(root.join(format!("after-{last}.json"))).unwrap(),
        last,
    )
    .unwrap();
    assert!(boundary.is_empty());
    let empty = anketa_scout::source::normalize_after(
        &source,
        &std::fs::read(root.join(format!("after-{}.json", last + 1))).unwrap(),
        last,
    )
    .unwrap();
    assert!(empty.is_empty());
    let report = application::Report {
        source: source.page(),
        processed: messages.len(),
        profiles: messages
            .into_iter()
            .filter_map(|m| classify(m).ok())
            .collect(),
    };
    for width in [40, 80, 120] {
        let mut out = Vec::new();
        anketa_scout::output::render_with_options(
            &report,
            &mut out,
            anketa_scout::output::Options {
                width: Some(width),
                ..Default::default()
            },
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("html#"));
        assert!(!text.contains("Вложение"));
        assert!(
            text.lines()
                .all(|line| console::measure_text_width(line) == usize::from(width))
        );
        std::fs::write(root.join(format!("width-{width}.txt")), text).unwrap();
    }
    println!(
        "Проверены живые пакеты, граница, пустой ответ и ширины 40/80/120; анкет: {}",
        report.profiles.len()
    );
}
