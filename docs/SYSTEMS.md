# Каталог систем boyko-engine

Этот файл — справочник по всем подсистемам с указанием расположения кода, ключевых типов, методов и инвариантов. Используется агентами для навигации.

**Легенда статусов:**
- ✅ Реализовано на `master`
- 🚧 Реализовано на ветке `ecs`, не смержено
- 📋 Запланировано, не начато
- ⚠️ Заглушка (пустой файл на master)

---

## 1. Memory subsystem ✅

Подсистема памяти — единственная полностью реализованная на `master`.

### 1.1. Arena ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/arena.rs](../crates/boyko_ecs/src/ecs/memory/arena.rs)

**Тип:** `Arena`

Bump-аллокатор поверх 64 MB (по умолчанию) сырого блока памяти. Внутри использует [`MemFreeBlockMaster`](#13-memfreeblockmaster-) для отслеживания свободных блоков.

**Поля:**
- `ptr: NonNull<u8>` — указатель на начало блока
- `capacity: usize` — выровненная вместимость
- `cursor: UnsafeCell<usize>` — **не используется** (рудимент)
- `layout: Layout` — для корректного `dealloc` (хотя сейчас Drop не реализован → утечка!)
- `free_blocks: UnsafeCell<MemFreeBlockMaster>`

**Ключевые методы:**
- `with_capacity(capacity: usize) -> Self` — выравнивает по cache line, аллоцирует через `std::alloc::alloc`
- `new() -> Self` — `with_capacity(DEFAULT_ARENA_SIZE)` = 64 MB
- `allocate_layout(&self, layout: Layout) -> NonNull<u8>` — best-fit + alignment; **паникует при OOM**
- `allocate_from_free_blocks(&self, layout: Layout) -> Option<NonNull<u8>>` — без паники
- `allocate<T>() -> NonNull<T>` — обёртка для одного объекта

**Инварианты:**
- Указатель валиден до Drop арены (Drop не реализован — арена живёт до конца программы)
- Все allocations выровнены минимум по `layout.align()`
- `&self` — внутренняя мутабельность через `UnsafeCell`; **НЕ thread-safe**

**Известные проблемы:**
- ⚠️ Нет `impl Drop` → утечка системной памяти (для движка с одной long-lived arena это допустимо, но требует документирования)
- ⚠️ `cursor` объявлен, но нигде не читается/пишется
- ⚠️ `MIN_ALIGNMENT` константа объявлена, но в коде не используется

---

### 1.2. Chunk\<T\> ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs)

**Тип:** `Chunk<T: Component>`

Типизированный буфер фиксированной capacity для компонентов одного типа.

**Поля:**
- `data: NonNull<T>` — указатель на arena-allocated массив
- `capacity: usize` — максимум элементов
- `count: usize` — текущее число элементов

**Ключевые методы:**
- `new(arena: &Arena, capacity: usize) -> Self` — выделяет `Layout::array::<T>(capacity)` через арену
- `with_default_capacity(arena: &Arena) -> Self` — capacity = `DEFAULT_COMPONENTS_PER_CHUNK` (1024)
- `add(&mut self, component: T) -> Option<usize>` — push в конец, O(1)
- `set(&mut self, index: usize, component: T) -> bool` — запись по индексу (⚠️ см. известные проблемы)
- `get(&self, index: usize) -> Option<&T>` / `get_mut`
- `as_slice() / as_mut_slice() -> &[T]` — для пакетной обработки/итерации
- `swap_remove(&mut self, index: usize) -> bool` — O(1) удаление с заменой последним
- `remove(&mut self, index: usize) -> bool` — O(n) удаление со сдвигом
- `clear(&mut self)` — вызывает `drop_in_place` для всех `count` элементов

**Drop:**
- ✅ Реализован — вызывает `clear()`, освобождая компоненты (но не освобождая память — она в арене)

**Известные проблемы:**
- 🐛 `set` (строки 65-90): при `index >= count` инкремент `count` происходит **до** проверки `if index < self.count`, поэтому `drop_in_place` может быть вызван на неинициализированной памяти. Логика должна сравнивать с **прошлым** count.
- ⚠️ `swap_remove` корректен, но в строке 215 `count - 1` работает только если ранний `if index >= self.count` отбросил `count == 0` (отбрасывает корректно)

---

### 1.3. MemFreeBlockMaster ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/free_mem_block.rs](../crates/boyko_ecs/src/ecs/memory/free_mem_block.rs)

**Тип:** `MemFreeBlockMaster`

Структура для отслеживания свободных блоков в арене. Поддерживает:
- Best-fit поиск (O(log n))
- Слияние соседних блоков (O(1) через `start_map`/`end_map`)
- Защагрузка от переиспользования слотов через `free_ind`

**Поля:**
- `blocks: Vec<MemFreeBlock>` — пул блоков (включая «удалённые», переиспользуются)
- `free_ind: Vec<usize>` — свободные слоты в `blocks` для переиспользования
- `mem_size_tree: BTreeMap<usize, Vec<usize>>` — индекс «size → блоки этого размера» (для best-fit)
- `start_map: HashMap<usize, usize>` — индекс «начало блока → его слот в blocks»
- `end_map: HashMap<usize, usize>` — индекс «конец блока → его слот»
- `size: usize` — число активных блоков

**Ключевые методы:**
- `new_init(arena_size: usize) -> Self` — создаёт мастер с одним блоком `[0, arena_size)`
- `insert(&mut self, block: MemFreeBlock)` — добавление с попыткой слияния через `try_merge_remove`
- `allocate(&mut self, size: usize) -> Option<MemFreeBlock>` — best-fit без alignment
- `allocate_aligned(&mut self, size: usize, align: usize) -> Option<MemFreeBlock>` — best-fit с выравниванием; излишек возвращается в пул
- `defragment(&mut self)` — компактирует `blocks` (убирает дыры от `free_ind`)
- `get_memory_stats() -> MemoryStats`

**Алгоритм best-fit:**
```rust
mem_size_tree.range(min_size..).next()
```
Первая запись в BTreeMap с размером ≥ min_size, O(log n).

**Алгоритм merge:**
При вставке блока `[s, e)`:
- Если `end_map[s]` существует → найден соседний слева блок `[s', s)`, мерджим
- Если `start_map[e]` существует → найден соседний справа блок `[e, e')`, мерджим

---

### 1.4. ComponentPool\<T\> ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs)

**Тип:** `ComponentPool<T: Component>`

Вектор пред-выделенных чанков для одного типа компонента. **Адаптивный размер чанка** в зависимости от `size_of::<T>()`:

| Размер T | Чанк | Source |
|----------|------|--------|
| ≤ 16 B (tiny) | 2048 элементов | `TINY_COMPONENTS_PER_CHUNK` |
| ≤ 64 B (small) | 1024 | `SMALL_COMPONENTS_PER_CHUNK` |
| ≤ 256 B (medium) | 512 | `MEDIUM_COMPONENTS_PER_CHUNK` |
| > 256 B (large) | 256 | `LARGE_COMPONENTS_PER_CHUNK` |

Решение в [component_pool.rs:76-87](../crates/boyko_ecs/src/ecs/memory/component_pool.rs).

**Поля:**
- `arena: NonNull<Arena>` — non-owning ссылка
- `chunks: Vec<Chunk<T>>` — пред-выделенные чанки
- `current_chunk_index: usize` — куда добавлять следующий компонент
- `count: usize` — общее число компонентов в пуле
- `capacity_per_chunk: usize`
- `component_id: usize`
- `_marker: PhantomData<T>`

**Ключевые методы:**
- `with_default_sizes(arena: &Arena) -> Self` — `DEFAULT_CHUNKS_PER_POOL` (128) чанков
- `add(&mut self, component: T) -> Option<UnitId>` — O(1), при заполнении текущего чанка переходит к следующему пред-выделенному
- `get(&self, index: UnitId) -> Option<&T>` / `get_mut`
- `swap_remove(&mut self, index: UnitId) -> bool`
- `chunk_components(&self, chunk_index: usize) -> Option<&[T]>` — slice для итерации

**Известные проблемы:**
- 🐛 `is_full()` (строка 202): `self.chunks.len() - 1` — underflow если `chunks` пуст
- ⚠️ `component_type_id`, `component_size`, `count`, `capacity` помечены `fn` (private), хотя выглядят как public API — возможно неполная реализация

---

### 1.5. UnitId ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/component_index.rs](../crates/boyko_ecs/src/ecs/memory/component_index.rs)

**Тип:** `#[derive(Debug, Clone, Copy, PartialEq, Eq)] UnitId { id_chunk: u32, id_inland: u32 }`

Двухуровневый индекс для адресации компонента в `ComponentPool<T>`. 8 байт, кэш-дружелюбен.

**Методы:** `new(chunk, inland)`, `chunk_index()`, `inland_index()`.

---

### 1.6. align_up (utils) ✅

**Файл:** [crates/boyko_ecs/src/ecs/memory/utils.rs](../crates/boyko_ecs/src/ecs/memory/utils.rs)

```rust
pub fn align_up(capacity: usize, cache_line_size: usize) -> usize {
    (capacity + cache_line_size - 1) & !(cache_line_size - 1)
}
```

Стандартный bit-trick для выравнивания вверх. Используется в `Arena::with_capacity` и `MemFreeBlockMaster::allocate_aligned`.

---

## 2. Component subsystem ✅

### 2.1. Component trait ✅

**Файл:** [crates/boyko_ecs/src/ecs/core/component.rs](../crates/boyko_ecs/src/ecs/core/component.rs)

```rust
pub type ComponentId = usize;

pub trait Component: 'static + Sized {
    fn component_id() -> ComponentId;
    fn debug_type_name() -> &'static str;
    fn type_id() -> TypeId;
    fn size() -> usize;
    fn alignment() -> usize;
}
```

Все методы `#[inline(always)]`. Реализация генерируется через `#[derive(Component)]`.

### 2.2. Component derive macro ✅

**Файл:** [crates/boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs)

```rust
static COMPONENT_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[proc_macro_derive(Component)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let component_id = COMPONENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    // генерирует impl с component_id() возвращающим этот id
}
```

**Особенности:**
- ID присваивается во время **компиляции** макроса — стабилен в рамках одного билда
- ⚠️ ID **нестабильны между сборками** — зависит от порядка раскрытия макросов компилятором
- Это означает: нельзя сериализовать `ComponentId` в файлы — после rebuild они могут означать другой тип

---

## 3. Entity subsystem ✅

**Файл:** [crates/boyko_ecs/src/ecs/core/entity.rs](../crates/boyko_ecs/src/ecs/core/entity.rs)

```rust
pub type EntityId = u32;

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Entity {
    pub id: EntityId,        // 4 bytes
    pub generation: u16,     // 2 bytes
}
// Total: 6 bytes (но с padding обычно 8)
```

**Методы:** `new(id, generation)`, `with_id(id)`, `id()`, `generation()`, `increment_generation()`, `is_same()`.

**Generation wrap-around:** `wrapping_add(1)` — после 65535 generation вернётся к 0. ⚠️ Это может вызывать ABA-проблему при долгом времени жизни — требует более широкого generation для production (u32 или u64).

**EntityMaster:** ⚠️ На master нет. На ветке `ecs` — есть полноценный `EntityMaster` с генерацией / переиспользованием ID.

---

## 4. Archetype subsystem 🚧

⚠️ Файл [crates/boyko_ecs/src/ecs/core/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype.rs) на `master` **пуст**.

На ветке `ecs`:
- `archetype/archetype.rs` — `Archetype` с component_pools, signature, entity_ids
- `archetype/archetype_master.rs` — управление всеми архетипами
- `archetype/archetype_registry.rs` — поиск по signature/mask
- `archetype/archetype_signature.rs` — wrapper над `ComponentMask`
- `archetype/archetype_bundle.rs` — bundle компонентов для batch-операций

Для просмотра: `git show origin/ecs:crates/boyko_ecs/src/ecs/core/archetype/archetype.rs`

---

## 5. EcsMaster (top-level API) 🚧

⚠️ Файл [crates/boyko_ecs/src/ecs/core/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master.rs) на `master` **пуст**.

На ветке `ecs` (`crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs`):
```rust
pub struct EcsMaster {
    entity_master: EntityMaster,
    archetype_master: ArchetypeMaster,
    arena: Arena,
}
```

API: `create_archetype`, `get_or_create_archetype`, `create_entity`, `delete_entity`, ...

Использует `anyhow::Result` — спорно для библиотечного кода, **обсудить с архитектором при merge**.

---

## 6. Query subsystem 🚧

На ветке `ecs`:
- `core/iters/query.rs` — `Query<'a>` хранит `Vec<&'a Archetype>`
- `core/iters/component_set.rs` — компоненты для query
- `core/iters/sparse_iter.rs` — итератор по sparse-данным

Конструкторы Query: `with_component_ids`, `with_mask`, `with_exact_mask`, `from_archetypes`.

---

## 7. Event subsystem 🚧

На ветке `ecs`:
- `core/events/event.rs` — `Event` trait с `Participants` и `Parameters` ассоциированными типами
- `core/events/event_pool.rs` — пул событий
- `core/events/event_pool_bundle.rs` — связка пулов разных типов
- `core/events/event_registry.rs` — регистр
- `core/events/participants/` — участники (entities)
- `core/events/parameters/` — параметры события

---

## 8. BitSet utilities 🚧

На ветке `ecs` в `crates/boyko_utils/`:
- `bit_mask/bit_mask.rs` — высокоуровневая `ComponentMask`
- `bit_mask/bit_set.rs` — общий битсет
- `bit_mask/bit_set512.rs` — фиксированный 512-битный сет (8×u64)
- `bit_mask/bit_storage.rs` — нижний слой хранения

---

## 9. Запланированные подсистемы 📋

Эти подсистемы планируются, но архитектурно не описаны:

- **Scheduler / System runner** — выполнение пользовательских систем, dependency graph, work-stealing
- **Change detection** — отслеживание изменений компонентов (tick counters / version-based)
- **Resource management** — глобальные ресурсы (`Resources` в Bevy / singletons)
- **Command buffer** — отложенные операции (`commands.spawn(...)` в Bevy)
- **Serialization** — сохранение/загрузка миров
- **Hot-reload** — не цель проекта

---

## 10. Константы (constants.rs) ✅

**Файл:** [crates/boyko_ecs/src/ecs/constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)

| Константа | Значение | Использование |
|-----------|----------|---------------|
| `DEFAULT_ARENA_SIZE` | 64 MB | `Arena::new()` |
| `CACHE_LINE_SIZE` | 64 B | `Arena::with_capacity`, выравнивание |
| `MIN_ALIGNMENT` | 8 B | ⚠️ не используется |
| `DEFAULT_COMPONENTS_PER_CHUNK` | 1024 | `Chunk::with_default_capacity` |
| `DEFAULT_CHUNKS_PER_POOL` | 128 | `ComponentPool::with_default_sizes` |
| `TINY/SMALL/MEDIUM/LARGE_COMPONENTS_PER_CHUNK` | 2048 / 1024 / 512 / 256 | `ComponentPool::get_optimal_chunk_capacity` |
| `TINY/SMALL/MEDIUM_COMPONENT_THRESHOLD` | 16 / 64 / 256 B | Same |
| `INITIAL_ENTITY_CAPACITY` | 1024 | ⚠️ не используется |
| `GROWTH_FACTOR` | 1.5 | ⚠️ не используется |
| `MAX_EXPANSION_FACTOR` | 8 | ⚠️ не используется |
| `COMPACTION_THRESHOLD` | 0.25 | ⚠️ не используется |
| `MIN_COMPONENTS_FOR_COMPACTION` | 16 | ⚠️ не используется |
| `INITIAL_FREE_SLOTS_CAPACITY` | 1024 | ⚠️ не используется (в коде литерал `1024` в `MemFreeBlockMaster::new`) |
| `MAX_EMPTY_CHUNKS_RATIO` | 0.2 | ⚠️ не используется |

Половина констант не используется — заготовка под будущую логику (компактация, expansion).

---

## Стиль и conventions проекта

- **Языки комментариев:** микс — [chunk.rs](../crates/boyko_ecs/src/ecs/memory/chunk.rs) на русском, [component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) на английском. ⚠️ стоит унифицировать
- **Doc-комменты** через `///`
- **Internal комменты** через `//` или `/* ... */`
- **`#[inline(always)]`** — на trait-аксессорах `Component`, в `align_up`
- **`#[inline]`** — на конструкторах `Entity`, `UnitId`
