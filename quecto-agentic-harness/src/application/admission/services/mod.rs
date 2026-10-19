//! The runtime admission authority and attempt lifecycle (#1910): the
//! admission service and its registry/client/dispatcher/recovery vocabulary,
//! and the durable, credential-gated authority over it.

pub mod inference_admission;
pub mod inference_authority;
