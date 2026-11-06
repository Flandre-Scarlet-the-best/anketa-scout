use crate::{
    Failure,
    domain::{Attachment, MediaKind, Message, Origin},
};
use regex::Regex;
use scraper::{ElementRef, Html, Node};
use serde::Deserialize;
use std::{collections::HashSet, sync::LazyLock};
use url::Url;

pub const MAX_BODY: usize = 20 * 1024 * 1024;
pub const MAX_POSTS: usize = 20_000;
pub const MAX_COMMENT: usize = 128 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Проверенный адрес треда Двача.
///
/// ```compile_fail
/// use anketa_scout::source::ThreadUrl;
/// let forged = ThreadUrl { board: "../secret".into(), thread: 1 };
/// ```
pub struct ThreadUrl {
    board: String,
    thread: u64,
}

impl ThreadUrl {
    pub fn parse(input: &str) -> Result<Self, Failure> {
        static PATH: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^/([a-z0-9_]+)/res/([1-9][0-9]*)\.(?:html|json)$").expect("Шаблон пути")
        });
        let bad = Failure::Config(
            "Неподдерживаемый URL: нужен HTTPS-тред 2ch.org без параметров, учётных данных и фрагмента.",
        );
        let url = Url::parse(input).map_err(|_| bad)?;
        if url.scheme() != "https"
            || url.host_str() != Some("2ch.org")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(bad);
        }
        let captures = PATH.captures(url.path()).ok_or(bad)?;
        Ok(Self {
            board: captures[1].to_owned(),
            thread: captures[2].parse().map_err(|_| bad)?,
        })
    }

    pub fn page(&self) -> String {
        format!("https://2ch.org/{}/res/{}.html", self.board, self.thread)
    }

    pub fn board(&self) -> &str {
        &self.board
    }

    pub fn thread(&self) -> u64 {
        self.thread
    }

    pub fn endpoint(&self) -> String {
        format!("https://2ch.org/{}/res/{}.json", self.board, self.thread)
    }

    pub fn after_endpoint(&self, cursor: u64) -> String {
        format!(
            "https://2ch.org/api/mobile/v2/after/{}/{}/{cursor}",
            self.board, self.thread
        )
    }
}

fn decode_percent_encoding(bytes: &[u8], strict: bool) -> Option<Vec<u8>> {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let escape = bytes
                .get(index + 1)
                .copied()
                .and_then(hex)
                .zip(bytes.get(index + 2).copied().and_then(hex));
            if let Some((high, low)) = escape {
                decoded.push((high << 4) | low);
                index += 3;
            } else if strict {
                return None;
            } else {
                decoded.push(b'%');
                index += 1;
            }
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}

fn safe_attachment_path(path: &str, prefix: &str) -> bool {
    let Some(relative) = path.strip_prefix(prefix) else {
        return false;
    };
    if path.contains(['?', '#', '\\']) {
        return false;
    }
    relative.split('/').all(|segment| {
        let mut decoded = segment.as_bytes().to_vec();
        for depth in 0..4 {
            if matches!(decoded.as_slice(), b"." | b"..")
                || decoded.contains(&b'/')
                || decoded.contains(&b'\\')
            {
                return false;
            }
            let Some(next) = decode_percent_encoding(&decoded, depth == 0) else {
                return false;
            };
            if next == decoded {
                return true;
            }
            decoded = next;
        }
        false
    })
}

fn contact_href(input: &str) -> Option<String> {
    let mut url = Url::parse(input.trim()).ok()?;
    if url.scheme() == "mailto" {
        return Some(url.to_string());
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.host_str(),
            Some("t.me" | "telegram.me" | "vk.com" | "vk.ru")
        )
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

#[derive(Deserialize)]
struct Envelope {
    board: Board,
    current_thread: u64,
    posts_count: usize,
    threads: Vec<Thread>,
}

#[derive(Deserialize)]
struct Board {
    id: String,
}
#[derive(Deserialize)]
struct Thread {
    posts: Vec<Post>,
}
#[derive(Deserialize)]
struct Post {
    num: u64,
    parent: u64,
    comment: String,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default)]
    date: Option<String>,
    #[serde(deserialize_with = "nullable_files")]
    files: Vec<File>,
}

fn nullable_files<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<File>, D::Error> {
    Ok(Option::<Vec<File>>::deserialize(deserializer)?.unwrap_or_default())
}
#[derive(Deserialize)]
struct File {
    #[serde(default, rename = "type", deserialize_with = "optional_media_type")]
    media_type: Option<u64>,
    path: String,
    #[serde(default)]
    displayname: Option<String>,
    // Makaba передаёт приближённый целый размер в КиБ, а не Content-Length в байтах.
    #[serde(default, rename = "size")]
    size_kib: Option<u64>,
}

fn optional_media_type<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    // Новое необязательное поле не превращает ранее читаемый пост в ошибку треда.
    Ok(Option::<serde_json::Value>::deserialize(deserializer)?.and_then(|value| value.as_u64()))
}

fn media_kind(code: Option<u64>, path: &str) -> MediaKind {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match code {
        Some(1 | 2 | 9) => MediaKind::Image,
        Some(6 | 10) => MediaKind::Video,
        _ if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") => MediaKind::Image,
        _ if matches!(ext.as_str(), "webm" | "mp4" | "mov") => MediaKind::Video,
        _ => MediaKind::Other,
    }
}

pub fn normalize(source: &ThreadUrl, bytes: &[u8]) -> Result<Vec<Message>, Failure> {
    if bytes.len() > MAX_BODY {
        return Err(Failure::Limit);
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| Failure::Format)?;
    if envelope.board.id != source.board
        || envelope.current_thread != source.thread
        || envelope.threads.len() != 1
    {
        return Err(Failure::Format);
    }
    let posts = envelope
        .threads
        .into_iter()
        .next()
        .ok_or(Failure::Format)?
        .posts;
    if posts.len() > MAX_POSTS {
        return Err(Failure::Limit);
    }
    if posts.len() != envelope.posts_count {
        return Err(Failure::Partial);
    }
    normalize_posts(source, posts, true)
}

#[derive(Deserialize)]
struct Updates {
    result: u8,
    posts: Vec<Post>,
}

pub fn normalize_after(
    source: &ThreadUrl,
    bytes: &[u8],
    cursor: u64,
) -> Result<Vec<Message>, Failure> {
    if bytes.len() > MAX_BODY {
        return Err(Failure::Limit);
    }
    let updates: Updates = serde_json::from_slice(bytes).map_err(|_| Failure::Format)?;
    if updates.result != 1 {
        return Err(Failure::Unavailable);
    }
    let messages = normalize_posts(source, updates.posts, false)?;
    if messages.iter().any(|message| message.origin.post < cursor) {
        return Err(Failure::Format);
    }
    Ok(messages
        .into_iter()
        .filter(|message| message.origin.post > cursor)
        .collect())
}

fn normalize_posts(
    source: &ThreadUrl,
    posts: Vec<Post>,
    require_op: bool,
) -> Result<Vec<Message>, Failure> {
    if posts.len() > MAX_POSTS {
        return Err(Failure::Limit);
    }
    let mut seen = HashSet::new();
    let mut messages = Vec::with_capacity(posts.len());
    for post in posts {
        if post.num == 0
            || !seen.insert(post.num)
            || (post.num != source.thread && post.parent != source.thread)
            || (post.num == source.thread && post.parent != 0)
        {
            return Err(Failure::Format);
        }
        if post.comment.len() > MAX_COMMENT || post.files.len() > 100 {
            return Err(Failure::Limit);
        }
        let (text, authored_text) = normalize_html(&post.comment)?;
        let mut attachments = Vec::new();
        for file in post.files {
            // Вложения только показываем. Принимаем локальный путь доски, без иных хостов.
            let prefix = format!("/{}/src/", source.board);
            if !safe_attachment_path(&file.path, &prefix) {
                return Err(Failure::Format);
            }
            let link = Url::parse("https://2ch.org")
                .expect("Постоянный URL")
                .join(&file.path)
                .map_err(|_| Failure::Format)?;
            if link.host_str() != Some("2ch.org") || !link.path().starts_with(&prefix) {
                return Err(Failure::Format);
            }
            attachments.push(Attachment {
                kind: media_kind(file.media_type, &file.path),
                media_type: file.media_type,
                name: file.displayname,
                link: link.into(),
                size_kib: file.size_kib,
            });
        }
        let date = post
            .timestamp
            .and_then(|stamp| chrono::DateTime::from_timestamp(stamp, 0))
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S %:z")
                    .to_string()
            });
        messages.push(Message {
            origin: Origin {
                source: "2ch.org".to_owned(),
                board: source.board.clone(),
                thread: source.thread,
                post: post.num,
                link: format!("{}#{}", source.page(), post.num),
            },
            original: post.comment,
            text,
            authored_text,
            date,
            timestamp: post.timestamp,
            source_date: post.date,
            attachments,
            introductory: post.num == source.thread,
        });
    }
    if require_op && !seen.contains(&source.thread) {
        return Err(Failure::Partial);
    }
    Ok(messages)
}

fn walk(
    element: ElementRef<'_>,
    omit_quotes: bool,
    depth: usize,
    output: &mut String,
) -> Result<(), Failure> {
    if depth > 128 {
        return Err(Failure::Limit);
    }
    let name = element.value().name();
    if matches!(name, "script" | "style" | "iframe" | "template") {
        return Ok(());
    }
    let quote = matches!(name, "blockquote" | "q")
        || element
            .value()
            .classes()
            .any(|class| matches!(class, "unkfunc" | "quote"));
    if omit_quotes && (quote || matches!(name, "code" | "pre")) {
        output.push('\n');
        return Ok(());
    }
    let block = matches!(name, "br" | "p" | "div" | "li" | "blockquote");
    if block {
        output.push('\n');
    }
    let start = output.len();
    for child in element.children() {
        if let Node::Text(text) = child.value() {
            output.push_str(text);
        } else if let Some(element) = ElementRef::wrap(child) {
            walk(element, omit_quotes, depth + 1, output)?;
        }
    }
    if name == "a"
        && let Some(href) = element.attr("href").and_then(contact_href)
    {
        let address = href.strip_prefix("mailto:").unwrap_or(&href);
        if omit_quotes {
            output.truncate(start);
            output.push_str(address);
        } else if !output[start..].contains(address) {
            output.push_str(" (");
            output.push_str(address);
            output.push(')');
        }
    }
    if block {
        output.push('\n');
    }
    Ok(())
}

pub fn normalize_html(input: &str) -> Result<(String, String), Failure> {
    if input.len() > MAX_COMMENT {
        return Err(Failure::Limit);
    }
    let html = Html::parse_fragment(input);
    let mut text = String::new();
    let mut authored = String::new();
    walk(html.root_element(), false, 0, &mut text)?;
    walk(html.root_element(), true, 0, &mut authored)?;
    fn tidy(text: &str) -> String {
        text.lines()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
    Ok((tidy(&text), tidy(&authored)))
}
