use crate::{
    Failure,
    domain::{Attachment, Message, Origin},
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
pub struct ThreadUrl {
    pub board: String,
    pub thread: u64,
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

    pub fn endpoint(&self) -> String {
        format!("https://2ch.org/{}/res/{}.json", self.board, self.thread)
    }
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
    path: String,
    #[serde(default)]
    displayname: Option<String>,
    // Makaba передаёт приближённый целый размер в КиБ, а не Content-Length в байтах.
    #[serde(default, rename = "size")]
    size_kib: Option<u64>,
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
            if !file.path.starts_with(&prefix)
                || file.path.contains(['?', '#', '\\'])
                || file.path.split('/').any(|part| part == "..")
            {
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
                name: file.displayname,
                link: link.into(),
                size_kib: file.size_kib,
            });
        }
        let date = post
            .timestamp
            .and_then(|stamp| chrono::DateTime::from_timestamp(stamp, 0))
            .map(|date| date.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .or(post.date.filter(|date| !date.trim().is_empty()));
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
            attachments,
            introductory: post.num == source.thread,
        });
    }
    if !seen.contains(&source.thread) {
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
        && let Some(href) = element.attr("href")
    {
        let href = href.trim();
        if (href.starts_with("https://t.me/")
            || href.starts_with("http://t.me/")
            || href.starts_with("https://telegram.me/")
            || href.starts_with("http://telegram.me/")
            || href.starts_with("mailto:"))
            && !output[start..].contains(href.trim_start_matches("mailto:"))
        {
            output.push_str(" (");
            output.push_str(href.trim_start_matches("mailto:"));
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
