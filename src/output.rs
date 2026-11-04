pub mod style;

use crate::{application::Report, domain::ContactKind};
use clap::ValueEnum;
use comfy_table::{
    ColumnConstraint, ContentArrangement, ContentLineStyle, Row, Table, Width,
    presets::{NOTHING, UTF8_FULL},
};
use std::io::{self, Write};
use style::{ColorDepth, Palette, Role, Theme};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum View {
    #[default]
    Full,
    Compact,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub width: Option<u16>,
    pub view: View,
    pub palette: Palette,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: None,
            view: View::Full,
            palette: Palette {
                theme: Theme::Dark,
                depth: ColorDepth::None,
            },
        }
    }
}

pub fn safe(input: &str) -> String {
    sanitize(input, false)
}

fn sanitize(input: &str, multiline: bool) -> String {
    // ZWJ/ZWNJ сохраняют составные графемы. Bidi, ESC/C1 и невидимые разделители
    // не могут управлять терминалом или маскировать показанный контакт.
    input.chars().filter_map(|c| {
        if c == '\n' && multiline { return Some(c); }
        if matches!(c, '\n' | '\r' | '\t') { return Some(' '); }
        (!c.is_control()
            && !matches!(c, '\u{061c}' | '\u{200b}' | '\u{200e}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')).then_some(c)
    }).collect()
}

fn cell_text(input: &str, multiline: bool) -> String {
    let mut result = String::new();
    for c in sanitize(input, multiline).chars() {
        // Источник не может породить разделитель ячейки или её доверенный маркер.
        if matches!(c, '\u{2500}'..='\u{257f}' | '|' | '›') {
            use std::fmt::Write;
            let _ = write!(result, "\\u{{{:04X}}}", c as u32);
        } else {
            result.push(c);
        }
    }
    result
}

fn table(width: u16) -> Table {
    let mut table = Table::new();
    table
        .load_style(
            UTF8_FULL
                .with_rounded_corners()
                .header_lines(ContentLineStyle::new('│', '│', '│'))
                .content_lines(ContentLineStyle::new('│', '│', '│')),
        )
        .set_content_arrangement(ContentArrangement::DynamicFullWidth)
        .set_width(width);
    table
}

fn write_table(
    mut output: impl Write,
    mut table: Table,
    roles: &[Role],
    palette: Palette,
) -> io::Result<()> {
    // Пробел после маркера отделяет его даже от начального комбинируемого знака.
    for column in table.column_iter_mut() {
        column.set_padding((2, 1));
    }
    let mut header = true;
    for line in table.lines() {
        if line.starts_with('╞') {
            header = false;
        }
        if line.starts_with('│') {
            write!(output, "{}", palette.paint(Role::Border, "│"))?;
            for (index, cell) in line.split('│').skip(1).enumerate() {
                if index == roles.len() {
                    break;
                }
                let role = if header {
                    if roles == [Role::Brand] {
                        Role::Brand
                    } else {
                        Role::Heading
                    }
                } else {
                    roles[index]
                };
                // После переноса каждая непустая физическая строка данных получает маркер.
                // Разделители здесь созданы только таблицей: удалённые уже экранированы.
                let content = if !header && !cell.trim().is_empty() {
                    cell.replacen(' ', "›", 1)
                } else {
                    cell.to_owned()
                };
                write!(
                    output,
                    "{}{}",
                    palette.paint(role, &content),
                    palette.paint(Role::Border, "│")
                )?;
            }
            writeln!(output)?;
        } else {
            writeln!(output, "{}", palette.paint(Role::Border, &line))?;
        }
    }
    Ok(())
}

fn field(
    mut output: impl Write,
    width: u16,
    label: &str,
    value: &str,
    role: Role,
    palette: Palette,
) -> io::Result<()> {
    let mut grid = table(width);
    grid.set_header(vec![label]).add_row(vec![value]);
    write_table(&mut output, grid, &[role], palette)
}

pub fn render(report: &Report, output: impl Write) -> io::Result<()> {
    render_with_options(report, output, Options::default())
}

fn preview(text: &str) -> String {
    // Перенос и сокращение между графемами выполняет тот же стек, что и таблицы.
    let mut grid = Table::new();
    grid.load_style(NOTHING)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(100)
        .set_truncation_indicator("…");
    let mut row = Row::from(vec![text.replace('\n', " ")]);
    row.max_height(1);
    grid.add_row(row);
    for column in grid.column_iter_mut() {
        column.set_padding((0, 0));
    }
    grid.trim_fmt()
}

pub fn render_with_options(
    report: &Report,
    mut output: impl Write,
    options: Options,
) -> io::Result<()> {
    // Минимальная поддерживаемая ширина оставляет место рамке, маркеру,
    // изолирующему пробелу и широкой графеме без обрезания данных.
    let width = options.width.unwrap_or(96).max(8);
    let palette = options.palette;
    field(
        &mut output,
        width,
        "Anketa Scout",
        match options.view {
            View::Full => "Полный просмотр анкет",
            View::Compact => "Компактный обзор · текст сокращён; полный просмотр: --view full",
        },
        Role::Brand,
        palette,
    )?;
    field(
        &mut output,
        width,
        "Источник",
        &cell_text(&report.source, false),
        Role::Contact,
        palette,
    )?;
    if width >= 24 {
        let mut summary = table(width);
        summary.set_header(vec!["Сообщений", "Анкет"]).add_row(vec![
            report.processed.to_string(),
            report.profiles.len().to_string(),
        ]);
        write_table(
            &mut output,
            summary,
            &[Role::Metadata, Role::Success],
            palette,
        )?;
    } else {
        field(
            &mut output,
            width,
            "Сообщений",
            &report.processed.to_string(),
            Role::Metadata,
            palette,
        )?;
        field(
            &mut output,
            width,
            "Анкет",
            &report.profiles.len().to_string(),
            Role::Success,
            palette,
        )?;
    }
    if report.processed == 0 || report.profiles.is_empty() {
        field(
            &mut output,
            width,
            "Результат",
            if report.processed == 0 {
                "Источник вернул пустой тред."
            } else {
                "Самостоятельных анкет с собственным контактом не найдено."
            },
            Role::Warning,
            palette,
        )?;
    }
    for (index, profile) in report.profiles.iter().enumerate() {
        let message = &profile.message;
        let post = format!("Анкета {} · пост №{}", index + 1, message.origin.post);
        let date = message.date.as_deref().map(|s| cell_text(s, false));
        let link = cell_text(&message.origin.link, false);
        let contacts: Vec<_> = profile
            .contacts
            .iter()
            .map(|contact| {
                let kind = match contact.kind {
                    ContactKind::Telegram => "Telegram",
                    ContactKind::Discord => "Discord",
                    ContactKind::Email => "Почта",
                    ContactKind::Vk => "VK",
                    ContactKind::Unknown => "Платформа неизвестна",
                };
                (kind, cell_text(&contact.value, false))
            })
            .collect();
        let text = cell_text(&message.text, true);
        let text = match options.view {
            View::Full => text,
            View::Compact => preview(&text),
        };
        if width >= 76 {
            let mut grid = table(width);
            grid.set_header(vec![
                "Пост / дата",
                "Контакты",
                match options.view {
                    View::Full => "Текст",
                    View::Compact => "Превью",
                },
            ]);
            let metadata = format!(
                "{post}\n{link}{}",
                date.as_ref()
                    .map(|s| format!("\nДата: {s}"))
                    .unwrap_or_default()
            );
            grid.add_row(vec![
                metadata,
                contacts
                    .iter()
                    .map(|(kind, value)| format!("{kind}: {value}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                text,
            ]);
            grid.set_constraints([
                ColumnConstraint::Absolute(Width::Fixed(24)),
                ColumnConstraint::Absolute(Width::Fixed(24)),
            ]);
            write_table(
                &mut output,
                grid,
                &[Role::Post, Role::Contact, Role::Text],
                palette,
            )?;
        } else {
            field(
                &mut output,
                width,
                "Анкета / пост",
                &post,
                Role::Post,
                palette,
            )?;
            field(
                &mut output,
                width,
                "Оригинал",
                &link,
                Role::Contact,
                palette,
            )?;
            if let Some(date) = date {
                field(&mut output, width, "Дата", &date, Role::Metadata, palette)?;
            }
            for (kind, value) in &contacts {
                field(&mut output, width, kind, value, Role::Contact, palette)?;
            }
            field(
                &mut output,
                width,
                match options.view {
                    View::Full => "Текст",
                    View::Compact => "Превью",
                },
                &text,
                Role::Text,
                palette,
            )?;
        }
        // Вложения остаются полными даже в обзоре: ни ссылки, ни имена не теряются.
        for (number, attachment) in message.attachments.iter().enumerate() {
            let mut value = cell_text(&attachment.link, false);
            if let Some(name) = &attachment.name {
                value.push_str(&format!("\nИмя: {}", cell_text(name, false)));
            }
            if let Some(size) = attachment.size_kib {
                value.push_str(&format!("\nРазмер: {size} КиБ"));
            }
            field(
                &mut output,
                width,
                &format!("Вложение {}", number + 1),
                &value,
                Role::Attachment,
                palette,
            )?;
        }
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_paragraph_breaks_and_indentation_stay_in_the_cell() {
        let value = cell_text("\u{0338}Первый абзац\n\n\n  Второй абзац\n", true);
        let mut grid = table(40);
        grid.set_header(vec!["Текст"]).add_row(vec![value]);
        let mut bytes = vec![];
        write_table(&mut bytes, grid, &[Role::Text], Options::default().palette).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = text
            .lines()
            .filter(|s| s.starts_with("│") && !s.contains("Текст"))
            .collect();
        assert!(lines[0].starts_with("│› \u{0338}Первый абзац"));
        assert_eq!(lines[1].trim_matches(['│', ' ']), "");
        assert_eq!(lines[2].trim_matches(['│', ' ']), "");
        assert!(lines[3].starts_with("│›   Второй абзац"));
    }
}
