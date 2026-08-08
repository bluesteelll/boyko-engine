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
//! # This rung has no sink
//!
//! The enabled arm calls [`__l0_no_sink_yet`](crate::__l0_no_sink_yet), which evaluates the
//! arguments and discards them. It is deliberately unpleasant to read: rung L1 replaces it with
//! `emit_impl`, and a stub that looked like a finished call is a stub that ships.

/// `error!(Target, code, "fmt", args…)` — the caller could not do what was asked.
///
/// Carries a diagnostic code; the code registry lands at a later rung, so at this one the code
/// expression is evaluated and discarded like any other argument.
#[macro_export]
macro_rules! error {
    ($T:ty, $code:expr, $fmt:literal $(, $a:expr)* $(,)?) => {
        if <$T as $crate::LogTarget>::STATIC_CEILING as u8 >= $crate::Level::Error as u8
            && $crate::GLOBAL_CEILING as u8 >= $crate::Level::Error as u8
            && $crate::runtime_ceiling(<$T as $crate::LogTarget>::ID)
                >= $crate::Level::Error as u8
        {
            $crate::__l0_no_sink_yet($fmt, ($code, $($a,)*));
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
            $crate::__l0_no_sink_yet($fmt, ($code, $($a,)*));
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
            $crate::__l0_no_sink_yet($fmt, ($($a,)*));
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
            $crate::__l0_no_sink_yet($fmt, ($($a,)*));
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
            $crate::__l0_no_sink_yet($fmt, ($($a,)*));
        }
    };
}
