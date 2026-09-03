//! The `.ui` component VOCABULARY — one declaration the parser, the writer and
//! the gates all read (P3 Decision 3, closed-vocabulary dispatch).
//!
//! # Why this module exists
//!
//! The format previously carried the same closed set in FOUR hand-maintained
//! places: the parser's `match` in [`dispatch`](crate::text::dispatch), the
//! writer's chain of `if let Some(..)` in [`serialize`](crate::text::serialize),
//! the hot-reload patcher in
//! [`reconcile`](crate::reload::reconcile), and a corpus in the round-trip test.
//! Four lists of one vocabulary diverge silently, and they did: the writer
//! emitted 8 of the 20 non-exempt names while `serialize → parse → serialize`
//! stayed byte-identical, because a component the writer never writes never
//! enters that comparison; and the reconcile patched 10 of the 21, ignoring ten
//! of the eleven it left out (only `ComputedRect` was left out on purpose), so
//! editing a button's action or a label's colour in a live `.ui` file changed
//! nothing on a node that survived the reload.
//!
//! The cure is the one this repository has already measured (`ui_pack_inputs!`,
//! `boyko_log`'s `codes!`): declare the set ONCE and route every consumer through
//! an EXHAUSTIVE `match` on it, so the compiler — not a reviewer — reds a
//! consumer that forgot the new member (`E0004`). The parser's dispatch, the
//! writer's per-node emitter and the reconcile's per-node patcher all match on
//! [`UiTextComponent`]; adding a variant below fails the build in all three until
//! each is handled.
//!
//! Exhaustiveness cannot see an arm that is present but EMPTY, so the compile-time
//! gate is paired with behavioural ones: `p3_world_round_trip.rs` builds a world
//! holding every [`WriterPolicy::Emit`] member with non-default values, serializes
//! it, re-spawns the text into a SECOND world and compares the two WORLDS;
//! `p3_reload_patch_vocabulary.rs` reloads a document whose every authored value
//! moved and compares the patched survivor against a fresh spawn of the same
//! text. A silently empty arm reds there.
//!
//! # The name invariant
//!
//! A variant's Rust identifier IS its `.ui` text name, and by the P3 invariant
//! that is also the component's Rust type name ([`UiTextComponent::name`] is
//! `stringify!` of the variant). `UiName` is deliberately NOT a member: it is
//! authored by the `#name` sigil, never as a component literal.

/// Whether the canonical writer emits a vocabulary member, or omits it on purpose.
///
/// An omission carries its REASON in the type, so "the writer does not write this"
/// is a decision recorded at the declaration rather than an absence a reader has to
/// notice (ERG-04: the sentinel is a type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriterPolicy {
    /// The writer emits this component whenever the node carries it.
    Emit,
    /// The writer deliberately never emits it; the payload is why.
    Omit(&'static str),
}

impl WriterPolicy {
    /// `true` when the canonical writer is required to emit this component.
    #[inline]
    pub const fn is_emitted(self) -> bool {
        matches!(self, WriterPolicy::Emit)
    }
}

/// What the hot-reload reconcile does with a vocabulary member on a node that
/// SURVIVES a reload.
///
/// The `.ui` file is the document's source of truth for everything it can author,
/// so the default is [`ReloadPolicy::PatchAndRemove`]: an edited value lands, and
/// a line the author DELETED removes the component. The two departures from that
/// carry their reason in the type rather than in a reader's memory (ERG-04: the
/// sentinel is a type), exactly as [`WriterPolicy::Omit`] does for the writer.
///
/// The declaration is what the behavioural census in
/// `p3_reload_patch_vocabulary.rs` reads: the compiler proves every member is
/// HANDLED (the patcher's `match` is exhaustive), and the census proves each is
/// handled the way the policy here says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadPolicy {
    /// The value is patched set-if-changed, and a component the file DROPS is
    /// removed from the node.
    PatchAndRemove,
    /// The value is patched, but an omission PRESERVES the live component; the
    /// payload is why.
    PatchKeepOnOmit(&'static str),
    /// The reconcile never touches this member on a survivor; the payload is why.
    Skip(&'static str),
}

impl ReloadPolicy {
    /// `true` when the reconcile must re-parse this member from the text and set
    /// it on a survivor whose authored value changed.
    #[inline]
    pub const fn is_patched(self) -> bool {
        matches!(self, ReloadPolicy::PatchAndRemove | ReloadPolicy::PatchKeepOnOmit(_))
    }

    /// `true` when deleting this member's line from the file removes the
    /// component from the node.
    #[inline]
    pub const fn removes_on_omit(self) -> bool {
        matches!(self, ReloadPolicy::PatchAndRemove)
    }
}

/// Declares the vocabulary once: the enum, the `ALL` roster, the text name, the
/// writer policy and the reload policy are all generated from a single list, so
/// they cannot diverge. Members are separated by `;` because each carries two
/// comma-bearing policy expressions.
macro_rules! ui_text_vocabulary {
    ($( $(#[$meta:meta])* $variant:ident => writer: $policy:expr, reload: $reload:expr );+ $(;)?) => {
        /// One member of the `.ui` closed component vocabulary.
        ///
        /// Every consumer matches on this EXHAUSTIVELY, so a new member is a
        /// compile error (`E0004`) in the parser and in the writer until both
        /// handle it. See the module docs for why exhaustiveness alone is not
        /// enough and what the behavioural gate adds.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum UiTextComponent {
            $( $(#[$meta])* $variant, )+
        }

        impl UiTextComponent {
            /// Every member, in the canonical order the writer emits them.
            pub const ALL: &'static [UiTextComponent] = &[ $( UiTextComponent::$variant ),+ ];

            /// The member's `.ui` text name — identical to its Rust type name.
            #[inline]
            pub const fn name(self) -> &'static str {
                match self {
                    $( UiTextComponent::$variant => stringify!($variant), )+
                }
            }

            /// Whether the canonical writer emits this member (and, if not, why).
            #[inline]
            pub const fn writer_policy(self) -> WriterPolicy {
                match self {
                    $( UiTextComponent::$variant => $policy, )+
                }
            }

            /// What the hot-reload reconcile does with this member on a node
            /// that survives a reload (and, where it departs from the uniform
            /// rule, why).
            #[inline]
            pub const fn reload_policy(self) -> ReloadPolicy {
                match self {
                    $( UiTextComponent::$variant => $reload, )+
                }
            }

            /// Resolves a text name to its member, or `None` for a name outside the
            /// vocabulary (the parser's "unknown component" path).
            #[inline]
            pub fn from_name(name: &str) -> Option<UiTextComponent> {
                match name {
                    $( stringify!($variant) => Some(UiTextComponent::$variant), )+
                    _ => None,
                }
            }
        }
    };
}

ui_text_vocabulary! {
    UiLayout => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchKeepOnOmit(
        "the node's head line — a node REQUIRES a `UiLayout`, so a document that \
         dropped it is degenerate rather than an authored removal (the live one stays)",
    );
    UiSpacing => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiAlign => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiAbsolute => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    ContentSize => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    StackIndex => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    ComputedClip => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiRoot => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiText => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiImage => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiGrid => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    UiAnchor => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    Button => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    Bar => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    BarFill => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    OnClick => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    OnHover => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    OnSubmit => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchAndRemove;
    /// The two data-bind columns are re-inserted from the text on every reload,
    /// but a bind DELETED from the file is preserved — see the `PatchKeepOnOmit`
    /// reason and `reconcile::patch_bind_text`.
    BindText => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchKeepOnOmit(
        "preserve-by-omission, inherited from GUI #27 and NOT re-decided by the \
         pass that closed the patch vocabulary: removing a bind on reload changes \
         LIVE binding semantics, so it needs its own decision and its own gate",
    );
    BindValue => writer: WriterPolicy::Emit, reload: ReloadPolicy::PatchKeepOnOmit(
        "preserve-by-omission, same decision as `BindText`",
    );
    /// Layout OUTPUT, not authored state: `spawn_ui_tree` seeds it and the very
    /// next `ui_layout_apply` overwrites it, so writing it back would pin a
    /// stale rect into the document (P3 Decision 14 / §6).
    ComputedRect => writer: WriterPolicy::Omit(
        "layout output — a spawn-time seed `ui_layout_apply` overwrites (Decision 14)",
    ), reload: ReloadPolicy::Skip(
        "layout output — patching a survivor from the document would pin a stale \
         rect the next layout pass overwrites anyway (Decision 14)",
    );
}

impl UiTextComponent {
    /// The members the canonical writer must emit, in canonical order.
    #[inline]
    pub fn emitted() -> impl Iterator<Item = UiTextComponent> {
        Self::ALL.iter().copied().filter(|c| c.writer_policy().is_emitted())
    }

    /// The members the hot-reload reconcile must patch on a survivor, in
    /// canonical order.
    #[inline]
    pub fn patched() -> impl Iterator<Item = UiTextComponent> {
        Self::ALL.iter().copied().filter(|c| c.reload_policy().is_patched())
    }
}
