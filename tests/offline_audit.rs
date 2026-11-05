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
