//! Strict, non-retaining identification above the semantic error budget.
//! The line observer already bounds input to MAX_SSE_LINE_BYTES. This visitor
//! retains only two booleans and an event category, discarding nested payloads
//! with StrictDiscard: no oversized error Value or typed declarations are built.
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

#[derive(Default)]
struct Envelope {
    kind: Kind,
    error: ErrorShape,
}
impl<'de> Deserialize<'de> for Envelope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(field_identifier)]
        enum Field {
            #[serde(rename = "type")]
            Kind,
            #[serde(rename = "error")]
            Error,
            #[serde(other)]
            Other,
        }
        struct EnvelopeVisitor;
        impl<'de> Visitor<'de> for EnvelopeVisitor {
            type Value = Envelope;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("event object")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Envelope, E> {
                Ok(Envelope::default())
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Envelope, E> {
                Ok(Envelope::default())
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Envelope, E> {
                Ok(Envelope::default())
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Envelope, E> {
                Ok(Envelope::default())
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Envelope, E> {
                Ok(Envelope::default())
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Envelope, E> {
                Ok(Envelope::default())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Envelope, A::Error> {
                while a.next_element::<StrictDiscard>()?.is_some() {}
                Ok(Envelope::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Envelope, A::Error> {
                let mut envelope = Envelope::default();
                while let Some(field) = a.next_key::<Field>()? {
                    match field {
                        Field::Kind => envelope.kind = a.next_value()?,
                        Field::Error => envelope.error = a.next_value()?,
                        Field::Other => {
                            a.next_value::<StrictDiscard>()?;
                        }
                    }
                }
                Ok(envelope)
            }
        }
        d.deserialize_any(EnvelopeVisitor)
    }
}
#[derive(Default)]
enum Kind {
    #[default]
    Untyped,
    Error,
    Failed,
    Incomplete,
    Other,
}
impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct KindVisitor;
        impl<'de> Visitor<'de> for KindVisitor {
            type Value = Kind;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("event type")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<Kind, E> {
                Ok(match s {
                    "error" => Kind::Error,
                    "response.failed" => Kind::Failed,
                    "response.incomplete" => Kind::Incomplete,
                    _ => Kind::Other,
                })
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Kind, E> {
                Ok(Kind::Untyped)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Kind, E> {
                Ok(Kind::Untyped)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Kind, E> {
                Ok(Kind::Untyped)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Kind, E> {
                Ok(Kind::Untyped)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Kind, E> {
                Ok(Kind::Untyped)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Kind, A::Error> {
                while a.next_element::<StrictDiscard>()?.is_some() {}
                Ok(Kind::Untyped)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Kind, A::Error> {
                while a.next_entry::<String, StrictDiscard>()?.is_some() {}
                Ok(Kind::Untyped)
            }
        }
        d.deserialize_any(KindVisitor)
    }
}
#[derive(Default)]
struct ErrorShape(bool);
impl<'de> Deserialize<'de> for ErrorShape {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ErrorVisitor;
        impl<'de> Visitor<'de> for ErrorVisitor {
            type Value = ErrorShape;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("error shape")
            }
            fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<ErrorShape, E> {
                Ok(ErrorShape(!s.is_empty()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<ErrorShape, E> {
                Ok(ErrorShape(false))
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<ErrorShape, E> {
                Ok(ErrorShape(false))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<ErrorShape, E> {
                Ok(ErrorShape(false))
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<ErrorShape, E> {
                Ok(ErrorShape(false))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<ErrorShape, E> {
                Ok(ErrorShape(false))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> Result<ErrorShape, A::Error> {
                while a.next_element::<StrictDiscard>()?.is_some() {}
                Ok(ErrorShape(false))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<ErrorShape, A::Error> {
                while a.next_entry::<String, StrictDiscard>()?.is_some() {}
                Ok(ErrorShape(true))
            }
        }
        d.deserialize_any(ErrorVisitor)
    }
}

pub(super) fn failure(
    data: &str,
    vendor: super::Vendor,
) -> Result<Option<super::TerminalEvent>, serde_json::Error> {
    let shape = serde_json::from_str::<Envelope>(data)?;
    use super::TerminalEvent;
    Ok(match vendor {
        super::Vendor::OpenAi if shape.error.0 => Some(TerminalEvent::Error),
        super::Vendor::Codex => match shape.kind {
            Kind::Error => Some(TerminalEvent::Error),
            Kind::Failed => Some(TerminalEvent::ResponseFailed),
            Kind::Incomplete => Some(TerminalEvent::ResponseIncomplete),
            Kind::Untyped if shape.error.0 => Some(TerminalEvent::Error),
            _ => None,
        },
        _ => None, // Anthropic dispatch uses the event line.
    })
}

/// Unlike serde's IgnoredAny, deserialize_any visits every JSON string through
/// the normal string decoder. Invalid escapes, map keys, numbers and recursion
/// therefore have the same acceptance as Value, without retaining the payload.
struct StrictDiscard;
impl<'de> Deserialize<'de> for StrictDiscard {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct DiscardVisitor;
        impl<'de> Visitor<'de> for DiscardVisitor {
            type Value = StrictDiscard;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<StrictDiscard, E> {
                Ok(StrictDiscard)
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<StrictDiscard, E> {
                Ok(StrictDiscard)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<StrictDiscard, E> {
                Ok(StrictDiscard)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<StrictDiscard, E> {
                Ok(StrictDiscard)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<StrictDiscard, E> {
                Ok(StrictDiscard)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<StrictDiscard, E> {
                Ok(StrictDiscard)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<StrictDiscard, A::Error> {
                while a.next_element::<StrictDiscard>()?.is_some() {}
                Ok(StrictDiscard)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<StrictDiscard, A::Error> {
                while a.next_entry::<String, StrictDiscard>()?.is_some() {}
                Ok(StrictDiscard)
            }
        }
        d.deserialize_any(DiscardVisitor)
    }
}
