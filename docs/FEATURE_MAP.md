# Карта фич — где что искать (ветка `ecs`)

Точка первого контакта для агентов. Если ищешь, где реализован тот или иной функционал — смотри сначала сюда. Для деталей переходи в [SYSTEMS.md](SYSTEMS.md) и затем в код.

**Легенда:**
- ✅ Реализовано
- ⚠️ Реализовано, но есть проблемы / билд не проходит
- 📋 Запланировано, ещё не написано

> ⚠️ Билд ветки сейчас сломан. Большинство фич реализованы в коде, но запустить нельзя до фикса.

---

## Память и аллокация

| Что хочешь делать | Где смотреть | Метод / тип |
|-------------------|--------------|-------------|
| Выделить блок памяти на N байт | [memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs) ✅ | `Arena::allocate_layout(layout)` |
| Выделить с alignment | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::allocate_aligned(size, align)` |
| Найти best-fit свободный блок | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::find_best_fit` |
| Вернуть память в пул | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::insert` (с автоматическим merge) |
| Выровнять адрес/размер | [memory/utils.rs](../crates/boyko_ecs/src/ecs/memory/utils.rs) ✅ | `align_up(value, alignment)` |
| Освободить арену | — ⚠️ | `impl Drop for Arena` **отсутствует** (утечка) |
| Дефрагментировать список свободных блоков | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::defragment` |
| Получить статистику памяти | [memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs) ✅ | `MemFreeBlockMaster::get_memory_stats` |

---

## Type-erased component storage

| Что хочешь делать | Где | Метод |
|-------------------|-----|-------|
| Создать пул для компонента с известным ComponentId | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::new(arena, component_id, num_chunks, components_per_chunk)` |
| Добавить компонент в пул (через byte slice) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::add(...)` |
| Получить компонент по индексу | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::get(...)` |
| Удалить компонент (swap) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::swap_remove(...)` |
| Получить размер/выравнивание компонента из registry | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `get_layout`, `get_component_size`, `get_component_alignment` |

---

## Чанки (metadata-окна в buffer'е пула)

| Что | Где | Метод |
|-----|-----|-------|
| Создать chunk metadata | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::new(start_index, capacity)` |
| Узнать start_index в buffer'е пула | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::start_index()` |
| Capacity чанка | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::capacity()` |
| Пометить чанк как изменённый | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::mark_dirty()` |
| Проверить флаг изменения | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::is_dirty()` |
| Сбросить флаг изменения | [memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) ✅ | `Chunk::clear_dirty_flag()` |

> Note: `Chunk` теперь хранит только metadata, без данных. Данные — в `ComponentPool::buffer`.

---

## Прямой указатель на компонент (Unit)

| Что | Где | Метод |
|-----|-----|-------|
| Создать Unit | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::new(ptr, buffer_index)` |
| Получить указатель | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::ptr()` |
| Получить позицию в buffer'е | [memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs) ✅ | `Unit::buffer_index()` |

---

## Итерация компонентов

| Что | Где | Тип/метод |
|-----|-----|-----------|
| Итератор по компонентам одного пула | [memory/sparse_iter_component_pool.rs](../crates/boyko_ecs/src/ecs/memory/sparse_iter_component_pool.rs) ✅ | `ComponentPoolSparseIter` / `ComponentPoolSparseIterMut` |
| Wrapper для shared указателя | [memory/sparse_iter_component_pool.rs](../crates/boyko_ecs/src/ecs/memory/sparse_iter_component_pool.rs) ✅ | `ComponentPtr` |
| Wrapper для mutable указателя | [memory/sparse_iter_component_pool.rs](../crates/boyko_ecs/src/ecs/memory/sparse_iter_component_pool.rs) ✅ | `ComponentMutPtr` |
| Итератор по нескольким пулам одновременно | [memory/multi_pool_sparse_iter.rs](../crates/boyko_ecs/src/ecs/memory/multi_pool_sparse_iter.rs) ✅ | `MultiPoolSparseIter` / `MultiPoolSparseIterMut` |
| Итератор по query результатам | [core/iters/sparse_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/sparse_iter.rs) ✅ | `SparseIter` / `SparseIterMut` |

---

## Компоненты

| Что хочешь делать | Где | Как |
|-------------------|-----|-----|
| Определить новый компонент | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Component)] struct MyComp { ... }` |
| Получить уникальный ID типа компонента | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::component_id()` |
| Получить размер компонента (compile-time) | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::mem_size()` |
| Получить alignment | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::alignment()` |
| Получить TypeId | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::type_id()` |
| Получить имя типа | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::debug_type_name()` |
| Зарегистрировать layout компонента | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `register_layout::<T>(component_id)` (вызывается макросом) |
| Получить layout из registry | [core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs) ✅ | `get_layout(id)`, `get_layout_unchecked(id)` |
| Bitmask набор компонентов | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask` (поверх `BitSet512`) |
| Собрать пулы разных типов для одного архетипа | [core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs) ✅ | `ComponentPoolBundle` |
| Tuple-based bundle | [core/containers/tuple/component_tuple.rs](../crates/boyko_ecs/src/ecs/core/containers/tuple/component_tuple.rs) ✅ | `ComponentTuple` |

---

## Сущности (Entity)

| Что хочешь делать | Где | Как |
|-------------------|-----|-----|
| Создать Entity напрямую | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::new(id, generation)` |
| Создать с generation = 0 | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::with_id(id)` |
| Сравнить две Entity (id + generation) | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `e1 == e2` |
| Аллоцировать entity / переиспользовать ID | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::allocate_entity()` |
| Зарегистрировать entity в архетипе | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::register_entity(entity, archetype_id, unit_index)` |
| Получить EntityInland (метаданные) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::get_entity_inland(entity)` |
| Обновить unit_index после swap | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::update_entity_unit_index(entity, new_idx)` |
| Удалить entity | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::deallocate_entity(entity)` → `Option<EntityInland>` |
| Проверить валидность entity (id+generation) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::is_entity_valid(entity)` |
| Итерация по активным entities | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::iter_entities()` |

---

## Архетипы

| Что хочешь делать | Где | Как |
|-------------------|-----|-----|
| Создать архетип из набора `ComponentId` | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs) ✅ | `Archetype::create_by_ids(id, &[ComponentId], &arena)` |
| Управлять всеми архетипами | [core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs) ✅ | `ArchetypeMaster` |
| Создать / получить архетип по компонентам | [core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs) ✅ | `ArchetypeMaster::get_or_create_archetype(&[ComponentId])` |
| Найти архетип по signature/mask | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `ArchetypeRegistry::find_exact_match(&ComponentMask)` |
| Bitmask "из каких компонентов состоит" | [core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs) ✅ | `ArchetypeSignature` (поверх `ComponentMask`) |
| Bundle компонентов для batch-операций | [core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs) ✅ | `ArchetypeBundle` |
| Создать entity в архетипе | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs) ✅ | `Archetype::create_entity(entity_id, &mut EntityInland, Vec<(ComponentId, &[u8])>)` |
| Удалить entity из архетипа | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs) ✅ | `Archetype::remove_entity(&EntityInland)` |

---

## ECS top-level API

| Что | Где |
|-----|-----|
| Создать ECS world | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::new()` |
| Создать с pre-allocated capacity | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::with_capacity(entities, archetypes)` |
| Создать entity с компонентами | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::create_entity(archetype_id, Vec<(ComponentId, &[u8])>)` → `anyhow::Result<Entity>` |
| Удалить entity | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::delete_entity(entity)` |
| Создать/получить архетип | [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | `EcsMaster::get_or_create_archetype(&[ComponentId])` |

---

## Queries

| Что | Где |
|-----|-----|
| Query по ComponentId | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::with_component_ids(master, &[ComponentId])` |
| Query по маске (включающий) | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::with_mask(master, &mask)` |
| Query с точным совпадением маски | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::with_exact_mask(master, &mask)` |
| Query из готовых архетипов | [core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs) ✅ | `Query::from_archetypes(Vec<&Archetype>)` |
| Tuple trait для query параметров | [core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs) ✅ | `ComponentSet` trait |

---

## События (Events)

| Что | Где | Как |
|-----|-----|-----|
| Определить событие | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Event)] struct MyEvent { ... }` |
| Event trait | [core/events/event.rs](../crates/boyko_ecs/src/ecs/core/events/event.rs) ✅ | `Event` с `type Participants`, `type Parameters` |
| Зарегистрировать event | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `register_event::<E>(event_id)` |
| Получить metadata | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `get_event_info(id)`, `get_event_layout(id)` |
| Validate event types | [core/events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `validate_event_types::<E>(id)` |
| Пул событий | [core/events/event_pool.rs](../crates/boyko_ecs/src/ecs/core/events/event_pool.rs) ✅ | `EventPool` |
| Bundle разнотипных пулов | [core/events/event_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/events/event_pool_bundle.rs) ✅ | `EventPoolBundle` |
| Participants trait | [core/events/participants/participants.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs) ✅ | `Participants`, `ParticipantInfo` |
| Buffer для participants | [core/events/participants/participants_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants_buffer.rs) ✅ | `ParticipantBuffer` |
| Parameters trait | [core/events/parameters/parameters.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters.rs) ✅ | `Parameters` |
| Buffer для parameters | [core/events/parameters/parameters_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters_buffer.rs) ✅ | `ParametersBuffer` |

---

## Битовые операции (boyko_utils)

| Что | Где | Тип |
|-----|-----|-----|
| Универсальный битсет | [boyko_utils/src/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSet<T: BitInteger>` |
| Iterator по битам | [boyko_utils/src/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSetIterator<T>` |
| Фиксированный 512-битный (8×u64) | [boyko_utils/src/bit_mask/bit_set512.rs](../crates/boyko_utils/src/bit_mask/bit_set512.rs) ✅ | `BitSet512` |
| Маска компонентов (поверх BitSet512) | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask` |
| Generic BitMask | [boyko_utils/src/bit_mask/bit_mask.rs](../crates/boyko_utils/src/bit_mask/bit_mask.rs) ✅ | `BitMask<T: BitStorage>` |
| Trait для bit storage | [boyko_utils/src/bit_mask/bit_storage.rs](../crates/boyko_utils/src/bit_mask/bit_storage.rs) ✅ | `BitStorage` |

---

## Sparse maps (boyko_utils)

| Что | Где | Тип |
|-----|-----|-----|
| Общий sparse map | [boyko_utils/src/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) ✅ | `SparseMap<U>` |
| Sparse slot map с generation | [boyko_utils/src/sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) ✅ | `SparseSlotMap<U>` |
| Trait для sparse коллекций | [boyko_utils/src/sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) ⚠️ | `SparseCollection<K, V>` (declared, but unused) |

---

## Identifiers / Slots

| Что | Где | Тип |
|-----|-----|-----|
| Все ID-типы boyko_ecs | [boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `EntityId`, `ArchetypeId`, `ComponentId`, ... |
| Generation | [boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs) ✅ | `Generation = usize` |
| Slot { index, generation } | [boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) ✅ | `Slot` |

---

## Системы и scheduler 📋

**Не реализовано** — следующая большая фича. Когда будет — здесь появятся:

- Регистрация систем
- Dependency graph
- Параллельное выполнение
- Stage/phase API

---

## Resources / global state 📋

**Не реализовано** — аналог `Resource` в Bevy / singleton'ов в Unity.

---

## Change detection 📋

**Не реализовано** в виде полноценной фичи. Есть заготовка — `Chunk::is_dirty` флаг.

---

## Сериализация 📋

**Не реализовано** — отложено.

---

## Тесты и бенчмарки

| Что | Где |
|-----|-----|
| Unit-тесты | ⚠️ есть только в [entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) (4 теста). Покрытие минимальное. |
| Integration-тесты | 📋 **отсутствуют** — целевое: `crates/boyko_ecs/tests/*.rs` |
| Benchmarks | 📋 **отсутствуют** — целевое: `crates/boyko_ecs/benches/*.rs` (через `criterion`) |
| Loom-тесты для lock-free | 📋 **отсутствуют** |
| Property-based (proptest) | 📋 **отсутствуют** |

⚠️ **Запуск любых тестов сейчас невозможен — билд не проходит.**

---

## Стиль и инфраструктура

| Что | Где |
|-----|-----|
| Workspace конфиг | [Cargo.toml](../Cargo.toml) |
| Ядро ECS Cargo.toml | [crates/boyko_ecs/Cargo.toml](../crates/boyko_ecs/Cargo.toml) |
| Proc-macro Cargo.toml | [crates/boyko_macros/Cargo.toml](../crates/boyko_macros/Cargo.toml) |
| Utils Cargo.toml | [crates/boyko_utils/Cargo.toml](../crates/boyko_utils/Cargo.toml) |
| Все константы движка | [crates/boyko_ecs/src/ecs/constants.rs](../crates/boyko_ecs/src/ecs/constants.rs) |
| Правила для агентов | [../CLAUDE.md](../CLAUDE.md) |
| Архитектурный обзор | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Детальный каталог систем | [SYSTEMS.md](SYSTEMS.md) |
| TODO список (от автора) | [TODOI.md](TODOI.md) |

---

## Шпаргалка: «что-то не нашёл — куда смотреть?»

1. **Это про память / аллокацию / Arena?** → раздел "Память" + [SYSTEMS.md §2](SYSTEMS.md)
2. **Это про type-erased ComponentPool / Unit / Chunk?** → разделы выше + [SYSTEMS.md §2.3-2.6](SYSTEMS.md)
3. **Это про компоненты / Component derive / Registry?** → разделы "Компоненты" + [SYSTEMS.md §3](SYSTEMS.md)
4. **Это про entity / EntityMaster / generation?** → разделы "Сущности" + [SYSTEMS.md §4](SYSTEMS.md)
5. **Это про архетипы?** → "Архетипы" + [SYSTEMS.md §5](SYSTEMS.md)
6. **Это про top-level API (EcsMaster)?** → "ECS top-level API" + [SYSTEMS.md §6](SYSTEMS.md)
7. **Это про query / iteration?** → "Queries" + [SYSTEMS.md §7](SYSTEMS.md)
8. **Это про события?** → "События" + [SYSTEMS.md §8](SYSTEMS.md)
9. **Это про BitSet / SparseMap / boyko_utils?** → разделы "Битовые операции" / "Sparse maps" + [SYSTEMS.md §10](SYSTEMS.md)
10. **Этого вообще нет?** → проверь раздел 📋 «Запланировано» выше. Если там тоже нет — фичу ещё никто не описывал, это работа `architect`'а.
11. **Билд сломан, не могу проверить?** → задача `cargo check ecs` в TaskList. См. также [TODOI.md](TODOI.md).
