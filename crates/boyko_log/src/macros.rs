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
            target: <$T as $crate::LogTarget>::ID,
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
/// `code` is placed in the per-site `static`, so it must be a constant expression. It is
/// therefore **not** an argument and is not evaluated per call.
#[macro_export]
macro_rules! error {
    ($T:ty, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Error as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Error as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Error as u8
        {
            $crate::__log_site_emit!(
                $crate::Level::Error, $T, b'E', $code, $fmt $(, $a)*
            );
        }
    };
}

/// `warn!(Target, code, "fmt", args…)` — the engine did something the caller probably did not
/// intend.
#[macro_export]
macro_rules! warn {
    ($T:ty, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Warn as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Warn as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Warn as u8
        {
            $crate::__log_site_emit!(
                $crate::Level::Warn, $T, b'W', $code, $fmt $(, $a)*
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
