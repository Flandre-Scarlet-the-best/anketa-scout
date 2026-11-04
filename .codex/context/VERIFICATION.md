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

Тесты в `tests/` проверяют публичное поведение; транспортные проверки находятся
в `src/transport.rs` и используют локальный TCP-сервер.

| Требование | Проверки |
| --- | --- |
| Полная, короткая анкета, естественная цель, несколько контактов | `full_profile_preserves_own_contact_and_original`, `short_profile_needs_no_demographics_or_at_sign`, `natural_goal_without_search_keyword_is_sufficient`, `multiple_contacts_have_types_and_evidence` |
| Отсутствие одного из двух признаков, контакт-дополнение | `description_without_contact_is_rejected`, `contact_only_and_invitation_are_not_profiles`, `forgotten_contact_is_not_combined_with_previous_description`, `one_post_is_one_profile_and_contacts_are_never_merged` |
| Цитаты, шаблоны, реклама, вступление, чужая принадлежность | `quoted_contacts_and_descriptions_do_not_count_as_authored`, `templates_advertising_and_reposts_are_rejected`, `introductory_post_is_never_a_profile`, `ambiguous_mentions_are_not_owned_contacts` |
| Формат источника, происхождение, даты, вложения, `null` | `confirmed_json_shape_preserves_origin_date_files_and_raw_body`, `confirmed_null_files_means_no_attachments`, `empty_thread_and_missing_optional_fields_remain_distinct` |
| Неверный и частичный тред, повтор номера, чужой parent | `invalid_json_and_missing_required_fields_fail_closed`, `partial_count_or_missing_op_is_not_complete`, `wrong_identity_duplicates_and_wrong_parent_are_rejected` |
| Конфиг: создание, повтор, гонка, явный путь, отсутствие файла | `first_run_creates_default_and_second_run_never_overwrites`, `concurrent_first_runs_read_complete_configuration`, `explicit_path_wins_and_does_not_create_default`, `missing_explicit_path_neither_creates_nor_falls_back` |
| Конфиг: ошибки TOML, источники, размер и I/O | `invalid_toml_unknown_fields_and_wrong_types_are_errors_without_content_leaks`, `empty_multiple_and_unsupported_sources_are_rejected_before_fetch`, `config_size_boundary_is_enforced_without_overwriting`, `creation_failure_and_unreadable_path_are_specific_config_errors` |
| CLI, диагностика и коды | `help_and_version_need_no_config_or_network`, `invalid_arguments_have_russian_safe_diagnostics_and_code_two`, `explicit_missing_or_invalid_config_fails_before_network_with_empty_stdout`, `error_categories_have_stable_distinct_exit_codes_without_remote_data` |
| Безопасный вывод и различие пустого результата/ошибок | `plain_redirected_output_has_contacts_origin_text_date_and_attachments`, `all_terminal_controls_and_bidi_overrides_are_removed`, `empty_thread_and_no_profiles_have_different_summaries`, `failures_propagate_without_becoming_zero_profiles` |
| HTTP: статусы, запрет перенаправлений, лимит, обрыв, таймаут | `status_errors_and_redirects_keep_their_categories`, `declared_and_streamed_body_size_limits_are_enforced`, `truncated_http_body_is_network_failure`, `timeout_is_bounded_and_is_not_empty_success` |
| URL, вложения, разметка и ограничения | `only_supported_public_thread_urls_are_accepted`, `attachment_urls_cannot_change_host_or_escape_board`, `comment_size_boundary_and_deep_markup_are_bounded` |
| Контактные ссылки и ресурсные псевдоконтакты | `labelled_telegram_url_is_not_truncated_to_https_as_username`, `resource_links_bots_and_invalid_contact_suffixes_are_rejected` |
| Согласованность Rust/MSRV и извлечение Renovate | `cargo_version_and_toolchain_are_canonical_and_synchronized`, `renovate_extracts_only_canonical_msrv_from_actual_manifest` |

## Отчёт

Для каждой существенной проверки указать объект/ревизию, команду или метод,
фактический результат и предел доказательства. После новых правок повторить
затронутые проверки. Не объявлять незапущенное успешным, отсутствие CI — зелёным CI,
а собственную перечитку — независимым ревью. Публикацию проверяет владелец процедуры Git/GitHub.
