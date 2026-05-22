# Архитектура boyko-engine

## Цели и не-цели

**Цели:**
- Производительность уровня state-of-the-art ECS-движков (Bevy / flecs / Unity DOTS / EnTT)
- Кеш-локальность через SoA + chunked storage + adaptive chunk size
- Lock-free параллелизм с возможностью партиционирования работ по потокам
- Минимальный footprint per-entity / per-component
- Zero-cost generics — никакой динамической диспетчеризации в hot path

**Не-цели (на текущей стадии):**
- Поддержка скриптинга (Lua, Wasm)
- Hot-reload компонентов
- Сериализация/десериализация (отложено до стабилизации модели)
- Кроссплатформенность за пределы x86_64 (ARM/RISC-V — потенциально позже)

## Структура workspace

```
boyko-engine/
├── Cargo.toml                        # workspace + основной бинарь
├── src/main.rs                       # точка входа (сейчас пустая)
├── crates/
│   ├── boyko_ecs/                    # ядро ECS
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                # pub mod ecs;
│   │       └── ecs/
│   │           ├── mod.rs            # pub mod core, memory, constants
│   │           ├── constants.rs      # размеры, выравнивание, пороги
│   │           ├── core/             # ECS-сущности верхнего уровня
│   │           │   ├── mod.rs
│   │           │   ├── component.rs  # Component trait
│   │           │   ├── entity.rs     # Entity (id + generation)
│   │           │   ├── archetype.rs  # ПУСТО (master) / реализовано на ветке ecs
│   │           │   └── ecs_master.rs # ПУСТО (master) / реализовано на ветке ecs
│   │           └── memory/           # подсистема памяти
│   │               ├── mod.rs
│   │               ├── arena.rs           # bump+best-fit аллокатор на 64MB
│   │               ├── free_mem_block.rs  # tracker свободных блоков
│   │               ├── chunk.rs           # типизированный буфер
│   │               ├── component_pool.rs  # вектор чанков для одного типа
│   │               ├── component_index.rs # UnitId = {chunk, inland}
│   │               ├── utils.rs           # align_up
│   │               └── iterators.rs       # ПУСТО (заглушка)
│   ├── boyko_macros/                 # proc-macros
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                # #[derive(Component)]
│   └── boyko_utils/                  # ТОЛЬКО НА ВЕТКЕ ecs
│       └── src/
│           └── bit_mask/             # BitSet, BitMask, BitSet512
└── docs/                             # эта документация
```

## Зависимости между крейтами

```
boyko-engine (main binary)
    ├── boyko_ecs
    └── boyko_macros
            └── boyko_ecs (для путей в раскрытом коде макроса)
```

На ветке `ecs` добавляется:
```
boyko_ecs → boyko_utils (для BitSet'ов)
```

## Слои архитектуры (снизу вверх)

```
┌─────────────────────────────────────────────────────────┐
│  Layer 4: Game/User Code (использует ECS API)           │
└─────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────┐
│  Layer 3: ECS API   [⚠️ только на ветке ecs]            │
│  EcsMaster, ArchetypeMaster, Query, Event               │
└─────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────┐
│  Layer 2: ECS Core                                      │
│  Component (trait + derive), Entity, ComponentId        │
└─────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────┐
│  Layer 1: Memory                                        │
│  Arena → ComponentPool → Chunk → UnitId                 │
│  + MemFreeBlockMaster (free-block tracker)              │
└─────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────┐
│  Layer 0: Constants, Utils                              │
│  CACHE_LINE_SIZE=64, align_up, ...                      │
└─────────────────────────────────────────────────────────┘
```

На текущей master существуют только Layer 0-2. Layer 3 — в активной разработке на ветке `ecs`.

## Поток данных при типичной операции

### Создание entity с компонентами `(Position, Velocity)` *(целевая модель — ветка `ecs`)*

```
EcsMaster::create_entity(archetype_id, components)
    ↓
EntityMaster::allocate_entity()          → Entity { id, generation }
    ↓
ArchetypeMaster::get_archetype_mut(...)  → &mut Archetype
    ↓
Archetype::create_entity(...)
    ├── для каждого ComponentId:
    │     ComponentPoolBundle::get_pool(comp_id)
    │       → ComponentPool<T>::add(component)
    │         → Chunk<T>::add(component)
    │           → ptr::write на arena-allocated память
    └── EntityInland { archetype_id, unit_index, generation }
    ↓
EntityMaster::register_entity(entity, archetype_id, unit_index)
```

### Аллокация памяти (Layer 1)

```
ComponentPool<T>::new(arena, num_chunks, size_per_chunk)
    ↓
for _ in 0..num_chunks:
    Chunk::<T>::new(arena, size_per_chunk)
        ↓
        Layout::array::<T>(size_per_chunk)
        ↓
        Arena::allocate_layout(layout)
            ↓
            MemFreeBlockMaster::allocate_aligned(size, align)
                ├── BTreeMap::range(min_size..).next()  // best-fit
                ├── split на выровненную часть + остатки
                └── вернуть выровненный адрес
```

## Стратегия веток

- **`master`** — стабильный фундамент. Содержит только то, что прошло ревью и считается корректным. Сейчас здесь только память + базовые типы.
- **`ecs`** — активная разработка ECS-уровня. Содержит архетипы, queries, events. **Не смержена** в master.
- `memory`, `utils` — старые feature-ветки, видимо вмёрзлы в `master` через PR #1.

При работе над фичей ECS-уровня — смотри `ecs` через `git show origin/ecs:путь/к/файлу.rs`. Не дублируй то, что там уже реализовано — лучше предложи merge или взять как baseline.

## Ключевые архитектурные решения и их обоснование

| Решение | Где | Обоснование |
|---------|-----|-------------|
| Bump+best-fit аллокатор вместо системного `malloc` | [arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) | Контроль над фрагментацией, предсказуемая латентность, отсутствие system call в hot path |
| Адаптивный размер чанка по размеру компонента | [component_pool.rs:76-87](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) | Баланс cache-line utilization (мелкие компоненты пакуются плотно) и memory-waste (крупные не плодят пустые слоты) |
| Двухуровневая адресация `UnitId{chunk, inland}` | [component_index.rs](../crates/boyko_ecs/src/ecs/memory/component_index.rs) | Компактность (8 байт вместо 16), эффективный swap_remove внутри чанка, простая итерация по чанкам |
| `ComponentId` через атомарный счётчик в proc-macro | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) | Compile-time стабильный ID; trade-off: ID нестабильны между сборками (зависят от порядка компиляции) |
| Cache-line alignment арены (64 байта) | [constants.rs:7](../crates/boyko_ecs/src/ecs/constants.rs), [arena.rs:23-25](../crates/boyko_ecs/src/ecs/memory/arena.rs) | Все allocations начинаются на cache line — нет cross-line чтений |
| `UnsafeCell<usize>` для cursor в Arena | [arena.rs:13](../crates/boyko_ecs/src/ecs/memory/arena.rs) | Interior mutability без проверок borrow checker'а; **сейчас не используется** — рудимент от старого bump-only аллокатора |
| `swap_remove` как основная стратегия удаления | [chunk.rs:201-229](../crates/boyko_ecs/src/ecs/memory/chunk.rs) | O(1) удаление без сдвига; нарушение порядка не страшно для DOD |

## Многопоточная модель *(дизайн-цель, частично реализовано)*

Целевая модель:
1. **Read-heavy parallelism**: множественные потоки могут параллельно итерироваться по разным `ComponentPool` через различные `Query` — Rust borrow checker через системный scheduler гарантирует отсутствие конфликтов.
2. **Partitioned writes**: при параллельной обработке одного пула — деление чанков между потоками (1 поток = 1+ чанк).
3. **Lock-free инфра**: аллокации, регистрация компонентов, доступ к архетипам — через атомики, без блокировок.
4. **Work-stealing scheduler** для систем (не реализован).

Текущее состояние:
- `Arena` использует `UnsafeCell<MemFreeBlockMaster>` — **не thread-safe** для multi-writer.
- `ComponentPool<T>` mutability через `&mut self` — синхронизация на ответственности вызывающего.
- Нет ни одного атомика в hot path кроме `ComponentId`-счётчика в макросе (compile-time).

Это означает: фундамент готов под single-writer / multi-reader, переход к multi-writer требует:
- Lock-free `MemFreeBlockMaster` или per-thread arenas с merging на синхро-точках
- Перепроектирование `ComponentPool::add` под atomic chunk-index reservation

## Цели по производительности

Эти числа — не текущие измерения, а целевые ориентиры (предполагается, что бенчмарки покажут близкие значения после следующего раунда оптимизаций):

| Операция | Target |
|----------|--------|
| `Arena::allocate_aligned` (best-fit, нет фрагментации) | ≤ 50 ns |
| `ComponentPool::add` (есть место в текущем чанке) | ≤ 5 ns |
| `Chunk::get(index)` | ≤ 2 ns |
| `Chunk::swap_remove` | ≤ 5 ns |
| Линейная итерация `Chunk::as_slice` для T=8B | пиковая пропускная способность L1 (~32 GB/s) |
| Создание Entity (для целевой ветки `ecs`) | ≤ 100 ns |

Цифры должны подтверждаться criterion-бенчмарками. Если фича замедляет существующие цифры — это регрессия, REWORK.
