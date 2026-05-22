# boyko-engine

Rust ECS-движок для игр с фокусом на **ультимативную производительность, кеш-локальность и нативный параллелизм**. Без компромиссов в пользу удобства против скорости.

## Структура

Workspace из крейтов:

- [`crates/boyko_ecs/`](crates/boyko_ecs/) — ядро ECS (память, компоненты, сущности; на ветке `ecs` ещё архетипы, queries, events)
- [`crates/boyko_macros/`](crates/boyko_macros/) — proc-macros (`#[derive(Component)]`)
- `crates/boyko_utils/` — битмаски и битсеты (есть только на ветке `ecs`)
- [`src/main.rs`](src/main.rs) — исполняемая обёртка (сейчас пустая; проект библиотечный)

## Принципы (НЕРУШИМЫЕ)

1. **Zero runtime overhead** — никаких `dyn Trait`/`Box`/`HashMap`/`Vec::new()` в hot path без обоснования
2. **Data-Oriented Design** — Struct of Arrays, hot/cold split полей
3. **Cache optimization (D-cache + I-cache)** — оба уровня кэша одинаково важны:
   - **D-cache (данные)**: `#[repr(C)]` где layout важен, alignment по cache line (64 B), SoA + hot/cold split полей, padding против false sharing, sequential access patterns, software prefetching для предсказуемых паттернов, non-temporal stores для streaming-записей. Working set hot loop'ов держим в пределах L1d (~32 KB) / L2 (~256-512 KB) где это критично.
   - **I-cache (инструкции)**: hot path компактный, никакого blind `#[inline(always)]` (см. принцип 7), `#[cold]` / `#[inline(never)]` для error paths и редких веток, контролируемая branch density, минимизированный размер hot loop. PGO (`-Cprofile-use=...`) применяется когда есть профиль исполнения.
4. **Lock-free параллелизм** — без `Mutex`/`RwLock`/`RefCell` в hot path
5. **Минимум аллокаций** — preallocate в setup, reuse во время игры
6. **SIMD-friendly layout** — данные готовы к векторизации
7. **Measured inlining** — `#[inline]` для cross-crate тривиальных функций и generic-методов (иначе тело недоступно для LTO). `#[inline(always)]` ТОЛЬКО когда профайлер/ассемблер показал, что компилятор не инлайнит сам и это критично. `#[cold]` / `#[inline(never)]` для error paths и редких веток. Чрезмерный inlining раздувает L1i cache и **снижает** perf — решения должны опираться на измерения, не на доктрину.
8. **Unsafe оправдан** — но **каждый** `unsafe` блок имеет `// SAFETY:` коммент с инвариантами

## Команды сборки

```powershell
cargo check --all-targets                          # быстрая проверка типов
cargo build --release                              # релизная сборка
cargo clippy --all-targets -- -D warnings          # линтер
cargo test --all-targets                           # тесты
cargo bench                                        # бенчмарки
cargo +nightly miri test                           # UB-детектор (если nightly)
```

## Целевая платформа

- ОС: Windows / Linux (x86_64)
- SIMD: AVX2 baseline; AVX-512 опционально через `cfg(target_feature)`
- Edition: Rust 2024

## Текущее состояние веток

| Ветка | Что есть |
|-------|----------|
| `master` | Только подсистема памяти: `Arena`, `ComponentPool`, `Chunk`, `MemFreeBlockMaster`, базовые типы `Entity`/`Component` |
| `ecs` | Полная архитектура: `Archetype`, `ArchetypeMaster`, `EcsMaster`, `Query`, `Event`, `BitSet`. **Не смержена** |

Если задача касается ECS-уровня (queries, archetypes, scheduler) — изучай ветку `ecs` через `git show origin/ecs:путь/к/файлу.rs`.

## Документация — два слоя

### Internal (для агентов, не для публикации)

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — обзор архитектуры и зависимостей между крейтами
- [docs/SYSTEMS.md](docs/SYSTEMS.md) — каталог всех подсистем с указанием файлов и ключевых типов
- [docs/FEATURE_MAP.md](docs/FEATURE_MAP.md) — быстрый поиск: «где реализовано X»

**Любой агент** перед началом работы должен заглянуть в эти файлы. `FEATURE_MAP.md` — точка первого контакта.

### Public (mdBook + cargo doc, деплоится на GitHub Pages)

- [book.toml](book.toml) — конфиг mdBook
- [book/src/](book/src/) — исходники публичной книги
- [book/src/SUMMARY.md](book/src/SUMMARY.md) — оглавление (любая новая страница регистрируется здесь)
- [.github/workflows/docs.yml](.github/workflows/docs.yml) — CI деплой на GitHub Pages

Деплой URL: `https://bluesteelll.github.io/boyko-engine/` (книга) + `/api/` (rustdoc).

Локальный preview:
```powershell
cargo install mdbook mdbook-mermaid    # один раз
mdbook serve --open
```

Публичную документацию пишет агент `doc-writer` — другие агенты её не редактируют.

## Агенты

В [.claude/agents/](.claude/agents/) определены:

| Агент | Назначение |
|-------|------------|
| `architect` | Проектирует архитектуру новых фич |
| `researcher` | Собирает практики из Bevy/flecs/EnTT/Unity DOTS |
| `architecture-critic` | Критикует план до реализации |
| `developer` | Реализует код по плану |
| `code-reviewer` | Ревьюит написанный код |
| `tester` | Билд + unit/integration/proptest/loom + criterion |
| `results-analyst` | Финальный вердикт после реализации фичи |
| `project-analyst` | Свободный анализ кодовой базы, security audit, ответы на вопросы |
| `doc-writer` | Пишет публичную документацию в `book/src/` для деплоя на GitHub Pages |

Главный Claude в чате выступает **оркестратором** — выбирает агентов под задачу и управляет циклами правок.

## Правила для агентов

### Что нельзя в hot path
- `Box<dyn Trait>`, `Rc`, `Arc<Mutex<_>>`
- `HashMap` (используй массив с `ComponentId`-индексом)
- `Vec::new()`, `format!()`, `String::from()` (всё preallocate)
- `clone()` больших структур
- Виртуальная диспетчеризация

### Что нужно для каждого `unsafe`
```rust
// SAFETY: <конкретные инварианты, которые гарантируют корректность>
unsafe { ... }
```

### Разделение обязанностей
- `developer` пишет код, но **не запускает тесты** — это работа `tester`'а
- `code-reviewer` находит проблемы, но **не правит код** — это работа `developer`'а
- `architecture-critic` критикует план, но **не диктует решение** — это работа `architect`'а
- `project-analyst` отвечает на вопросы, но **не редактирует** ничего

### Git
- Никогда не коммитить без явного запроса пользователя
- Никогда не использовать `--force`/`--no-verify` без явного разрешения
- Коммиты только от автора репозитория. **Никогда** не добавлять `Co-Authored-By: Claude ...` (или аналогичные пометки об AI-ассистенте) в commit message. История должна выглядеть как авторская работа.

## Соглашения по коду

- **Имена**: snake_case (функции/переменные), CamelCase (типы/трейты), SCREAMING_SNAKE_CASE (константы)
- **Doc-комменты** (`///`) для всех public items
- **Комментарии «зачем», не «что»** — никаких `// increment counter` над `x += 1`
- **`expect("инвариант: ...")`** вместо `unwrap()` там, где panic возможен по дизайну
- **`debug_assert!`** для проверки инвариантов в hot path (исчезают в release)
- **Импорты группами**: std → external → crate → self
