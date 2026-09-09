//! Read port for admission activity (#1679 P4): bounded, fresh, orthogonal to
//! process lifecycle. Producers live in infrastructure around the gate.
use crate::domain::inference_admission::AdmissionActivity;

pub trait AdmissionObservation: Send + Sync {
    fn snapshot(&self) -> AdmissionActivity;
}
