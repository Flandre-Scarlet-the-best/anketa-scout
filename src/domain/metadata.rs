use super::{Attachment, re};
use regex::Regex;
use std::sync::LazyLock;

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
                r"(?i)^(?:(?:я|о себе|пол)\s*[:—-]?\s*)?(кунчик|кун|парень|мужчина|м|тянка|тян|девушка|женщина|ж|д)\b",
            )
        });
        static COMPACT: LazyLock<Regex> =
            LazyLock::new(|| re(r"(?i)^(?:я\s+)?([мжд])\s*([1-9][0-9])(\+?)(?:\s|[,.;]|$)"));
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
        for line in text
            .lines()
            .filter(|line| !line.trim_start().starts_with('>'))
        {
            // Самоописание принимается только в начале самостоятельного предложения.
            for statement in line.split(['\n', ';', '!']) {
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
                if let Some(c) = COMPACT.captures(rest) {
                    genders.push(if c[1].eq_ignore_ascii_case("м") {
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
                } else if let Some(c) = SELF.captures(rest) {
                    genders.push(match c[1].to_lowercase().as_str() {
                        "кун" | "кунчик" | "парень" | "мужчина" | "м" => {
                            Gender::Male
                        }
                        _ => Gender::Female,
                    });
                    rest = rest[c[0].len()..].trim_start_matches([',', ' ', ':']);
                }
                if let Some(c) = AGE.captures(rest).or_else(|| {
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
                if let Some(c) = ALIAS.captures(rest).filter(|c| {
                    !c[1].to_lowercase().starts_with("дс")
                        || !rest[c[0].len()..].contains('@')
                            && !rest.to_lowercase().contains("discord")
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
                    let city = match city.to_lowercase().as_str() {
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
            let ext = attachment
                .link
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            match attachment.media_type {
                Some(1 | 2 | 9) => value.images += 1,
                Some(6 | 10) => value.videos += 1,
                _ if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") => {
                    value.images += 1
                }
                _ if matches!(ext.as_str(), "webm" | "mp4" | "mov") => value.videos += 1,
                _ => {}
            }
        }
        value
    }
}
