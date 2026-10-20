//! The runtime admission authority and attempt lifecycle (#1910): the
//! in-memory admission service that implements the `ports::inference_admission`
//! ports over the domain policy, and the credential-gated authority over it.
//! The authority's durability is the admission use cases': they make its
//! ledger durable through the journal port.

pub mod inference_admission;
pub mod inference_authority;
