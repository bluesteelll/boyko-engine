//! The leg-(2) candidate list (cut §4.2) and the rule that locates each one in an object.
//!
//! This is the CANDIDATE list, not the pin list. Probes (i) and (ii) read these bodies; the pin
//! list is frozen at capture to the candidates present, and each absent one is recorded with its
//! reason (03 §6 "Missing symbols").

use crate::llvm::Sym;
use crate::normalize::norm_name;

/// How a candidate is recognised among an object's normalised demangled names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The whole normalised name.
    Exact(&'static str),
    /// Every defined function whose name contains the substring (closures, `Bencher` wrappers).
    Contains(&'static str),
    /// The lexicographically first defined function whose name starts with the prefix (one
    /// instantiation of a generic, cut decision D5).
    FirstWithPrefix(&'static str),
}

/// One leg-(2) candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The cut's id (`P29-1`, `R-1`, `H-2`, …).
    pub id: &'static str,
    /// Subject keys to look in, in order of preference.
    pub subjects: &'static [&'static str],
    /// The locate rules; a candidate may have several (H-2: the bench closures AND `delete_entity`).
    pub rules: &'static [Rule],
}

const GAME: &[&str] = &["boyko_demo", "clear"];

/// The candidates of cut §4.2, in its order.
pub const CANDIDATES: &[Candidate] = &[
    Candidate { id: "P29-1", subjects: GAME, rules: &[Rule::Exact("<boyko_ecs::ecs::memory::component_pool::ComponentPool>::grow_rows")] },
    Candidate { id: "P29-2", subjects: GAME, rules: &[Rule::Exact("<boyko_ecs::ecs::memory::component_pool::ComponentPool>::commit_subregion")] },
    Candidate { id: "P29-3", subjects: GAME, rules: &[Rule::Exact("boyko_ecs::ecs::core::change_detection::check_ticks::run_check_ticks_scan")] },
    Candidate { id: "P29-4", subjects: GAME, rules: &[Rule::Exact("<boyko_threadpool::block::ScopeBlock>::grow")] },
    Candidate {
        id: "R-1",
        subjects: &["clear"],
        rules: &[Rule::Exact("boyko_ecs::ecs::core::component::component_registry::register_new::<boyko_scene::transform::Transform>")],
    },
    Candidate {
        id: "R-2",
        subjects: &["clear"],
        rules: &[Rule::Exact("<boyko_scene::transform::Transform as boyko_ecs::ecs::core::component::component::Component>::component_id")],
    },
    Candidate { id: "D-2", subjects: &["clear", "boyko_demo"], rules: &[Rule::Exact("boyko_ecs::ecs::core::iters::query::query_type_registry::register_new")] },
    Candidate { id: "D-3", subjects: &["clear", "boyko_demo"], rules: &[Rule::Exact("boyko_ecs::ecs::core::bundle::bundle_type_registry::register_new")] },
    Candidate { id: "D-4", subjects: &["clear", "boyko_demo"], rules: &[Rule::FirstWithPrefix("boyko_ecs::ecs::core::resources::resource_registry::register_new::<")] },
    Candidate { id: "D-5", subjects: &["clear", "boyko_demo"], rules: &[Rule::FirstWithPrefix("boyko_ecs::ecs::core::events::event_registry::register_event_new::<")] },
    Candidate { id: "R-3", subjects: &["clear"], rules: &[Rule::Exact("boyko_ecs::ecs::core::component::component_registry::try_register_dynamic")] },
    Candidate { id: "R-4", subjects: &["swap_remove", "query_dsl"], rules: &[Rule::FirstWithPrefix("boyko_ecs::ecs::core::component::component_registry::register_layout::<")] },
    Candidate { id: "S-1", subjects: &["clear"], rules: &[Rule::FirstWithPrefix("<boyko_ecs::ecs::core::schedule::schedule_builder::ScheduleBuilder>::add_system::<")] },
    Candidate { id: "H-1", subjects: &["query_dsl"], rules: &[Rule::Contains("bench_query_ref_iter")] },
    Candidate {
        id: "H-2",
        subjects: &["swap_remove"],
        rules: &[Rule::Contains("bench_swap_remove"), Rule::Exact("<boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster>::delete_entity")],
    },
    Candidate {
        id: "H-3",
        subjects: &["phase9_scheduler", "clear", "boyko_demo"],
        rules: &[Rule::Exact("<boyko_ecs::ecs::core::schedule::schedule::Schedule>::run"), Rule::Contains("bench_schedule_two_disjoint")],
    },
];

/// `true` for an nm class that names code.
#[must_use]
pub fn is_code(class: char) -> bool {
    matches!(class, 't' | 'T')
}

/// The defined CODE symbols of `syms` that `rule` selects (raw names), in symbol-table order.
///
/// SEH funclets (`?dtor$…`) are never selected here; they are pulled in with their owner's section.
#[must_use]
pub fn locate(syms: &[Sym], rule: Rule) -> Vec<&Sym> {
    let code = syms.iter().filter(|s| is_code(s.class) && !s.raw.starts_with('?'));
    match rule {
        Rule::Exact(name) => code.filter(|s| norm_name(&s.name) == name).collect(),
        Rule::Contains(sub) => code.filter(|s| s.name.contains(sub)).collect(),
        Rule::FirstWithPrefix(prefix) => {
            let mut v: Vec<&Sym> = code.filter(|s| norm_name(&s.name).starts_with(prefix)).collect();
            v.sort_by_key(|s| norm_name(&s.name));
            v.truncate(1);
            v
        }
    }
}
