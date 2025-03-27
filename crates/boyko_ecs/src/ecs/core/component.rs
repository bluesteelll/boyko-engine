use std::any::TypeId;
use crate::ecs::identifiers::primitives::ComponentId;

pub trait Component: 'static + Sized {
    #[inline(always)]
    fn component_id() -> ComponentId;

    #[inline(always)]
    fn debug_type_name() -> &'static str{
        std::any::type_name::<Self>()
    }

    #[inline(always)]
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }

    #[inline(always)]
    fn mem_size() -> usize {
        std::mem::size_of::<Self>()
    }

    #[inline(always)]
    fn alignment() -> usize {
        std::mem::align_of::<Self>()
    }
}
