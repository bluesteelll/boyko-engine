//! `#[derive(Bundle)]` implementation.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, Type, parse_macro_input};

/// Implementation of `#[derive(Bundle)]` (see the public entry in `lib.rs`).
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    /// Maximum component count for a derived `Bundle` (Phase 22: 8 → 16).
    ///
    /// Kept in lock-step with the `MAX_BUNDLE_ARITY` stack-collector ceilings
    /// in `boyko_ecs` (`spawn_at_command.rs` / `insert_command.rs` /
    /// `migration_helpers.rs`): rejecting wider bundles at macro time makes
    /// the runtime debug_asserts unreachable for derived bundles.
    const MAX_BUNDLE_ARITY: usize = 16;

    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_span = name.span();

    // SBC1 / Phase 8.5 scope: reject generics outright. The per-impl
    // `static INFO: OnceLock<BundleStaticInfo>` works only when the impl is
    // non-generic — otherwise monomorphization would create one static per
    // (B, T1, ..., Tn) tuple, defeating the cache and breaking SBC2.
    if !input.generics.params.is_empty() {
        return syn::Error::new(
            name_span,
            "Bundle derive does not support generics (Phase 8.5 scope)",
        )
        .to_compile_error()
        .into();
    }

    let data = match &input.data {
        Data::Struct(s) => s,
        Data::Enum(_) | Data::Union(_) => {
            return syn::Error::new(
                name_span,
                "Bundle can only be derived for structs",
            )
            .to_compile_error()
            .into();
        }
    };

    let fields: Vec<BundleField> = match &data.fields {
        Fields::Named(named) => named
            .named
            .iter()
            .enumerate()
            .map(|(idx, f)| BundleField {
                local_ident: format_ident!("__bundle_field_{}", idx),
                accessor: {
                    let ident = f.ident.clone().expect("named field");
                    quote! { self.#ident }
                },
                ty: f.ty.clone(),
            })
            .collect(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(idx, f)| {
                let idx_lit = syn::Index::from(idx);
                BundleField {
                    local_ident: format_ident!("__bundle_field_{}", idx),
                    accessor: quote! { self.#idx_lit },
                    ty: f.ty.clone(),
                }
            })
            .collect(),
        Fields::Unit => {
            return syn::Error::new(
                name_span,
                "Bundle requires at least one field; \
                 to spawn an entity with zero components use Commands::spawn_empty()",
            )
            .to_compile_error()
            .into();
        }
    };

    if fields.is_empty() {
        // Defensive: tuple struct `Foo()` and named struct `Foo {}` both
        // arrive here with zero fields. Treat identically to unit struct.
        return syn::Error::new(
            name_span,
            "Bundle requires at least one field; \
             to spawn an entity with zero components use Commands::spawn_empty()",
        )
        .to_compile_error()
        .into();
    }

    // Phase 22: hard arity ceiling, mirrored by the runtime stack collectors.
    if fields.len() > MAX_BUNDLE_ARITY {
        return syn::Error::new(
            name_span,
            format!(
                "Bundle supports at most {MAX_BUNDLE_ARITY} components (MAX_BUNDLE_ARITY); \
                 split the bundle and insert the remainder with EntityCommands::insert"
            ),
        )
        .to_compile_error()
        .into();
    }

    let n_fields = fields.len();

    // Per-field token fragments, indexed in declaration order.
    let field_types: Vec<&Type> = fields.iter().map(|f| &f.ty).collect();
    let field_locals: Vec<&Ident> = fields.iter().map(|f| &f.local_ident).collect();
    let field_accessors: Vec<&TokenStream2> = fields.iter().map(|f| &f.accessor).collect();

    // §6.1 build_info: each field's `T::component_id()`.
    let component_id_exprs: Vec<TokenStream2> = field_types
        .iter()
        .map(|ty| {
            quote! {
                <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id()
            }
        })
        .collect();

    // §6.3 sort-array entries: (ComponentId, *const u8, usize) triples derived
    // from the ManuallyDrop locals. C5: pointer + length (not &[u8]) sidesteps
    // E0521 (MaybeUninit/array lifetime invariance) — we materialize the slice
    // inside the dispatch loop via slice::from_raw_parts.
    let sort_entries: Vec<TokenStream2> = fields
        .iter()
        .map(|f| {
            let ty = &f.ty;
            let local = &f.local_ident;
            quote! {
                (
                    <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id(),
                    &raw const *#local as *const u8,
                    ::std::mem::size_of::<#ty>(),
                )
            }
        })
        .collect();

    // Phase 22.1 D-E: per-field push fragments for the `data`-only walk. Each
    // push is wrapped in `if size_of::<FieldTy>() != 0 { ... }`. Because the
    // size is a monomorphisation-time constant, the branch folds entirely:
    // a ZST field's entry never enters the array, so the subsequent sort and
    // dispatch loop run over data columns only — the ZST byte-copy is elided
    // BEFORE the runtime sort (unlike a post-sort `bytes.is_empty()` guard,
    // which would launder into a per-column-per-row runtime branch).
    let data_push_stmts: Vec<TokenStream2> = fields
        .iter()
        .map(|f| {
            let ty = &f.ty;
            let local = &f.local_ident;
            quote! {
                if ::std::mem::size_of::<#ty>() != 0 {
                    // SAFETY (C5 / §6.3, identical to `for_each_component_bytes`):
                    //   `__data_len < #n_fields` because at most `#n_fields`
                    //   entries are ever pushed (one per field, and only when
                    //   non-ZST). `&raw const *#local` is a valid `*const u8`
                    //   for `size_of::<#ty>()` bytes for this function's scope.
                    unsafe {
                        *__data_sorted.get_unchecked_mut(__data_len) = (
                            <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id(),
                            &raw const *#local as *const u8,
                            ::std::mem::size_of::<#ty>(),
                        );
                    }
                    __data_len += 1;
                }
            }
        })
        .collect();

    // `where T: Component` for every field — gives a sharper diagnostic than
    // letting the `component_id()` reference fail down in the impl body. Per
    // step spec acceptance §9 Step 4 bullet "Bound check".
    let component_bounds: Vec<TokenStream2> = field_types
        .iter()
        .map(|ty| {
            quote! {
                #ty: ::boyko_ecs::ecs::core::component::component::Component
            }
        })
        .collect();

    // Decision 4 (D4b): the typed-write threshold. Mirrors
    // `boyko_ecs::ecs::core::bundle::MAX_TYPED_WRITE_ARITY` (the macro cannot
    // import boyko_ecs; the cross-check `const _: () = assert!(... == 16)` in
    // bundle.rs pins this literal in lock-step).
    const MAX_TYPED_WRITE_ARITY: usize = 16;
    let has_typed_write = n_fields <= MAX_TYPED_WRITE_ARITY;

    // Decision 4 (W3): per-field perm-build statements for `write_row_perm`.
    // For each DECLARATION field `k`:
    //   - ZST field (`size_of::<Tk>() == 0`) → `out_perm[k] = PERM_SKIP`
    //     (const-folds: the branch picks the SKIP arm at monomorphisation);
    //   - otherwise find `Tk::component_id()`'s position in the canonical
    //     `data_component_ids` slice (linear scan; arity is ≤ 16) and store it
    //     as the canonical data-column slot. Correct-by-construction IDENTITY:
    //     the slot is keyed on the field's ComponentId, never on size.
    let perm_build_stmts: Vec<TokenStream2> = fields
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let ty = &f.ty;
            quote! {
                if ::std::mem::size_of::<#ty>() == 0 {
                    out_perm[#k] =
                        ::boyko_ecs::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP;
                } else {
                    let __cid = <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id();
                    let mut __slot = usize::MAX;
                    let mut __i = 0usize;
                    while __i < data_component_ids.len() {
                        if data_component_ids[__i] == __cid {
                            __slot = __i;
                            break;
                        }
                        __i += 1;
                    }
                    debug_assert!(
                        __slot != usize::MAX,
                        "write_row_perm: field {} ComponentId not found in canonical data_component_ids", #k
                    );
                    debug_assert!(
                        __slot < (::boyko_ecs::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP as usize),
                        "write_row_perm: data-column slot exceeds PERM_SKIP sentinel"
                    );
                    out_perm[#k] = __slot as u8;
                }
            }
        })
        .collect();

    // Decision 4: per-field typed-store statements for `write_row_typed`,
    // const-unrolled in DECLARATION order. Each field `k`:
    //   - skips at compile time if `size_of::<Tk>() == 0` (ZST const-folds out
    //     — the whole `if` block disappears, matching the byte path's ZST
    //     elision and the per-batch `data_pool_ids` filter);
    //   - otherwise relocates field `k` via `ManuallyDrop::take` (suppressing
    //     the source Drop — B4/O1) into its destination column with a
    //     FIXED-WIDTH `ptr::write::<Tk>` store at `base.add(row * STRIDE_k)`.
    // The destination column slot is `dst.perm(k)` (declaration field → data
    // column slot, built once per batch by the caller). W3 IDENTITY assert +
    // Q1 row-bound assert are debug-only.
    let typed_write_stmts: Vec<TokenStream2> = fields
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let ty = &f.ty;
            let local = &f.local_ident;
            quote! {
                if ::std::mem::size_of::<#ty>() != 0 {
                    // Declaration field `k` → its canonical data-column slot.
                    let __slot = dst.perm(#k) as usize;
                    debug_assert!(
                        __slot != (::boyko_ecs::ecs::core::bundle::BundleColumnPtrs::PERM_SKIP as usize),
                        "write_row_typed: non-ZST field {} mapped to PERM_SKIP", #k
                    );
                    // W3 IDENTITY: the resolved column's ComponentId must equal
                    // this field's ComponentId — two same-size, different-type
                    // components must NEVER silently swap columns.
                    #[cfg(debug_assertions)]
                    debug_assert_eq!(
                        dst.column_comp_id(__slot),
                        <#ty as ::boyko_ecs::ecs::core::component::component::Component>::component_id(),
                        "write_row_typed: column identity mismatch for field {}", #k
                    );
                    // Stride parity: the resolved column's stride must equal the
                    // field's size (the registry layout reflects `size_of::<Tk>()`).
                    #[cfg(debug_assertions)]
                    debug_assert_eq!(
                        dst.column_stride(__slot),
                        ::std::mem::size_of::<#ty>(),
                        "write_row_typed: column stride mismatch for field {}", #k
                    );
                    // Q1: restore the `idx < committed_rows` guard the byte path
                    // has at write_at_unchecked_initialized (the typed path
                    // bypasses it).
                    #[cfg(debug_assertions)]
                    debug_assert!(
                        row < dst.column_committed_rows(__slot),
                        "write_row_typed: row {} >= committed_rows for field {}", row, #k
                    );
                    // SAFETY (D4 §"Unsafe delta"): the caller's `write_row_typed`
                    //   contract (1)-(4) guarantees:
                    //   - `dst.column_base(__slot)` carries write-capable,
                    //     unaliased provenance for this column resolved once per
                    //     batch under a single &mut that has ended (W2);
                    //   - `row < committed_rows` (debug-asserted above; the
                    //     caller pre-grew via reserve_capacity);
                    //   - `base.add(row * size_of::<Tk>())` is in-bounds of the
                    //     column data sub-region and `Tk`-aligned (column-base
                    //     alignment contract);
                    //   - the slot is uninit (commit happens post-loop, B4), so
                    //     no Drop runs on the destination;
                    //   - `ManuallyDrop::take` performs a bitwise relocation that
                    //     suppresses the source Drop and cannot panic (no user
                    //     code), so this body is panic-free — the only batch
                    //     panic source is `iter.next()` between rows.
                    unsafe {
                        let __dst_ptr = dst
                            .column_base(__slot)
                            .add(row * ::std::mem::size_of::<#ty>())
                            as *mut #ty;
                        ::std::ptr::write(
                            __dst_ptr,
                            ::std::mem::ManuallyDrop::take(&mut #local),
                        );
                    }
                }
            }
        })
        .collect();

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::bundle::bundle::sealed::BundleSealed for #name {}

        impl ::boyko_ecs::ecs::core::bundle::bundle::Bundle for #name
        where
            #(#component_bounds),*
        {
            fn static_info() -> &'static ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo {
                // O3 coalesced static (Decision SBC-D5). One OnceLock holds
                // BundleTypeId + canonical-sorted component_ids slice. Cached
                // path: single Acquire load.
                static INFO: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo
                > = ::std::sync::OnceLock::new();

                INFO.get_or_init(|| {
                    // B1 canonical order: collect declaration-order IDs into a
                    // fixed-size stack array, sort ascending by ComponentId.0,
                    // then leak the boxed array to obtain a `&'static` slice.
                    // Leak is bounded by SBC8 (one slice per Bundle type per
                    // process — at most MAX_BUNDLE_TYPES × N_max × 8 B).
                    let mut arr: [
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId;
                        #n_fields
                    ] = [#(#component_id_exprs),*];
                    arr.sort_unstable_by_key(|id| id.0);
                    let leaked: &'static [
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId;
                        #n_fields
                    ] = ::std::boxed::Box::leak(::std::boxed::Box::new(arr));

                    ::boyko_ecs::ecs::core::bundle::bundle::BundleStaticInfo {
                        // BundleTypeId minted exactly once per Bundle type per
                        // process — OnceLock::get_or_init enforces single
                        // winner across threads (§7.3).
                        type_id: ::boyko_ecs::ecs::core::bundle::bundle_type_registry::register_new(),
                        component_ids: leaked.as_slice(),
                    }
                })
            }

            fn cached_archetype_id(
                world: &mut ::boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster,
            ) -> ::boyko_ecs::ecs::identifiers::primitives::ArchetypeId {
                // Delegate to the per-world cache helper. The helper performs
                // the hot-path Acquire load on `bundle_archetype_cache[id.0]`
                // and falls back to a cold ArchetypeMaster registration on
                // the first call per (Bundle, world) pair (§6.2).
                world.bundle_archetype_id_for::<Self>()
            }

            fn for_each_component_bytes<F>(self, mut f: F)
            where
                F: ::std::ops::FnMut(
                    ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                    &[u8],
                ),
            {
                // §6.3 MANDATORY codegen template — C5 pointer-based pattern.
                //
                // Step 1: ManuallyDrop-wrap EVERY destructured field UPFRONT,
                // before any callback can run. This is the B4 panic-safety
                // contract: on callback panic mid-iteration, the remaining
                // fields' `Drop` impls are suppressed unconditionally (they
                // leak — never double-drop alongside archetype-side ownership).
                #(
                    let #field_locals = ::std::mem::ManuallyDrop::new(#field_accessors);
                )*

                // Step 2: build the sort array as (ComponentId, *const u8,
                // usize). The *const u8 + len triple sidesteps E0521 — the
                // borrow checker treats `&[u8]` as lifetime-invariant inside
                // array/MaybeUninit contexts, but raw pointers are fine. The
                // slice is reconstructed inside the dispatch loop.
                let mut sorted: [
                    (
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                        *const u8,
                        usize,
                    );
                    #n_fields
                ] = [#(#sort_entries),*];

                // Step 3: B1 canonical sort. unstable acceptable because
                // ComponentId values are unique per Bundle (a Bundle that
                // declares the same Component twice fails at archetype
                // registration, not here).
                sorted.sort_unstable_by_key(|(id, _, _)| id.0);

                // Step 4: dispatch in canonical order, materializing the
                // shared byte slice on each iteration.
                for &(id, ptr, len) in &sorted {
                    // SAFETY (C5 / §6.3):
                    //   (i)   `ptr` was derived from `&raw const *ManuallyDrop<T>`,
                    //         where T is a live stack local in this function — ptr
                    //         is valid for `len = size_of::<T>()` bytes for the
                    //         duration of this loop.
                    //   (ii)  `len` is exactly `size_of::<T>()` matching the
                    //         component type — no over-read.
                    //   (iii) The slice we materialize is shared (immutable) and
                    //         non-overlapping with any other live borrow: each
                    //         ManuallyDrop local is borrowed exactly once in this
                    //         scope (via the iter slot above).
                    //   (iv)  ManuallyDrop suppresses Drop on the local
                    //         unconditionally at end-of-scope (does not "leak"
                    //         semantically — never invokes Drop). For components
                    //         that the callback successfully consumed (memcpy'd
                    //         into ECS storage via create_entity), ownership has
                    //         transferred to the archetype, and that storage now
                    //         owns the eventual Drop on entity despawn. For
                    //         components that the callback did not reach because
                    //         `f` panicked on an earlier iteration, their bytes
                    //         remain in the stack ManuallyDrop locals and leak
                    //         unconditionally — Drop is suppressed regardless of
                    //         panic state. This is the documented B4 panic-safety
                    //         guarantee: panic → leak, never double-drop.
                    let bytes: &[u8] = unsafe { ::std::slice::from_raw_parts(ptr, len) };
                    f(id, bytes);
                }
            }

            fn for_each_data_component_bytes<F>(self, mut f: F)
            where
                F: ::std::ops::FnMut(
                    ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                    &[u8],
                ),
            {
                // Phase 22.1 D-E: identical to `for_each_component_bytes`
                // EXCEPT zero-size (ZST tag) fields are filtered out at
                // monomorphisation (the `size_of::<FieldTy>() != 0` guards
                // below const-fold). The callback is invoked once per
                // NON-ZST component in canonical `ComponentId.0` order.
                //
                // B4 panic-safety is unchanged: EVERY field (ZST or not) is
                // ManuallyDrop-wrapped upfront, so a callback panic leaks the
                // remaining fields' bytes rather than double-dropping. ZST
                // fields carry no bytes — their `ManuallyDrop` is a no-op —
                // but the wrap is uniform to keep the contract obvious.
                #(
                    let #field_locals = ::std::mem::ManuallyDrop::new(#field_accessors);
                )*

                // Worst-case-sized stack array (all fields non-ZST). The
                // const-folded push guards keep `__data_len` at exactly the
                // non-ZST field count; only `__data_sorted[..__data_len]` is
                // ever read. A single placeholder initialiser keeps the array
                // a plain `[T; N]` (no MaybeUninit churn) — every written slot
                // is overwritten before use.
                let mut __data_sorted: [
                    (
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId,
                        *const u8,
                        usize,
                    );
                    #n_fields
                ] = [
                    (
                        ::boyko_ecs::ecs::identifiers::primitives::ComponentId(0),
                        ::std::ptr::null(),
                        0usize,
                    );
                    #n_fields
                ];
                let mut __data_len: usize = 0;
                #(#data_push_stmts)*

                // B1 canonical sort over the populated prefix only.
                __data_sorted[..__data_len].sort_unstable_by_key(|(id, _, _)| id.0);

                for &(id, ptr, len) in &__data_sorted[..__data_len] {
                    debug_assert!(len != 0, "ZST entry leaked into the data walk");
                    // SAFETY (C5 / §6.3): `ptr` was derived from
                    //   `&raw const *ManuallyDrop<T>` for a NON-ZST live stack
                    //   local; it is valid for `len = size_of::<T>()` bytes for
                    //   the duration of this loop. The slice is shared,
                    //   non-overlapping (each local borrowed once), and Drop is
                    //   suppressed by ManuallyDrop (B4: panic → leak, never
                    //   double-drop). Identical invariants to
                    //   `for_each_component_bytes`.
                    let bytes: &[u8] = unsafe { ::std::slice::from_raw_parts(ptr, len) };
                    f(id, bytes);
                }
            }

            // Decision 4 (D4b): every derived bundle within the arity ceiling
            // takes the typed fixed-width write path. The retained byte path
            // (`for_each_data_component_bytes`) is the fallback for any future
            // lowering of the threshold.
            const HAS_TYPED_WRITE: bool = #has_typed_write;

            fn write_row_perm(
                data_component_ids: &[::boyko_ecs::ecs::identifiers::primitives::ComponentId],
                out_perm: &mut [u8],
            ) {
                debug_assert!(
                    out_perm.len() >= #n_fields,
                    "write_row_perm: out_perm too short for declaration arity"
                );
                #(#perm_build_stmts)*
            }

            unsafe fn write_row_typed(
                self,
                dst: &::boyko_ecs::ecs::core::bundle::BundleColumnPtrs,
                row: usize,
            ) {
                // B4 / O1 drop-suppression discipline (identical to
                // `for_each_component_bytes`): ManuallyDrop-wrap EVERY field
                // UPFRONT, before any relocation runs. `write_row_typed` is
                // panic-free internally (ptr::write / ManuallyDrop::take cannot
                // panic), so on this path the wrap simply makes the move-out
                // explicit — each field is relocated exactly once into its
                // column slot and never dropped at end-of-scope.
                #(
                    let mut #field_locals =
                        ::std::mem::ManuallyDrop::new(#field_accessors);
                )*

                // Const-unrolled, DECLARATION-order, fixed-width stores. ZST
                // fields const-fold out (their `if size_of != 0` block
                // disappears at monomorphisation).
                #(#typed_write_stmts)*
            }
        }
    };

    expanded.into()
}

/// Internal helper: a single destructured Bundle field.
///
/// `accessor` carries the original `self.<ident>` or `self.<index>` token
/// stream (so the same struct shape is faithfully reproduced inside the
/// derive output). `local_ident` is the synthetic `__bundle_field_N` ident
/// used as the ManuallyDrop binding — uniform across named and tuple
/// structs to keep the generated code identical in shape.
struct BundleField {
    local_ident: Ident,
    accessor: TokenStream2,
    ty: Type,
}
