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
const MAX_BODY_CHARS: usize = 10_000;
/// Узкое исключение: PR зависимостей от аккаунта бота из его собственного префикса ветки.
/// Логин с суффиксом `[bot]` принадлежит GitHub App и не может быть занят пользователем.
const BOT_BRANCHES: &[(&str, &str)] = &[
    ("renovate[bot]", "renovate/"),
    ("dependabot[bot]", "dependabot/"),
];

const PERFORMANCE: &str = "Производительность";
const PERFORMANCE_STATUSES: &[&str] = &["измерено", "не применимо", "не измерено"];
const MAX_PERFORMANCE_CHARS: usize = 2_200;
const MAX_SIMPLE_PERFORMANCE_CHARS: usize = 650;
const NO_EVIDENCE_FIELD: &str = "Доказательство производительности";
const NO_EVIDENCE_VALUE: &str = "отсутствует";
const MEASURED_FIELDS: &[&str] = &["Сценарий", "База", "Кандидат", "Результат", "Вывод", "Регрессии"];
const RESULT_UNITS: &[&str] = &["ms", "µs", "ns", "s", "%", "KiB", "MiB", "GiB", "bytes", "count", "мс", "с"];
const PLACEHOLDERS: &[&str] = &[
    "", "-", "—", "–", "n/a", "na", "none", "нет", "todo", "tbd", "...", "…",
    "не применимо", "заполнить", "уточнить", "см. ниже", "см. выше", "?",
];
const STUB_SECTIONS: &[&str] = &["todo", "tbd", "заполнить", "уточнить"];

/// Канон таблицы — `.agents/skills/scout-coderabbit-review/references/review-workflow.md`.
const CODERABBIT: &str = "CodeRabbit review и disposition";
const CODERABBIT_HEADER: &[&str] = &["Итерация / reviewed HEAD", "Уровень", "Путь", "Влияние", "Решение", "Исправление"];
const CODERABBIT_MODES: &[&str] = &["simple", "deep", "deep + fresh"];
const CODERABBIT_DECISIONS: &[&str] = &[
    "partially confirmed", "confirmed", "duplicate", "stale", "rejected", "needs evidence", "clean", "incomplete",
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

/// Раздел `##` описания PR: `text` включает содержимое code fence (для оценки содержательности),
/// `prose` — только отображаемый текст вне fence (для полей и таблиц, которые читает человек).
struct Section {
    title: String,
    text: String,
    prose: String,
}

fn push_line(sections: &mut [Section], line: &str, in_prose: bool) {
    if let Some(section) = sections.last_mut() {
        section.text.push_str(line);
        section.text.push('\n');
        if in_prose {
            section.prose.push_str(line);
            section.prose.push('\n');
        }
    }
}

fn parse_sections(normalized: &str) -> Result<Vec<Section>, String> {
    let mut parts: Vec<Section> = Vec::new();
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
            } else {
                push_line(&mut parts, line, false);
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
            push_line(&mut parts, &visible, true);
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
            push_line(&mut parts, &visible, true);
            continue;
        }
        let indent = visible.chars().take_while(|ch| *ch == ' ').count();
        if indent <= 3 {
            if let Some(title) = visible[indent..].trim_end().strip_prefix("## ") {
                parts.push(Section {
                    title: title.trim_end_matches('#').trim().to_owned(),
                    text: String::new(),
                    prose: String::new(),
                });
                paragraph_open = false;
                continue;
            }
        }
        if !visible.trim().is_empty() {
            paragraph_open = true;
        }
        push_line(&mut parts, &visible, true);
    }
    if fence.is_some() || inside_comment {
        return Err("PR body содержит незакрытый блок кода или HTML-комментарий".into());
    }
    Ok(parts)
}

/// Строка «Имя: значение» вне таблиц, в том числе пунктом списка и с полужирным именем.
fn field(line: &str) -> Option<(String, String)> {
    let mut rest = line.trim_start();
    if rest.starts_with('|') {
        return None;
    }
    if let Some(marker) = rest.chars().next().filter(|ch| matches!(ch, '-' | '*' | '+')) {
        let after = &rest[marker.len_utf8()..];
        if after.starts_with(char::is_whitespace) {
            rest = after.trim_start();
        }
    }
    let rest = rest.strip_prefix("**").unwrap_or(rest);
    let (name, value) = rest.split_once(':')?;
    let name = name.strip_suffix("**").unwrap_or(name).trim();
    if name.is_empty() || name.contains(['|', '*']) {
        return None;
    }
    let value = value.strip_prefix("**").unwrap_or(value);
    Some((name.to_owned(), value.trim().trim_matches('*').trim().to_owned()))
}

fn fields(content: &str) -> Vec<(String, String)> {
    content.lines().filter_map(field).collect()
}

fn field_values<'a>(found: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    found.iter().filter(|(key, _)| key == name).map(|(_, value)| value.as_str()).collect()
}

fn placeholder(value: &str) -> bool {
    let text = value.trim().trim_matches(['`', '*', '_', ' ']).to_lowercase();
    let angle = text.len() >= 2
        && text.starts_with('<')
        && text.ends_with('>')
        && !text[1..text.len() - 1].contains(['<', '>']);
    PLACEHOLDERS.contains(&text.as_str()) || angle
}

/// Хотя бы одно конкретное пояснение, а не шаблонная заглушка.
fn substantive(values: &[&str], minimum: usize) -> bool {
    values.iter().any(|value| {
        !placeholder(value) && value.trim().chars().count() >= minimum && value.split_whitespace().count() >= 3
    })
}

/// Число с единицей измерения: `12 ms`, `1,5 MiB`, `3 %`, `40 мс`.
fn has_numeric_result(value: &str) -> bool {
    value.char_indices().filter(|(_, ch)| ch.is_ascii_digit()).any(|(start, _)| {
        let tail = value[start..]
            .trim_start_matches(|ch: char| ch.is_ascii_digit() || matches!(ch, '.' | ',') || ch.is_whitespace());
        RESULT_UNITS.iter().any(|unit| {
            tail.strip_prefix(unit).is_some_and(|after| {
                *unit == "%" || !after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')
            })
        })
    })
}

fn scenario_blocks(content: &str) -> Vec<(String, String)> {
    let mut blocks: Vec<(String, String)> = Vec::new();
    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            blocks.push((heading.trim().to_owned(), String::new()));
        } else if let Some((_, block)) = blocks.last_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    blocks
}

fn declares_missing_evidence(found: &[(String, String)]) -> bool {
    field_values(found, NO_EVIDENCE_FIELD)
        .iter()
        .any(|value| value.to_lowercase() == NO_EVIDENCE_VALUE)
}

fn is_full_report_table(line: &str) -> bool {
    line.trim_start()
        .strip_prefix('|')
        .and_then(|rest| rest.trim_start().strip_prefix("Метрика"))
        .is_some_and(|tail| tail.trim_start().starts_with('|'))
}

/// Компактный вывод о производительности; истинность измерений требует отдельной проверки.
fn check_performance(section: &Section) -> Vec<String> {
    let prefix = format!("## {PERFORMANCE}: ");
    let mut errors = Vec::new();
    // Лимиты относятся ко всему разделу вместе с его заголовком.
    let length = format!("## {PERFORMANCE}\n").chars().count() + section.text.trim().chars().count();
    if length > MAX_PERFORMANCE_CHARS {
        errors.push(format!(
            "{prefix}{length} символов, максимум {MAX_PERFORMANCE_CHARS}; сократите отчёт до выводов"
        ));
    }
    let found = fields(&section.prose);
    let statuses: Vec<String> = field_values(&found, "Статус")
        .iter()
        .map(|value| value.trim_matches('`').to_lowercase())
        .collect();
    let status = match statuses.as_slice() {
        [status] if PERFORMANCE_STATUSES.contains(&status.as_str()) => status.as_str(),
        _ => {
            errors.push(format!(
                "{prefix}нужна ровно одна строка «Статус: измерено», «Статус: не применимо» или «Статус: не измерено»"
            ));
            return errors;
        }
    };
    if status != "измерено" && length > MAX_SIMPLE_PERFORMANCE_CHARS {
        errors.push(format!(
            "{prefix}краткое решение занимает более {MAX_SIMPLE_PERFORMANCE_CHARS} символов"
        ));
    }
    match status {
        "не применимо" => {
            if !substantive(&field_values(&found, "Причина"), 40) {
                errors.push(format!("{prefix}«не применимо» требует конкретной причины не короче 40 символов"));
            }
        }
        "не измерено" => {
            for name in ["Причина", "Диагностика"] {
                if !substantive(&field_values(&found, name), 20) {
                    errors.push(format!("{prefix}«не измерено» требует фактической строки «{name}:»"));
                }
            }
            if !declares_missing_evidence(&found) {
                errors.push(format!(
                    "{prefix}«не измерено» требует строки «{NO_EVIDENCE_FIELD}: {NO_EVIDENCE_VALUE}»"
                ));
            }
        }
        _ => errors.extend(check_measured_scenarios(&prefix, &section.prose)),
    }
    errors
}

fn check_measured_scenarios(prefix: &str, prose: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let blocks = scenario_blocks(prose);
    if blocks.is_empty() {
        errors.push(format!("{prefix}каждый сценарий «измерено» описывается в отдельном `###`-подразделе"));
    }
    let mut measured = 0;
    for (heading, block) in &blocks {
        let info = fields(block);
        if heading.ends_with("— не измерено") {
            for name in ["Причина", "Диагностика"] {
                let values = field_values(&info, name);
                if values.is_empty() || values.iter().all(|value| placeholder(value)) {
                    errors.push(format!("{prefix}неизмеренный сценарий требует строки «{name}:» ({heading})"));
                }
            }
            if !declares_missing_evidence(&info) {
                errors.push(format!("{prefix}неизмеренный сценарий требует отсутствия доказательства ({heading})"));
            }
            continue;
        }
        measured += 1;
        for name in MEASURED_FIELDS {
            let values = field_values(&info, name);
            if values.len() != 1 || values.iter().any(|value| placeholder(value)) {
                errors.push(format!("{prefix}для измерения нужна фактическая строка «{name}:» ({heading})"));
            }
        }
        if !field_values(&info, "Результат").iter().any(|value| has_numeric_result(value)) {
            errors.push(format!("{prefix}«Результат» требует числового значения с единицей ({heading})"));
        }
        if block.lines().any(is_full_report_table) {
            errors.push(format!(
                "{prefix}не вставляйте полную таблицу отчёта: оставьте компактный вывод ({heading})"
            ));
        }
    }
    if !blocks.is_empty() && measured == 0 {
        errors.push(format!("{prefix}«измерено» требует хотя бы один измеренный сценарий"));
    }
    errors
}

/// Ячейки строки таблицы GFM; `None` — строка не может быть строкой таблицы.
/// Отступ от четырёх пробелов — код-блок, пустая строка завершает таблицу.
fn table_row(line: &str) -> Option<Vec<String>> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let trimmed = line.trim();
    if trimmed.is_empty() || indent > 3 {
        return None;
    }
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for ch in trimmed.chars() {
        if ch == '|' && !escaped {
            cells.push(String::new());
        } else if let Some(cell) = cells.last_mut() {
            cell.push(ch);
        }
        escaped = ch == '\\' && !escaped;
    }
    if cells.len() == 1 {
        return None;
    }
    if trimmed.starts_with('|') {
        cells.remove(0);
    }
    if trimmed.ends_with('|') && !trimmed.ends_with("\\|") {
        cells.pop();
    }
    Some(cells.into_iter().map(|cell| cell.trim().to_owned()).collect())
}

fn is_delimiter_cell(cell: &str) -> bool {
    let core = cell.strip_prefix(':').unwrap_or(cell);
    let core = core.strip_suffix(':').unwrap_or(core);
    !core.is_empty() && core.chars().all(|ch| ch == '-')
}

/// Первая ячейка по канону: `<номер> · \`<reviewed HEAD>\` · <режим>`.
fn is_iteration_cell(cell: &str) -> bool {
    let parts: Vec<&str> = cell.split('·').map(str::trim).collect();
    let [number, head, mode] = parts.as_slice() else {
        return false;
    };
    let number_ok = !number.is_empty() && !number.starts_with('0') && number.chars().all(|ch| ch.is_ascii_digit());
    let head_ok = head
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
        .is_some_and(|sha| (7..=40).contains(&sha.len()) && sha.chars().all(|ch| matches!(ch, '0'..='9' | 'a'..='f')));
    number_ok && head_ok && CODERABBIT_MODES.contains(mode)
}

fn decision_label(cell: &str) -> Option<&'static str> {
    CODERABBIT_DECISIONS.iter().copied().find(|label| {
        cell.strip_prefix(label)
            .is_some_and(|rest| !rest.starts_with(|ch: char| ch.is_alphanumeric()))
    })
}

/// Одна накопительная таблица GFM: заголовок, разделитель и строки данных подряд —
/// только так GitHub собирает их в одну таблицу.
fn check_coderabbit(section: &Section) -> Vec<String> {
    let lines: Vec<&str> = section.prose.lines().collect();
    let header: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| table_row(line).is_some_and(|row| row == CODERABBIT_HEADER))
        .map(|(index, _)| index)
        .collect();
    let start = match header.as_slice() {
        [] => return vec!["в разделе CodeRabbit отсутствует заголовок накопительной таблицы".into()],
        [start] => *start,
        _ => {
            return vec!["в разделе CodeRabbit заголовок накопительной таблицы повторяется: нужна одна таблица".into()];
        }
    };
    let delimiter = lines.get(start + 1).and_then(|line| table_row(line));
    let delimiter_ok = delimiter
        .is_some_and(|row| row.len() == CODERABBIT_HEADER.len() && row.iter().all(|cell| is_delimiter_cell(cell)));
    if !delimiter_ok {
        return vec![format!(
            "в разделе CodeRabbit за заголовком таблицы нет строки-разделителя из {} столбцов: GitHub не соберёт таблицу",
            CODERABBIT_HEADER.len()
        )];
    }
    let mut errors = Vec::new();
    let mut completed = false;
    let mut end = lines.len();
    for (offset, line) in lines.iter().enumerate().skip(start + 2) {
        let Some(row) = table_row(line) else {
            end = offset;
            break;
        };
        if row.len() != CODERABBIT_HEADER.len() {
            errors.push(format!(
                "строка таблицы CodeRabbit содержит {} столбцов вместо {}: {}",
                row.len(),
                CODERABBIT_HEADER.len(),
                line.trim()
            ));
            continue;
        }
        if !is_iteration_cell(&row[0]) || row[1..].iter().any(String::is_empty) {
            errors.push(format!(
                "строка таблицы CodeRabbit не связана с номером, reviewed HEAD и режимом или содержит пустые ячейки: {}",
                line.trim()
            ));
            continue;
        }
        match decision_label(&row[4]) {
            None => errors.push(format!("строка таблицы CodeRabbit содержит неизвестное решение: {}", line.trim())),
            Some("incomplete") => {}
            Some(_) => completed = true,
        }
    }
    if !completed {
        errors.push("в разделе CodeRabbit отсутствует строка завершённой итерации".into());
    }
    for line in &lines[end..] {
        if table_row(line).is_some_and(|row| row.first().is_some_and(|cell| is_iteration_cell(cell))) {
            errors.push(format!("строка итерации CodeRabbit оторвана от накопительной таблицы: {}", line.trim()));
        }
    }
    errors
}

fn check_pr_body(body: &str) -> Result<(), String> {
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.chars().count() > MAX_BODY_CHARS {
        return Err(format!("PR body превышает {MAX_BODY_CHARS} символов"));
    }
    let parts = parse_sections(&normalized)?;
    let mut expected_index = 0;
    for section in &parts {
        if expected_index == REQUIRED_SECTIONS.len() {
            continue;
        }
        let Some(position) = REQUIRED_SECTIONS.iter().position(|required| *required == section.title) else {
            continue;
        };
        if position != expected_index {
            let expected = REQUIRED_SECTIONS[expected_index];
            return Err(format!(
                "раздел {}: ожидается «{}», найден «{}»",
                expected_index + 1,
                expected,
                section.title
            ));
        }
        let expected = REQUIRED_SECTIONS[expected_index];
        let meaningful = section.text.chars().filter(|c| c.is_alphanumeric()).count();
        let stub = STUB_SECTIONS.contains(&section.text.trim().to_lowercase().as_str());
        if meaningful < 12 || stub {
            return Err(format!("раздел «{expected}» не содержит содержательного описания"));
        }
        expected_index += 1;
    }
    if expected_index < REQUIRED_SECTIONS.len() {
        return Err(format!("не хватает разделов PR: {} из {}", expected_index, REQUIRED_SECTIONS.len()));
    }
    let mut titles = BTreeSet::new();
    for section in &parts {
        if !titles.insert(&section.title) {
            return Err(format!("повторяется раздел «{}»", section.title));
        }
    }
    let mut errors = Vec::new();
    for section in &parts {
        if section.title == PERFORMANCE {
            errors.extend(check_performance(section));
        } else if section.title == CODERABBIT {
            errors.extend(check_coderabbit(section));
        } else if section.title.contains("CodeRabbit") {
            errors.push(format!(
                "раздел CodeRabbit должен называться «{CODERABBIT}», найден «{}»",
                section.title
            ));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

/// PR зависимостей от проверенного бота в его собственной ветке.
fn is_automation_pr(author: &str, head_ref: &str) -> bool {
    BOT_BRANCHES
        .iter()
        .any(|(login, prefix)| author == *login && head_ref.starts_with(prefix))
}

fn check_pr_body_command(file: &str, options: &[String]) -> Result<(), String> {
    let mut author = None;
    let mut head_ref = None;
    let mut rest = options;
    while let [flag, value, tail @ ..] = rest {
        match flag.as_str() {
            "--author" if author.is_none() => author = Some(value.as_str()),
            "--head-ref" if head_ref.is_none() => head_ref = Some(value.as_str()),
            _ => return Err(format!("неизвестный или повторный параметр pr-body: {flag}")),
        }
        rest = tail;
    }
    if let [flag] = rest {
        return Err(format!("параметр pr-body без значения: {flag}"));
    }
    if let (Some(author), Some(head_ref)) = (author, head_ref) {
        if is_automation_pr(author, head_ref) {
            println!("PR dependency-бота: формат ручных PR не применяется");
            return Ok(());
        }
    }
    read(Path::new(file)).and_then(|body| check_pr_body(&body))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let result = match args.as_slice() {
        [_, cmd, root] if cmd == "skills" => check_skills(&PathBuf::from(root)),
        [_, cmd, file, options @ ..] if cmd == "pr-body" => check_pr_body_command(file, options),
        _ => Err(
            "использование: agent_contracts skills <корень> | pr-body <файл> [--author <логин> --head-ref <ветка>]"
                .into(),
        ),
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

    const NOT_MEASURED: &str = "Статус: не измерено\nПричина: изменение не затрагивает горячий путь CLI.\nДиагностика: сопоставимая пара прогонов не снималась.\nДоказательство производительности: отсутствует\n";
    const CODERABBIT_TABLE: &str = "| Итерация / reviewed HEAD | Уровень | Путь | Влияние | Решение | Исправление |\n| --- | --- | --- | --- | --- | --- |\n| 1 · `86e4ad1` · simple | minor | `src/output.rs` | Обрезка ячейки | confirmed | `dd4718f` |\n";

    fn good() -> String {
        REQUIRED_SECTIONS.iter()
            .map(|section| match *section {
                PERFORMANCE => format!("## {section}\n{NOT_MEASURED}"),
                _ => format!("## {section}\nПодтверждённый результат и реальное ограничение для этого раздела.\n"),
            })
            .collect::<Vec<_>>().join("\n")
    }
    fn with_performance(content: &str) -> String {
        good().replace(NOT_MEASURED, content)
    }
    fn with_coderabbit(table: &str) -> String {
        format!("{}\n## {CODERABBIT}\n{table}", good())
    }

    #[test]
    fn accepts_canonical_performance_statuses() {
        let not_applicable = "Статус: не применимо\nПричина: PR меняет только документацию и не затрагивает исполняемый код.\n";
        let measured = "Статус: измерено\n\n### Разбор треда\n- Сценарий: 700 постов из синтетического снимка\n- База: `33d2c56`\n- Кандидат: `6ba1698`\n- Результат: медиана 41 ms против 44 ms, −7 %\n- Вывод: ускорение в пределах шума не заявляется\n- Регрессии: не обнаружены\n\n### Холодный старт — не измерено\nПричина: нет стабильного раннера.\nДиагностика: разброс выше 20 %.\nДоказательство производительности: отсутствует\n";
        for content in [NOT_MEASURED, not_applicable, measured] {
            assert_eq!(check_pr_body(&with_performance(content)), Ok(()), "{content}");
        }
        let bold = "- **Статус:** не измерено\n- **Причина:** изменение не затрагивает горячий путь CLI.\n- **Диагностика:** сопоставимая пара прогонов не снималась.\n- **Доказательство производительности:** отсутствует\n";
        assert_eq!(check_pr_body(&with_performance(bold)), Ok(()));
    }
    #[test]
    fn rejects_incomplete_performance_decisions() {
        for content in [
            "Не измерено: изменение не затрагивает горячий путь CLI.\n",
            "Статус: не измерено\nСтатус: измерено\nПричина: двойной статус недопустим здесь.\n",
            "Статус: не измерено\nПричина: изменение не затрагивает горячий путь.\nДиагностика: —\nДоказательство производительности: отсутствует\n",
            "Статус: не измерено\nПричина: изменение не затрагивает горячий путь.\nДиагностика: сопоставимая пара не снималась.\n",
            "Статус: не применимо\nПричина: только документация.\n",
            "Статус: измерено\nРезультат: быстрее, чем раньше на глаз\n",
            "Статус: измерено\n\n### Разбор\n- Сценарий: снимок\n- База: a\n- Кандидат: b\n- Результат: заметно быстрее\n- Вывод: лучше\n- Регрессии: нет\n",
            "Статус: измерено\n\n### Разбор\n- Сценарий: снимок\n- База: a\n- Кандидат: b\n- Результат: 3 ms\n- Вывод: лучше\n- Регрессии: не обнаружены\n| Метрика | База |\n| --- | --- |\n",
            "```text\nСтатус: не измерено\nПричина: спрятано в код-блоке и не видно.\nДиагностика: спрятано в код-блоке и не видно.\nДоказательство производительности: отсутствует\n```\n",
        ] {
            assert!(check_pr_body(&with_performance(content)).is_err(), "{content}");
        }
        let long = format!("{NOT_MEASURED}{}\n", "Пояснение. ".repeat(60));
        assert!(check_pr_body(&with_performance(&long)).is_err());
    }
    #[test]
    fn numeric_result_requires_unit() {
        for value in ["41 ms", "1,5 MiB", "−7 %", "40 мс", "12 count", "3.2 s;"] {
            assert!(has_numeric_result(value), "{value}");
        }
        for value in ["быстрее", "41", "3 sec", "12 msx", "5 сек"] {
            assert!(!has_numeric_result(value), "{value}");
        }
    }
    #[test]
    fn accepts_canonical_coderabbit_table() {
        assert_eq!(check_pr_body(&with_coderabbit(CODERABBIT_TABLE)), Ok(()));
        let clean = CODERABBIT_TABLE.replace(
            "| 1 · `86e4ad1` · simple | minor | `src/output.rs` | Обрезка ячейки | confirmed | `dd4718f` |",
            "| 1 · `86e4ad1` · deep + fresh | — | — | замечаний нет | clean | — |\n| 2 · `dd4718f` · deep | — | — | обрыв сессии | incomplete: лимит провайдера | — |",
        );
        assert_eq!(check_pr_body(&with_coderabbit(&clean)), Ok(()));
    }
    #[test]
    fn rejects_noncanonical_coderabbit_section() {
        let renamed = format!("{}\n## CodeRabbit\n{CODERABBIT_TABLE}", good());
        assert!(check_pr_body(&renamed).is_err());
        let row = "| 1 · `86e4ad1` · simple | minor | `src/output.rs` | Обрезка ячейки | confirmed | `dd4718f` |";
        for table in [
            "Замечаний нет.\n".to_owned(),
            CODERABBIT_TABLE.replace("| --- | --- | --- | --- | --- | --- |\n", ""),
            CODERABBIT_TABLE.replace(row, "| 1 · `86e4ad1` | minor | `src/output.rs` | Обрезка ячейки | confirmed | `dd4718f` |"),
            CODERABBIT_TABLE.replace(row, "| 1 · `86E4AD1` · simple | minor | `src/output.rs` | Обрезка | confirmed | `dd4718f` |"),
            CODERABBIT_TABLE.replace(row, "| 1 · `86e4ad1` · simple | minor | `src/output.rs` | Обрезка | принято | `dd4718f` |"),
            CODERABBIT_TABLE.replace(row, "| 1 · `86e4ad1` · simple | minor |  | Обрезка | confirmed | `dd4718f` |"),
            CODERABBIT_TABLE.replace(row, "| 1 · `86e4ad1` · simple | minor | `src/output.rs` | confirmed | `dd4718f` |"),
            CODERABBIT_TABLE.replace(row, "| 1 · `86e4ad1` · simple | — | — | обрыв | incomplete | — |"),
            format!("{CODERABBIT_TABLE}\n{row}\n"),
            format!("{CODERABBIT_TABLE}\n{CODERABBIT_TABLE}"),
        ] {
            assert!(check_pr_body(&with_coderabbit(&table)).is_err(), "{table}");
        }
    }
    #[test]
    fn splits_gfm_rows_only_on_unescaped_pipes() {
        assert_eq!(table_row("| a \\| b | c |"), Some(vec!["a \\| b".to_owned(), "c".to_owned()]));
        assert_eq!(table_row("a | b"), Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(table_row("    | код | блок |"), None);
        assert_eq!(table_row("без таблицы"), None);
    }
    #[test]
    fn exempts_only_dependency_bots_in_their_branches() {
        assert!(is_automation_pr("renovate[bot]", "renovate/all-dependencies"));
        assert!(is_automation_pr("dependabot[bot]", "dependabot/cargo/serde-1.0.1"));
        assert!(!is_automation_pr("renovate[bot]", "feat/manual"));
        assert!(!is_automation_pr("renovate", "renovate/all-dependencies"));
        assert!(!is_automation_pr("Flandre-Scarlet-the-best", "renovate/all-dependencies"));
        assert!(!is_automation_pr("dependabot[bot]", "renovate/all-dependencies"));
    }
    #[test]
    fn pr_body_command_checks_options_before_exemption() {
        let options = |items: &[&str]| items.iter().map(|item| (*item).to_owned()).collect::<Vec<_>>();
        let missing = "несуществующий-файл.md";
        assert_eq!(
            check_pr_body_command(missing, &options(&["--author", "renovate[bot]", "--head-ref", "renovate/x"])),
            Ok(())
        );
        assert!(check_pr_body_command(missing, &options(&["--author", "user", "--head-ref", "renovate/x"])).is_err());
        assert!(check_pr_body_command(missing, &options(&["--author", "renovate[bot]"])).is_err());
        assert!(check_pr_body_command(missing, &options(&["--author", "a", "--author", "b"])).is_err());
        assert!(check_pr_body_command(missing, &options(&["--unknown", "x"])).is_err());
        assert!(check_pr_body_command(missing, &options(&["--author"])).is_err());
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

