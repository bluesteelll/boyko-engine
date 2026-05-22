# Архитектура boyko-engine (ветка `ecs`)

> Эта документация отражает состояние **ветки `ecs`**. Сравнение с master — в конце.

## Цели и не-цели

**Цели:**
- Производительность уровня state-of-the-art ECS-движков (Bevy / flecs / Unity DOTS / EnTT)
- Кеш-локальность через type-erased chunked storage + archetype-grouping компонентов
- Lock-free параллелизм с партиционированием работ по чанкам/архетипам
- Минимальный footprint per-entity / per-component
- Zero-cost generics — никакой динамической диспетчеризации в hot path

**Не-цели (на текущей стадии):**
- Поддержка скриптинга (Lua, Wasm)
- Hot-reload компонентов
- Сериализация/десериализация (отложено до стабилизации модели)
- Кроссплатформенность за пределы x86_64

## Структура workspace

```
boyko-engine/
├── Cargo.toml                            # workspace + основной бинарь
├── src/main.rs                           # точка входа (пустая)
├── crates/
│   ├── boyko_ecs/                        # ядро ECS
│   │   ├── Cargo.toml                    # deps: rand, anyhow, boyko-utils
│   │   └── src/
│   │       ├── lib.rs                    # pub mod ecs;
│   │       └── ecs/
│   │           ├── mod.rs                # core, memory, constants, identifiers
│   │           ├── constants.rs          # размеры, выравнивание, пороги
│   │           ├── identifiers/
│   │           │   └── primitives.rs     # type aliases: EntityId, ArchetypeId, ComponentId, ...
│   │           ├── core/
│   │           │   ├── component/        # type-erased: Component trait, ComponentMask, ComponentPoolBundle, ComponentRegistry
│   │           │   ├── entity/           # Entity, EntityInland, EntityMaster (recycling)
│   │           │   ├── archetype/        # Archetype, ArchetypeMaster, ArchetypeRegistry, ArchetypeSignature, ArchetypeBundle
│   │           │   ├── ecs_master/       # EcsMaster — top-level фасад
│   │           │   ├── iters/            # Query, SparseIter, ComponentSet
│   │           │   ├── events/           # Event trait + EventPool/EventRegistry, Participants, Parameters
│   │           │   └── containers/tuple/ # ComponentTuple для batch операций
│   │           └── memory/
│   │               ├── arena.rs              # 64 MB arena с best-fit аллокатором
│   │               ├── free_mem_block.rs     # tracker свободных блоков
│   │               ├── chunk.rs              # type-erased: только metadata (start_index, capacity, dirty)
│   │               ├── component_pool.rs     # type-erased: NonNull<u8> + Vec<Unit> + chunks
│   │               ├── id_unit.rs            # Unit { ptr: *mut u8, buffer_index }
│   │               ├── utils.rs              # align_up
│   │               ├── sparse_iter_component_pool.rs   # итератор по пулу
│   │               ├── multi_pool_sparse_iter.rs       # итератор по нескольким пулам
│   │               └── iterators.rs          # ⚠️ пустой файл-заглушка
│   ├── boyko_macros/                     # proc-macros
│   │   ├── Cargo.toml                    # deps: syn, quote, proc-macro2, boyko-ecs
│   │   └── src/lib.rs                    # #[derive(Component)] + #[derive(Event)]
│   └── boyko_utils/                      # переиспользуемые коллекции
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── identifiers/
│           │   ├── primitives.rs         # Generation = usize
│           │   └── slot.rs               # Slot { index, generation }
│           ├── bit_mask/
│           │   ├── bit_storage.rs        # BitStorage trait
│           │   ├── bit_mask.rs           # BitMask<T: BitStorage>
│           │   ├── bit_set.rs            # BitSet<T: BitInteger> + iterator
│           │   └── bit_set512.rs         # BitSet512 — фиксированный 8×u64
│           └── sparse_map/
│               ├── sparse_collection.rs  # SparseCollection trait
│               ├── sparse_map.rs         # SparseMap<U>
│               └── sparse_slot_map.rs    # SparseSlotMap<U>
└── docs/                                 # внутренняя документация
```

## Зависимости между крейтами

```
boyko-engine (main binary)
    ├── boyko_ecs
    │       └── boyko_utils       ← новое на ecs
    └── boyko_macros
            └── boyko_ecs         (для путей в раскрытом коде макроса)
```

Внешние зависимости:
- `boyko_ecs`: `rand`, `anyhow`, `boyko-utils`
- `boyko_macros`: `syn`, `quote`, `proc-macro2`, `boyko-ecs`
- `boyko_utils`: (нет внешних)

## Слои архитектуры

```
┌────────────────────────────────────────────────────────────────┐
│  Layer 4: Game/User Code (использует ECS API)                  │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 3: ECS API                                              │
│  EcsMaster, ArchetypeMaster, Query, Event, EventRegistry       │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 2: ECS Core                                             │
│  Entity, EntityMaster, EntityInland                            │
│  Archetype, ArchetypeRegistry, ArchetypeSignature              │
│  Component (trait + derive), ComponentMask, ComponentRegistry  │
│  Event (trait + derive), Participants, Parameters              │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 1: Type-Erased Memory                                   │
│  Arena → ComponentPool (type-erased) → Chunk → Unit            │
│  MemFreeBlockMaster (free-block tracker)                       │
└────────────────────────────────────────────────────────────────┘
                                ↑
┌────────────────────────────────────────────────────────────────┐
│  Layer 0: Utils (boyko_utils)                                  │
│  BitSet<T>, BitMask<T>, BitSet512                              │
│  SparseMap<U>, SparseSlotMap<U>                                │
│  Slot, identifiers/primitives                                  │
└────────────────────────────────────────────────────────────────┘
```

## Поток данных при создании entity

```
User → EcsMaster::create_entity(archetype_id, components)
    ├─ EntityMaster::allocate_entity()
    │      └─ либо взять из free_entity_ids,
    │         либо bump next_entity_id → Entity { id, generation=0 }
    ├─ ArchetypeMaster::get_archetype_mut(id) → &mut Archetype
    ├─ Archetype::create_entity(entity_id, &mut inland, components)
    │      └─ для каждой пары (component_id, &[u8]):
    │             ComponentPoolBundle::get_pool_mut(component_id)
    │               → ComponentPool::add(...)
    │                   ├─ если нужно: ComponentRegistry::get_layout(id)
    │                   ├─ если первая аллокация: arena.allocate_layout(...)
    │                   ├─ ptr::copy(src=bytes, dst=buffer + offset, size=layout.size())
    │                   └─ units.push(Unit { ptr, buffer_index })
    └─ EntityMaster::register_entity(entity, archetype_id, unit_index)
              └─ entity_map.insert(entity.id, EntityInland { archetype_id, unit_index, generation })
```

## Ключевые архитектурные решения

### 1. Type-erased `ComponentPool` (отказ от generic `<T>`)

**Где:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

На master `ComponentPool<T: Component>` был generic — один пул на один тип. На ecs пул хранит **сырые байты** (`buffer: NonNull<u8>`, `buffer_capacity_bytes: usize`) и работает через `ComponentId` + `Layout` из global `ComponentRegistry`.

**Зачем:**
- Архетип содержит много **разных** типов компонентов. С generic-пулом нельзя положить `Vec<ComponentPool<?>>` без `Box<dyn Trait>` или enum.
- Type erasure через `Layout` — стандартный подход (Bevy `Table`, flecs `ecs_table_t`).

**Цена:**
- Каждый `add` / `get` теряет compile-time проверку типа — корректность опирается на инвариант «ComponentId соответствует правильному типу».
- Доступ к компоненту требует `unsafe { &*(ptr as *const T) }` с SAFETY-комментом.

### 2. Прямой указатель в `Unit` вместо двухуровневой адресации

**Где:** [crates/boyko_ecs/src/ecs/memory/id_unit.rs](../crates/boyko_ecs/src/ecs/memory/id_unit.rs)

На master был `UnitId { chunk: u32, inland: u32 }` — 8 байт, требовал вычисления адреса при каждом доступе. На ecs `Unit { ptr: *mut u8, buffer_index: usize }` — 16 байт, но доступ к компоненту прямой (`*ptr`).

**Trade-off:** удвоение размера индекса ради устранения индирекции при чтении.

### 3. Global `ComponentRegistry` / `EventRegistry`

**Где:** [component_registry.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry.rs), [event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs)

`static` storage с `register_layout<T>(component_id)` для регистрации типа. Регистрация автоматически вызывается из кода, сгенерированного `#[derive(Component)]`.

**Зачем:**
- Type erasure требует runtime metadata (size, align, TypeId).
- Один источник истины для всех `ComponentPool`.

**Риски:**
- `ComponentId` зависит от порядка раскрытия макроса (AtomicUsize counter в proc-macro) — нестабильны между сборками.
- Регистрация при первом использовании — нужно гарантировать, что регистрация прошла до первого доступа.
- Thread-safety регистрации требует валидации (вероятно нужен `Mutex` или `OnceLock`).

### 4. `EntityMaster` с recycling через free list

**Где:** [entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs)

`free_entity_ids: Vec<EntityId>` для переиспользования слотов. `entity_map: SparseMap<EntityInland>` для O(1) lookup по `EntityId`.

`Generation` инкрементируется при deallocate — предотвращает stale references.

### 5. `anyhow::Result` в `EcsMaster`

**Где:** [ecs_master.rs:9](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs)

`anyhow` используется для propagation ошибок верхнего уровня. ⚠️ Спорное решение для библиотеки — `anyhow` обычно для приложений. При стабилизации API стоит заменить на domain-specific error type.

### 6. Адаптивный размер чанка по размеру компонента

То же, что на master — `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` (см. [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)).

## Многопоточная модель (дизайн-цель)

Целевая модель:
1. **Read-heavy parallelism**: множественные потоки итерируются по разным `Query` параллельно — Rust borrow checker через системный scheduler гарантирует отсутствие конфликтов component access.
2. **Partitioned writes**: при параллельной обработке одного архетипа — деление чанков между потоками (1 поток = 1+ чанк).
3. **Lock-free инфра**: аллокации, registry, доступ к архетипам — через атомики.

Текущее состояние:
- `Arena` — `UnsafeCell<MemFreeBlockMaster>`, **не thread-safe** для multi-writer.
- `ComponentPool` mutability через `&mut self`.
- `ComponentRegistry` / `EventRegistry` — `static` storage, нужна проверка thread-safety регистрации.
- Никакого scheduler'а пока нет.

## Цели по производительности

Целевые ориентиры (требуют валидации через criterion-бенчи после фикса билда):

| Операция | Target | Notes |
|----------|--------|-------|
| `Arena::allocate_aligned` (no fragmentation) | ≤ 50 ns | BTreeMap lookup + 2 HashMap ops |
| `ComponentPool::add` (есть место в чанке) | ≤ 10 ns | Type-erased: указатель + memcpy + Vec::push(Unit) |
| Доступ к компоненту через `Unit::ptr` | ≤ 2 ns | Прямая dereference указателя |
| Линейная итерация пула | ~32 GB/s для tiny компонентов | Sequential через buffer |
| `EcsMaster::create_entity` | ≤ 150 ns | EntityMaster + ArchetypeMaster + ComponentPool::add × N |
| Query construction (cached signature) | ≤ 50 ns | Фильтр архетипов по маске |

Цифры — таргеты. Сейчас бенчей нет.

## Что отличается от ветки `master`

| Аспект | master | ecs |
|--------|--------|-----|
| ComponentPool | `ComponentPool<T: Component>` (generic) | type-erased + `ComponentRegistry` |
| Chunk | `Chunk<T>` хранит данные | `Chunk` — только metadata (start_index, capacity, dirty) |
| Адресация | `UnitId { chunk: u32, inland: u32 }` | `Unit { ptr: *mut u8, buffer_index: usize }` |
| Entity ID | `u32` + generation `u16` | `usize` + generation `usize` |
| EntityMaster | ⚠️ нет | ✅ с recycling |
| Archetype | ⚠️ пустой файл-заглушка | ✅ полная реализация |
| EcsMaster | ⚠️ пустой файл-заглушка | ✅ есть, использует anyhow |
| Query | ⚠️ нет | ✅ есть |
| Event subsystem | ⚠️ нет | ✅ есть (с Participants + Parameters) |
| boyko_utils | ⚠️ нет | ✅ есть (BitSet, SparseMap, Slot) |
| Билд | ✅ собирается | ❌ не компилируется |
