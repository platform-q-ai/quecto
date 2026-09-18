//! Session environment application capability (#1369, #1939): listing,
//! explicit kill and final-member finalization over the pure domain
//! registry and the capability-local ports; and the container-runtime
//! diagnosis behind `quecto container doctor` (#2024 S4b).

pub mod dto;
pub mod ports;
pub mod use_cases;
