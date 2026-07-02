//! Shared helpers for the `boyko-macros` proc-macro implementations.
//!
//! Items here are used by more than one macro module (e.g. `FieldAccess` by both
//! the `component` and `relationship` derives) and are therefore `pub(crate)`.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Ident;

/// A field accessor for `offset_of!` — either a named field or a tuple index.
pub(crate) enum FieldAccess {
    Named(Ident),
    Index(usize),
}

impl FieldAccess {
    /// Emits the field selector token for `::core::mem::offset_of!(Ty, <sel>)`.
    pub(crate) fn offset_of_selector(&self) -> TokenStream2 {
        match self {
            FieldAccess::Named(id) => quote! { #id },
            FieldAccess::Index(i) => {
                let idx = syn::Index::from(*i);
                quote! { #idx }
            }
        }
    }
}
