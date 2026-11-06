use super::{Attachment, MediaKind, re};
use regex::Regex;
use std::sync::LazyLock;

fn split_statements(text: &str) -> Vec<&str> {
    fn is_abbreviation(statement: &str) -> bool {
        let token = statement.split_whitespace().last().unwrap_or("");
        if token.chars().any(char::is_numeric) {
            return false;
        }
        let letters = token
            .chars()
            .filter(|character| character.is_alphabetic())
            .flat_map(|character| character.to_lowercase())
            .collect::<String>();
        letters.chars().count() == 1
            || matches!(
                letters.as_str(),
                "г" | "ул"
                    | "д"
                    | "пос"
                    | "п"
                    | "с"
                    | "см"
                    | "м"
                    | "км"
                    | "тд"
                    | "тп"
                    | "те"
                    | "им"
                    | "стр"
                    | "рис"
                    | "ок"
                    | "пр"
                    | "др"
            )
    }

    let mut statements = Vec::new();
    for clause in text.split(';') {
        let mut start = 0;
        for (offset, punctuation) in clause.char_indices() {
            let end = offset + punctuation.len_utf8();
            let tail = &clause[end..];
            let boundary = punctuation == '!'
                || (matches!(punctuation, '.' | '?')
                    && tail.chars().next().is_some_and(char::is_whitespace)
                    && !tail.trim().is_empty()
                    && (punctuation != '.' || !is_abbreviation(&clause[start..end])));
            if boundary {
                statements.push(&clause[start..offset]);
                start = end;
            }
        }
        if start < clause.len() {
            statements.push(&clause[start..]);
        }
    }
    statements
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gender {
    Male,
    Female,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Age {
    Exact(u8),
    AtLeast(u8),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    pub gender: Option<Gender>,
    pub age: Option<Age>,
    pub location: Option<String>,
    pub online: Option<bool>,
    pub images: usize,
    pub videos: usize,
}

impl Metadata {
    pub(super) fn extract(text: &str, attachments: &[Attachment]) -> Self {
        static SELF: LazyLock<Regex> = LazyLock::new(|| {
            re(
                r"(?i)^(?:(я|о себе|пол)\s*[:—-]?\s*)?(кунчик|кун|парень|мужчина|м|тянка|тян|девушка|женщина|ж|д)\b",
            )
        });
        static COMPACT: LazyLock<Regex> =
            LazyLock::new(|| re(r"(?i)^(?:я\s+)?([мжд])\s*([1-9][0-9])(\+?)(?:\s|[,.;]|$)"));
        static EXPLICIT_AGE: LazyLock<Regex> = LazyLock::new(|| {
            re(
                r"(?i)^(?:мне|возраст)\s*[:—-]?\s*([1-9][0-9])\s*(\+|лет\b|год(?:а)?\b|лвл\b|lvl\b)?",
            )
        });
        static AGE: LazyLock<Regex> = LazyLock::new(|| {
            re(
                r"(?i)^(?:(?:мне|возраст)\s*[:—-]?\s*)?([1-9][0-9])\s*(\+|лет\b|год(?:а)?\b|лвл\b|lvl\b)",
            )
        });
        static BARE_AGE: LazyLock<Regex> =
            LazyLock::new(|| re(r"^([1-9][0-9])(\+?)(?:\s*[,;.]|\s*$)"));
        static CITY: LazyLock<Regex> = LazyLock::new(|| {
            re(
                r"(?i)^(?:город|место|местоположение|живу в|я из)\s*[:—-]?\s*([\p{L}][\p{L} -]{1,60})[.]?$",
            )
        });
        static ALIAS: LazyLock<Regex> = LazyLock::new(|| {
            re(r"(?i)^(дс1|дс2|дс|мск|москва|спб|питер|санкт-петербург)(?:\s*[,;.]|\s*$)")
        });
        let mut value = Self::default();
        let mut genders = Vec::new();
        let mut ages = Vec::new();
        let mut cities = Vec::new();
        let mut online = Vec::new();
        let lines = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('>'))
            .collect::<Vec<_>>();
        for (line_index, line) in lines.iter().enumerate() {
            // Самоописание принимается только в начале самостоятельного предложения.
            for statement in split_statements(line) {
                let statement = statement.trim();
                static TARGET: LazyLock<Regex> = LazyLock::new(|| {
                    re(
                        r"(?i)^(?:кун|парень|мужчина|тян|девушка|женщина)\b.*\b(?:нужен|нужна|нужны|ищется|требуется)\b",
                    )
                });
                if TARGET.is_match(statement) {
                    continue;
                }
                let mut rest = statement;
                let mut compact_found = false;
                while let Some(c) = COMPACT.captures(rest) {
                    compact_found = true;
                    genders.push(if c[1].to_lowercase() == "м" {
                        Gender::Male
                    } else {
                        Gender::Female
                    });
                    if let Ok(n) = c[2].parse() {
                        ages.push(if &c[3] == "+" {
                            Age::AtLeast(n)
                        } else {
                            Age::Exact(n)
                        });
                    }
                    rest = rest[c[0].len()..].trim_start_matches([',', ' ', '.']);
                }
                if !compact_found && let Some(c) = SELF.captures(rest) {
                    let label = c[2].to_lowercase();
                    let ambiguous_abbreviation = c.get(1).is_none()
                        && matches!(label.as_str(), "м" | "ж" | "д")
                        && matches!(rest[c[0].len()..].chars().next(), Some('.' | '/'));
                    if !ambiguous_abbreviation {
                        genders.push(match label.as_str() {
                            "кун" | "кунчик" | "парень" | "мужчина" | "м" => {
                                Gender::Male
                            }
                            _ => Gender::Female,
                        });
                        rest = rest[c[0].len()..].trim_start_matches([',', ' ', ':']);
                    }
                }
                let explicit_age = EXPLICIT_AGE.captures(rest).filter(|capture| {
                    let tail = &rest[capture[0].len()..];
                    let decimal_fraction = tail.strip_prefix('.').is_some_and(|fraction| {
                        fraction
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_digit())
                    });
                    !tail.chars().next().is_some_and(char::is_alphanumeric) && !decimal_fraction
                });
                if let Some(c) = explicit_age {
                    if let Ok(n) = c[1].parse() {
                        ages.push(if c.get(2).is_some_and(|suffix| suffix.as_str() == "+") {
                            Age::AtLeast(n)
                        } else {
                            Age::Exact(n)
                        });
                    }
                    rest = rest[c[0].len()..].trim_start_matches([',', ' ', '.']);
                } else if let Some(c) = AGE.captures(rest).or_else(|| {
                    (rest != statement)
                        .then(|| BARE_AGE.captures(rest))
                        .flatten()
                }) {
                    if let Ok(n) = c[1].parse() {
                        ages.push(if &c[2] == "+" {
                            Age::AtLeast(n)
                        } else {
                            Age::Exact(n)
                        });
                    }
                    rest = rest[c[0].len()..].trim_start_matches([',', ' ', '.']);
                }
                let next_line_starts_with_contact = lines
                    .get(line_index + 1)
                    .is_some_and(|next| next.trim_start().starts_with('@'));
                if let Some(c) = ALIAS.captures(rest).filter(|c| {
                    !c[1].to_lowercase().starts_with("дс")
                        || !rest[c[0].len()..].contains('@')
                            && !rest.to_lowercase().contains("discord")
                            && !(c[1].to_lowercase() == "дс" && next_line_starts_with_contact)
                }) {
                    // «ДС: username» и «ДС Discord» не являются географией.
                    cities.push(
                        if matches!(
                            c[1].to_lowercase().as_str(),
                            "дс2" | "спб" | "питер" | "санкт-петербург"
                        ) {
                            "Санкт-Петербург".into()
                        } else {
                            "Москва".into()
                        },
                    );
                } else if let Some(c) = CITY.captures(rest) {
                    let city = c[1].trim();
                    let normalized = city.to_lowercase();
                    if !matches!(
                        normalized.as_str(),
                        "нет"
                            | "неизвестно"
                            | "неизвестен"
                            | "неизвестна"
                            | "неизвестны"
                            | "не знаю"
                            | "не указан"
                            | "не указана"
                            | "не указано"
                            | "не указаны"
                            | "не задано"
                            | "нет данных"
                            | "неважно"
                            | "не важно"
                            | "не имеет значения"
                            | "unknown"
                            | "unspecified"
                            | "not known"
                            | "not specified"
                            | "none"
                            | "na"
                            | "n/a"
                    ) {
                        let city = match normalized.as_str() {
                            "дс" | "дс1" | "мск" | "москва" => "Москва",
                            "дс2" | "спб" | "питер" | "санкт-петербург" => {
                                "Санкт-Петербург"
                            }
                            _ => city,
                        };
                        if city.chars().next().is_some_and(char::is_uppercase) {
                            cities.push(city.to_owned());
                        }
                    }
                }
                let lower = rest.to_lowercase();
                if matches!(
                    lower.trim_end_matches('.'),
                    "онлайн"
                        | "интернет"
                        | "сеть"
                        | "онлайн: да"
                        | "общаюсь онлайн"
                        | "предпочитаю онлайн"
                        | "онлайн-общение интересует"
                ) {
                    online.push(true);
                }
                if matches!(
                    lower.trim_end_matches('.'),
                    "онлайн-общение не интересует" | "онлайн: нет" | "онлайн не интересует"
                ) {
                    online.push(false);
                }
            }
        }
        fn unique<T: Eq + Clone>(items: &[T]) -> Option<T> {
            items
                .first()
                .filter(|first| items.iter().all(|item| item == *first))
                .cloned()
        }
        value.gender = unique(&genders);
        value.age = unique(&ages);
        value.location = unique(&cities);
        value.online = unique(&online);
        for attachment in attachments {
            match attachment.kind {
                MediaKind::Image => value.images += 1,
                MediaKind::Video => value.videos += 1,
                MediaKind::Other => {}
            }
        }
        value
    }
}
