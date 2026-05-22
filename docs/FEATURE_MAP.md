# Карта фич — где что искать

Этот файл — точка первого контакта для агентов. Если ищешь, где реализован тот или иной функционал — смотри сначала сюда. Для деталей переходи в [SYSTEMS.md](SYSTEMS.md) и затем в код.

**Легенда:**
- ✅ Есть на `master`
- 🚧 Есть только на ветке `ecs` (не смержено)
- 📋 Запланировано, ещё не написано
- ⚠️ Заявлено, но не работает / заглушка / баг

---

## Память и аллокация

| Что хочешь делать | Где смотреть | Метод / тип |
|-------------------|--------------|-------------|
| Выделить блок памяти на N байт | [memory/arena.rs:44](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `Arena::allocate_layout` |
| Выделить под конкретный тип | [memory/arena.rs:66](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `Arena::allocate::<T>()` |
| Найти best-fit свободный блок | [memory/free_mem_block.rs:132](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::find_best_fit` |
| Вернуть память в пул | [memory/free_mem_block.rs:71](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::insert` (с автоматическим merge соседних) |
| Выровнять адрес/размер | [memory/utils.rs:3](../crates/boyko_ecs/src/ecs/memory/utils.rs) ✅ | `align_up(value, alignment)` |
| Освободить арену | — ⚠️ | `impl Drop for Arena` **отсутствует** (утечка) |
| Дефрагментировать список свободных блоков | [memory/free_mem_block.rs:249](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::defragment` |
| Получить статистику памяти | [memory/free_mem_block.rs:240](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::get_memory_stats` |

---

## Чанки (хранилище компонентов одного типа)

| Что хочешь делать | Где | Метод |
|-------------------|-----|-------|
| Создать чанк фиксированной capacity | [memory/chunk.rs:22](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::new(arena, capacity)` |
| Добавить компонент в чанк | [memory/chunk.rs:44](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::add(component)` |
| Получить компонент по индексу | [memory/chunk.rs:93](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::get(idx)` / `get_mut` |
| Итерироваться по всем компонентам в чанке | [memory/chunk.rs:143](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::as_slice()` / `as_mut_slice()` |
| Удалить компонент со сдвигом (O(n), порядок сохраняется) | [memory/chunk.rs:171](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::remove(idx)` |
| Удалить компонент быстро (O(1), порядок нарушается) | [memory/chunk.rs:201](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::swap_remove(idx)` |
| Очистить чанк (с Drop) | [memory/chunk.rs:157](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::clear()` |
| Установить компонент по произвольному индексу | [memory/chunk.rs:65](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ⚠️ | `Chunk::set(idx, c)` — **возможен баг с drop on uninit** |

---

## Component pool (вектор чанков для одного типа)

| Что хочешь делать | Где | Метод |
|-------------------|-----|-------|
| Создать пул с дефолтным размером (адаптивно к size_of::<T>) | [memory/component_pool.rs:70](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::<T>::with_default_sizes(&arena)` |
| Добавить компонент | [memory/component_pool.rs:93](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::add(comp)` → `UnitId` |
| Получить компонент по UnitId | [memory/component_pool.rs:125](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::get(uid)` |
| Удалить (swap) | [memory/component_pool.rs:147](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::swap_remove(uid)` |
| Получить весь чанк как slice | [memory/component_pool.rs:165](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::chunk_components(chunk_idx)` |
| Узнать, заполнен ли пул | [memory/component_pool.rs:202](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ⚠️ | `ComponentPool::is_full()` — **underflow при пустом chunks** |

**Выбор размера чанка по `size_of::<T>()`:**

| Размер T | Чанк | Откуда |
|----------|------|--------|
| ≤ 16 B | 2048 элементов | `TINY_COMPONENTS_PER_CHUNK` |
| 17–64 B | 1024 | `SMALL_COMPONENTS_PER_CHUNK` |
| 65–256 B | 512 | `MEDIUM_COMPONENTS_PER_CHUNK` |
| > 256 B | 256 | `LARGE_COMPONENTS_PER_CHUNK` |

Логика выбора: [component_pool.rs:76](../crates/boyko_ecs/src/ecs/memory/component_pool.rs).

---

## Компоненты

| Что хочешь делать | Где | Как |
|-------------------|-----|-----|
| Определить новый компонент | [boyko_macros/src/lib.rs:15](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Component)] struct MyComp { ... }` |
| Получить уникальный ID типа компонента | [core/component.rs:6](../crates/boyko_ecs/src/ecs/core/component.rs) ✅ | `MyComp::component_id()` |
| Получить размер компонента (compile-time) | [core/component.rs:20](../crates/boyko_ecs/src/ecs/core/component.rs) ✅ | `MyComp::size()` |
| Получить alignment | [core/component.rs:25](../crates/boyko_ecs/src/ecs/core/component.rs) ✅ | `MyComp::alignment()` |
| Получить TypeId | [core/component.rs:15](../crates/boyko_ecs/src/ecs/core/component.rs) ✅ | `MyComp::type_id()` |
| Получить имя типа для отладки | [core/component.rs:10](../crates/boyko_ecs/src/ecs/core/component.rs) ✅ | `MyComp::debug_type_name()` |
| Зарегистрировать компонент с маской | 🚧 | `ComponentRegistry` на ветке `ecs` |
| Bitmask набор компонентов | 🚧 | `ComponentMask` на ветке `ecs` (boyko_utils) |

---

## Сущности (Entity)

| Что хочешь делать | Где | Как |
|-------------------|-----|-----|
| Создать Entity со значениями | [core/entity.rs:9](../crates/boyko_ecs/src/ecs/core/entity.rs) ✅ | `Entity::new(id, generation)` |
| Создать с generation = 0 | [core/entity.rs:16](../crates/boyko_ecs/src/ecs/core/entity.rs) ✅ | `Entity::with_id(id)` |
| Сравнить две Entity (id + generation) | [core/entity.rs:37](../crates/boyko_ecs/src/ecs/core/entity.rs) ✅ | `e1 == e2` (через `PartialEq`) |
| Инкрементировать generation (wrapping) | [core/entity.rs:31](../crates/boyko_ecs/src/ecs/core/entity.rs) ✅ | `Entity::increment_generation()` |
| Аллоцировать entity / переиспользовать ID | 🚧 | `EntityMaster::allocate_entity` на ветке `ecs` |
| Сохранить расположение entity (archetype + index) | 🚧 | `EntityInland` на ветке `ecs` |

---

## Архетипы (только на ветке `ecs`)

| Что хочешь делать | Где (на ветке `ecs`) |
|-------------------|----------------------|
| Создать архетип из набора `ComponentId` | `core/archetype/archetype.rs::Archetype::create_by_ids` 🚧 |
| Управлять всеми архетипами | `core/archetype/archetype_master.rs::ArchetypeMaster` 🚧 |
| Найти архетип по signature/mask | `core/archetype/archetype_registry.rs` 🚧 |
| Bitmask "из каких компонентов состоит" | `core/archetype/archetype_signature.rs` 🚧 |
| Bundle компонентов для batch-операций | `core/archetype/archetype_bundle.rs` 🚧 |
| Создать entity в архетипе | `core/archetype/archetype.rs::Archetype::create_entity` 🚧 |
| Удалить entity из архетипа | `core/archetype/archetype.rs::Archetype::remove_entity` 🚧 |

Просмотр: `git show origin/ecs:crates/boyko_ecs/src/ecs/core/archetype/<file>.rs`

---

## ECS top-level (только на ветке `ecs`)

| Что | Где |
|-----|-----|
| Создать ECS world | `core/ecs_master/ecs_master.rs::EcsMaster::new()` 🚧 |
| Создать с pre-allocated capacity | `EcsMaster::with_capacity(entities, archetypes)` 🚧 |
| Создать entity с компонентами | `EcsMaster::create_entity(archetype_id, components)` 🚧 |
| Удалить entity | `EcsMaster::delete_entity(entity)` 🚧 |
| Создать/получить архетип | `EcsMaster::get_or_create_archetype(comp_ids)` 🚧 |

---

## Queries (только на ветке `ecs`)

| Что | Где |
|-----|-----|
| Query по ComponentId | `core/iters/query.rs::Query::with_component_ids` 🚧 |
| Query по маске | `core/iters/query.rs::Query::with_mask` 🚧 |
| Query с точным совпадением маски | `core/iters/query.rs::Query::with_exact_mask` 🚧 |
| Итерация по архетипам query | `core/iters/sparse_iter.rs` 🚧 |
| Итерация по нескольким пулам параллельно | `memory/multi_pool_sparse_iter.rs` 🚧 |

---

## События (только на ветке `ecs`)

| Что | Где |
|-----|-----|
| Определить событие | `core/events/event.rs::Event` trait 🚧 |
| Пул событий | `core/events/event_pool.rs::EventPool` 🚧 |
| Регистр всех типов событий | `core/events/event_registry.rs::EventRegistry` 🚧 |
| Участники события (entities) | `core/events/participants/participants.rs` 🚧 |
| Параметры события | `core/events/parameters/parameters.rs` 🚧 |

---

## Битовые операции (только на ветке `ecs`, крейт boyko_utils)

| Что | Где |
|-----|-----|
| Универсальный битсет | `crates/boyko_utils/src/bit_mask/bit_set.rs::BitSet` 🚧 |
| Фиксированный 512-битный (8×u64) | `crates/boyko_utils/src/bit_mask/bit_set512.rs::BitSet512` 🚧 |
| Маска компонентов | `crates/boyko_utils/src/bit_mask/bit_mask.rs::BitMask` 🚧 |
| Низкоуровневое хранилище битов | `crates/boyko_utils/src/bit_mask/bit_storage.rs` 🚧 |

---

## Системы и scheduler 📋

**Не реализовано** — это запланированная следующая большая фича. Когда будет — здесь появятся:

- Регистрация систем
- Dependency graph
- Параллельное выполнение
- Stage/phase API

---

## Resources / global state 📋

**Не реализовано** — аналог `Resource` в Bevy / singleton'ов в Unity. Запланировано.

---

## Change detection 📋

**Не реализовано** — отслеживание модификаций компонентов (Bevy использует tick counters). Запланировано.

---

## Сериализация 📋

**Не реализовано** — отложено до стабилизации модели.

---

## Тесты и бенчмарки

| Что | Где |
|-----|-----|
| Unit-тесты | 📋 **отсутствуют** — целевое размещение: `#[cfg(test)] mod tests { ... }` в конце каждого `.rs` |
| Integration-тесты | 📋 **отсутствуют** — целевое размещение: `crates/boyko_ecs/tests/*.rs` |
| Benchmarks | 📋 **отсутствуют** — целевое размещение: `crates/boyko_ecs/benches/*.rs` (через `criterion`) |
| Loom-тесты для lock-free | 📋 **отсутствуют** — целевое: `#[cfg(loom)] mod loom_tests` |
| Property-based (proptest) | 📋 **отсутствуют** |

---

## Стиль и инфраструктура

| Что | Где |
|-----|-----|
| Workspace конфиг | [Cargo.toml](../Cargo.toml) |
| Ядро ECS Cargo.toml | [crates/boyko_ecs/Cargo.toml](../crates/boyko_ecs/Cargo.toml) |
| Proc-macro Cargo.toml | [crates/boyko_macros/Cargo.toml](../crates/boyko_macros/Cargo.toml) |
| Все константы движка | [crates/boyko_ecs/src/ecs/constants.rs](../crates/boyko_ecs/src/ecs/constants.rs) |
| Правила для агентов | [../CLAUDE.md](../CLAUDE.md) |
| Архитектурный обзор | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Детальный каталог систем | [SYSTEMS.md](SYSTEMS.md) |

---

## Шпаргалка: «что-то не нашёл — куда смотреть?»

1. **Это про память / аллокацию / чанки?** → раздел "Память" выше + [SYSTEMS.md §1](SYSTEMS.md)
2. **Это про компоненты / entity?** → разделы выше + [SYSTEMS.md §2-3](SYSTEMS.md)
3. **Это про архетипы / queries / events?** → ветка `ecs`, [SYSTEMS.md §4-7](SYSTEMS.md)
4. **Этого вообще нет?** → проверь раздел 📋 «Запланировано» в [SYSTEMS.md §9](SYSTEMS.md). Если там тоже нет — фичу ещё никто не описывал, это работа `architect`'а.
5. **Есть ли это в Bevy/flecs/EnTT?** → запусти `researcher`'а, он сравнит.
