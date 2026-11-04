//! Детерминированные проверки агентских контрактов и структуры PR без внешних зависимостей.
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process;

const REQUIRED_SKILLS: &[&str] = &[
    "scout-adapter-development",
    "scout-code-review",
    "scout-coderabbit-review",
    "scout-git-workflow",
    "scout-live-testing",
    "scout-performance-benchmarking",
];
const REQUIRED_SECTIONS: &[&str] = &[
    "Цель", "Область изменений", "Реализация", "Проверки",
    "Производительность", "CI", "Готовность", "Ограничения",
];

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn frontmatter_name(text: &str) -> Result<String, String> {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return Err("отсутствует начальный YAML frontmatter".into());
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            if name.replace(value.trim().to_string()).is_some() {
                return Err("дублируется name".into());
            }
        }
        if let Some(value) = line.strip_prefix("description:") {
            if description.replace(value.trim().to_string()).is_some() {
                return Err("дублируется description".into());
            }
        }
    }
    if !closed {
        return Err("не закрыт YAML frontmatter".into());
    }
    let name = name.ok_or("нет name")?;
    if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
        return Err("недопустимое name".into());
    }
    if description.is_none_or(|value| value.is_empty() || value == "TODO") {
        return Err("пустое description".into());
    }
    Ok(name)
}

fn frontmatter_has_line(text: &str, expected: &str) -> bool {
    let mut lines = text.lines();
    if lines.next() != Some("---") {
        return false;
    }
    let mut found = false;
    for line in lines {
        if line.trim() == "---" {
            return found;
        }
        found |= line.trim() == expected;
    }
    false
}

fn links_in(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("](") {
        rest = &rest[pos + 2..];
        if let Some(end) = rest.find(')') {
            let raw_target = rest[..end].trim();
            let target = if let Some(angled) = raw_target.strip_prefix('<') {
                angled.split_once('>').map(|(target, _)| target).unwrap_or("")
            } else {
                raw_target.split_whitespace().next().unwrap_or("")
            };
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
            let dest = source_path.parent().unwrap_or(root).join(target);
            if !dest.exists() {
                errors.push(format!("битая ссылка: {} -> {target}", source_path.display()));
            }
        }
    }
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
                && !frontmatter_has_line(&body, "disable-model-invocation: true")
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
        if !policy.lines().any(|line| line.trim() == "allow_implicit_invocation: false") {
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

fn check_pr_body(body: &str) -> Result<(), String> {
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.chars().count() > 10_000 {
        return Err("PR body превышает 10 000 символов".into());
    }
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut inside_comment = false;
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
        if let Some(opening) = fence_prefix(&visible) {
            fence = Some(opening);
            continue;
        }
        let indent = visible.chars().take_while(|ch| *ch == ' ').count();
        if indent <= 3 {
            if let Some(title) = visible[indent..].trim_end().strip_prefix("## ") {
                parts.push((title.trim_end_matches('#').trim().to_owned(), String::new()));
                continue;
            }
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
    fn checks_claude_invocation_flag_only_in_frontmatter() {
        let valid = "---\nname: scout-test\ndescription: Тест\ndisable-model-invocation: true\n---\nТекст";
        let misplaced = "---\nname: scout-test\ndescription: Тест\n---\nТекст\ndisable-model-invocation: true";
        assert!(frontmatter_has_line(valid, "disable-model-invocation: true"));
        assert!(!frontmatter_has_line(misplaced, "disable-model-invocation: true"));
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

