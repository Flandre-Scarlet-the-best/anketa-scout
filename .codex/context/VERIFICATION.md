# Политика проверки

Единственный владелец выбора проверок. Набор соразмерен изменению; успешная команда
не заменяет чтение результата и итогового diff.

## Инфраструктура и данные

- Перечитать изменённые файлы и реальный diff, включая новые файлы; выполнить
  `git diff --check` и затем `git diff --cached --check` для подготовленного индекса.
- Разобрать JSON, YAML, TOML; Renovate проверять актуальным официальным
  `renovate-config-validator --strict renovate.json`, CodeRabbit — по
  [официальной JSON Schema](https://www.coderabbit.ai/integrations/schema.v2.json).
  Валидатор запускать доступным инструментом без добавления проектной автоматизации.
  Различать ошибку схемы, сетевой сбой и отсутствие инструмента.
- При изменении Renovate проверить не только схему, но и фактически применяемые наборы настроек,
  группировку major вместе с остальными, исключения и извлечение закреплённого Rust.
  Не запускать настоящий бот с записью в GitHub для такой проверки.
- Проверить относительные Markdown-ссылки и импорты, фронтматтер навыков, наличие
  канонических целей входов Claude и политику только явного вызова CodeRabbit.
- Проверить UTF-8/LF, неизменность лицензии, исключения `.gitignore` через
  `git check-ignore`, включая локальные данные, и доступность Cargo.lock для Git.
- Не включать shell-скрипты, приватные данные или временные отчёты в изменения.
  Временные инструменты и доказательства хранить вне отслеживаемых файлов.
- Отслеживаемый скрипт не на Rust (shell, PowerShell, Python и т. п., включая
  многострочные `run:` в workflow) добавлять только вместе с его проверкой:
  тестами, штатным статическим анализатором языка (для shell — shellcheck,
  для workflow — actionlint) или исполнением в CI. Скрипт, который нигде не
  проверяется, в изменения не включать. Workflow `.github/workflows/scripts.yml`
  запускает закреплённые actionlint и shellcheck (shellcheck — для отслеживаемых
  shell-скриптов и, через actionlint, для `run:` на bash/sh). Отклоняются, пока
  для языка не добавлена проверка в этот workflow: файлы с расширениями других
  языков сценариев и с не-shell shebang, шаги с `shell:` кроме bash/sh и
  `actions/github-script`. Версии, SHA-256 и размеры инструментов обновлять вместе.

## Агентские контракты и CI

Канонический валидатор: `.github/tools/agent_contracts.rs` (Rust без внешних библиотек).
Он проверяет наличие skills, соответствие frontmatter и имён, входы Claude,
запрет неявного запуска CodeRabbit и относительные Markdown-ссылки.
Отдельный режим проверяет структуру опубликованного PR body, предел 10 000 символов,
формат раздела «Производительность» и таблицу CodeRabbit по
[pr-body.md](../../.agents/skills/scout-git-workflow/references/pr-body.md).
PR dependency-ботов в их собственных ветках от формата ручных PR освобождены.
Команды из корня репозитория:
- `rustc --edition=2021 --test .github/tools/agent_contracts.rs -o <временный бинарник>`, затем выполнить тесты;
- `rustc --edition=2021 .github/tools/agent_contracts.rs -o <временный бинарник>`;
- `<временный бинарник> skills .`;
- `<временный бинарник> pr-body <UTF-8-файл> [--author <логин> --head-ref <ветка>]`.

`Агентские контракты (preview)` и `Структура PR (preview)` запускаются на PR,
но компилируют валидатор из head-кода. Это диагностические проверки; не назначать
их обязательными status checks. Доверенные `Доверенные агентские контракты` и
`Доверенная структура PR` получают валидатор только из base SHA, а файлы head
проверяют как данные, без их выполнения. Агентский trusted workflow проверяет
skills на `pull_request_target` и `main`; trusted PR-body workflow проверяет body
на `pull_request_target`.

При недоступном `pull_request_target` отсутствие доверенной проверки не считать
успехом. Событие надо отдельно разрешить в GitHub Actions policy, если применимы
ограничения платформы; не отключать защиту наугад.

**Действующий ruleset необходимо проверять через GitHub API, а не по этому файлу.**
После попадания trusted workflows в main настроить строгие required checks
`Доверенные агентские контракты` и `Доверенная структура PR`, предварительно
подтвердив применимость политики `pull_request_target`. «Строгие» означает
`strict_required_status_checks_policy: true` (ветка PR обязана быть актуальной
относительно base): иначе после изменения `main` неизменный HEAD PR сохранит
результат, полученный по устаревшему доверенному валидатору. Preview-контексты
`Агентские контракты (preview)` и `Структура PR (preview)` не делать обязательными.
Для приложения дополнительно требовать реально существующие Linux/Windows/macOS
jobs, не добавляя ещё не существующие контексты: иначе merge может быть заблокирован
навсегда. Отклонение настроек ruleset от желаемой политики — блокер настройки,
а не доказательство применения этого раздела.

См. [GitHub: rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/available-rules-for-rulesets),
[безопасный pull_request_target](https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target).

## Приложение

Команды и область тестов брать из фактических манифестов Cargo, документации и CI.
Для текущего пакета обязательный набор при изменении поведения:
`cargo fmt --all -- --check`,
`cargo clippy --locked --all-targets --all-features -- -D warnings`,
`cargo test --locked --all-features`, `cargo build --locked --release`.
При локальном исправлении начинать с затронутого integration test;
при новых рисках расширять набор.
Регрессии защищать наблюдаемым поведением, включая ошибки, неизвестные поля,
чужие цитаты контактов, безопасный вывод, частичный сбой и отмену по области задачи.
Использовать синтетические данные без настоящих контактов и сетевых зависимостей
по умолчанию. Проверка внешнего источника в реальной среде — отдельное доказательство,
она не подменяет воспроизводимые проверки и не публикует ответ источника.

GitHub Actions проверяет Linux полным набором, Windows нативной сборкой и всем
набором тестов, macOS — сборкой и базовым CLI/контрактами версий.
Все платформы запускают `--help` и `--version`; команды Cargo выбирают закреплённый
Rust через штатный rustup на runner. Linux-прогон не доказывает нативную Windows
или macOS-приёмку; недоступные платформы прямо указывать.

При изменении адаптера отдельно запускать настоящий CLI с настроенным источником.
Для доказательства сохранять только код завершения и сводку, без контактов и постов.
Не зависеть от живого источника в обычном наборе тестов и не обходить ограничения.

## Поведенческие доказательства

Тесты в `tests/` проверяют публичное поведение. Модульные тесты расположены рядом
с кодом: транспорт — `src/transport.rs` с локальным TCP-сервером, таблицы —
`src/output.rs`, коды вывода — `src/main.rs`, спиннер — `src/progress.rs`, уровни
диагностики — `src/diagnostics.rs`.

| Требование | Проверки |
| --- | --- |
| Полная, короткая анкета, естественная цель, несколько контактов | `full_profile_preserves_own_contact_and_original`, `short_profile_needs_no_demographics_or_at_sign`, `natural_goal_without_search_keyword_is_sufficient`, `multiple_contacts_have_types_and_evidence` |
| Отсутствие одного из двух признаков, контакт-дополнение | `description_without_contact_is_rejected`, `contact_only_and_invitation_are_not_profiles`, `forgotten_contact_is_not_combined_with_previous_description`, `one_post_is_one_profile_and_contacts_are_never_merged` |
| Цитаты, локальная чужая принадлежность, вступление | `quoted_contacts_and_descriptions_do_not_count_as_authored`, `unrelated_words_do_not_decide_classification`, `introductory_post_is_never_a_profile`, `ambiguous_mentions_are_not_owned_contacts` |
| Формат источника, происхождение, даты, вложения, `null` | `confirmed_json_shape_preserves_origin_date_files_and_raw_body`, `confirmed_null_files_means_no_attachments`, `missing_optional_fields_remain_absent` |
| Неверный и частичный тред, повтор номера, чужой parent | `invalid_json_and_missing_required_fields_fail_closed`, `partial_count_or_missing_op_is_not_complete`, `wrong_identity_duplicates_and_wrong_parent_are_rejected` |
| Конфиг: создание, повтор, гонка, явный путь, отсутствие файла | `first_run_creates_default_and_second_run_never_overwrites`, `concurrent_first_runs_read_complete_configuration`, `explicit_path_wins_and_does_not_create_default`, `missing_explicit_path_neither_creates_nor_falls_back` |
| Конфиг: ошибки TOML, источники, размер и I/O | `invalid_toml_unknown_fields_and_wrong_types_are_errors_without_content_leaks`, `empty_multiple_and_unsupported_sources_are_rejected_before_fetch`, `config_size_boundary_is_enforced_without_overwriting`, `creation_failure_and_unreadable_path_are_specific_config_errors` |
| CLI, диагностика и коды | `help_and_version_need_no_config_or_network`, `invalid_arguments_have_russian_safe_diagnostics_and_code_two`, `explicit_missing_or_invalid_config_fails_before_network_with_empty_stdout`, `error_categories_have_stable_distinct_exit_codes_without_remote_data` |
| Безопасный вывод и различие пустого результата/ошибок | `plain_redirected_output_has_contacts_origin_text_date_and_attachments`, `all_terminal_controls_and_bidi_overrides_are_removed`, `empty_thread_and_no_profiles_have_different_summaries`, `failures_propagate_without_becoming_zero_profiles` |
| HTTP: статусы, запрет перенаправлений, лимит, обрыв, таймаут | `status_errors_and_redirects_keep_their_categories`, `declared_and_streamed_body_size_limits_are_enforced`, `truncated_http_body_is_network_failure`, `timeout_is_bounded_and_is_not_empty_success` |
| URL, вложения, разметка и ограничения | `only_supported_public_thread_urls_are_accepted`, `attachment_urls_cannot_change_host_or_escape_board`, `comment_size_boundary_and_deep_markup_are_bounded` |
| Контактные ссылки и ресурсные псевдоконтакты | `labelled_telegram_url_is_not_truncated_to_https_as_username`, `resource_links_bots_and_invalid_contact_suffixes_are_rejected` |
| Цитаты, апострофы, платформа контакта, скрытые HTTP-ссылки | `single_quoted_contacts_and_descriptions_are_not_author_evidence`, `bare_handle_does_not_establish_contact_platform`, `hidden_http_telegram_link_remains_a_contact` |
| Отмена и закрытие активного соединения | `pre_cancelled_run_never_starts_the_source`, `cancellation_drops_pending_source_instead_of_returning_partial_report`, `cancellation_stops_in_flight_request_and_closes_socket` |
| FIFO, подделка служебных полей и пустой ответ без OP | `fifo_config_without_writer_is_rejected_without_blocking` (Unix), `remote_text_cannot_forge_report_headers_or_metadata_fields`, `missing_op_is_partial_even_when_count_is_zero` |
| Невозможность внешней подмены авторского текста | compile-fail doctest `Message::authored_text` |
| Полные таблицы, порядок, все контакты, URL и вложения | `full_tables_preserve_all_fields_and_order` |
| Ширина 8/24/40/75/76/80/96/120, неизвестная и минимальная геометрия, Unicode и абзацы | `unicode_tables_fit_width_and_preserve_paragraphs`, `repeated_paragraph_breaks_and_indentation_stay_in_the_cell` |
| Явный compact с ограниченным превью без разрыва графем | `compact_is_explicit_and_preview_is_bounded` |
| Изоляция ячеек при ANSI/bidi, поддельных полях и рамке | `untrusted_cells_cannot_forge_structure`, `remote_text_cannot_forge_report_headers_or_metadata_fields` |
| Цветовые приоритеты, две темы и RGB/256/16 | `color_policy_respects_explicit_choice_and_stream`, `themes_and_depths_style_real_output`, `presentation_flags_are_validated_and_documented` |
| Спиннер: кадры, очистка после успеха/ошибки/отмены, скрытые потоки | `spinner_animates_real_operation_and_clears_on_success_and_error`, `cancelled_request_drops_spinner_before_final_diagnostic`, `hidden_streams_have_no_animation_or_control_output` (управляемый `TermLike`, без сети) |
| BrokenPipe и ошибки записи/flush | `broken_pipe_is_success_and_other_output_failures_keep_code_eight`, `output_write_failure_is_not_reported_as_success`, `final_flush_failure_is_not_lost` |
| Согласованность Rust/MSRV и извлечение Renovate | `cargo_version_and_toolchain_are_canonical_and_synchronized`, `renovate_extracts_only_canonical_msrv_from_actual_manifest` |

## Отчёт

Для каждой существенной проверки указать объект/ревизию, команду или метод,
фактический результат и предел доказательства. После новых правок повторить
затронутые проверки. Не объявлять незапущенное успешным, отсутствие CI — зелёным CI,
а собственную перечитку — независимым ревью. Публикацию проверяет владелец процедуры Git/GitHub.
Контакты и посты не входят в диагностические доказательства.

Контактный контракт: canonical_contacts_are_all_preserved_and_deduplicated,
local_restoration_operations_precede_validation,
geography_and_unsupported_instructions_do_not_invent_contacts,
colloquial_discord_labels_are_not_geography.
Явно включаемый offline_snapshot_classification использует только локальный
SCOUT_AUDIT_JSON; для сравнения до/после обязательно сверять SHA-256 снимка.
Его вывод содержит номера постов и категории, без контактов и тел сообщений.
Уровни и отсутствие утечек проверяет logging_levels_and_streams_never_expose_remote_content
в отдельных процессах; logging_probe вызывается этим тестом автоматически.
Пограничные формы снимка защищены snapshot_restoration_shapes_keep_unknown_platform_explicit,
literal_instructions_do_not_truncate_invalid_identifiers,
adjacent_contacts_keep_platform_and_independent_values,
contact_anchor_caption_does_not_supply_independent_text и multiline_quotes_cannot_become_owned_contacts.
