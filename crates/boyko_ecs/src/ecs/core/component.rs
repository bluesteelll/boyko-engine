use std::any::TypeId;

pub type ComponentId = usize;

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
    fn size() -> usize {
        std::mem::size_of::<Self>()
    }

    #[inline(always)]
    fn alignment() -> usize {
        std::mem::align_of::<Self>()
    }
}
