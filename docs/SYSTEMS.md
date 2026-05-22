# Каталог систем boyko-engine (ветка `ecs`)

Справочник по всем подсистемам с указанием расположения кода, ключевых типов, методов и инвариантов. Используется агентами для навигации.

**Легенда статусов:**
- ✅ Реализовано
- ⚠️ Есть, но с проблемами / неполное / не компилируется
- 📋 Запланировано

> ⚠️ Ветка `ecs` сейчас **не компилируется**. Описания ниже отражают намерение и фактический код, но запуск/тестирование заблокированы билдом.

---

## 1. Identifiers (типы ID) ✅

**Файлы:**
- [crates/boyko_ecs/src/ecs/identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs)
- [crates/boyko_utils/src/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs)

Все ID унифицированы как `usize`:
```rust
pub type EntityId            = usize;
pub type ArchetypeId         = usize;
pub type ChunkId             = usize;
pub type InlandChunkId       = usize;
pub type ComponentId         = usize;
pub type InlandUnitId        = usize;
pub type InlandPoolId        = usize;
pub type InlandComponentId   = usize;
pub type InlandArchetypeId   = usize;
pub type Generation          = usize;
```

`Slot` (в boyko_utils):
```rust
pub struct Slot {
    index: usize,
    generation: Generation,
}
```
Используется как «общий ключ» для sparse-map-структур. `Entity` реализует `From<Slot> + Into<Slot>`.

---

## 2. Memory subsystem ✅

### 2.1. Arena ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs)

То же что на master — 64 MB предвыделенная арена с `MemFreeBlockMaster` для best-fit аллокации.

```rust
pub struct Arena {
    ptr: NonNull<u8>,
    capacity: usize,
    cursor: UnsafeCell<usize>,         // ⚠️ не используется
    layout: Layout,
    free_blocks: UnsafeCell<MemFreeBlockMaster>,
}
```

### 2.2. Chunk (type-erased) ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs)

**Резко изменился vs master.** Теперь это просто metadata-структура, без данных и без `<T>`:

```rust
pub struct Chunk {
    start_index: usize,    // позиция в общем buffer'е пула
    capacity: usize,
    is_dirty: bool,        // флаг изменения (для change detection)
}
```

Данные живут в `ComponentPool::buffer`, чанки — это «окна» в этот буфер.

### 2.3. ComponentPool (type-erased) ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

```rust
pub struct ComponentPool {
    arena: NonNull<Arena>,
    buffer: NonNull<u8>,                  // single allocated buffer для всех компонентов
    buffer_capacity_bytes: usize,
    max_components: usize,
    units: Vec<Unit>,                     // densely packed прямые указатели
    pub chunks: Vec<Chunk>,               // metadata окон в buffer
    components_per_chunk: usize,
    component_id: usize,
    component_layout: Layout,             // size + align из ComponentRegistry
}
```

Ключевая идея: пул аллоцирует **один большой блок** в арене, затем выдаёт компонентам места внутри. `units` — densely packed массив прямых указателей в `buffer`. При swap_remove перемещается последний `Unit`.

### 2.4. MemFreeBlockMaster ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs)

То же, что на master. `BTreeMap<size, Vec<idx>>` + `start_map`/`end_map` для O(1) merge соседей.

### 2.5. Unit ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs)

```rust
pub struct Unit {
    ptr: *mut u8,          // прямой указатель в ComponentPool::buffer
    buffer_index: usize,   // позиция (для bounds-checking / возврата в пул)
}
```

Заменяет `UnitId` с master. **Не Sync/Send** по умолчанию из-за `*mut u8`.

### 2.6. Iterators ✅

- [sparse_iter_component_pool.rs](../crates/boyko_ecs/src/ecs/memory/sparse_iter_component_pool.rs) — `ComponentPoolSparseIter`, `ComponentPoolSparseIterMut`, `ComponentPtr`, `ComponentMutPtr`
- [multi_pool_sparse_iter.rs](../crates/boyko_ecs/src/ecs/memory/multi_pool_sparse_iter.rs) — `MultiPoolSparseIter`, `MultiPoolSparseIterMut` для одновременной итерации по нескольким пулам (компонентам одного entity)
- [iterators.rs](../crates/boyko_ecs/src/ecs/memory/iterators.rs) — ⚠️ **пустой файл** (заглушка)

---

## 3. Component subsystem ✅

### 3.1. Component trait

**Файл:** [crates/boyko_ecs/src/ecs/core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs)

```rust
pub trait Component: 'static + Sized {
    fn component_id() -> ComponentId;
    fn debug_type_name() -> &'static str { type_name::<Self>() }
    fn type_id() -> TypeId { TypeId::of::<Self>() }
    fn mem_size() -> usize { size_of::<Self>() }
    fn alignment() -> usize { align_of::<Self>() }
}
```

`mem_size()` переименовано из `size()` на master.

⚠️ Все методы имеют `#[inline(always)]`, что вызывает warning в новых Rust версиях для required trait methods (см. ниже про билд).

### 3.2. ComponentMask

**Файл:** [crates/boyko_ecs/src/ecs/core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs)

Высокоуровневая обёртка над `BitSet512` для маски «какие компоненты содержит архетип».

### 3.3. ComponentPoolBundle

**Файл:** [crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs)

Коллекция type-erased `ComponentPool`-ов в одном архетипе (по одному на каждый `ComponentId`). Имеет `swap_remove_unit` возвращающий `anyhow::Result<()>`.

### 3.4. ComponentRegistry (global static)

**Файл:** [crates/boyko_ecs/src/ecs/core/component/component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs)

Global storage для `ComponentLayout { layout: Layout, type_name, ... }`.

API:
- `register_layout<T: 'static>(component_id)` — вызывается макросом при `#[derive(Component)]`
- `get_layout(component_id) -> Option<&'static ComponentLayout>`
- `get_layout_unchecked(component_id) -> &'static ComponentLayout` (unsafe fast path)
- `get_component_size`, `get_component_alignment`, `get_component_memory_layout`

### 3.5. `#[derive(Component)]` macro

**Файл:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)

Использует `AtomicUsize` counter для присвоения `ComponentId`. Генерирует:
- `impl Component for T { fn component_id() -> ComponentId { N } }`
- Регистрация layout в registry (через `register_layout::<T>(N)`)

⚠️ `ComponentId` нестабилен между сборками (зависит от порядка раскрытия макроса).

---

## 4. Entity subsystem ✅

### 4.1. Entity

**Файл:** [crates/boyko_ecs/src/ecs/core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs)

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Entity {
    pub id: EntityId,        // usize
    pub generation: usize,
}
```

Реализует `From<Slot> + Into<Slot>` для совместимости с sparse-коллекциями.

### 4.2. EntityInland

**Файл:** [crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs)

Внутреннее представление, известное `EntityMaster`:
```rust
pub struct EntityInland {
    archetype_id: ArchetypeId,
    unit_index: InlandPoolId,    // позиция entity внутри архетипа
    generation: Generation,
}
```

### 4.3. EntityMaster

**Файл:** [crates/boyko_ecs/src/ecs/core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

```rust
pub struct EntityMaster {
    free_entity_ids: Vec<EntityId>,           // для recycling
    entities: Vec<Entity>,                    // все entities (включая удалённые)
    entity_map: SparseMap<EntityInland>,      // O(1) lookup по EntityId
    next_entity_id: EntityId,
    active_count: usize,
}
```

Методы: `allocate_entity`, `register_entity`, `update_entity_inland`, `update_entity_unit_index`, `deallocate_entity`, `get_entity_inland(_mut)`, `is_entity_valid`, `iter_entities`, `clear`, `compact`, `memory_usage`.

Есть unit-тесты внизу файла (`test_entity_allocation`, `test_entity_registration`, `test_entity_deallocation_and_reuse`, `test_entity_inland_update`).

---

## 5. Archetype subsystem ✅

### 5.1. Archetype

**Файл:** [crates/boyko_ecs/src/ecs/core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs)

```rust
pub struct Archetype {
    id: ArchetypeId,
    component_pools: ComponentPoolBundle,
    current_index: usize,
    signature: ArchetypeSignature,
    arena: NonNull<Arena>,
    component_ids: Vec<ComponentId>,
    entity_ids: Vec<EntityId>,                // indexed by unit_index
}
```

Ключевые методы: `new(id, &arena)`, `create_by_ids(id, &[ComponentId], &arena)`, `register_component`, `create_entity`, `remove_entity`, `init_entity_inland`, `id()`.

### 5.2. ArchetypeSignature

**Файл:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs)

```rust
pub struct ArchetypeSignature {
    pub mask: ComponentMask,
}
```

Маска компонентов архетипа. Используется для матчинга `Query`.

### 5.3. ArchetypeBundle

**Файл:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs)

Bundle компонентов для batch-операций (создание нескольких entity сразу).

### 5.4. ArchetypeRegistry

**Файл:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs)

Регистр архетипов, поиск по `ComponentMask`/`ArchetypeSignature`. Методы: `find_exact_match`, и т.д.

### 5.5. ArchetypeMaster

**Файл:** [crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs)

Top-level manager архетипов. Методы: `new`, `with_capacity`, `create_archetype`, `get_or_create_archetype`, `get_archetype(_mut)`, `find_archetypes_with_components`, `find_matching_archetypes`, `archetype_registry`.

---

## 6. EcsMaster (top-level API) ✅

**Файл:** [crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs)

```rust
pub struct EcsMaster {
    entity_master: EntityMaster,
    archetype_master: ArchetypeMaster,
    arena: Arena,
}
```

API:
- `new() -> Self`
- `with_capacity(entity_capacity, archetype_capacity) -> Self`
- `create_archetype(component_ids) -> ArchetypeId`
- `get_or_create_archetype(component_ids) -> ArchetypeId`
- `create_entity(archetype_id, Vec<(ComponentId, &[u8])>) -> anyhow::Result<Entity>`
- `delete_entity(entity) -> bool`

⚠️ Использует `anyhow::Result` — спорно для библиотечного API, обсудить при стабилизации.

---

## 7. Query subsystem ✅

### 7.1. Query

**Файл:** [crates/boyko_ecs/src/ecs/core/iters/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query.rs)

```rust
pub struct Query<'a> {
    archetypes: Vec<&'a Archetype>,
}
```

Конструкторы: `from_archetypes`, `with_component_ids`, `with_mask`, `with_exact_mask`.

Хранит прямые ссылки на `&Archetype` — максимум perf при итерации, никакой индирекции.

### 7.2. SparseIter / SparseIterMut

**Файл:** [crates/boyko_ecs/src/ecs/core/iters/sparse_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/sparse_iter.rs)

Итераторы по результатам query.

### 7.3. ComponentSet

**Файл:** [crates/boyko_ecs/src/ecs/core/iters/component_set.rs](../crates/boyko_ecs/src/ecs/core/iters/component_set.rs)

```rust
pub trait ComponentSet { /* ... */ }
```

Описание набора компонентов для query — вероятно, реализуется для tuple типов (`(A, B, C)`).

---

## 8. Event subsystem ✅

### 8.1. Event trait

**Файл:** [crates/boyko_ecs/src/ecs/core/events/event.rs](../crates/boyko_ecs/src/ecs/core/events/event.rs)

```rust
pub type EventId = u64;

pub trait Event: 'static + Sized {
    type Participants: Participants;
    type Parameters: Parameters;
    
    fn event_id() -> EventId;
    fn event_name() -> &'static str;
    fn layout() -> Layout { Layout::new::<Self>() }
    fn type_id() -> TypeId { TypeId::of::<Self>() }
}
```

### 8.2. EventPool / EventPoolBundle

- [event_pool.rs](../crates/boyko_ecs/src/ecs/core/events/event_pool.rs) — `EventPool`, `EventPoolIter<'a, E>`
- [event_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/events/event_pool_bundle.rs) — `EventPoolBundle`

### 8.3. EventRegistry (global)

**Файл:** [event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

API: `register_event<E>`, `get_event_info`, `get_event_layout`, `get_participants_layout`, `get_parameters_layout`, `get_event_participants`, `get_event_type_name`, `is_event_registered`, `registered_event_count`, `iter_registered_events`, `get_event_type_ids`, `validate_event_types<E>`.

### 8.4. Participants

- [participants.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants.rs) — `Participants` trait, `ParticipantInfo`
- [participants_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/participants/participants_buffer.rs) — `ParticipantBuffer`

### 8.5. Parameters

- [parameters.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters.rs) — `Parameters` trait
- [parameters_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/parameters/parameters_buffer.rs) — `ParametersBuffer`

### 8.6. `#[derive(Event)]` macro

**Файл:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) — функция `event_derive`.

---

## 9. Containers (boyko_ecs::core::containers) ✅

### ComponentTuple

- [containers/tuple/component_tuple.rs](../crates/boyko_ecs/src/ecs/core/containers/tuple/component_tuple.rs)
- [containers/tuple/component_tuple_trait.rs](../crates/boyko_ecs/src/ecs/core/containers/tuple/component_tuple_trait.rs)

Tuple-based bundle компонентов для batch операций / ergonomic API.

---

## 10. boyko_utils — переиспользуемые коллекции ✅

### 10.1. BitMask family

- [bit_mask/bit_storage.rs](../crates/boyko_utils/src/bit_mask/bit_storage.rs) — `BitStorage` trait
- [bit_mask/bit_mask.rs](../crates/boyko_utils/src/bit_mask/bit_mask.rs) — `BitMask<T: BitStorage>` (598 строк — большой)
- [bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) — `BitSet<T: BitInteger>` + iterator
- [bit_mask/bit_set512.rs](../crates/boyko_utils/src/bit_mask/bit_set512.rs) — `BitSet512` (8×u64 = 512 бит)

`ComponentMask` в boyko_ecs построен поверх `BitSet512`.

### 10.2. SparseMap family

- [sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) — `SparseCollection<K, V>` trait (⚠️ trait объявлен, но в коде не используется)
- [sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) — `SparseMap<U>` (общий)
- [sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) — `SparseSlotMap<U>` (с generation-based slots)

`EntityMaster::entity_map: SparseMap<EntityInland>` использует это.

### 10.3. identifiers

- [identifiers/primitives.rs](../crates/boyko_utils/src/identifiers/primitives.rs) — `Generation = usize`
- [identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) — `Slot { index, generation }`

---

## 11. Запланированные подсистемы 📋

- **Scheduler / System runner** — выполнение пользовательских систем, dependency graph, work-stealing
- **Change detection** — отслеживание изменений компонентов (`is_dirty` в `Chunk` — заготовка)
- **Resource management** — глобальные ресурсы
- **Command buffer** — отложенные операции
- **Serialization** — отложено
- **Hot-reload** — не цель

---

## 12. Константы (constants.rs) ✅

То же что на master — см. [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs).

| Константа | Значение | Использование |
|-----------|----------|---------------|
| `DEFAULT_ARENA_SIZE` | 64 MB | `Arena::new()` |
| `CACHE_LINE_SIZE` | 64 B | `Arena::with_capacity` |
| `MIN_ALIGNMENT` | 8 B | ⚠️ не используется |
| `DEFAULT_COMPONENTS_PER_CHUNK` | 1024 | |
| `DEFAULT_CHUNKS_PER_POOL` | 128 | `ComponentPool::with_default_sizes` |
| `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` | 2048 / 1024 / 512 / 256 | `ComponentPool` |
| `TINY/SMALL/MEDIUM_COMPONENT_THRESHOLD` | 16 / 64 / 256 B | `ComponentPool` |
| `INITIAL_ENTITY_CAPACITY` | 1024 | ⚠️ возможно не используется |
| `GROWTH_FACTOR / MAX_EXPANSION_FACTOR / ...` | | ⚠️ заготовки, не используются |

---

## 13. Текущее состояние билда ⚠️

Ветка не компилируется на момент написания. Последняя попытка фикса — `299a6b6 Blanket trait impl error fixed` — не до конца сработала.

Из коммитов и кода видны заведомо проблемные места:
- Множество `unused import` warnings — в boyko_ecs и boyko_utils
- `#[inline]` attribute cannot be used on required trait methods — в [component.rs:5](../crates/boyko_ecs/src/ecs/core/component/component.rs) и аналогичных файлах. Это **error в новых Rust версиях**.
- `unused variable` в `archetype_registry.rs`, `archetype_master.rs`
- Возможны blanket trait impl коллизии (судя по последнему commit message)

Полный список ошибок будет собран через `cargo check ecs` (см. TaskList).

---

## 14. Стиль и conventions проекта

- Языки комментариев: микс русский/английский. Стоит унифицировать.
- Doc-комменты через `///`, internal через `//`.
- `#[inline]` / `#[inline(always)]` — measured (см. CLAUDE.md принцип 7).
- `expect("инвариант: ...")` вместо `unwrap()` где panic возможен по дизайну.
- `debug_assert!` для проверки инвариантов в hot path.
