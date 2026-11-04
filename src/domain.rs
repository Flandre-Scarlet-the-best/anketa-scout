use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub struct Origin {
    pub source: String,
    pub board: String,
    pub thread: u64,
    pub post: u64,
    pub link: String,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: Option<String>,
    pub link: String,
    pub size_kib: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub origin: Origin,
    pub(crate) original: String,
    pub(crate) text: String,
    pub(crate) authored_text: String,
    pub date: Option<String>,
    pub attachments: Vec<Attachment>,
    pub introductory: bool,
}

impl Message {
    pub fn original(&self) -> &str {
        &self.original
    }
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Свидетельства авторства формирует адаптер вместе с исходным и читаемым текстом.
    ///
    /// ```compile_fail
    /// use anketa_scout::domain::Message;
    /// fn forge(message: &mut Message) {
    ///     message.authored_text = "Люблю го. Мой Тг: synthetic_user".into();
    /// }
    /// ```
    pub fn authored_text(&self) -> &str {
        &self.authored_text
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContactKind {
    Telegram,
    Discord,
    Email,
    Vk,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ownership {
    ExplicitSelf,
    ProfileSignature,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contact {
    pub kind: ContactKind,
    pub value: String,
    pub ownership: Ownership,
    pub restoration: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub message: Message,
    pub contacts: Vec<Contact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejection {
    Introductory,
    ForeignContact,
    ContactOnly,
    NoOwnedContact,
    UnresolvedContact,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("Корректное постоянное регулярное выражение")
}

static CONTACTS: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?ix)(?P<email>[a-z0-9.!\#$%&'*+/=?^_~-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)+)",
        r"|(?P<url>(?:https?://)?(?:t\.me|telegram\.me|vk\.com|vk\.ru)/[^\s<>]+)",
        r"|(?P<postfix>[a-z0-9_.\#]{2,32}[\x20\t]+(?:discord|дискорд)\b)",
        r"|(?P<label>\b(?:telegram|телеграмм?|телега|тгшка|тг|tg|discord|дискордик|дискорд|диск|дс|вк|vk)\b[\x20\t]*[:=—-]?[\x20\t]*@?[a-z0-9_][a-z0-9_.\#()/:+~-]*)",
        r"|(?P<handle>@[a-z][a-z0-9_().+~-]*)",
        r"|(?P<unlabelled>(?m:^[\x20\t]*[a-z][a-z0-9_]*(?:[\x20\t]+[a-z0-9_]+)*))"
    ))
});
static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)^(telegram|телеграмм?|телега|тгшка|тг|tg|discord|дискордик|дискорд|диск|дс|вк|vk)\s*[:=—-]?\s*",
    )
});
static FOREIGN: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)(?:его|её|ее|чужой|контакт\s+друга|пример|например|смотрю|читаю|мой\s+канал|наш\s+сервер)\s*[:—-]?\s*$",
    )
});
static DECORATION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)\b(?:мой|моя|мои|вот|пишите|сюда|мне|контакт|связь|почта|mailto|email|e-mail|telegram|телеграмм?|тг|tg|дискорд|discord|забыл|забыла|добавить|добавлю|к\s+прошлому\s+посту)\b",
    )
});
static INSTRUCTION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)(?:убери(?:те)?|убрать|удали(?:те)?|удалить|без\s+(?:пробел|нижн)|замени(?:те)?)[^\n]*",
    )
});
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?s)"[^"]*"|«[^»]*»|“[^”]*”|‘[^’]*’|\x60[^\x60]*\x60"#));

fn restore(raw: &str, instruction: &str) -> Option<String> {
    let mut value = raw.to_owned();
    let original_instruction = instruction;
    let instruction = instruction.to_lowercase();
    if instruction.is_empty() {
        return Some(value);
    }
    let mut applied = false;
    if instruction.contains("пробел") {
        value.retain(|c| !c.is_whitespace());
        applied = true;
    }
    if instruction.contains("нул") {
        value = value.replace('0', "");
        applied = true;
    }
    if instruction.contains("скоб") {
        value = value.replace(['(', ')'], "");
        applied = true;
    }
    if instruction.contains("подчеркиван") || instruction.contains("подчёркиван")
    {
        value = value.replace('_', "");
        applied = true;
    }
    static REPLACE: LazyLock<Regex> = LazyLock::new(|| {
        re(r#"(?i)замени(?:те)?\s+['"«]?([a-z0-9_+.-]+)['"»]?\s+на\s+['"«]?([a-z0-9_+.-]+)"#)
    });
    if let Some(c) = REPLACE.captures(original_instruction) {
        if !value.contains(&c[1]) {
            return None;
        }
        value = value.replace(&c[1], &c[2]);
        applied = true;
    }
    static REMOVE: LazyLock<Regex> =
        LazyLock::new(|| re(r#"(?i)(?:символ|букву)\s*['"«]?([a-z0-9_+.-])['"»]?(?:\s|$|\))"#));
    if let Some(c) = REMOVE.captures(original_instruction) {
        if !value.contains(&c[1]) {
            return None;
        }
        value = value.replace(&c[1], "");
        applied = true;
    }
    applied.then_some(value)
}

fn canonical(kind: ContactKind, raw: &str) -> Option<String> {
    let raw = raw
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("t.me/")
        .trim_start_matches("telegram.me/")
        .trim_start_matches("vk.com/")
        .trim_start_matches("vk.ru/")
        .trim_end_matches([',', ';']);
    let raw = if kind == ContactKind::Discord {
        raw
    } else {
        raw.trim_end_matches('.')
    };
    let normalized = if kind == ContactKind::Discord && !raw.contains('#') {
        raw.to_ascii_lowercase()
    } else {
        raw.to_owned()
    };
    let raw = normalized.as_str();
    let valid = match kind {
        ContactKind::Telegram => {
            static VALID: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][a-zA-Z0-9_]{4,31}$"));
            VALID.is_match(raw)
                && !raw.to_lowercase().ends_with("bot")
                && !matches!(
                    raw.to_lowercase().as_str(),
                    "joinchat"
                        | "share"
                        | "proxy"
                        | "addstickers"
                        | "addemoji"
                        | "socks"
                        | "login"
                        | "boost"
                )
        }
        ContactKind::Discord => {
            static VALID: LazyLock<Regex> = LazyLock::new(|| {
                re(r"^(?:[a-z0-9_][a-z0-9_.]{1,31}|[a-zA-Z0-9_.]{2,32}#[0-9]{4})$")
            });
            VALID.is_match(raw) && !raw.contains("..")
        }
        ContactKind::Vk => {
            static VALID: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][a-zA-Z0-9_.]{1,63}$"));
            VALID.is_match(raw) && !raw.starts_with("club") && !raw.starts_with("public")
        }
        ContactKind::Unknown => {
            static VALID: LazyLock<Regex> = LazyLock::new(|| re(r"^@?[a-zA-Z][a-zA-Z0-9_]{1,63}$"));
            VALID.is_match(raw)
        }
        ContactKind::Email => raw.split_once('@').is_some_and(|(local, domain)| {
            !local.starts_with('.')
                && !local.ends_with('.')
                && !local.contains("..")
                && domain
                    .split('.')
                    .all(|s| !s.is_empty() && !s.starts_with('-') && !s.ends_with('-'))
        }),
    };
    if !valid {
        return None;
    }
    Some(match kind {
        ContactKind::Telegram => format!("@{}", raw.to_ascii_lowercase()),
        ContactKind::Vk => raw.to_ascii_lowercase(),
        ContactKind::Email => {
            let (local, domain) = raw.split_once('@')?;
            format!("{local}@{}", domain.to_ascii_lowercase())
        }
        _ => raw.to_owned(),
    })
}

fn exclude_single_quotes(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    let mut quoted = false;
    let mut quote = String::new();
    let mut instruction_start = INSTRUCTION
        .find(text.lines().next().unwrap_or(""))
        .map(|m| m.start());
    while let Some((index, c)) = chars.next() {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        if c == '\n' {
            let start = index + 1;
            instruction_start = INSTRUCTION
                .find(text[start..].lines().next().unwrap_or(""))
                .map(|m| start + m.start());
        }
        if c == '\'' && !quoted && instruction_start.is_some_and(|start| start < index) {
            result.push(c);
        } else if c == '\'' && !quoted && !previous.is_some_and(word) {
            quoted = true;
            quote.clear();
            quote.push(c);
        } else if c == '\'' && quoted && !chars.peek().is_some_and(|(_, c)| word(*c)) {
            quoted = false;
            quote.clear();
            result.push_str("\n\n");
        } else if quoted {
            quote.push(c);
        } else if !quoted {
            result.push(c);
        }
        previous = Some(c);
    }
    if quoted {
        result.push_str(&quote);
    }
    result
}

pub fn classify(message: Message) -> Result<Profile, Rejection> {
    if message.introductory {
        return Err(Rejection::Introductory);
    }
    static EXAMPLE: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?i)^\s*(?:цитирую|шаблон\s+анкеты|пример\s+анкеты)\s*:"));
    if EXAMPLE.is_match(&message.authored_text) {
        return Err(Rejection::ForeignContact);
    }
    let authored = message
        .authored_text
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");
    let unquoted = QUOTED.replace_all(&authored, |capture: &regex::Captures<'_>| {
        let matched = capture.get(0).unwrap();
        let prefix = authored[..matched.start()]
            .rsplit('\n')
            .next()
            .unwrap_or("");
        if INSTRUCTION.is_match(prefix) {
            matched.as_str().to_owned()
        } else {
            "\n".to_owned()
        }
    });
    let authored = exclude_single_quotes(&unquoted);
    let mut remainder = authored.clone();
    let mut contacts = Vec::new();
    let mut previous_contact: Option<(usize, ContactKind)> = None;
    let mut foreign = false;
    let mut unresolved = false;
    for cap in CONTACTS.captures_iter(&authored) {
        let m = cap.get(0).unwrap();
        let prefix = authored[..m.start()]
            .rsplit(['\n', '.', '!', '?', ';'])
            .next()
            .unwrap_or("")
            .trim();
        if FOREIGN.is_match(prefix) {
            foreign = true;
            continue;
        }
        let line_end = authored[m.end()..]
            .find('\n')
            .map_or(authored.len(), |n| m.end() + n);
        let next_end = authored[line_end..]
            .get(1..)
            .and_then(|s| s.find('\n').map(|n| line_end + 1 + n))
            .unwrap_or(authored.len());
        let nearby = &authored[m.end()..next_end];
        let instruction = INSTRUCTION
            .find(nearby)
            .filter(|i| {
                CONTACTS
                    .captures_iter(&nearby[..i.start()])
                    .all(|c| c.name("unlabelled").is_some())
            })
            .map(|m| m.as_str())
            .unwrap_or("");
        let (kind, mut raw) = if cap.name("postfix").is_some() {
            (
                ContactKind::Discord,
                m.as_str().split_whitespace().next().unwrap().to_owned(),
            )
        } else if cap.name("unlabelled").is_some() {
            if instruction.is_empty() {
                continue;
            }
            (ContactKind::Unknown, m.as_str().trim().to_owned())
        } else if cap.name("email").is_some() {
            (ContactKind::Email, m.as_str().to_owned())
        } else if cap.name("url").is_some() {
            let url = m.as_str().trim_end_matches([')', '.', ',']);
            let (host, path) = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split_once('/')
                .unwrap();
            (
                if host.starts_with("vk.") {
                    ContactKind::Vk
                } else {
                    ContactKind::Telegram
                },
                path.to_owned(),
            )
        } else if cap.name("label").is_some() {
            let label = LABEL.find(m.as_str()).unwrap();
            let name = label.as_str().to_lowercase();
            if name.starts_with("дс")
                && !name.contains(':')
                && m.as_str()[label.end()..].starts_with(|c: char| c.is_ascii_digit())
            {
                continue;
            }
            let kind = if name.starts_with('д') || name.starts_with("discord") {
                ContactKind::Discord
            } else if name.starts_with("вк") || name.starts_with("vk") {
                ContactKind::Vk
            } else {
                ContactKind::Telegram
            };
            (
                kind,
                m.as_str()[label.end()..].trim_start_matches('@').to_owned(),
            )
        } else {
            let remaining_prefix = remainder[..m.start()]
                .rsplit(['\n', '.', '!', '?', ';'])
                .next()
                .unwrap_or("");
            static SEPARATOR: LazyLock<Regex> =
                LazyLock::new(|| re(r"^\s*[,;/]*\s*(?:(?:и|или)\s*)?$"));
            let cleaned = DECORATION.replace_all(remaining_prefix, "");
            if !SEPARATOR.is_match(&cleaned)
                && !cleaned
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .is_empty()
            {
                continue;
            }
            let continued = previous_contact.filter(|(end, kind)| {
                *kind == ContactKind::Telegram
                    && !authored[*end..m.start()].contains('\n')
                    && SEPARATOR.is_match(&authored[*end..m.start()])
            });
            if continued.is_some() {
                (
                    ContactKind::Telegram,
                    m.as_str().trim_start_matches('@').to_owned(),
                )
            } else {
                (ContactKind::Unknown, m.as_str().to_owned())
            }
        };
        let suffix = &authored[m.end()..];
        if suffix.starts_with(|c: char| {
            c.is_alphanumeric() || matches!(c, '_' | '/' | '?' | '#' | '=' | '-' | ':' | '+' | '~')
        }) {
            continue;
        }
        let mut end = m.end();
        if !instruction.is_empty() {
            let tail = nearby.split('\n').next().unwrap_or("");
            let tail = INSTRUCTION.find(tail).map_or(tail, |i| &tail[..i.start()]);
            let tail = tail.trim_end().trim_end_matches('(').trim_end();
            if tail
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '(' | ')'))
            {
                raw.push_str(tail);
                end += tail.len();
            }
            let Some(value) = restore(&raw, instruction) else {
                unresolved = true;
                continue;
            };
            raw = value;
        }
        let Some(value) = canonical(kind, &raw) else {
            unresolved = true;
            continue;
        };
        // Стираем байты, сохраняя позиции последующих совпадений UTF-8.
        remainder.replace_range(m.start()..end, &" ".repeat(end - m.start()));
        previous_contact = Some((end, kind));
        let contact = Contact {
            kind,
            value,
            restoration: if instruction.is_empty() {
                vec![]
            } else {
                let lower = instruction.to_lowercase();
                [
                    ("пробел", "Удаление пробелов"),
                    ("нул", "Удаление нулей"),
                    ("скоб", "Удаление скобок"),
                    ("подчеркиван", "Удаление подчёркивания"),
                    ("подчёркиван", "Удаление подчёркивания"),
                    ("замени", "Замена литерала"),
                    ("символ", "Удаление литерала"),
                ]
                .into_iter()
                .filter(|(key, _)| lower.contains(key))
                .map(|(_, name)| name)
                .collect()
            },
            ownership: if prefix.to_lowercase().contains("мой") {
                Ownership::ExplicitSelf
            } else {
                Ownership::ProfileSignature
            },
        };
        if !contacts
            .iter()
            .any(|c: &Contact| c.kind == contact.kind && c.value == contact.value)
        {
            contacts.push(contact);
        }
    }
    if contacts.is_empty() {
        return Err(if foreign {
            Rejection::ForeignContact
        } else if unresolved {
            Rejection::UnresolvedContact
        } else {
            Rejection::NoOwnedContact
        });
    }
    let remainder = INSTRUCTION.replace_all(&remainder, "");
    let remainder = DECORATION.replace_all(&remainder, "");
    static CONTACT_LIST: LazyLock<Regex> =
        LazyLock::new(|| re(r"^[\s\p{P}]*(?:(?:и|или)[\s\p{P}]*)*$"));
    if CONTACT_LIST.is_match(&remainder) || !remainder.chars().any(char::is_alphanumeric) {
        return Err(Rejection::ContactOnly);
    }
    Ok(Profile { message, contacts })
}
