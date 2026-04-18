//! Compile-time symbol identity for cross-venue pairs.
//!
//! Iron law §1 requires that the long-leg and short-leg of a funding-arb
//! pair carry **byte-identical** symbol strings. The runtime `PairDecision`
//! already enforces this at the data level (a single `symbol: String`
//! field shared between both legs), but that is a *data* guarantee, not a
//! *type* guarantee — a mistaken constructor could still produce a logical
//! cross-symbol pair by substituting the venue+direction of a different
//! decision.
//!
//! This module provides a PhantomData-parameterized witness
//! [`SameSymbol<S>`] that can only be constructed for a specific marker
//! type `S`. Callers attach it to their pair values so the compiler
//! refuses to mix a `PairView<BtcSymbol>` with an `ExposureView<EthSymbol>`.
//!
//! # Usage
//!
//! ```
//! use bot_types::sym::{BtcMarker, EthMarker, SameSymbol, SymbolTag, symbol_tag};
//!
//! // Build a tag from a literal symbol string at runtime…
//! let btc_tag: SymbolTag = symbol_tag("BTC");
//! let eth_tag: SymbolTag = symbol_tag("ETH");
//! assert_ne!(btc_tag, eth_tag);
//! // …and bind a same-symbol witness to it. PhantomData erases the
//! // runtime bytes; equality is enforced at the binding site.
//! let w: SameSymbol<BtcMarker> = SameSymbol::new();
//! # let _ = w;
//! ```
//!
//! The marker types are typically one-off zero-sized structs per symbol
//! owned by the decision crate. See [`SymbolMarker`] for the sealed trait.

use core::marker::PhantomData;

/// Sealed trait so downstream crates cannot forge new symbol markers.
/// Implement via the crate-internal macro in `bot_runtime`.
pub trait SymbolMarker: sealed::Sealed + 'static {
    /// Human-readable symbol string for diagnostics.
    const NAME: &'static str;
}

mod sealed {
    pub trait Sealed {}
}

/// Compile-time witness that two values share the same symbol.
///
/// Zero-sized (`size_of::<SameSymbol<_>>() == 0`). Construction is free;
/// the compiler rejects any attempt to use a `SameSymbol<A>` where a
/// `SameSymbol<B>` is required.
///
/// # Type-mismatch is a compile error
///
/// ```compile_fail
/// use bot_types::sym::{SameSymbol, BtcMarker, EthMarker};
/// fn requires_eth(_: SameSymbol<EthMarker>) {}
/// let b: SameSymbol<BtcMarker> = SameSymbol::new();
/// requires_eth(b); // ERROR: mismatched markers
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SameSymbol<S: SymbolMarker>(PhantomData<S>);

impl<S: SymbolMarker> SameSymbol<S> {
    #[inline]
    pub const fn new() -> Self {
        Self(PhantomData)
    }

    #[inline]
    pub const fn name() -> &'static str {
        S::NAME
    }
}

/// Runtime symbol tag — a static string wrapper. Useful when a marker
/// can't be chosen statically (e.g. multi-symbol portfolio loops).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolTag(pub &'static str);

/// Construct a `SymbolTag` from a static string. For dynamic strings the
/// caller must Box::leak or use a `String` field directly.
#[inline]
pub const fn symbol_tag(s: &'static str) -> SymbolTag {
    SymbolTag(s)
}

// ─────────────────────────────────────────────────────────────────────────────
// Concrete markers — exported so downstream crates don't each re-declare.
// Add a marker per symbol the bot actively trades.
// ─────────────────────────────────────────────────────────────────────────────

macro_rules! declare_markers {
    ($($name:ident = $str:literal),* $(,)?) => {
        $(
            #[doc = concat!("Compile-time marker for `", $str, "`.")]
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct $name;
            impl sealed::Sealed for $name {}
            impl SymbolMarker for $name {
                const NAME: &'static str = $str;
            }
        )*
    };
}

declare_markers! {
    BtcMarker   = "BTC",
    EthMarker   = "ETH",
    SolMarker   = "SOL",
    BnbMarker   = "BNB",
    ArbMarker   = "ARB",
    AvaxMarker  = "AVAX",
    SuiMarker   = "SUI",
    XauMarker   = "XAU",
    XagMarker   = "XAG",
    PaxgMarker  = "PAXG",
    HypeMarker  = "HYPE",
}

/// Dispatch a runtime `&str` symbol to a compile-time `SymbolMarker` type
/// and evaluate the given closure with that marker chosen.
///
/// This is the I-SAME ingress boundary: callers that loop over symbol
/// strings (e.g. the multi-symbol tick loop) use this macro to project
/// each iteration into a statically-tagged scope where
/// `TypedPairDecision<'_, S>` becomes available.
///
/// Returns `None` if the symbol string is not in the whitelist below.
/// The whitelist intentionally matches the symbols the demo/live bot
/// actually trades; adding a new symbol requires:
///
///   1. Declaring its marker in `declare_markers!` above
///   2. Adding its arm to this macro
///
/// Both edits happen in this file, so the compile-time whitelist can
/// never drift from the marker list.
///
/// # Example
///
/// ```
/// use bot_types::{with_symbol_marker, sym::SymbolMarker};
/// let result = with_symbol_marker!("BTC", |M| {
///     assert_eq!(M::NAME, "BTC");
///     42
/// });
/// assert_eq!(result, Some(42));
///
/// let unknown = with_symbol_marker!("UNKNOWN", |M| {
///     let _ = M::NAME;
///     0
/// });
/// assert!(unknown.is_none());
/// ```
#[macro_export]
macro_rules! with_symbol_marker {
    ($sym:expr, |$M:ident| $body:expr) => {{
        // Each arm instantiates the closure's body for a specific marker
        // type. The caller sees exactly one monomorphized body per symbol.
        match $sym {
            "BTC" => {
                type $M = $crate::sym::BtcMarker;
                Some($body)
            }
            "ETH" => {
                type $M = $crate::sym::EthMarker;
                Some($body)
            }
            "SOL" => {
                type $M = $crate::sym::SolMarker;
                Some($body)
            }
            "BNB" => {
                type $M = $crate::sym::BnbMarker;
                Some($body)
            }
            "ARB" => {
                type $M = $crate::sym::ArbMarker;
                Some($body)
            }
            "AVAX" => {
                type $M = $crate::sym::AvaxMarker;
                Some($body)
            }
            "SUI" => {
                type $M = $crate::sym::SuiMarker;
                Some($body)
            }
            "XAU" => {
                type $M = $crate::sym::XauMarker;
                Some($body)
            }
            "XAG" => {
                type $M = $crate::sym::XagMarker;
                Some($body)
            }
            "PAXG" => {
                type $M = $crate::sym::PaxgMarker;
                Some($body)
            }
            "HYPE" => {
                type $M = $crate::sym::HypeMarker;
                Some($body)
            }
            _ => None,
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same symbol marker → identity holds by construction.
    #[test]
    fn same_symbol_is_zero_sized() {
        assert_eq!(std::mem::size_of::<SameSymbol<BtcMarker>>(), 0);
        assert_eq!(std::mem::size_of::<SameSymbol<EthMarker>>(), 0);
    }

    #[test]
    fn marker_name_accessible() {
        assert_eq!(SameSymbol::<BtcMarker>::name(), "BTC");
        assert_eq!(SameSymbol::<EthMarker>::name(), "ETH");
    }

    #[test]
    fn symbol_tag_equality() {
        let t1 = symbol_tag("BTC");
        let t2 = symbol_tag("BTC");
        let t3 = symbol_tag("ETH");
        assert_eq!(t1, t2);
        assert_ne!(t1, t3);
    }
}
