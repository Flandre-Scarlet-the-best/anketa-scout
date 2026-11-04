use crate::{application::Report, domain::ContactKind};
use std::io::{self, Write};

pub fn safe(input: &str) -> String {
    sanitize(input, false)
}

fn sanitize(input: &str, multiline: bool) -> String {
    // ESC/C1, изменение направления текста и прочие невидимые управляющие символы
    // не могут управлять терминалом или маскировать показанный контакт.
    input.chars().filter_map(|c| {
        if c == '\n' && multiline { return Some(c); }
        if matches!(c, '\n' | '\r' | '\t') { return Some(' '); }
        (!c.is_control()
            && !matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')).then_some(c)
    }).collect()
}

pub fn render(report: &Report, mut output: impl Write) -> io::Result<()> {
    writeln!(output, "Источник: {}", safe(&report.source))?;
    writeln!(
        output,
        "Обработано сообщений: {}; анкет: {}",
        report.processed,
        report.profiles.len()
    )?;
    if report.processed == 0 {
        writeln!(output, "Источник вернул пустой тред.")?;
    } else if report.profiles.is_empty() {
        writeln!(
            output,
            "Самостоятельных анкет с собственным контактом не найдено."
        )?;
    }
    for (index, profile) in report.profiles.iter().enumerate() {
        let message = &profile.message;
        writeln!(
            output,
            "\n── Анкета {} · пост №{} ──",
            index + 1,
            message.origin.post
        )?;
        writeln!(output, "Оригинал: {}", safe(&message.origin.link))?;
        if let Some(date) = &message.date {
            writeln!(output, "Дата: {}", safe(date))?;
        }
        for contact in &profile.contacts {
            let kind = match contact.kind {
                ContactKind::Telegram => "Telegram",
                ContactKind::Discord => "Discord",
                ContactKind::Email => "Почта",
            };
            writeln!(output, "{kind}: {}", safe(&contact.value))?;
        }
        writeln!(output, "\nТекст:")?;
        for line in sanitize(&message.text, true).lines() {
            writeln!(output, "  | {line}")?;
        }
        for attachment in &message.attachments {
            write!(output, "Вложение: {}", safe(&attachment.link))?;
            if let Some(name) = &attachment.name {
                write!(output, " · {}", safe(name))?;
            }
            if let Some(size) = attachment.size_kib {
                write!(output, " · {size} КиБ")?;
            }
            writeln!(output)?;
        }
    }
    output.flush()
}
