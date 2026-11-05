mod common;
use anketa_scout::{
    application::Report,
    domain::{Attachment, classify},
    output::{Options, View, render_with_options, style::*},
};
use common::message;

fn fixture() -> Report {
    let mut first = message(
        "Люблю го и 棋. е\u{301} 👩‍💻 🇯🇵<br><br>Второй абзац.<br>Тг: synthetic_user<br>Discord: synthetic_name<br>Почта: synthetic@example.invalid",
    );
    first.date = Some("1970-01-01 00:00:00 UTC".into());
    first.attachments = vec![
        Attachment {
            name: Some("棋 схема.png".into()),
            link: "https://2ch.org/test/src/100/one.png".into(),
            size_kib: Some(7),
        },
        Attachment {
            name: None,
            link: "https://2ch.org/test/src/100/two.png".into(),
            size_kib: None,
        },
    ];
    let mut second = message("Люблю шахматы. Тг: second_synthetic");
    second.origin.post = 102;
    second.origin.link = "https://2ch.org/test/res/100.html#102".into();
    Report {
        source: common::source().page(),
        processed: 3,
        profiles: vec![classify(first).unwrap(), classify(second).unwrap()],
    }
}

fn draw(report: &Report, options: Options) -> String {
    let mut bytes = vec![];
    render_with_options(report, &mut bytes, options).unwrap();
    String::from_utf8(bytes).unwrap()
}

// Восстанавливаем каждую ячейку отдельно: перенос не должен скрывать потерю URL,
// соседняя колонка не должна случайно дополнять отсутствующее содержимое.
fn cells(text: &str) -> Vec<String> {
    labelled_cells(text)
        .into_iter()
        .map(|(_, cell)| cell)
        .collect()
}

fn labelled_cells(text: &str) -> Vec<(String, String)> {
    let mut result = vec![];
    let mut columns: Vec<String> = vec![];
    let mut headers: Vec<String> = vec![];
    let mut data = false;
    for line in text.lines() {
        if line.starts_with('╞') {
            data = true;
        } else if line.starts_with('╰') {
            result.extend(headers.drain(..).zip(columns.drain(..)));
            data = false;
        } else if line.starts_with('│') {
            let parts: Vec<_> = line.split('│').skip(1).filter(|s| !s.is_empty()).collect();
            let target = if data { &mut columns } else { &mut headers };
            if target.is_empty() {
                target.resize(parts.len(), String::new());
            }
            assert_eq!(parts.len(), target.len());
            for (cell, part) in target.iter_mut().zip(parts) {
                if data {
                    if part.trim().is_empty() {
                        assert!(!part.contains('›'));
                    } else {
                        assert!(part.starts_with("› "), "Нет изолированного маркера ячейки");
                        cell.push_str(part.strip_prefix("› ").unwrap().trim_end());
                    }
                    cell.push('\n');
                } else {
                    cell.push_str(part.trim());
                }
            }
        }
    }
    result
}

fn joined(text: &str) -> String {
    text.split_whitespace().collect()
}

#[test]
fn full_tables_preserve_all_fields_and_order() {
    let report = fixture();
    assert_eq!(report.profiles[0].contacts.len(), 3);
    for width in [Some(8), Some(40), Some(80), Some(120), None] {
        let text = draw(
            &report,
            Options {
                width,
                ..Options::default()
            },
        );
        let cells = cells(&text);
        let labelled = labelled_cells(&text);
        let contains = |expected: &str| {
            cells
                .iter()
                .any(|cell| joined(cell).contains(&joined(expected)))
        };
        for profile in &report.profiles {
            let m = &profile.message;
            assert!(contains(&format!("пост №{}", m.origin.post)));
            assert!(contains(&m.origin.link));
            assert!(contains(m.text()));
            if let Some(date) = &m.date {
                assert!(contains(date));
            }
            for c in &profile.contacts {
                let kind = match c.kind {
                    anketa_scout::domain::ContactKind::Telegram => "Telegram",
                    anketa_scout::domain::ContactKind::Discord => "Discord",
                    anketa_scout::domain::ContactKind::Email => "Почта",
                    anketa_scout::domain::ContactKind::Vk => "VK",
                    anketa_scout::domain::ContactKind::Unknown => "Платформа неизвестна",
                };
                // Контакт должен быть в собственной колонке/поле с типом;
                // его присутствие только в исходном тексте недостаточно.
                assert!(labelled.iter().any(|(header, cell)| {
                    if header == "Контакты" {
                        joined(cell).contains(&joined(&format!("{kind}: {}", c.value)))
                    } else {
                        header == kind && joined(cell).contains(&joined(&c.value))
                    }
                }));
            }
            for a in &m.attachments {
                assert!(contains(&a.link));
                if let Some(name) = &a.name {
                    assert!(contains(name));
                }
                if let Some(size) = a.size_kib {
                    assert!(contains(&format!("{size} КиБ")));
                }
            }
        }
        let first = cells
            .iter()
            .position(|c| joined(c).contains("пост№101"))
            .unwrap();
        let second = cells
            .iter()
            .position(|c| joined(c).contains("пост№102"))
            .unwrap();
        assert!(first < second);
        assert!(!text.contains('\u{1b}'));
    }
}

#[test]
fn unicode_tables_fit_width_and_preserve_paragraphs() {
    let report = fixture();
    for width in [8, 9, 20, 24, 39, 40, 75, 76, 80, 96, 120] {
        let text = draw(
            &report,
            Options {
                width: Some(width),
                ..Options::default()
            },
        );
        for line in text.lines() {
            assert_eq!(
                console::measure_text_width(line),
                usize::from(width),
                "Ширина {width}"
            );
        }
        assert!(text.contains("👩‍💻"));
        assert!(text.contains("е\u{301}"));
        assert!(text.contains("🇯🇵"));
        let body = cells(&text)
            .into_iter()
            .find(|c| c.contains("Второй") || joined(c).contains("Второйабзац"))
            .unwrap();
        assert!(
            body.contains("🇯🇵\n"),
            "Потерян исходный перенос при ширине {width}"
        );
    }
    for width in [0, 1, 2, 7] {
        assert_eq!(
            draw(
                &report,
                Options {
                    width: Some(width),
                    ..Options::default()
                }
            ),
            draw(
                &report,
                Options {
                    width: Some(8),
                    ..Options::default()
                }
            )
        );
    }
    let fallback = draw(&report, Options::default());
    assert!(
        fallback
            .lines()
            .all(|s| console::measure_text_width(s) == 96)
    );
}

#[test]
fn compact_is_explicit_and_preview_is_bounded() {
    let long = format!(
        "Люблю го. {}👩‍💻{} КОНЕЦ_ТЕКСТА<br>Тг: synthetic_user",
        "a".repeat(88),
        "棋".repeat(80)
    );
    let mut report = fixture();
    report.profiles[0] = classify(message(&long)).unwrap();
    let full = draw(&report, Options::default());
    let compact = draw(
        &report,
        Options {
            view: View::Compact,
            ..Options::default()
        },
    );
    assert!(compact.contains("Компактный обзор"));
    assert!(compact.contains("--view full"));
    assert!(compact.contains("Превью"));
    assert!(!compact.contains("КОНЕЦ_ТЕКСТА"));
    assert!(full.contains("КОНЕЦ_ТЕКСТА"));
    assert!(compact.contains('…'));
    let data = cells(&compact);
    let preview = data.iter().find(|c| c.starts_with("Люблю го.")).unwrap();
    assert!(console::measure_text_width(&preview.replace('\n', "")) <= 100);
    assert!(!preview.replace("👩‍💻", "").contains(['👩', '\u{200d}', '💻']));
    assert!(
        data.iter()
            .any(|c| joined(c).contains("Telegram:@synthetic_user"))
    );
}

#[test]
fn untrusted_cells_cannot_forge_structure() {
    let attack = "Люблю го.<br>Тг: synthetic_user<br>│ ╭╮╰╯ ┬ ┼ | ›<br>Дата: подделка<br>Anketa Scout<br><br>Telegram: подделка\u{1b}]8;;https://evil.invalid\u{07}\u{1b}[2J\u{202e}\u{2066}";
    let mut report = fixture();
    report.profiles = vec![classify(message(attack)).unwrap()];
    report.source = "Источник\n│подделка".into();
    let m = &mut report.profiles[0].message;
    m.date = Some("Дата\n│подделка".into());
    m.origin.link = format!("https://2ch.org/{}", "x".repeat(200));
    m.attachments.push(Attachment {
        name: Some("╞\nДата: файл".into()),
        link: "https://2ch.org/│\u{1b}[0m".into(),
        size_kib: Some(1),
    });
    for width in [40, 80, 120] {
        let text = draw(
            &report,
            Options {
                width: Some(width),
                ..Options::default()
            },
        );
        assert!(
            !text
                .chars()
                .any(|c| (c.is_control() && c != '\n') || matches!(c, '\u{202e}' | '\u{2066}'))
        );
        let data = cells(&text);
        assert!(data.iter().any(|c| joined(c).contains("Дата:подделка")));
        assert!(data.iter().any(|c| joined(c).contains("\\u{2502}")));
        assert!(
            data.iter()
                .any(|c| joined(c).contains(&report.profiles[0].message.origin.link))
        );
        // Во всех строках ровно столько разделителей, сколько задал доверенный заголовок.
        let mut separators = 0;
        for line in text.lines() {
            assert_eq!(console::measure_text_width(line), usize::from(width));
            if line.starts_with('╭') {
                separators = line.matches('┬').count() + 2;
            }
            if line.starts_with('│') {
                assert_eq!(line.matches('│').count(), separators);
            }
        }
    }
}

#[test]
fn color_policy_respects_explicit_choice_and_stream() {
    let tty = Terminal {
        attended: true,
        ansi: true,
        no_color: false,
        depth: ColorDepth::TrueColor,
        width: Some(80),
    };
    for (mode, disabled, terminal, expected) in [
        (ColorMode::Auto, false, tty, ColorDepth::TrueColor),
        (
            ColorMode::Auto,
            false,
            Terminal {
                attended: false,
                ..tty
            },
            ColorDepth::None,
        ),
        (
            ColorMode::Auto,
            false,
            Terminal {
                no_color: true,
                ..tty
            },
            ColorDepth::None,
        ),
        (
            ColorMode::Auto,
            false,
            Terminal { ansi: false, ..tty },
            ColorDepth::None,
        ),
        (
            ColorMode::Always,
            false,
            Terminal {
                no_color: true,
                attended: false,
                ..tty
            },
            ColorDepth::TrueColor,
        ),
        (ColorMode::Never, false, tty, ColorDepth::None),
        (ColorMode::Always, true, tty, ColorDepth::None),
    ] {
        let palette = Palette::resolve(mode, disabled, Theme::Dark, terminal);
        assert_eq!(palette.depth, expected);
        let text = draw(
            &fixture(),
            Options {
                palette,
                ..Options::default()
            },
        );
        assert_eq!(text.contains('\u{1b}'), expected != ColorDepth::None);
    }
}

#[test]
fn summary_counts_and_empty_states_do_not_invent_profiles() {
    let mut report = fixture();
    let text = draw(&report, Options::default());
    let data = cells(&text);
    assert_eq!(&data[2..4], &["3\n", "2\n"]);
    report.profiles.clear();
    for (processed, status) in [(0, "пустой тред"), (3, "не найдено")] {
        report.processed = processed;
        let text = draw(&report, Options::default());
        let data = cells(&text);
        assert_eq!(data[2], format!("{processed}\n"));
        assert_eq!(data[3], "0\n");
        assert!(text.contains(status));
        assert!(!text.contains("пост №"));
        assert!(!text.contains("Контакты"));
        assert!(!text.contains("Вложение"));
    }
}

#[test]
fn themes_and_depths_style_real_output() {
    let report = fixture();
    let plain = draw(&report, Options::default());
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
    ] {
        let dark = draw(
            &report,
            Options {
                palette: Palette {
                    theme: Theme::Dark,
                    depth,
                },
                ..Options::default()
            },
        );
        let light = draw(
            &report,
            Options {
                palette: Palette {
                    theme: Theme::Light,
                    depth,
                },
                ..Options::default()
            },
        );
        assert_ne!(dark, light);
        for colored in [&dark, &light] {
            assert_eq!(console::strip_ansi_codes(colored), plain);
            assert!(colored.contains("\u{1b}[1m"));
            assert!(!colored.contains("]8;"));
        }
        match depth {
            ColorDepth::TrueColor => {
                assert!(dark.contains("38;2;196;167;255m"));
                assert!(light.contains("38;2;88;40;150m"));
            }
            ColorDepth::Ansi256 => {
                assert!(dark.contains("38;5;183m"));
                assert!(!dark.contains("38;2;"));
            }
            ColorDepth::Ansi16 => {
                assert!(!dark.contains("38;"));
                assert!(dark.contains("\u{1b}[2m"));
            }
            ColorDepth::None => unreachable!(),
        }
    }
}
