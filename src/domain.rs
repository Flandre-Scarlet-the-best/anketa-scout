use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

mod metadata;
pub use metadata::{Age, Gender, Metadata};

#[derive(Clone, Debug)]
pub struct Origin {
    pub source: String,
    pub board: String,
    pub thread: u64,
    pub post: u64,
    pub link: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKind {
    Image,
    Video,
    Other,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub kind: MediaKind,
    pub media_type: Option<u64>,
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
    pub timestamp: Option<i64>,
    pub source_date: Option<String>,
    pub attachments: Vec<Attachment>,
    pub(crate) introductory: bool,
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

    /// Возвращает установленный адаптером признак вступительного поста.
    ///
    /// ```compile_fail
    /// fn forge(message: &mut anketa_scout::domain::Message) {
    ///     message.introductory = false;
    /// }
    /// ```
    pub fn is_introductory(&self) -> bool {
        self.introductory
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ContactKind {
    Telegram,
    Discord,
    Email,
    Vk,
    Max,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ownership {
    ExplicitSelf,
    ProfileSignature,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contact {
    pub assumed_platform: bool,
    pub kind: ContactKind,
    pub value: String,
    pub ownership: Ownership,
    pub restoration: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub message: Message,
    pub contacts: Vec<Contact>,
    pub metadata: Metadata,
    pub description: String,
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
        r"|(?P<url>\b(?:https?://)?(?:t\.me|telegram\.me|vk\.com|vk\.ru)/[^\s<>]+)",
        r"|(?P<postfix>[a-z0-9_.\#]{2,32}[\x20\t]+(?:discord|дискорд)\b)",
        r"|(?P<label>\b(?:telegram|телеграмм?|телега|тгшка|тг|tg|discord|дискордик|дискорд|диск|дс|вк|vk|max|макс)\b[\x20\t]*[:=—-]?[\x20\t]*@?[a-z0-9_][a-z0-9_.\#()/:+~-]*)",
        r"|(?P<handle>@[a-z][a-z0-9_().+~-]*)",
        r"|(?P<unlabelled>(?m:^[\x20\t]*[a-z][a-z0-9_]*))"
    ))
});
static LABEL: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)^(telegram|телеграмм?|телега|тгшка|тг|tg|discord|дискордик|дискорд|диск|дс|вк|vk|max|макс)\s*[:=—-]?\s*",
    )
});

fn contact_kind_from_label(label: &str) -> ContactKind {
    let label = label.to_lowercase();
    if label.starts_with('д') || label.starts_with("discord") {
        ContactKind::Discord
    } else if label.starts_with("max") || label.starts_with("макс") {
        ContactKind::Max
    } else if label.starts_with("вк") || label.starts_with("vk") {
        ContactKind::Vk
    } else {
        ContactKind::Telegram
    }
}

const MAX_CONTACT_LABEL_CONTEXT: usize = 512;

fn preceding_contact_label(
    text: &str,
    position: usize,
    line_breaks: &[usize],
) -> Option<(usize, usize, ContactKind)> {
    let current_break = line_breaks.partition_point(|index| *index < position);
    let previous_break = current_break.checked_sub(1)?;
    let line_end = line_breaks[previous_break];
    let line_start = previous_break
        .checked_sub(1)
        .map_or(0, |index| line_breaks[index] + 1);
    let line = &text[line_start..line_end];
    if line.len() > MAX_CONTACT_LABEL_CONTEXT {
        return None;
    }
    let leading = line.len() - line.trim_start().len();
    let trimmed = line.trim();
    let label = LABEL.captures(trimmed)?;
    let matched = label.get(0)?;
    if matched.end() != trimmed.len() {
        return None;
    }
    let current_start = line_end + 1;
    if position - current_start > MAX_CONTACT_LABEL_CONTEXT {
        return None;
    }
    let prefix = DECORATION.replace_all(&text[current_start..position], "");
    if prefix.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some((
        line_start + leading,
        line_start + leading + matched.end(),
        contact_kind_from_label(&label[1]),
    ))
}

static FOREIGN: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)\b(?:его|её|ее|чужой|контакт\s+друга|пример|например|смотрю|читаю|мой\s+канал|наш\s+сервер)\s*[:—-]?\s*",
    )
});
static SELF_OWNERSHIP: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)мой"));
static DECORATION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?ix)\b(?:мой|моя|мои|вот|пишите|сюда|мне|в\s+(?:лс|личку)|контакт|связь|почта|mailto|email|e-mail|telegram|телеграмм?|тг|tg|дискорд|discord|забыл|забыла|добавить|добавлю|к\s+прошлому\s+посту)\b",
    )
});
static CONJUNCTION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:и|или)\b"));
static NON_SEPARATOR: LazyLock<Regex> = LazyLock::new(|| re(r"^[^\s\p{P}]$"));
static INSTRUCTION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)(?:убери(?:те)?|убрать|удали(?:те)?|удалить|без\s+(?:пробел|нижн)|замени(?:те)?)[^\n]*",
    )
});
static REPLACE_INSTRUCTION: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"(?i)замени(?:те)?\s+['"«]?([a-z0-9_+.-]+)['"»]?\s+на\s+['"«]?([a-z0-9_+.-]+)"#)
});
static REMOVE_INSTRUCTION: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)(?:символ|букву)\s*['"«]?([a-z0-9_+.-])['"»]?(?:\s|$|\))"#));
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
    if let Some(c) = REPLACE_INSTRUCTION.captures(original_instruction) {
        if !value.contains(&c[1]) {
            return None;
        }
        value = value.replace(&c[1], &c[2]);
        applied = true;
    }
    if let Some(c) = REMOVE_INSTRUCTION.captures(original_instruction) {
        if !value.contains(&c[1]) {
            return None;
        }
        value = value.replace(&c[1], "");
        applied = true;
    }
    applied.then_some(value)
}

fn canonical(kind: ContactKind, raw: &str) -> Option<String> {
    let mut raw = raw.trim();
    for prefix in [
        "https://",
        "http://",
        "t.me/",
        "telegram.me/",
        "vk.com/",
        "vk.ru/",
    ] {
        while raw
            .get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
        {
            raw = &raw[prefix.len()..];
        }
    }
    let raw = raw.trim_end_matches([',', ';']);
    let raw = if kind == ContactKind::Discord {
        if raw.contains('(') {
            raw
        } else {
            raw.trim_end_matches(')')
        }
    } else if raw.contains('(') {
        raw.trim_end_matches('.')
    } else {
        raw.trim_end_matches(['.', ')'])
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
        ContactKind::Max => {
            static VALID: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][a-zA-Z0-9_]{1,63}$"));
            VALID.is_match(raw)
        }
        ContactKind::Vk => {
            static VALID: LazyLock<Regex> = LazyLock::new(|| re(r"^[a-zA-Z][a-zA-Z0-9_.]{1,63}$"));
            VALID.is_match(raw)
                && !raw
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("club"))
                && !raw
                    .get(..6)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("public"))
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
        ContactKind::Telegram | ContactKind::Max => format!("@{}", raw.to_ascii_lowercase()),
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum CaptureKind {
    Email,
    Url,
    Postfix,
    Label,
    Handle,
    Unlabelled,
}

struct ContactCapture {
    start: usize,
    end: usize,
    kind: CaptureKind,
}

fn mark_matches(mask: &mut [bool], text: &str, regex: &Regex) {
    for matched in regex.find_iter(text) {
        mask[matched.start()..matched.end()].fill(true);
    }
}

fn feed_remainder_context(
    text: &str,
    start: usize,
    end: usize,
    noise: &[bool],
    conjunction: &[bool],
    non_separator_count: &mut usize,
    alphanumeric_count: &mut usize,
) {
    for (offset, character) in text[start..end].char_indices() {
        let index = start + offset;
        if noise[index] {
            continue;
        }
        if matches!(character, '\n' | '.' | '!' | '?' | ';') {
            *non_separator_count = 0;
            *alphanumeric_count = 0;
            continue;
        }
        if conjunction[index] {
            *alphanumeric_count += usize::from(character.is_alphanumeric());
            continue;
        }
        let mut encoded = [0; 4];
        if NON_SEPARATOR.is_match(character.encode_utf8(&mut encoded)) {
            *non_separator_count += 1;
        }
        if character.is_alphanumeric() {
            *alphanumeric_count += 1;
        }
    }
}

fn count_remainder_context(
    text: &str,
    start: usize,
    end: usize,
    noise: &[bool],
    conjunction: &[bool],
) -> (usize, usize) {
    let mut non_separator_count = 0;
    let mut alphanumeric_count = 0;
    for (offset, character) in text[start..end].char_indices() {
        let index = start + offset;
        if noise[index] || matches!(character, '\n' | '.' | '!' | '?' | ';') {
            continue;
        }
        if conjunction[index] {
            alphanumeric_count += usize::from(character.is_alphanumeric());
            continue;
        }
        let mut encoded = [0; 4];
        if NON_SEPARATOR.is_match(character.encode_utf8(&mut encoded)) {
            non_separator_count += 1;
        }
        alphanumeric_count += usize::from(character.is_alphanumeric());
    }
    (non_separator_count, alphanumeric_count)
}

fn mask_text(text: &str, spans: &mut [(usize, usize)]) -> String {
    spans.sort_unstable();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for &(start, end) in spans.iter() {
        let start = start.max(cursor);
        if start >= end {
            continue;
        }
        output.push_str(&text[cursor..start]);
        output.extend(std::iter::repeat_n(' ', text[start..end].chars().count()));
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

pub fn classify(message: Message) -> Result<Profile, Rejection> {
    if message.introductory {
        return Err(Rejection::Introductory);
    }
    static EXAMPLE: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?i)^\s*(?:цитирую|шаблон\s+анкеты|(?:вот\s+)?пример\s+анкеты)\s*:"));
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
    let captures = CONTACTS
        .captures_iter(&authored)
        .map(|capture| {
            let matched = capture.get(0).unwrap();
            ContactCapture {
                start: matched.start(),
                end: matched.end(),
                kind: if capture.name("postfix").is_some() {
                    CaptureKind::Postfix
                } else if capture.name("unlabelled").is_some() {
                    CaptureKind::Unlabelled
                } else if capture.name("email").is_some() {
                    CaptureKind::Email
                } else if capture.name("url").is_some() {
                    CaptureKind::Url
                } else if capture.name("label").is_some() {
                    CaptureKind::Label
                } else {
                    CaptureKind::Handle
                },
            }
        })
        .collect::<Vec<_>>();
    let mut non_unlabelled_prefix = Vec::with_capacity(captures.len() + 1);
    non_unlabelled_prefix.push(0);
    for capture in &captures {
        non_unlabelled_prefix.push(
            non_unlabelled_prefix.last().copied().unwrap()
                + usize::from(capture.kind != CaptureKind::Unlabelled),
        );
    }
    let instructions = INSTRUCTION
        .find_iter(&authored)
        .map(|matched| (matched.start(), matched.end()))
        .collect::<Vec<_>>();
    let foreign_markers = FOREIGN
        .find_iter(&authored)
        .map(|matched| (matched.start(), matched.end()))
        .collect::<Vec<_>>();
    let self_ownership_markers = SELF_OWNERSHIP
        .find_iter(&authored)
        .map(|matched| (matched.start(), matched.end()))
        .collect::<Vec<_>>();
    let line_breaks = authored
        .match_indices('\n')
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut noise = vec![false; authored.len()];
    mark_matches(&mut noise, &authored, &DECORATION);
    let mut conjunction = vec![false; authored.len()];
    mark_matches(&mut conjunction, &authored, &CONJUNCTION);
    let mut contacts: Vec<Contact> = Vec::new();
    let mut seen_contacts = HashSet::new();
    let mut previous_contact: Option<(usize, ContactKind)> = None;
    let mut previous_assumed = false;
    let mut foreign = false;
    let mut unresolved = false;
    let mut mask_spans = Vec::new();
    let mut contact_spans = Vec::new();
    let mut foreign_cursor = 0;
    let mut foreign_boundary = 0;
    let mut remainder_cursor = 0;
    let mut remainder_non_separator_count = 0;
    let mut remainder_alphanumeric_count = 0;
    let mut instruction_cursor = 0;
    for (capture_index, capture) in captures.iter().enumerate() {
        let m_start = capture.start;
        let m_end = capture.end;
        for (offset, character) in authored[foreign_cursor..m_start].char_indices() {
            if matches!(character, '\n' | '.' | '!' | '?' | ';') {
                foreign_boundary = foreign_cursor + offset + character.len_utf8();
            }
        }
        foreign_cursor = m_start;
        feed_remainder_context(
            &authored,
            remainder_cursor,
            m_start,
            &noise,
            &conjunction,
            &mut remainder_non_separator_count,
            &mut remainder_alphanumeric_count,
        );
        remainder_cursor = m_start;
        let marker_index = foreign_markers.partition_point(|(_, end)| *end < m_start);
        if let Some(&(marker_start, marker_end)) = foreign_markers.get(marker_index)
            && marker_end == m_start
            && marker_start >= foreign_boundary
        {
            foreign = true;
            let (non_separator_count, alphanumeric_count) =
                count_remainder_context(&authored, marker_start, marker_end, &noise, &conjunction);
            remainder_non_separator_count =
                remainder_non_separator_count.saturating_sub(non_separator_count);
            remainder_alphanumeric_count =
                remainder_alphanumeric_count.saturating_sub(alphanumeric_count);
            mask_spans.push((marker_start, m_end));
            remainder_cursor = m_end;
            continue;
        }
        let self_marker_index =
            self_ownership_markers.partition_point(|(_, end)| *end <= foreign_boundary);
        let explicitly_owned = self_ownership_markers
            .get(self_marker_index)
            .is_some_and(|(start, _)| *start < m_start);
        let first_break = line_breaks.partition_point(|index| *index < m_end);
        let next_end = line_breaks
            .get(first_break + 1)
            .copied()
            .unwrap_or(authored.len());
        let nearby = &authored[m_end..next_end];
        while instructions
            .get(instruction_cursor)
            .is_some_and(|(start, _)| *start < m_end)
        {
            instruction_cursor += 1;
        }
        let next_instruction = instructions.get(instruction_cursor).filter(|(start, _)| {
            *start < next_end && {
                let first_candidate = capture_index + 1;
                let candidate_end = captures.partition_point(|candidate| candidate.start < *start);
                non_unlabelled_prefix[candidate_end] == non_unlabelled_prefix[first_candidate]
            }
        });
        let instruction = next_instruction
            .map(|(start, end)| &authored[*start..*end])
            .unwrap_or("");
        let raw_match = &authored[m_start..m_end];
        let mut assumed_platform = false;
        let (mut kind, mut raw) = if capture.kind == CaptureKind::Postfix {
            (
                ContactKind::Discord,
                raw_match.split_whitespace().next().unwrap().to_owned(),
            )
        } else if capture.kind == CaptureKind::Unlabelled {
            if instruction.is_empty() {
                continue;
            }
            (ContactKind::Unknown, raw_match.trim().to_owned())
        } else if capture.kind == CaptureKind::Email {
            (ContactKind::Email, raw_match.to_owned())
        } else if capture.kind == CaptureKind::Url {
            let url = raw_match.trim_end_matches([')', '.', ',']);
            let lower = url.to_ascii_lowercase();
            let scheme_len = if lower.starts_with("https://") {
                "https://".len()
            } else if lower.starts_with("http://") {
                "http://".len()
            } else {
                0
            };
            let (host, path) = url[scheme_len..].split_once('/').unwrap();
            (
                if host.to_ascii_lowercase().starts_with("vk.") {
                    ContactKind::Vk
                } else {
                    ContactKind::Telegram
                },
                path.to_owned(),
            )
        } else if capture.kind == CaptureKind::Label {
            let label = LABEL.captures(raw_match).unwrap();
            let matched = label.get(0).unwrap();
            let name = label[1].to_lowercase();
            let tail = &raw_match[matched.end()..];
            let explicit_separator = matched
                .as_str()
                .chars()
                .any(|character| matches!(character, ':' | '=' | '—' | '-'));
            if (name == "дс"
                && !matched.as_str().contains(':')
                && tail.starts_with(|character: char| character.is_ascii_digit()))
                || (matches!(name.as_str(), "max" | "макс")
                    && !explicit_separator
                    && !tail.starts_with('@'))
            {
                continue;
            }
            (
                contact_kind_from_label(&label[1]),
                raw_match[matched.end()..]
                    .trim_start_matches('@')
                    .to_owned(),
            )
        } else if let Some((start, end, kind)) =
            preceding_contact_label(&authored, m_start, &line_breaks)
        {
            mask_spans.push((start, end));
            (kind, raw_match.trim_start_matches('@').to_owned())
        } else {
            static SEPARATOR: LazyLock<Regex> =
                LazyLock::new(|| re(r"^\s*[,;/]*\s*(?:(?:и|или)\s*)?$"));
            let continued = previous_contact.filter(|(end, kind)| {
                *kind == ContactKind::Telegram
                    && !authored[*end..m_start].contains('\n')
                    && SEPARATOR.is_match(&authored[*end..m_start])
            });
            if continued.is_none()
                && remainder_non_separator_count > 0
                && remainder_alphanumeric_count > 0
            {
                continue;
            }
            if continued.is_some() {
                assumed_platform = previous_assumed;
                (
                    ContactKind::Telegram,
                    raw_match.trim_start_matches('@').to_owned(),
                )
            } else {
                assumed_platform = true;
                (
                    ContactKind::Telegram,
                    raw_match.trim_start_matches('@').to_owned(),
                )
            }
        };
        let suffix = &authored[m_end..];
        if suffix.starts_with(|c: char| {
            c.is_alphanumeric() || matches!(c, '_' | '/' | '?' | '#' | '=' | '-' | ':' | '+' | '~')
        }) {
            mask_spans.push((m_start, m_end));
            remainder_cursor = m_end;
            continue;
        }
        let mut end = m_end;
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
        let canonical_value = canonical(kind, &raw).or_else(|| {
            if assumed_platform {
                kind = ContactKind::Unknown;
                assumed_platform = false;
                canonical(kind, &format!("@{}", raw.trim_start_matches('@')))
            } else {
                None
            }
        });
        let Some(value) = canonical_value else {
            unresolved = true;
            mask_spans.push((m_start, end));
            remainder_cursor = end;
            continue;
        };
        contact_spans.push((m_start, m_end));
        mask_spans.push((m_start, end));
        remainder_cursor = end;
        previous_contact = Some((end, kind));
        previous_assumed = assumed_platform;
        let contact = Contact {
            assumed_platform,
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
            ownership: if explicitly_owned {
                Ownership::ExplicitSelf
            } else {
                Ownership::ProfileSignature
            },
        };
        if seen_contacts.insert((contact.kind, contact.value.clone())) {
            contacts.push(contact);
        } else if let Some(existing) = contacts
            .iter_mut()
            .find(|item| item.kind == contact.kind && item.value == contact.value)
        {
            if contact.ownership == Ownership::ExplicitSelf {
                existing.ownership = Ownership::ExplicitSelf;
            }
            if !contact.assumed_platform {
                existing.assumed_platform = false;
            }
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
    let remainder = mask_text(&authored, &mut mask_spans);
    let remainder = INSTRUCTION.replace_all(&remainder, "");
    let remainder = DECORATION.replace_all(&remainder, "");
    static CONTACT_LIST: LazyLock<Regex> =
        LazyLock::new(|| re(r"^[\s\p{P}]*(?:(?:и|или)[\s\p{P}]*)*$"));
    if CONTACT_LIST.is_match(&remainder) || !remainder.chars().any(char::is_alphanumeric) {
        return Err(Rejection::ContactOnly);
    }
    let metadata = Metadata::extract(&authored, &message.attachments);
    let description = clean_description(&authored, &mask_spans, &contact_spans);
    Ok(Profile {
        message,
        contacts,
        metadata,
        description,
    })
}

fn is_contact_restoration_instruction(text: &str) -> bool {
    let instruction = text.trim();
    let Some(matched) = INSTRUCTION.find(instruction) else {
        return false;
    };
    if matched.start() != 0 || matched.end() != instruction.len() {
        return false;
    }

    let supported_operation = [
        restore(" ", instruction),
        restore("0", instruction),
        restore("()", instruction),
        restore("_", instruction),
        REPLACE_INSTRUCTION
            .captures(instruction)
            .and_then(|capture| restore(&capture[1], instruction)),
        REMOVE_INSTRUCTION
            .captures(instruction)
            .and_then(|capture| restore(&capture[1], instruction)),
    ]
    .into_iter()
    .any(|result| result.is_some());
    if !supported_operation {
        return false;
    }

    let lower = instruction.to_lowercase();
    let words = REMOVE_INSTRUCTION
        .replace_all(
            &REPLACE_INSTRUCTION.replace_all(&lower, " замена "),
            " удаление ",
        )
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < words.len() {
        match words[index].as_str() {
            "и" => {
                if index == 0 || index + 1 == words.len() {
                    return false;
                }
                index += 1;
            }
            "после" => {
                if index + 1 == words.len() {
                    return false;
                }
                index += 2;
            }
            "убери"
            | "уберите"
            | "убрать"
            | "удали"
            | "удалите"
            | "удалить"
            | "без"
            | "пробел"
            | "пробела"
            | "пробелы"
            | "пробелов"
            | "пробелами"
            | "нул"
            | "нули"
            | "нуля"
            | "нулей"
            | "скобка"
            | "скобки"
            | "скобок"
            | "скобку"
            | "скобками"
            | "нижнего"
            | "нижнее"
            | "подчеркивания"
            | "подчёркивания"
            | "подчеркивание"
            | "подчёркивание"
            | "замена"
            | "удаление" => index += 1,
            _ => return false,
        }
    }
    true
}

fn clean_description(
    text: &str,
    spans: &[(usize, usize)],
    contact_spans: &[(usize, usize)],
) -> String {
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        re(r"(?i)(?:\b(?:мой|моя|почта|mailto|email|e-mail|контакт|связь)\s*[:=—-]?\s*)+$")
    });
    static REPLY: LazyLock<Regex> = LazyLock::new(|| re(r">>[0-9]+"));
    static PARENTHETICAL: LazyLock<Regex> = LazyLock::new(|| re(r"\([^()\n]*\)"));
    let mut spans = spans.to_vec();
    for (start, _) in &mut spans {
        if let Some(prefix) = PREFIX.find(&text[..*start]) {
            *start = prefix.start();
        }
    }
    for matched in PARENTHETICAL.find_iter(text) {
        let start = matched.start();
        let end = matched.end();
        let instruction = &text[start + 1..end - 1];
        let follows_contact = contact_spans
            .partition_point(|(_, contact_end)| *contact_end <= start)
            .checked_sub(1)
            .is_some_and(|index| {
                text[contact_spans[index].1..start]
                    .chars()
                    .all(|character| !character.is_alphanumeric())
            });
        if follows_contact && is_contact_restoration_instruction(instruction) {
            let end = if text
                .as_bytes()
                .get(end)
                .is_some_and(|character| matches!(character, b'.' | b'!' | b'?'))
            {
                end + 1
            } else {
                end
            };
            spans.push((start, end));
        }
    }
    let masked = mask_text(text, &mut spans);
    REPLY
        .replace_all(&masked, "")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let without_noise = DECORATION.replace_all(line, "");
            if !without_noise.chars().any(char::is_alphanumeric) {
                None
            } else {
                Some(line.split_whitespace().collect::<Vec<_>>().join(" "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
