//! Inference: what an agent's requests to its model cost and how they went,
//! as the agent itself counts them (#2436). Pure records and counters; the
//! request's trace (`request_observation`) reports each attempt that ends.
pub mod events;
pub mod services;
pub mod value_objects;
