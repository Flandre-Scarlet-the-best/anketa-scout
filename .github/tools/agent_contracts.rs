//! Детерминированные проверки агентских контрактов и структуры PR без внешних зависимостей.
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;

const REQUIRED_SKILLS: &[&str] = &[
    "scout-code-review",
    "scout-coderabbit-review",
    "scout-git-workflow",
];
const REQUIRED_SECTIONS: &[&str] = &[
    "Цель", "Область изменений", "Реализация", "Проверки",
    "Производительность", "CI", "Готовность", "Ограничения",
];

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Ключ YAML-отображения, значение в его строке и все последующие вложенные строки.
struct YamlEntry {
    key: String,
    value: String,
    continuation: Vec<String>,
}

fn is_yaml_noise(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn line_indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Разбирает одно блочное YAML-отображение с отступом `indent`. Всё, что не укладывается
/// в простое подмножество (повторы ключей, кавычки в ключах, строки вне отображения), — ошибка:
/// проверка контрактов должна отказывать, а не угадывать смысл.
fn yaml_mapping<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    indent: usize,
) -> Result<Vec<YamlEntry>, String> {
    let mut entries: Vec<YamlEntry> = Vec::new();
    for line in lines {
        if is_yaml_noise(line) || line_indent(line) > indent || line.starts_with('\t') {
            match entries.last_mut() {
                Some(entry) => entry.continuation.push(line.to_owned()),
                None if is_yaml_noise(line) => {}
                None => return Err(format!("вложенная строка до первого ключа: {line}")),
            }
            continue;
        }
        if line_indent(line) < indent {
            return Err(format!("нарушен отступ YAML: {line}"));
        }
        let content = &line[indent..];
        let Some((key, value)) = content.split_once(':') else {
            return Err(format!("неподдерживаемая строка YAML: {line}"));
        };
        if key.is_empty()
            || !key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            || !(value.is_empty() || value.starts_with([' ', '\t']))
        {
            return Err(format!("неподдерживаемый ключ YAML: {line}"));
        }
        if entries.iter().any(|entry| entry.key == key) {
            return Err(format!("дублируется {key}"));
        }
        entries.push(YamlEntry { key: key.to_owned(), value: value.trim().to_owned(), continuation: Vec::new() });
    }
    Ok(entries)
}

fn frontmatter_entries(text: &str) -> Result<Vec<YamlEntry>, String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("отсутствует начальный YAML frontmatter".into());
    }
    let body: Vec<&str> = lines.by_ref().take_while(|line| line.trim_end() != "---").collect();
    if text.lines().skip(1 + body.len()).next().is_none() {
        return Err("не закрыт YAML frontmatter".into());
    }
    yaml_mapping(body, 0)
}

fn without_plain_comment(value: &str) -> &str {
    if value.starts_with('#') {
        return "";
    }
    value.find(" #").map_or(value, |pos| &value[..pos]).trim()
}

/// Простой скаляр без кавычек, блоков и вложенных строк; иначе `None`.
fn plain_scalar(entry: &YamlEntry) -> Option<&str> {
    let value = without_plain_comment(&entry.value);
    let simple = !value.is_empty()
        && !value.starts_with(['"', '\'', '|', '>', '{', '[', '&', '*', '!'])
        && entry.continuation.iter().all(|line| is_yaml_noise(line));
    simple.then_some(value)
}

/// Текстовое значение скаляра: снимает кавычки и собирает блочные и многострочные значения.
fn scalar_text(entry: &YamlEntry) -> String {
    let value = entry.value.as_str();
    let continued = || {
        entry.continuation.iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    if let Some(quoted) = value.strip_prefix('"') {
        let mut result = String::new();
        let mut chars = quoted.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => return result,
                '\\' => result.extend(chars.next()),
                _ => result.push(ch),
            }
        }
        return result;
    }
    if let Some(quoted) = value.strip_prefix('\'') {
        let mut result = String::new();
        let mut chars = quoted.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '\'' {
                result.push(ch);
            } else if chars.next_if_eq(&'\'').is_some() {
                result.push('\'');
            } else {
                break;
            }
        }
        return result;
    }
    if value.starts_with(['|', '>']) {
        return continued();
    }
    let plain = without_plain_comment(value);
    if plain.is_empty() {
        continued()
    } else {
        format!("{plain} {}", continued()).trim().to_owned()
    }
}

fn frontmatter_name(text: &str) -> Result<String, String> {
    let entries = frontmatter_entries(text)?;
    let find = |key: &str| entries.iter().find(|entry| entry.key == key);
    let name = find("name").and_then(plain_scalar).ok_or("нет name")?;
    if !name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
        return Err("недопустимое name".into());
    }
    let description = find("description").map(scalar_text).unwrap_or_default();
    if description.trim().is_empty() || description.trim() == "TODO" {
        return Err("пустое description".into());
    }
    Ok(name.to_owned())
}

/// Верхнеуровневый ключ frontmatter существует и равен простому скаляру `expected`.
fn frontmatter_flag(text: &str, key: &str, expected: &str) -> bool {
    frontmatter_entries(text).is_ok_and(|entries| {
        entries.iter()
            .find(|entry| entry.key == key)
            .and_then(plain_scalar)
            .is_some_and(|value| value == expected)
    })
}

fn policy_disables_implicit_invocation(text: &str) -> bool {
    let Ok(root) = yaml_mapping(text.lines(), 0) else {
        return false;
    };
    let Some(policy) = root.iter().find(|entry| entry.key == "policy") else {
        return false;
    };
    if !without_plain_comment(&policy.value).is_empty() {
        return false;
    }
    let Some(child_indent) = policy.continuation.iter()
        .find(|line| !is_yaml_noise(line))
        .map(|line| line_indent(line))
        .filter(|indent| *indent > 0)
    else {
        return false;
    };
    let Ok(children) = yaml_mapping(policy.continuation.iter().map(String::as_str), child_indent) else {
        return false;
    };
    children.iter()
        .find(|entry| entry.key == "allow_implicit_invocation")
        .and_then(plain_scalar)
        .is_some_and(|value| value == "false")
}

/// Текст Markdown без блоков кода, встроенного кода и HTML-комментариев: примеры не являются ссылками.
fn markdown_prose(text: &str) -> String {
    let mut without_blocks = String::new();
    let mut fence: Option<(char, usize)> = None;
    let mut inside_comment = false;
    for line in text.lines() {
        if let Some((marker, minimum)) = fence {
            if fence_prefix(line).is_some_and(|(closing, count)| {
                closing == marker && count >= minimum && line.trim().chars().all(|ch| ch == marker)
            }) {
                fence = None;
            }
            continue;
        }
        let visible = strip_comments(line, &mut inside_comment);
        if let Some(opening) = fence_prefix(&visible) {
            fence = Some(opening);
            continue;
        }
        without_blocks.push_str(&visible);
        without_blocks.push('\n');
    }

    let mut result = String::new();
    let mut rest = without_blocks.as_str();
    while let Some(start) = rest.find('`') {
        result.push_str(&rest[..start]);
        let run = rest[start..].chars().take_while(|ch| *ch == '`').count();
        let after = &rest[start + run..];
        let closing = (0..after.len())
            .filter(|pos| after.is_char_boundary(*pos) && after[*pos..].starts_with(&"`".repeat(run)))
            .find(|pos| {
                !after[..*pos].ends_with('`') && !after[*pos + run..].starts_with('`')
            });
        match closing {
            Some(end) => {
                result.push(' ');
                rest = &after[end + run..];
            }
            None => {
                result.push_str(&rest[start..start + run]);
                rest = after;
            }
        }
    }
    result.push_str(rest);
    result
}

fn link_destination(raw_target: &str) -> &str {
    let raw_target = raw_target.trim();
    if let Some(angled) = raw_target.strip_prefix('<') {
        angled.split_once('>').map(|(target, _)| target).unwrap_or("")
    } else {
        raw_target.split_whitespace().next().unwrap_or("")
    }
}

fn links_in(text: &str) -> Vec<String> {
    let prose = markdown_prose(text);
    let mut result = Vec::new();
    for line in prose.lines() {
        let indent = line.chars().take_while(|ch| *ch == ' ').count();
        if indent > 3 || !line[indent..].starts_with('[') {
            continue;
        }
        if let Some((label, destination)) = line[indent + 1..].split_once("]:") {
            let target = link_destination(destination);
            if !label.trim().is_empty() && !label.contains(']') && !target.is_empty() {
                result.push(target.to_string());
            }
        }
    }
    let mut rest = prose.as_str();
    while let Some(pos) = rest.find("](") {
        rest = &rest[pos + 2..];
        if let Some(end) = rest.find(')') {
            let target = link_destination(&rest[..end]);
            if !target.is_empty() {
                result.push(target.to_string());
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    result
}

fn checked_path(
    root: &Path,
    relative: &Path,
    directory: bool,
    errors: &mut Vec<String>,
) -> Option<PathBuf> {
    let components: Vec<_> = relative.components().collect();
    if components.is_empty()
        || components
            .iter()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        errors.push(format!("недопустимый путь: {}", relative.display()));
        return None;
    }

    let mut path = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            errors.push(format!("недопустимый путь: {}", relative.display()));
            return None;
        };
        path.push(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                errors.push(format!("путь недоступен: {}: {error}", path.display()));
                return None;
            }
        };
        if metadata.file_type().is_symlink() {
            errors.push(format!("символическая ссылка недопустима: {}", path.display()));
            return None;
        }

        let is_last = index + 1 == components.len();
        if (!is_last || directory) && !metadata.is_dir() {
            errors.push(format!("ожидался каталог: {}", path.display()));
            return None;
        }
        if is_last && !directory && !metadata.is_file() {
            errors.push(format!("ожидался обычный файл: {}", path.display()));
            return None;
        }
    }
    Some(path)
}

fn read_dir_safely(
    root: &Path,
    relative: &Path,
    errors: &mut Vec<String>,
) -> Option<fs::ReadDir> {
    let path = checked_path(root, relative, true, errors)?;
    match fs::read_dir(&path) {
        Ok(entries) => Some(entries),
        Err(error) => {
            errors.push(format!("каталог недоступен: {}: {error}", path.display()));
            None
        }
    }
}

fn read_file_safely(
    root: &Path,
    relative: &Path,
    errors: &mut Vec<String>,
) -> Option<String> {
    let path = checked_path(root, relative, false, errors)?;
    match fs::read_to_string(&path) {
        Ok(body) => Some(body),
        Err(error) => {
            errors.push(format!("не удалось прочитать {}: {error}", path.display()));
            None
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum MarkdownTarget<'a> {
    Ignore,
    Absolute(&'a str),
    Local(&'a str),
}

fn classify_markdown_target(raw_target: &str) -> MarkdownTarget<'_> {
    let target = raw_target.split('#').next().unwrap_or("").trim();
    if target.is_empty()
        || target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with("mailto:")
        || target.starts_with("data:")
    {
        return MarkdownTarget::Ignore;
    }

    let bytes = target.as_bytes();
    let windows_drive_path = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    if Path::new(target).is_absolute()
        || target.starts_with('/')
        || target.starts_with('\\')
        || windows_drive_path
    {
        MarkdownTarget::Absolute(target)
    } else {
        MarkdownTarget::Local(target)
    }
}

fn check_markdown_target(
    root: &Path,
    source: &Path,
    raw_target: &str,
    errors: &mut Vec<String>,
) {
    let source_path = root.join(source);
    match classify_markdown_target(raw_target) {
        MarkdownTarget::Ignore => {}
        MarkdownTarget::Absolute(target) => {
            errors.push(format!("абсолютная ссылка: {} -> {target}", source_path.display()));
        }
        MarkdownTarget::Local(target) => {
            let base = source.parent().unwrap_or(Path::new(""));
            match local_target_within(root, &base.join(target)) {
                LocalTarget::Exists => {}
                LocalTarget::Missing => {
                    errors.push(format!("битая ссылка: {} -> {target}", source_path.display()));
                }
                LocalTarget::Outside => {
                    errors.push(format!("ссылка за пределы репозитория: {} -> {target}", source_path.display()));
                }
                LocalTarget::Symlink => {
                    errors.push(format!("ссылка через символическую ссылку: {} -> {target}", source_path.display()));
                }
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum LocalTarget {
    Exists,
    Missing,
    Outside,
    Symlink,
}

/// Лексически нормализует путь относительно корня копии и проверяет существование
/// без выхода за корень и без следования символическим ссылкам.
fn local_target_within(root: &Path, relative: &Path) -> LocalTarget {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return LocalTarget::Outside;
                }
            }
            Component::Normal(name) => parts.push(name),
            Component::RootDir | Component::Prefix(_) => return LocalTarget::Outside,
        }
    }
    let mut path = root.to_path_buf();
    for name in parts {
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => return LocalTarget::Symlink,
            Ok(_) => {}
            Err(_) => return LocalTarget::Missing,
        }
    }
    LocalTarget::Exists
}

fn scan_markdown(root: &Path, folder: &Path, errors: &mut Vec<String>) {
    let Some(entries) = read_dir_safely(root, folder, errors) else {
        return;
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("ошибка чтения каталога {}: {error}", folder.display()));
                continue;
            }
        };
        let path = folder.join(entry.file_name());
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!("не удалось определить тип {}: {error}", path.display()));
                continue;
            }
        };
        if file_type.is_symlink() {
            errors.push(format!("символическая ссылка недопустима: {}", path.display()));
        } else if file_type.is_dir() {
            scan_markdown(root, &path, errors);
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "md") {
            let Some(body) = read_file_safely(root, &path, errors) else {
                continue;
            };
            for target in links_in(&body) {
                check_markdown_target(root, &path, &target, errors);
            }
        }
    }
}

fn check_skills(root: &Path) -> Result<(), String> {
    let agents = Path::new(".agents/skills");
    let claude = Path::new(".claude/skills");
    let mut found = BTreeSet::new();
    let mut errors = Vec::new();
    let Some(entries) = read_dir_safely(root, agents, &mut errors) else {
        return Err(errors.join("\n"));
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("ошибка чтения каталога {}: {error}", agents.display()));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let dir = agents.join(&name);
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!("не удалось определить тип {}: {error}", dir.display()));
                continue;
            }
        };
        if file_type.is_symlink() {
            errors.push(format!("символическая ссылка недопустима: {}", dir.display()));
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }
        let canonical = dir.join("SKILL.md");
        let wrapper = claude.join(&name).join("SKILL.md");
        if let Some(text) = read_file_safely(root, &canonical, &mut errors) {
            match frontmatter_name(&text) {
                Ok(declared) if declared == name => {}
                Ok(declared) => errors.push(format!("{name}: name отличается ({declared})")),
                Err(error) => errors.push(format!("{name}: {error}")),
            }
        }
        if let Some(body) = read_file_safely(root, &wrapper, &mut errors) {
            match frontmatter_name(&body) {
                Ok(declared) if declared == name => {}
                Ok(declared) => errors.push(format!("{name}: неверное name входа Claude: {declared}")),
                Err(error) => errors.push(format!("{name}: вход Claude: {error}")),
            }
            let reference = format!("../../../.agents/skills/{name}/SKILL.md");
            if !body.contains(&reference) {
                errors.push(format!("{name}: вход Claude не указывает канонический skill"));
            }
            if name == "scout-coderabbit-review"
                && !frontmatter_flag(&body, "disable-model-invocation", "true")
            {
                errors.push("CodeRabbit: у Claude разрешён неявный вызов".into());
            }
        }
        found.insert(name);
    }
    for name in REQUIRED_SKILLS {
        if !found.contains(*name) {
            errors.push(format!("отсутствует обязательный skill {name}"));
        }
    }
    if let Some(entries) = read_dir_safely(root, claude, &mut errors) {
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("ошибка чтения каталога {}: {error}", claude.display()));
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().to_string();
            let path = claude.join(&name);
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    errors.push(format!("не удалось определить тип {}: {error}", path.display()));
                    continue;
                }
            };
            if file_type.is_symlink() {
                errors.push(format!("символическая ссылка недопустима: {}", path.display()));
            } else if file_type.is_dir() && !found.contains(&name) {
                errors.push(format!("вход Claude без канонического skill: {name}"));
            }
        }
    }
    let policy_path = agents.join("scout-coderabbit-review/agents/openai.yaml");
    if let Some(policy) = read_file_safely(root, &policy_path, &mut errors) {
        if !policy_disables_implicit_invocation(&policy) {
            errors.push("CodeRabbit: у Codex разрешён неявный вызов".into());
        }
    }
    scan_markdown(root, Path::new(".agents"), &mut errors);
    scan_markdown(root, Path::new(".claude"), &mut errors);
    scan_markdown(root, Path::new(".codex"), &mut errors);
    let agents_md = Path::new("AGENTS.md");
    if let Some(body) = read_file_safely(root, agents_md, &mut errors) {
        for target in links_in(&body) {
            check_markdown_target(root, agents_md, &target, &mut errors);
        }
    }
    if errors.is_empty() {
        println!("Агентские контракты: OK ({} skills)", found.len());
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn strip_comments(mut line: &str, inside: &mut bool) -> String {
    let mut result = String::new();
    while !line.is_empty() {
        if *inside {
            if let Some(end) = line.find("-->") {
                line = &line[end + 3..];
                *inside = false;
            } else {
                break;
            }
        } else if let Some(start) = line.find("<!--") {
            result.push_str(&line[..start]);
            line = &line[start + 4..];
            *inside = true;
        } else {
            result.push_str(line);
            break;
        }
    }
    result
}

fn fence_prefix(line: &str) -> Option<(char, usize)> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return None;
    }
    let remaining = &line[indent..];
    let marker = remaining.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = remaining.chars().take_while(|ch| *ch == marker).count();
    (count >= 3).then_some((marker, count))
}

/// Имена тегов HTML-блоков CommonMark типа 6: могут прерывать абзац и закрываются пустой строкой.
const HTML_BLOCK_TAGS: &[&str] = &[
    "address", "article", "aside", "base", "basefont", "blockquote", "body", "caption",
    "center", "col", "colgroup", "dd", "details", "dialog", "dir", "div", "dl", "dt",
    "fieldset", "figcaption", "figure", "footer", "form", "frame", "frameset", "h1", "h2",
    "h3", "h4", "h5", "h6", "head", "header", "hr", "html", "iframe", "legend", "li", "link",
    "main", "menu", "menuitem", "nav", "noframes", "ol", "optgroup", "option", "p", "param",
    "search", "section", "summary", "table", "tbody", "td", "tfoot", "th", "thead", "title",
    "tr", "track", "ul",
];
/// Теги HTML-блоков CommonMark типа 1: закрываются строкой с соответствующим закрывающим тегом.
const HTML_RAW_TAGS: &[&str] = &["pre", "script", "style", "textarea"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HtmlBlock {
    UntilBlank,
    UntilClose(&'static str),
}

/// Начало сырого HTML-блока: внутри него строки `##` отображаются как текст, а не заголовки.
fn html_block_start(line: &str, interrupts_paragraph: bool) -> Option<HtmlBlock> {
    let indent = line.chars().take_while(|ch| *ch == ' ').count();
    if indent > 3 {
        return None;
    }
    let tag = line[indent..].strip_prefix('<')?;
    let closing = tag.starts_with('/');
    let tag = tag.strip_prefix('/').unwrap_or(tag);
    let name_len = tag.chars().take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-').count();
    if name_len == 0 || !tag.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return None;
    }
    let name = tag[..name_len].to_ascii_lowercase();
    let after = &tag[name_len..];
    if !closing {
        if let Some(raw) = HTML_RAW_TAGS.iter().find(|raw| **raw == name) {
            if after.is_empty() || after.starts_with([' ', '\t', '>']) {
                return Some(HtmlBlock::UntilClose(raw));
            }
        }
    }
    let type6_boundary = after.is_empty() || after.starts_with([' ', '\t', '>']) || after.starts_with("/>");
    if HTML_BLOCK_TAGS.contains(&name.as_str()) && type6_boundary {
        return Some(HtmlBlock::UntilBlank);
    }
    // Тип 7: полный открывающий или закрывающий тег один в строке, не прерывающий абзац.
    let single_tag = line.trim_end().find('>').is_some_and(|end| end + 1 == line.trim_end().len());
    (!interrupts_paragraph && single_tag).then_some(HtmlBlock::UntilBlank)
}

fn check_pr_body(body: &str) -> Result<(), String> {
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.chars().count() > 10_000 {
        return Err("PR body превышает 10 000 символов".into());
    }
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut inside_comment = false;
    let mut html_block: Option<HtmlBlock> = None;
    let mut paragraph_open = false;
    for line in normalized.lines() {
        if let Some((marker, minimum)) = fence {
            let closes_fence = if let Some((closing_marker, count)) = fence_prefix(line) {
                closing_marker == marker
                    && count >= minimum
                    && line.trim().chars().all(|ch| ch == marker)
            } else {
                false
            };
            if closes_fence {
                fence = None;
            } else if let Some((_, text)) = parts.last_mut() {
                text.push_str(line);
                text.push('\n');
            }
            continue;
        }
        let visible = strip_comments(line, &mut inside_comment);
        if let Some(kind) = html_block {
            match kind {
                HtmlBlock::UntilBlank if line.trim().is_empty() => html_block = None,
                HtmlBlock::UntilClose(tag) if line.to_ascii_lowercase().contains(&format!("</{tag}>")) => {
                    html_block = None;
                }
                _ => {}
            }
            if let Some((_, text)) = parts.last_mut() {
                text.push_str(&visible);
                text.push('\n');
            }
            paragraph_open = false;
            continue;
        }
        if line.trim().is_empty() {
            paragraph_open = false;
        }
        if let Some(opening) = fence_prefix(&visible) {
            fence = Some(opening);
            paragraph_open = false;
            continue;
        }
        if let Some(kind) = html_block_start(&visible, paragraph_open) {
            let closes_on_same_line = matches!(kind, HtmlBlock::UntilClose(tag)
                if visible.to_ascii_lowercase().contains(&format!("</{tag}>")));
            html_block = (!closes_on_same_line).then_some(kind);
            paragraph_open = false;
            if let Some((_, text)) = parts.last_mut() {
                text.push_str(&visible);
                text.push('\n');
            }
            continue;
        }
        let indent = visible.chars().take_while(|ch| *ch == ' ').count();
        if indent <= 3 {
            if let Some(title) = visible[indent..].trim_end().strip_prefix("## ") {
                parts.push((title.trim_end_matches('#').trim().to_owned(), String::new()));
                paragraph_open = false;
                continue;
            }
        }
        if !visible.trim().is_empty() {
            paragraph_open = true;
        }
        if let Some((_, text)) = parts.last_mut() {
            text.push_str(&visible);
            text.push('\n');
        }
    }
    if fence.is_some() || inside_comment {
        return Err("PR body содержит незакрытый блок кода или HTML-комментарий".into());
    }
    let mut expected_index = 0;
    for (title, content) in &parts {
        if expected_index == REQUIRED_SECTIONS.len() {
            continue;
        }
        let Some(position) = REQUIRED_SECTIONS.iter().position(|section| *section == title.as_str()) else {
            continue;
        };
        if position != expected_index {
            let expected = REQUIRED_SECTIONS[expected_index];
            return Err(format!("раздел {}: ожидается «{}», найден «{}»", expected_index + 1, expected, title));
        }
        let expected = REQUIRED_SECTIONS[expected_index];
        let meaningful = content.chars().filter(|c| c.is_alphanumeric()).count();
        if meaningful < 12 || content.trim().eq_ignore_ascii_case("TODO") {
            return Err(format!("раздел «{expected}» не содержит содержательного описания"));
        }
        expected_index += 1;
    }
    if expected_index < REQUIRED_SECTIONS.len() {
        return Err(format!("не хватает разделов PR: {} из {}", expected_index, REQUIRED_SECTIONS.len()));
    }
    let mut titles = BTreeSet::new();
    for (title, _) in &parts {
        if !titles.insert(title) {
            return Err(format!("повторяется раздел «{title}»"));
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let result = match args.as_slice() {
        [_, cmd, root] if cmd == "skills" => check_skills(&PathBuf::from(root)),
        [_, cmd, file] if cmd == "pr-body" => read(Path::new(file)).and_then(|body| check_pr_body(&body)),
        _ => Err("использование: agent_contracts skills <корень> | pr-body <файл>".into()),
    };
    match result {
        Ok(()) => println!("Проверка пройдена"),
        Err(err) => {
            eprintln!("Проверка не пройдена:\n{err}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    fn good() -> String {
        REQUIRED_SECTIONS.iter()
            .map(|section| format!("## {section}\nПодтверждённый результат и реальное ограничение для этого раздела.\n"))
            .collect::<Vec<_>>().join("\n")
    }
    #[test]
    fn valid_body() { assert!(check_pr_body(&good()).is_ok()); }
    #[test]
    fn parses_markdown_link_destination_without_optional_title() {
        assert_eq!(
            links_in("[Правила](references/pr-body.md \"схема\")"),
            vec!["references/pr-body.md"],
        );
        assert_eq!(
            links_in("[Файл](<references/pr body.md> 'заголовок')"),
            vec!["references/pr body.md"],
        );
    }
    #[test]
    fn classifies_markdown_targets_for_root_and_nested_documents() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("anketa-scout-links-{}-{unique}", process::id()));
        for source in [Path::new("AGENTS.md"), Path::new(".agents/guide.md")] {
            let mut errors = Vec::new();
            for target in [
                "https://example.invalid",
                "http://example.invalid",
                "mailto:test@example.invalid",
                "data:text/plain,test",
                "#section",
            ] {
                check_markdown_target(&root, source, target, &mut errors);
            }
            check_markdown_target(&root, source, "httpd-notes.md", &mut errors);
            check_markdown_target(&root, source, "/x", &mut errors);
            check_markdown_target(&root, source, "C:\\private\\file.md", &mut errors);
            assert_eq!(errors.len(), 3);
            assert!(errors[0].contains("битая ссылка"));
            assert!(errors[1].contains("абсолютная ссылка"));
            assert!(errors[2].contains("абсолютная ссылка"));
        }
        assert_eq!(
            classify_markdown_target("notes.md#section"),
            MarkdownTarget::Local("notes.md"),
        );
    }
    #[cfg(any(unix, windows))]
    #[test]
    fn refuses_symlinked_candidate_directories_without_following_them() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = std::env::temp_dir().join(format!("anketa-scout-symlink-{}-{unique}", process::id()));
        let root = parent.join("checkout");
        let outside = parent.join("outside");
        fs::create_dir_all(root.join(".agents/skills")).unwrap();
        fs::create_dir_all(outside.join("skill")).unwrap();
        fs::write(outside.join("skill/SKILL.md"), "[битая](missing.md)").unwrap();
        let links = symlink_dir(&outside.join("skill"), &root.join(".agents/skills/escape"))
            .and_then(|()| symlink_dir(&root.join(".agents"), &root.join(".agents/loop")));
        if let Err(error) = links {
            let _ = fs::remove_dir_all(&parent);
            if cfg!(windows)
                && (error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(1314))
            {
                eprintln!("Проверка символических ссылок пропущена: Windows не разрешил создать ссылку");
                return;
            }
            panic!("не удалось создать тестовые символические ссылки: {error}");
        }

        let mut file_errors = Vec::new();
        assert!(read_file_safely(
            &root,
            Path::new(".agents/skills/escape/SKILL.md"),
            &mut file_errors,
        ).is_none());
        assert!(file_errors.iter().any(|error| error.contains("символическая ссылка")));

        let mut scan_errors = Vec::new();
        scan_markdown(&root, Path::new(".agents"), &mut scan_errors);
        assert!(scan_errors.iter().filter(|error| error.contains("символическая ссылка")).count() >= 2);
        assert!(!scan_errors.iter().any(|error| error.contains("битая ссылка")));

        fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn rejects_missing_heading() {
        let s = good().replace("## CI", "## Проверки CI");
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn rejects_duplicate_required_heading_after_all_required_sections() {
        let s = format!("{}\n## Цель\nДублирующий раздел с содержательным текстом.", good());
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn rejects_empty_section() {
        let s = good().replace("## Готовность\nПодтверждённый результат и реальное ограничение для этого раздела.", "## Готовность\n—");
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn rejects_fenced_fake_heading() {
        let s = format!("```md\n## Цель\n```\n{}", good().replace("## Цель", "## Что-то другое"));
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn rejects_oversized_body() {
        assert!(check_pr_body(&format!("{}{}", good(), "а".repeat(10_000))).is_err());
    }
    #[test]
    fn parses_frontmatter() {
        assert_eq!(frontmatter_name("---\nname: scout-test\ndescription: Тест\n---\nТекст").unwrap(), "scout-test");
        assert!(frontmatter_name("---\nname: scout-test\n---\n").is_err());
    }
    #[test]
    fn rejects_empty_quoted_and_block_descriptions() {
        for description in ["\"\"", "''", "\"  \"", "|", ">-\n  ", "# комментарий", "\"TODO\""] {
            let text = format!("---\nname: scout-test\ndescription: {description}\n---\n");
            assert!(frontmatter_name(&text).is_err(), "{description}");
        }
        for description in ["\"Тест\"", "'Это ''тест'''", "|\n  Многострочное\n  описание", "Тест: с двоеточием"] {
            let text = format!("---\nname: scout-test\ndescription: {description}\n---\n");
            assert_eq!(frontmatter_name(&text).unwrap(), "scout-test", "{description}");
        }
        assert!(frontmatter_name("---\nname: scout-test\ndescription: Тест\nname: other\n---\n").is_err());
    }
    #[test]
    fn checks_claude_invocation_flag_only_in_frontmatter() {
        let valid = "---\nname: scout-test\ndescription: Тест\ndisable-model-invocation: true\n---\nТекст";
        let misplaced = "---\nname: scout-test\ndescription: Тест\n---\nТекст\ndisable-model-invocation: true";
        assert!(frontmatter_flag(valid, "disable-model-invocation", "true"));
        assert!(!frontmatter_flag(misplaced, "disable-model-invocation", "true"));
    }
    #[test]
    fn ignores_decoy_claude_invocation_flags() {
        for decoy in [
            "---\nname: scout-test\ndescription: |\n  disable-model-invocation: true\n---\n",
            "---\nname: scout-test\ndescription: Тест\nmetadata:\n  disable-model-invocation: true\n---\n",
            "---\nname: scout-test\ndescription: Тест\ndisable-model-invocation: true\ndisable-model-invocation: false\n---\n",
            "---\nname: scout-test\ndescription: Тест\ndisable-model-invocation: \"true\"\n---\n",
        ] {
            assert!(!frontmatter_flag(decoy, "disable-model-invocation", "true"), "{decoy}");
        }
    }
    #[test]
    fn ignores_decoy_codex_invocation_policies() {
        for decoy in [
            "interface:\n  description: |\n    policy:\n      allow_implicit_invocation: false\n",
            "policy:\n  allow_implicit_invocation: false\npolicy:\n  allow_implicit_invocation: true\n",
            "policy:\n  allow_implicit_invocation: false\n  allow_implicit_invocation: true\n",
            "policy:\n  note: |\n    allow_implicit_invocation: false\n  allow_implicit_invocation: true\n",
            "policy: {allow_implicit_invocation: false}\n",
        ] {
            assert!(!policy_disables_implicit_invocation(decoy), "{decoy}");
        }
    }
    #[test]
    fn rejects_links_escaping_checkout() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = std::env::temp_dir().join(format!("anketa-scout-escape-{}-{unique}", process::id()));
        let root = parent.join("checkout");
        fs::create_dir_all(root.join(".agents")).unwrap();
        fs::write(parent.join("outside.md"), "вне копии").unwrap();
        fs::write(root.join("inside.md"), "внутри копии").unwrap();

        let mut errors = Vec::new();
        check_markdown_target(&root, Path::new(".agents/guide.md"), "../../outside.md", &mut errors);
        check_markdown_target(&root, Path::new("AGENTS.md"), "../outside.md", &mut errors);
        check_markdown_target(&root, Path::new(".agents/guide.md"), "../inside.md", &mut errors);
        check_markdown_target(&root, Path::new(".agents/guide.md"), "./../.agents/../inside.md", &mut errors);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().all(|error| error.contains("за пределы репозитория")));

        fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn ignores_links_inside_code_and_comments() {
        let text = "Пример:\n```md\n[битая](missing.md)\n```\n`[код](inline.md)` и ``[x](двойной.md)``\n<!-- [скрыто](hidden.md) -->\n[живая](live.md)\n";
        assert_eq!(links_in(text), vec!["live.md"]);
    }
    #[test]
    fn finds_reference_style_link_definitions() {
        let text = "См. [правила][pr].\n\n[pr]: references/pr-body.md \"заголовок\"\n   [угловая]: <references/pr body.md>\n";
        assert_eq!(links_in(text), vec!["references/pr-body.md", "references/pr body.md"]);
    }
    #[test]
    fn rejects_required_headings_inside_raw_html_block() {
        let hidden = good().replace("## Цель", "<div>\n## Цель");
        assert!(check_pr_body(&hidden).is_err());
        let hidden_pre = good().replace("## Цель", "<pre>\n\n## Цель").replace("## Область изменений", "</pre>\n## Область изменений");
        assert!(check_pr_body(&hidden_pre).is_err());
    }
    #[test]
    fn accepts_headings_after_closed_html_block_and_inline_html() {
        let details = good().replace("## Цель", "<details>\n<summary>Журнал</summary>\nКратко.\n</details>\n\n## Цель");
        assert!(check_pr_body(&details).is_ok());
        let inline = good().replace(
            "## Область изменений",
            "Текст абзаца\n<span>встроенный</span>\n## Область изменений",
        );
        assert!(check_pr_body(&inline).is_ok());
    }
    #[test]
    fn requires_codex_invocation_policy_inside_policy_mapping() {
        assert!(policy_disables_implicit_invocation(
            "policy:\n  allow_implicit_invocation: false\n"
        ));
        assert!(policy_disables_implicit_invocation(
            "interface:\n  display_name: Test\npolicy: # Codex policy\n  allow_implicit_invocation: false\n"
        ));
        assert!(!policy_disables_implicit_invocation(
            "allow_implicit_invocation: false\n"
        ));
        assert!(!policy_disables_implicit_invocation(
            "policy:\n  nested:\n    allow_implicit_invocation: false\n"
        ));
        assert!(!policy_disables_implicit_invocation(
            "policy:\n  allow_implicit_invocation: true\n"
        ));
    }
    #[test]
    fn allows_additional_sections_between_required_sections() {
        let s = good().replace(
            "## Проверки",
            "## Безопасность\nПодтверждённая граница обработки недоверенных данных.\n\n## Проверки",
        );
        assert!(check_pr_body(&s).is_ok());
    }
#[test]
    fn rejects_hidden_comment_heading() {
        let s = good().replace("## Цель", "<!--\n## Цель\n-->\n## Подмена");
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn rejects_indented_code_heading() {
        let s = good().replace("## Цель", "    ## Цель");
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn rejects_comment_only_content() {
        let s = good().replace(
            "## Готовность\nПодтверждённый результат и реальное ограничение для этого раздела.",
            "## Готовность\n<!-- Подтверждённый результат и реальное ограничение для этого раздела. -->"
        );
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn accepts_fenced_example_after_required_sections() {
        let s = format!("{}\n```md\n## Не настоящий заголовок\n```\n", good());
        assert!(check_pr_body(&s).is_ok());
    }
    #[test]
    fn counts_fenced_content_when_checking_section_body() {
        let s = good().replace(
            "## Проверки\nПодтверждённый результат и реальное ограничение для этого раздела.",
            "## Проверки\n```text\ncargo test --locked\nрезультат: 11 tests passed\n```",
        );
        assert!(check_pr_body(&s).is_ok());
    }

    #[test]
    fn finds_markdown_links() {
        assert_eq!(links_in("[Тест](../../file.md#heading)"), vec!["../../file.md#heading"]);
    }
}

