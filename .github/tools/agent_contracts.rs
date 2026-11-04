//! Детерминированные проверки агентских контрактов и структуры PR без внешних зависимостей.
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
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

fn links_in(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("](") {
        rest = &rest[pos + 2..];
        if let Some(end) = rest.find(')') {
            let target = rest[..end].trim();
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

fn scan_markdown(root: &Path, folder: &Path, errors: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(folder) else {
        errors.push(format!("каталог недоступен: {}", folder.display()));
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_markdown(root, &path, errors);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            let Ok(body) = read(&path) else {
                errors.push(format!("не удалось прочитать {}", path.display()));
                continue;
            };
            for target in links_in(&body) {
                let target = target.split('#').next().unwrap_or("").trim();
                if target.is_empty()
                    || target.starts_with("https://")
                    || target.starts_with("http://")
                    || target.starts_with("mailto:")
                    || target.starts_with("data:")
                {
                    continue;
                }
                if target.starts_with('/') {
                    errors.push(format!("абсолютная ссылка: {} -> {target}", path.display()));
                    continue;
                }
                let dest = path.parent().unwrap_or(root).join(target);
                if !dest.exists() {
                    errors.push(format!("битая ссылка: {} -> {target}", path.display()));
                }
            }
        }
    }
}

fn check_skills(root: &Path) -> Result<(), String> {
    let agents = root.join(".agents/skills");
    let claude = root.join(".claude/skills");
    let mut found = BTreeSet::new();
    let mut errors = Vec::new();
    let entries = fs::read_dir(&agents).map_err(|e| format!("{}: {e}", agents.display()))?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let canonical = dir.join("SKILL.md");
        let wrapper = claude.join(&name).join("SKILL.md");
        if fs::symlink_metadata(&wrapper).is_ok_and(|m| m.file_type().is_symlink()) {
            errors.push(format!("{name}: вход Claude не должен быть символической ссылкой"));
        }
        match read(&canonical).and_then(|t| frontmatter_name(&t)) {
            Ok(declared) if declared == name => {}
            Ok(declared) => errors.push(format!("{name}: name отличается ({declared})")),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
        match read(&wrapper) {
            Ok(body) => {
                match frontmatter_name(&body) {
                    Ok(declared) if declared == name => {}
                    Ok(declared) => errors.push(format!("{name}: неверное name входа Claude: {declared}")),
                    Err(e) => errors.push(format!("{name}: вход Claude: {e}")),
                }
                let reference = format!("../../../.agents/skills/{name}/SKILL.md");
                if !body.contains(&reference) {
                    errors.push(format!("{name}: вход Claude не указывает канонический skill"));
                }
                if name == "scout-coderabbit-review" && !body.lines().any(|line| line.trim() == "disable-model-invocation: true") {
                    errors.push("CodeRabbit: у Claude разрешён неявный вызов".into());
                }
            }
            Err(e) => errors.push(e),
        }
        found.insert(name);
    }
    for name in REQUIRED_SKILLS {
        if !found.contains(*name) {
            errors.push(format!("отсутствует обязательный skill {name}"));
        }
    }
    if let Ok(entries) = fs::read_dir(&claude) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !found.contains(&name) {
                    errors.push(format!("вход Claude без канонического skill: {name}"));
                }
            }
        }
    } else {
        errors.push("отсутствует каталог входов Claude".into());
    }
    let policy = read(&agents.join("scout-coderabbit-review/agents/openai.yaml"))?;
    if !policy.lines().any(|line| line.trim() == "allow_implicit_invocation: false") {
        errors.push("CodeRabbit: у Codex разрешён неявный вызов".into());
    }
    scan_markdown(root, &root.join(".agents"), &mut errors);
    scan_markdown(root, &root.join(".claude"), &mut errors);
    scan_markdown(root, &root.join(".codex"), &mut errors);
    let agents_md = root.join("AGENTS.md");
    if let Ok(body) = read(&agents_md) {
        for target in links_in(&body) {
            let target = target.split('#').next().unwrap_or("");
            if !target.is_empty() && !target.starts_with("http") && !root.join(target).exists() {
                errors.push(format!("AGENTS.md: битая ссылка {target}"));
            }
        }
    } else {
        errors.push("отсутствует AGENTS.md".into());
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
    let normalized = body.replace("\\r\\n", "\\n").replace('\\r', "\\n");
    if normalized.chars().count() > 10_000 {
        return Err("PR body превышает 10 000 символов".into());
    }
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut inside_comment = false;
    for line in normalized.lines() {
        if let Some((marker, minimum)) = fence {
            if let Some((closing_marker, count)) = fence_prefix(line) {
                if closing_marker == marker
                    && count >= minimum
                    && line.trim().chars().all(|ch| ch == marker)
                {
                    fence = None;
                }
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
            text.push('\\n');
        }
    }
    if fence.is_some() || inside_comment {
        return Err("PR body содержит незакрытый блок кода или HTML-комментарий".into());
    }
    if parts.len() < REQUIRED_SECTIONS.len() {
        return Err(format!("не хватает разделов PR: {} из {}", parts.len(), REQUIRED_SECTIONS.len()));
    }
    for (i, expected) in REQUIRED_SECTIONS.iter().enumerate() {
        let (title, content) = &parts[i];
        if title != expected {
            return Err(format!("раздел {}: ожидается «{}», найден «{}»", i + 1, expected, title));
        }
        let meaningful = content.chars().filter(|c| c.is_alphanumeric()).count();
        if meaningful < 12 || content.trim().eq_ignore_ascii_case("TODO") {
            return Err(format!("раздел «{expected}» не содержит содержательного описания"));
        }
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
    fn good() -> String {
        REQUIRED_SECTIONS.iter()
            .map(|section| format!("## {section}\nПодтверждённый результат и реальное ограничение для этого раздела.\n"))
            .collect::<Vec<_>>().join("\n")
    }
    #[test]
    fn valid_body() { assert!(check_pr_body(&good()).is_ok()); }
    #[test]
    fn rejects_missing_heading() {
        let s = good().replace("## CI", "## Проверки CI");
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
    fn rejects_hidden_comment_heading() {
        let s = good().replace("## Цель", "<!--\\n## Цель\\n-->\\n## Подмена");
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
            "## Готовность\\nПодтверждённый результат и реальное ограничение для этого раздела.",
            "## Готовность\\n<!-- Подтверждённый результат и реальное ограничение для этого раздела. -->"
        );
        assert!(check_pr_body(&s).is_err());
    }
    #[test]
    fn accepts_fenced_example_after_required_sections() {
        let s = format!("{}\\n```md\\n## Не настоящий заголовок\\n```\\n", good());
        assert!(check_pr_body(&s).is_ok());
    }

    #[test]
    fn finds_markdown_links() {
        assert_eq!(links_in("[Тест](../../file.md#heading)"), vec!["../../file.md#heading"]);
    }
}

