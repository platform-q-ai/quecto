//! The runtime admission authority and attempt lifecycle (#1910): the
//! in-memory admission service that implements the `ports::inference_admission`
//! ports over the domain policy, and the durable, credential-gated authority
//! over it.

pub mod inference_admission;
pub mod inference_authority;
