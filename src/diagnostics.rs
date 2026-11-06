//! Обезличенные JSON-события. API принимает только категории и счётчики.
use crate::domain::{ContactKind, Rejection};
use std::io::Write;

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

fn threshold() -> Level {
    parse_level(&std::env::var("SCOUT_LOG").unwrap_or_default())
}

fn parse_level(value: &str) -> Level {
    match value.to_ascii_uppercase().as_str() {
        "ERROR" => Level::Error,
        "WARN" => Level::Warn,
        "DEBUG" => Level::Debug,
        "TRACE" => Level::Trace,
        _ => Level::Info,
    }
}

fn emit(level: Level, value: serde_json::Value) {
    if level <= threshold() {
        // Сбой диагностического потока не меняет результат предметной операции.
        let _ = writeln!(std::io::stderr().lock(), "{value}");
    }
}

pub(crate) fn started() {
    emit(
        Level::Info,
        serde_json::json!({"level":"INFO","event":"Начало получения"}),
    );
}

pub(crate) fn failed(failure: crate::Failure) {
    emit(
        Level::Error,
        serde_json::json!({"level":"ERROR","event":"Получение прервано","code":failure.code()}),
    );
}

pub(crate) fn completed(processed: usize, accepted: usize, reasons: [usize; 5]) {
    emit(
        Level::Info,
        serde_json::json!({"level":"INFO","event":"Обработка завершена","processed":processed,"accepted":accepted,"rejected":processed-accepted,"reasons":{"Стартовый пост":reasons[0],"Чужой контакт":reasons[1],"Только контакт":reasons[2],"Нет контакта":reasons[3],"Невосстановленный контакт":reasons[4]}}),
    );
}

pub(crate) fn decision(post: u64, result: Result<usize, Rejection>) {
    if result == Err(Rejection::UnresolvedContact) {
        emit(
            Level::Warn,
            serde_json::json!({"level":"WARN","event":"Контакт не прошёл восстановление или проверку","post":post}),
        );
    }
    let (reason, count) = match result {
        Ok(count) => ("Анкета принята", count),
        Err(Rejection::Introductory) => ("Стартовый пост", 0),
        Err(Rejection::ForeignContact) => ("Чужой контакт или пример", 0),
        Err(Rejection::ContactOnly) => ("Только контакт или приписка", 0),
        Err(Rejection::NoOwnedContact) => ("Нет подтверждённого контакта", 0),
        Err(Rejection::UnresolvedContact) => ("Контакт не восстановлен или недействителен", 0),
    };
    emit(
        Level::Debug,
        serde_json::json!({"level":"DEBUG","event":"Решение отбора","post":post,"reason":reason,"contacts":count}),
    );
}

pub(crate) fn contact(post: u64, kind: ContactKind, operations: &[&'static str]) {
    let platform = match kind {
        ContactKind::Telegram => "Telegram",
        ContactKind::Discord => "Discord",
        ContactKind::Email => "Почта",
        ContactKind::Max => "MAX",
        ContactKind::Vk => "VK",
        ContactKind::Unknown => "Неизвестна",
    };
    emit(
        Level::Trace,
        serde_json::json!({"level":"TRACE","event":"Контакт нормализован","post":post,"platform":platform,"operations":operations}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_levels_filter_details_and_reject_arbitrary_values() {
        for name in ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
            let level = parse_level(name);
            assert!(Level::Error <= level);
            assert_eq!(Level::Trace <= level, name == "TRACE");
            assert_eq!(Level::Debug <= level, matches!(name, "DEBUG" | "TRACE"));
        }
        assert!(parse_level("private@example.invalid\u{1b}[31m") == Level::Info);
        assert!(parse_level("debug") == Level::Debug);
    }
}
