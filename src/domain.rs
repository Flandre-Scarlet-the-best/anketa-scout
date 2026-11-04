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
    pub original: String,
    pub text: String,
    pub authored_text: String,
    pub date: Option<String>,
    pub attachments: Vec<Attachment>,
    pub introductory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContactKind {
    Telegram,
    Discord,
    Email,
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
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub message: Message,
    pub contacts: Vec<Contact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejection {
    Introductory,
    TemplateOrThirdParty,
    NoDescription,
    NoOwnedContact,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("Корректное постоянное регулярное выражение")
}

static CONTACTS: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?ix)",
        r"(?P<email>[a-z0-9.!\#$%&'*+/=?^_`{|}~-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)+)",
        r"|(?P<tg>(?:https?://)?(?:t\.me|telegram\.me)/[a-z][a-z0-9_]{4,31}\b)",
        r"|(?P<label>(?:telegram|телеграм(?:м)?|телега|тг|tg)\s*[:=—-]?\s*",
        r"(?:(?:https?://)?(?:t\.me|telegram\.me)/)?@?[a-z][a-z0-9_]{4,31}\b)",
        r"|(?P<discord>(?:discord|дискорд)\s*[:=—-]?\s*[a-z0-9_][a-z0-9_.]{1,31}(?:\#\d{4})?)",
        r"|(?P<handle>@[a-z][a-z0-9_]{4,31}\b)",
    ))
});
static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)(?:\b(?:люблю|обожаю|увлекаюсь|интересуюсь|читаю|играю|рисую|занимаюсь|изучаю|ищу|хочу|мечтаю|любитель|любительница|поболтаем|поговорим|поговорю|обсудим|составлю\s+компанию)\s+[\p{L}\p{N}][\p{L}\p{N}-]*|(?:о\s+себе|мои\s+интересы|мои\s+увлечения|для\s+общения|познакомлюсь)\s*[:—-]?\s+\p{L}+|\bя\s+(?:\p{L}+\s+)?(?:художник|программист|студент|музыкант|интроверт|коллекционер)\b|\bi\s+(?:love|enjoy|like|am|want)\s+\p{L}+)",
    )
});
static FOREIGN_OR_TEMPLATE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)(?:чуж(?:ой|ая|ую|ие)|его\s+(?:контакт|телеграм|тг|ник)|е[её]\s+(?:контакт|телеграм|тг|ник)|(?:друга|подруги|автора)\s+(?:контакт|телеграм|тг|ник)|(?:контакт|телеграм|тг|ник)\s+(?:друга|подруги|автора)|цитирую|пересказ|он\s+(?:пишет|любит|ищет)|она\s+(?:пишет|любит|ищет)|правила\s+треда|шаблон\s+анкеты|заполните\s+анкету|список\s+(?:ресурсов|контактов)|реклам|подписывайтесь|оказываю\s+услуги|предлагаю\s+услуги|платн(?:ые|ая|ый)\s+услуг|продам|продаю|набор\s+в\s+|(?:наш|мой)\s+(?:канал|сервер)|рекомендую\s+(?:канал|аккаунт)|контакт\s+забы[лт])",
    )
});
static SELF_CONTACT: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)(?:(?:мой|моя|мои|my)\s*(?:телеграм|тг|telegram|discord|дискорд|ник|почта|контакт)?\s*[:=—-]?\s*$|(?:пишите\s+мне|связаться\s+со\s+мной|найти\s+меня)\s*[:=—-]?\s*$)",
    )
});
static SIGNATURE_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)^\s*(?:(?:контакт|связь|почта|e-?mail|telegram|тг|телеграм|discord|дискорд)\s*[:=—-]?\s*)?$",
    )
});
static QUOTED_TEXT: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?s)"[^"]*"|«[^»]*»|“[^”]*”|`[^`]*`"#));

pub fn classify(message: Message) -> Result<Profile, Rejection> {
    if message.introductory {
        return Err(Rejection::Introductory);
    }
    let authored = message
        .authored_text
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");
    let authored = QUOTED_TEXT.replace_all(&authored, "\n");
    if FOREIGN_OR_TEMPLATE.is_match(&authored) {
        return Err(Rejection::TemplateOrThirdParty);
    }
    // Контакт не может сам стать доказательством содержательного описания.
    let description = CONTACTS.replace_all(&authored, " ");
    if !DESCRIPTION.is_match(&description) {
        return Err(Rejection::NoDescription);
    }
    let mut contacts = Vec::new();
    for capture in CONTACTS.captures_iter(&authored) {
        let matched = capture.get(0).expect("Совпадение контакта");
        let suffix = &authored[matched.end()..];
        if suffix.starts_with("://")
            || suffix.starts_with(|c: char| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '?' | '#' | '=' | '-')
            })
        {
            continue;
        }
        let prefix = authored[..matched.start()]
            .rsplit(['\n', '.', '!', '?', ';'])
            .next()
            .unwrap_or("");
        let prefix = prefix.trim_end().trim_end_matches('(').trim_end();
        let explicit = SELF_CONTACT.is_match(prefix);
        if !explicit && !SIGNATURE_PREFIX.is_match(prefix) {
            continue;
        }
        let kind = if capture.name("email").is_some() {
            ContactKind::Email
        } else if capture.name("discord").is_some() {
            ContactKind::Discord
        } else {
            ContactKind::Telegram
        };
        if kind == ContactKind::Telegram {
            let username = matched
                .as_str()
                .rsplit(['/', ':', ' ', '=', '—'])
                .next()
                .unwrap_or("")
                .trim_start_matches('@')
                .to_ascii_lowercase();
            if username.ends_with("bot")
                || matches!(
                    username.as_str(),
                    "joinchat"
                        | "share"
                        | "proxy"
                        | "addstickers"
                        | "addemoji"
                        | "socks"
                        | "login"
                        | "boost"
                )
            {
                continue;
            }
        }
        let contact = Contact {
            kind,
            value: matched.as_str().to_owned(),
            ownership: if explicit {
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
        return Err(Rejection::NoOwnedContact);
    }
    Ok(Profile { message, contacts })
}
