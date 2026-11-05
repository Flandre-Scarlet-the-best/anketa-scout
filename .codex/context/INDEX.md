# Карта владельцев контекста

После корневого [AGENTS.md](../../AGENTS.md) выбирать документы по области задачи. README — пользовательское описание; ни он, ни PR не подменяют постоянные рабочие контракты.

| Область | Канонический владелец | Когда читать |
| --- | --- | --- |
| Предметная модель, источники, границы доверия | [ARCHITECTURE.md](ARCHITECTURE.md) | Изменение поведения, данных, адаптеров, хранения, UI |
| Rust, переносимость, инструменты | [DEVELOPMENT.md](DEVELOPMENT.md) | Реализация и конфигурация |
| Проверки, доказательства, обязательные CI | [VERIFICATION.md](VERIFICATION.md) | Любое изменение перед передачей |
| SemVer, MSRV, Cargo/toolchain | [VERSIONING.md](VERSIONING.md) | Изменение поведения, зависимостей или версии |
| Публикация, PR, Ready/merge | [scout-git-workflow](../../.agents/skills/scout-git-workflow/SKILL.md) | Commit, push, публикация, сопровождение PR |
| Независимое ревью | [scout-code-review](../../.agents/skills/scout-code-review/SKILL.md) | Ревью PR/ветки/diff, регрессии |
| Явное CodeRabbit | [scout-coderabbit-review](../../.agents/skills/scout-coderabbit-review/SKILL.md) | Только явное поручение CodeRabbit |

## Границы ответственности

- `AGENTS.md` задаёт лишь инварианты и маршрутизацию; подробная процедура находится у skill и его `references/`.
- `VERIFICATION.md` выбирает применимые тесты. Skills ревью и Git ссылаются на него, не создают второй реестр команд.
- `scout-git-workflow` — единственный владелец изменяющей Git/GitHub-публикации. CodeRabbit и самостоятельное ревью ему не дублируют push/PR.
- Не хранить здесь историю итераций агента, временные SHA, приватные снимки, номера PR или дубли процедур.

## Обнаружение и явный вызов

Codex обнаруживает `.agents/skills/<name>/SKILL.md`; Claude Code — обычные файлы `.claude/skills/<name>/SKILL.md` с ссылкой на канонический документ. Это не символические ссылки и работает на Windows с `core.symlinks=false`. При изменении skill синхронизировать описание входа Claude. Проверять все относительные ссылки в CI.

CodeRabbit дополнительно имеет `allow_implicit_invocation: false` в `.agents/skills/scout-coderabbit-review/agents/openai.yaml` и `disable-model-invocation: true` во входе Claude. Для Codex явно вызвать `$scout-coderabbit-review`, для Claude — `/scout-coderabbit-review` в начале сообщения, вместе с положительным поручением и областью. Фраза «Запусти CodeRabbit» выражает намерение, но сама по себе не гарантирует подгрузку skill. Упоминание в вопросе или обычное ревью не активируют его.

Новые skills проверить в новой сессии через `/skills` или `$` в Codex и меню `/` в Claude. Наличие в списке не доказывает фактического исполнения. Официальные источники: [Codex Skills](https://learn.chatgpt.com/docs/build-skills), [Claude Code Skills](https://code.claude.com/docs/en/skills).

## Происхождение

Методология разделения владельцев адаптирована из [anki-deck-toolkit](https://github.com/Flandre-Scarlet-the-best/anki-deck-toolkit). Anki-специфичные данные и процессы не переносятся автоматически. Для Scout сохранены применимые инварианты и отдельные сценарии.

