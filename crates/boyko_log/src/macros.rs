//! The five emission macros and their three-gate expansion.
//!
//! # The gate chain, and why it is `&&`
//!
//! ```text
//! if T::STATIC_CEILING as u8 >= LVL as u8            // (a) const: per-target compile ceiling
//!     && $crate::GLOBAL_CEILING as u8 >= LVL as u8   // (b) const: per-profile compile ceiling
//!     && $crate::runtime_ceiling(T::ID) >= LVL as u8 // (c) one Relaxed byte load
//! { … }
//! ```
//!
//! The `&&` short-circuit is the guarantee: **argument expressions are never evaluated when a
//! gate is false.** That is what makes `debug!(Ecs, "{}", expensive())` free in a shipping build
//! rather than merely quiet. It is also why the arguments must sit inside the `if`, never in a
//! `let` above it — a refactor that hoists them is not a style change, it deletes the whole
//! property, and no test of the *output* would notice.
//!
//! # What (a)+(b) buy that (c) cannot
//!
//! Gates (a) and (b) are **compile-time ceilings**: a `const false` short-circuits the chain and
//! the arm *and its operands* are deleted — no branch, no symbol, no argument evaluation. Gate (c)
//! is the **runtime flag**, and it is the site's floor: one `.bss` byte load plus one branch the
//! predictor always gets right, at every surviving site, in every frame, forever. A flag has to be
//! *read* in order to be a flag, so turning it off cannot drive gate (c) to zero. That is the
//! entire reason the design keeps two axes instead of one, and why a shipped binary can still be
//! asked for a log while a compile-only design cannot.
//!
//! # The per-site `static`
//!
//! Each expansion places one `static LogSite` beside the call. The record carries an 8-byte
//! pointer to it instead of re-carrying file, line, format literal and code — and none of that
//! data is touched on the emitting thread. `line!()` and `file!()` are expanded at the **call**
//! site, which is why they are in the macro rather than in `emit_impl`.

/// Build the per-site `static` and hand it to the producer path.
///
/// Internal. Kept as one macro so the five level macros cannot drift apart in the shape of the
/// site they declare — the failure mode being a copy-paste that leaves one level's `class` byte
/// or `fields` slice stale.
#[doc(hidden)]
#[macro_export]
macro_rules! __log_site_emit {
    ($lvl:expr, $T:ty, $class:expr, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {{
        static __BOYKO_LOG_SITE: $crate::LogSite = $crate::LogSite {
            target: ::core::option::Option::Some(<$T as $crate::LogTarget>::ID),
            level: $lvl,
            class: $class,
            code: $code,
            line: ::core::line!(),
            file: ::core::file!(),
            fmt: $fmt,
            fields: &[],
            prefix: "boyko",
        };
        $crate::emit_impl(&__BOYKO_LOG_SITE, ($($a,)*));
    }};
}

/// `error!(Target, code, "fmt", args…)` — the caller could not do what was asked.
///
/// `code` is an [`ErrorCode`](crate::codes::ErrorCode) — the **typed newtype**, not its number —
/// and that is what pairs the class byte with the number. Until this was so, the class came from
/// this macro's NAME and the number from an arbitrary `u16`, so `error!(T, codes::W2102.number(),
/// …)` compiled and printed an `E`-class line carrying a `W` code's number, which `explain` cannot
/// resolve. Every registry check stayed green throughout, because all of them key on the
/// identifier in source rather than on what the sink prints.
///
/// It is placed in the per-site `static`, so it must be a constant expression: **not** an argument,
/// and not evaluated per call.
#[macro_export]
macro_rules! error {
    ($T:ty, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Error as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Error as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Error as u8
        {
            $crate::__log_site_emit!(
                $crate::Level::Error,
                $T,
                b'E',
                $crate::codes::ErrorCode::number($code),
                $fmt $(, $a)*
            );
        }
    };
}

/// `warn!(Target, code, "fmt", args…)` — the engine did something the caller probably did not
/// intend.
///
/// `code` is a [`WarnCode`](crate::codes::WarnCode); see [`error!`] for why the typed newtype
/// rather than its number.
#[macro_export]
macro_rules! warn {
    ($T:ty, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Warn as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Warn as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Warn as u8
        {
            $crate::__log_site_emit!(
                $crate::Level::Warn,
                $T,
                b'W',
                $crate::codes::WarnCode::number($code),
                $fmt $(, $a)*
            );
        }
    };
}

/// `info!(Target, "fmt", args…)` — a lifecycle fact a developer wants without asking.
#[macro_export]
macro_rules! info {
    ($T:ty, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Info as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Info as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Info as u8
        {
            $crate::__log_site_emit!($crate::Level::Info, $T, 0u8, 0u16, $fmt $(, $a)*);
        }
    };
}

/// `debug!(Target, "fmt", args…)` — detail a developer asks for while working on a subsystem.
#[macro_export]
macro_rules! debug {
    ($T:ty, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Debug as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Debug as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Debug as u8
        {
            $crate::__log_site_emit!($crate::Level::Debug, $T, 0u8, 0u16, $fmt $(, $a)*);
        }
    };
}

/// `trace!(Target, "fmt", args…)` — per-item detail, expected to be expensive and expected to be
/// off.
#[macro_export]
macro_rules! trace {
    ($T:ty, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Trace as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Trace as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Trace as u8
        {
            $crate::__log_site_emit!($crate::Level::Trace, $T, 0u8, 0u16, $fmt $(, $a)*);
        }
    };
}

// ───────────────────────────── L10: the dynamic forms ─────────────────────────────
//
// # TWO gates, not three, and the missing one is stated rather than smoothed over
//
// ```text
// if $crate::GLOBAL_CEILING as u8 >= LVL as u8       // (b) const: per-profile compile ceiling
//     && $crate::runtime_ceiling($id) >= LVL as u8   // (c) one Relaxed byte load
// { … }
// ```
//
// Gate **(a)** — `T::STATIC_CEILING` — does not exist here and *cannot*. It is a `const` on a
// trait impl, and a dynamic target is a value, not a type; there is no impl to read a `const` from.
// The cost is real and Decision 18 refuses to smooth it: **a dynamic site cannot be compiled out
// per target.** Only `GLOBAL_CEILING` deletes it. A game that wants a category compiled away in
// shipping must give it a type, which is what the downstream band (L11a) is for.
//
// Everything else is identical, deliberately: same `&&` short-circuit, so arguments are still never
// evaluated behind a closed gate; same per-site `static`; same `emit_impl` shape one level down.

/// Build a **dynamic** site and hand it, with its runtime target, to the producer path.
///
/// Internal. The only difference from [`__log_site_emit!`] is `target: None` and the extra `id`
/// argument — see `LogSite::target` for why absence is the discriminant and what it obliges the
/// payload to carry.
#[doc(hidden)]
#[macro_export]
macro_rules! __log_site_emit_dyn {
    ($lvl:expr, $id:expr, $class:expr, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {{
        static __BOYKO_LOG_SITE_DYN: $crate::LogSite = $crate::LogSite {
            target: ::core::option::Option::None,
            level: $lvl,
            class: $class,
            code: $code,
            line: ::core::line!(),
            file: ::core::file!(),
            fmt: $fmt,
            fields: &[],
            prefix: "boyko",
        };
        $crate::emit_impl_dyn(&__BOYKO_LOG_SITE_DYN, $id, ($($a,)*));
    }};
}

/// `dyn_error!(id, code, "fmt", args…)` — the caller could not do what was asked, on a target
/// registered from data.
///
/// `id` is a [`TargetId`](crate::TargetId) obtained from
/// [`register_dynamic_target`](crate::target::register_dynamic_target) or
/// [`find_target`](crate::target::find_target). There is no way to pass an unregistered target:
/// both return `Option<TargetId>`, so "not registered yet" is `None` and does not type-check here.
#[macro_export]
macro_rules! dyn_error {
    ($id:expr, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if $crate::GLOBAL_CEILING as u8 >= $crate::Level::Error as u8 {
            let __boyko_id = $id;
            if $crate::runtime_ceiling(__boyko_id) >= $crate::Level::Error as u8 {
                $crate::__log_site_emit_dyn!(
                    $crate::Level::Error,
                    __boyko_id,
                    b'E',
                    $crate::codes::ErrorCode::number($code),
                    $fmt $(, $a)*
                );
            }
        }
    };
}

/// `dyn_warn!(id, code, "fmt", args…)` — the engine did something the caller probably did not
/// intend, on a target registered from data.
#[macro_export]
macro_rules! dyn_warn {
    ($id:expr, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if $crate::GLOBAL_CEILING as u8 >= $crate::Level::Warn as u8 {
            let __boyko_id = $id;
            if $crate::runtime_ceiling(__boyko_id) >= $crate::Level::Warn as u8 {
                $crate::__log_site_emit_dyn!(
                    $crate::Level::Warn,
                    __boyko_id,
                    b'W',
                    $crate::codes::WarnCode::number($code),
                    $fmt $(, $a)*
                );
            }
        }
    };
}

/// `dyn_info!(id, "fmt", args…)` — a lifecycle fact, on a target registered from data.
#[macro_export]
macro_rules! dyn_info {
    ($id:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if $crate::GLOBAL_CEILING as u8 >= $crate::Level::Info as u8 {
            let __boyko_id = $id;
            if $crate::runtime_ceiling(__boyko_id) >= $crate::Level::Info as u8 {
                $crate::__log_site_emit_dyn!(
                    $crate::Level::Info, __boyko_id, 0u8, 0u16, $fmt $(, $a)*
                );
            }
        }
    };
}

/// `dyn_debug!(id, "fmt", args…)` — detail a developer asks for, on a target registered from data.
#[macro_export]
macro_rules! dyn_debug {
    ($id:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if $crate::GLOBAL_CEILING as u8 >= $crate::Level::Debug as u8 {
            let __boyko_id = $id;
            if $crate::runtime_ceiling(__boyko_id) >= $crate::Level::Debug as u8 {
                $crate::__log_site_emit_dyn!(
                    $crate::Level::Debug, __boyko_id, 0u8, 0u16, $fmt $(, $a)*
                );
            }
        }
    };
}

/// `dyn_trace!(id, "fmt", args…)` — the finest detail, on a target registered from data.
#[macro_export]
macro_rules! dyn_trace {
    ($id:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if $crate::GLOBAL_CEILING as u8 >= $crate::Level::Trace as u8 {
            let __boyko_id = $id;
            if $crate::runtime_ceiling(__boyko_id) >= $crate::Level::Trace as u8 {
                $crate::__log_site_emit_dyn!(
                    $crate::Level::Trace, __boyko_id, 0u8, 0u16, $fmt $(, $a)*
                );
            }
        }
    };
}
