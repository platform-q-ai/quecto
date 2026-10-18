//! Source-layer precedence for catalogue resolution.

/// Source layers in ascending precedence order. `Ord` encodes the documented
/// precedence: `BuiltIn < Generated < Discovered < Extension < UserDefined <
/// UserOverride`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceLayer {
    BuiltIn,
    Generated,
    Discovered,
    Extension,
    UserDefined,
    UserOverride,
}
