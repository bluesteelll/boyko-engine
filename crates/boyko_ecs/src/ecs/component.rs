use std::any::TypeId;

pub type ComponentId = usize;

pub struct ComponentMetadata {
    pub type_id: TypeId,
    pub size: usize,
    pub alignment: usize
}
pub trait Component: 'static + Sized {
    #[inline(always)]
    fn component_id() -> ComponentId;

    #[inline(always)]
    fn debug_type_name() -> &'static str;

    #[inline(always)]
    fn metadata() -> &'static ComponentMetadata;
}

