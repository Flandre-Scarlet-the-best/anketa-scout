# Карта владельцев контекста

После корневого [AGENTS.md](../../AGENTS.md) выбрать документы по области задачи.
README описывает назначение и текущее состояние, но не дублирует правила разработки.

| Область | Канонический владелец | Когда читать |
| --- | --- | --- |
| Предметная модель, зависимости, границы доверия и данных | [ARCHITECTURE.md](ARCHITECTURE.md) | Изменение поведения, адаптеров, хранения или UI |
| Rust, переносимость разработки и конфигурации | [DEVELOPMENT.md](DEVELOPMENT.md) | Реализация и изменение инструментов |
| Выбор проверок и доказательства | [VERIFICATION.md](VERIFICATION.md) | Любое изменение перед передачей |
| SemVer, MSRV и источники версий | [VERSIONING.md](VERSIONING.md) | Изменение поведения, зависимостей или toolchain |
| Ветка, commit, push, Draft PR и передача | [scout-git-workflow](../../.agents/skills/scout-git-workflow/SKILL.md) | Публикуемая задача, «закоммить», «запушь», «создай PR» |
| Независимое ревью | [scout-code-review](../../.agents/skills/scout-code-review/SKILL.md) | «Проверь diff», «ревью PR», «найди регрессии» |
| CodeRabbit | [scout-coderabbit-review](../../.agents/skills/scout-coderabbit-review/SKILL.md) | Только явное поручение запустить CodeRabbit |

Обычное ревью не означает запуск внешнего сервиса. Проверочная политика принадлежит
VERIFICATION.md; отдельного дублирующего skill для неё нет.

## Обнаружение skills

Codex обнаруживает `.agents/skills/<name>/SKILL.md`. Claude Code обнаруживает
`.claude/skills/<name>/SKILL.md`; эти обычные файлы содержат только описание входа
и ссылку на канонический skill. Claude должен прочитать целевой файл перед работой.
Пути в ссылках разрешаются относительно файла, который их содержит.
Это не копии процедур и не symlink: clone работает при `core.symlinks=false`
на Windows без Developer Mode. Изменять процедуры нужно только в `.agents/skills/`.
Описания входов поддерживать согласованными с назначением владельца.

CodeRabbit у Codex дополнительно имеет `allow_implicit_invocation: false`,
у Claude — `disable-model-invocation: true`. Это настройки вызова, а не разрешение
на внешние действия. Для проверки обнаружения в новой сессии открыть список skills
Codex и меню `/` Claude Code; проверить наличие трёх `scout-*` и переход к владельцу.

## Происхождение организации

У [anki-deck-toolkit](https://github.com/Flandre-Scarlet-the-best/anki-deck-toolkit)
адаптированы карта владельцев, разделение Git / независимого ревью / CodeRabbit,
проверка фактического HEAD и публикации. Предметные Anki-правила, программы,
CI и Linux-процессы не переносились. Референс использует symlink для Claude;
здесь выбран описанный выше переносимый вход. Настройки пользовательских агентов
референсу не приписываются.

Первичные справочники: [skills Codex](https://learn.chatgpt.com/docs/build-skills),
[skills Claude Code](https://code.claude.com/docs/en/skills).
