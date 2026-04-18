//! `bot-types` — Newtype wrappers and core structs for the Dol v4 framework.
//!
//! All newtypes are `Copy + Debug + PartialEq + Serialize + Deserialize`.
//! No I/O, no async, no external runtime dependencies.

pub mod sym;

use hashbrown::HashMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Newtype wrappers (math-first typing — design spec Part A.1)
// ---------------------------------------------------------------------------

/// Annualized rate (dimensionless fraction per year, e.g. 0.05 = 5% APY).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnualizedRate(pub f64);

/// Hourly rate (dimensionless fraction per hour).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HourlyRate(pub f64);

/// Duration in hours.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hours(pub f64);

/// Dollar amount.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Usd(pub f64);

/// Fraction of AUM (in [0, 1]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AumFraction(pub f64);

/// Dimensionless ratio / coefficient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dimensionless(pub f64);

impl AnnualizedRate {
    /// Convert from hourly rate: `annual = hourly × 8760`.
    pub fn from_hourly(h: HourlyRate) -> Self {
        Self(h.0 * 8760.0)
    }

    /// Convert to hourly rate: `hourly = annual / 8760`.
    pub fn to_hourly(&self) -> HourlyRate {
        HourlyRate(self.0 / 8760.0)
    }
}

// ---------------------------------------------------------------------------
// Venue types (design spec Part C.1)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Venue {
    Pacifica,
    Backpack,
    Hyperliquid,
    Lighter,
}

/// Framework §17: Class P (orderbook-impact) vs Class M (EWMA mid).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VenueClass {
    /// Orderbook-impact based funding (Pacifica-homogeneous).
    P,
    /// EWMA mid-based funding (Backpack).
    M,
}

impl Venue {
    /// Returns the venue's funding class.
    ///
    /// Pacifica / Hyperliquid / Lighter → P (orderbook-impact).
    /// Backpack → M (EWMA mid).
    #[inline]
    pub const fn class(&self) -> VenueClass {
        match self {
            Venue::Pacifica | Venue::Hyperliquid | Venue::Lighter => VenueClass::P,
            Venue::Backpack => VenueClass::M,
        }
    }

    /// Conservative taker-fee estimate in basis points (1 bps = 0.01 %).
    ///
    /// These are public fee-schedule defaults used by the decision-layer
    /// cost model. They are intentionally higher than the real current
    /// schedules to keep the `income > cost` check conservative — a real
    /// production build would replace them with live-calibrated values
    /// via `slippage_calibration` (see the live-promotion checklist §5).
    #[inline]
    pub const fn taker_fee_bps(&self) -> f64 {
        match self {
            Venue::Pacifica => 4.0,
            Venue::Hyperliquid => 3.5,
            Venue::Lighter => 2.0,
            Venue::Backpack => 4.0,
        }
    }

    /// Conservative maker-fee (or rebate) estimate in basis points.
    /// Positive values mean the maker pays; negative values (rare on
    /// current DEXes) would mean the venue pays a rebate.
    #[inline]
    pub const fn maker_fee_bps(&self) -> f64 {
        match self {
            Venue::Pacifica => 1.5,
            Venue::Hyperliquid => 1.0,
            Venue::Lighter => 0.0,
            Venue::Backpack => 2.0,
        }
    }

    /// Short human-readable name used in decision-log strings and
    /// dashboard-facing JSON. Uppercase first letter matches the
    /// `Debug` impl that the dashboard already consumes.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Venue::Pacifica => "Pacifica",
            Venue::Backpack => "Backpack",
            Venue::Hyperliquid => "Hyperliquid",
            Venue::Lighter => "Lighter",
        }
    }
}

// ---------------------------------------------------------------------------
// Pair identifier (design spec Part C.1)
// ---------------------------------------------------------------------------

/// Pair identifier: (symbol, counter venue). Pivot venue is always Pacifica.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PairId {
    pub symbol: String,
    pub counter: Venue,
}

impl PairId {
    pub fn new(symbol: impl Into<String>, counter: Venue) -> Self {
        Self {
            symbol: symbol.into(),
            counter,
        }
    }
}

// ---------------------------------------------------------------------------
// LiveInputs (design spec Part C.1)
// ---------------------------------------------------------------------------

/// All inputs read from venues at each tick. Pure data, no state.
///
/// Corresponds to Python `cost_model.LiveInputs`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LiveInputs {
    pub timestamp_ms: i64,

    /// Total vault AUM in USD.
    pub aum: Usd,

    /// Current idle yield (e.g. Kamino USDC supply APY), annualized.
    pub r_idle: AnnualizedRate,

    /// `(symbol, venue)` → per-hour signed funding rate.
    pub funding_rate_h: HashMap<(String, Venue), HourlyRate>,

    /// `(symbol, venue)` → open interest in USD.
    pub open_interest: HashMap<(String, Venue), Usd>,

    /// `(symbol, venue)` → 24 h trading volume in USD.
    pub volume_24h: HashMap<(String, Venue), Usd>,

    /// Per-venue maker fee (fraction of notional per leg).
    pub fee_maker: HashMap<Venue, Dimensionless>,

    /// Per-venue taker fee (fraction of notional per leg).
    pub fee_taker: HashMap<Venue, Dimensionless>,

    /// `(pivot_venue, counter_venue)` → bridge round-trip cost (fraction).
    pub bridge_cost: HashMap<(Venue, Venue), Dimensionless>,

    /// `(symbol, venue)` → rolling history of `(timestamp_ms, rate)`.
    /// Minimum length: `mandate.persistence_lookback_h_min`.
    pub funding_history: HashMap<(String, Venue), Vec<(i64, HourlyRate)>>,

    /// `symbol` → rolling history of `(timestamp_ms, basis_divergence)`.
    pub basis_divergence_history: HashMap<String, Vec<(i64, f64)>>,
}

// ---------------------------------------------------------------------------
// Mandate (design spec Part C.2)
// ---------------------------------------------------------------------------

/// PM-set policy parameters. Immutable by construction; always passed as `&Mandate`.
///
/// Corresponds to Python `cost_model.Mandate`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mandate {
    // PM-set mandate targets
    pub customer_apy_min: AnnualizedRate, // 0.05
    pub customer_apy_max: AnnualizedRate, // 0.08
    pub buffer_apy_min: AnnualizedRate,   // 0.02
    pub buffer_apy_max: AnnualizedRate,   // 0.05
    pub cut_customer: Dimensionless,      // 0.65
    pub cut_buffer: Dimensionless,        // 0.25
    pub cut_reserve: Dimensionless,       // 0.10

    /// PRINCIPLES hard floor: cannot deploy more than (1 - α_min) of AUM.
    pub aum_buffer_floor: AumFraction, // 0.50
    /// Do not go fully idle when signals exist.
    pub aum_idle_cap: AumFraction, // 0.95

    // Statistical Z multipliers
    pub z_persistence: Dimensionless, // 5.0
    pub z_drawdown: Dimensionless,    // 3.0
    pub z_pnl_warn: Dimensionless,    // 1.0
    pub z_pnl_halve: Dimensionless,   // 2.0
    pub z_pnl_kill: Dimensionless,    // 3.0

    // Lookback windows (hours / days)
    pub persistence_lookback_h_min: u32, // 168  (1 week)
    pub persistence_lookback_h_max: u32, // 720  (30 days)
    pub basis_lookback_h: u32,           // 168
    pub pnl_lookback_d: u32,             // 30

    // Venue concentration caps
    pub max_single_venue_exposure: AumFraction, // 0.60
    pub max_simultaneous_pairs: u32,            // 46

    /// DEX whitelist (PRINCIPLES §2: no KYC CEXes).
    pub dex_venues: Vec<Venue>,

    /// Drawdown stop must fire this many times before a maintenance-margin call.
    pub leverage_safety_multiplier: Dimensionless, // 10
}

impl Default for Mandate {
    fn default() -> Self {
        Self {
            customer_apy_min: AnnualizedRate(0.05),
            customer_apy_max: AnnualizedRate(0.08),
            buffer_apy_min: AnnualizedRate(0.02),
            buffer_apy_max: AnnualizedRate(0.05),
            cut_customer: Dimensionless(0.65),
            cut_buffer: Dimensionless(0.25),
            cut_reserve: Dimensionless(0.10),
            aum_buffer_floor: AumFraction(0.50),
            aum_idle_cap: AumFraction(0.95),
            z_persistence: Dimensionless(5.0),
            z_drawdown: Dimensionless(3.0),
            z_pnl_warn: Dimensionless(1.0),
            z_pnl_halve: Dimensionless(2.0),
            z_pnl_kill: Dimensionless(3.0),
            persistence_lookback_h_min: 168,
            persistence_lookback_h_max: 720,
            basis_lookback_h: 168,
            pnl_lookback_d: 30,
            max_single_venue_exposure: AumFraction(0.60),
            max_simultaneous_pairs: 46,
            dex_venues: vec![
                Venue::Pacifica,
                Venue::Backpack,
                Venue::Hyperliquid,
                Venue::Lighter,
            ],
            leverage_safety_multiplier: Dimensionless(10.0),
        }
    }
}

// ---------------------------------------------------------------------------
// OU process parameters (design spec Part C.3)
// ---------------------------------------------------------------------------

/// Ornstein-Uhlenbeck process parameters for one funding-spread series.
///
/// Corresponds to the output of Python `stochastic.fit_ou`, with `sigma_ou`
/// rescaled from the raw Python output: `sigma_ou = fit.sigma * 8760.0`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OuParams {
    /// Long-run mean μ̃ (annualized).
    pub mu: AnnualizedRate,
    /// Mean-reversion speed θ^OU (per hour).
    pub theta_ou: HourlyRate,
    /// OU volatility σ^OU with hybrid units `[AnnualizedRate / √hour]`:
    /// numerator annualized to match `mu`, denominator hourly to match
    /// `theta_ou`. This is the Python `fit_ou.sigma` scaled by 8760. With
    /// this convention, `σ / √(2θ)` naturally produces `AnnualizedRate`
    /// because the `√hour` factors cancel between `σ_ou` and `√(θ_ou)`.
    /// (See Rust integration design spec Part C.3.)
    pub sigma_ou: Dimensionless,
}

impl OuParams {
    /// Stationary standard deviation: σ_stat = σ^OU / √(2θ^OU).
    ///
    /// Unit analysis: σ^OU is `[AnnualizedRate / √hour]`, √(2θ^OU) is
    /// `[1 / √hour]`, so the quotient is `AnnualizedRate`. The return type
    /// is therefore exact, not a bookkeeping wrapper.
    pub fn stationary_std(&self) -> AnnualizedRate {
        AnnualizedRate(self.sigma_ou.0 / (2.0 * self.theta_ou.0).sqrt())
    }

    /// Half-life = ln(2) / θ^OU (hours).
    pub fn half_life(&self) -> Hours {
        Hours(std::f64::consts::LN_2 / self.theta_ou.0)
    }
}

// ---------------------------------------------------------------------------
// MFG + orderbook impact parameters (design spec Part C.3)
// ---------------------------------------------------------------------------

/// Mean-field game and orderbook impact parameters (per pair or global policy).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImpactParams {
    /// Orderbook-impact coefficient θ^impact ∈ [0.3, 0.7].
    pub theta_impact: Dimensionless,
    /// Competitor density ρ^comp (from free-entry equilibrium).
    pub rho_comp: Dimensionless,
    /// Marginal arb operational cost C_op (USD/year).
    pub c_op_marginal: Usd,
}

// ---------------------------------------------------------------------------
// MandateAllocation (design spec Part D.9)
// ---------------------------------------------------------------------------

/// Result of `cap_routing`: the three slices of vault gross APY.
///
/// Conservation invariant (enforced by `debug_assert`):
///   `customer + buffer + reserve == vault_gross` to within 1e-12.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MandateAllocation {
    pub customer: AnnualizedRate,
    pub buffer: AnnualizedRate,
    pub reserve: AnnualizedRate,
}

// ---------------------------------------------------------------------------
// FrameworkError (design spec Part A.4)
// ---------------------------------------------------------------------------

/// All recoverable errors from the pure math layer.
///
/// Library code never `unwrap()`s — it returns `Result<_, FrameworkError>`.
/// `panic!` is reserved for invariant violations that cannot occur.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum FrameworkError {
    /// A supplied argument is outside its valid domain.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Not enough historical data points to perform the computation.
    #[error("insufficient history")]
    InsufficientHistory,

    /// The time-averaged OU spread is ≤ 0.
    #[error("spread is negative or zero")]
    NegativeSpread,

    /// After impact / competition discount the effective spread is ≤ 0.
    #[error("effective spread is negative or zero after impact/competition discount")]
    NegativeEffectiveSpread,

    /// Break-even fixed-point iteration did not converge within `max_iter`.
    #[error("fixed-point iteration did not converge")]
    FixedPointNotConverged,

    /// Dol's cost advantage over marginal competitor is zero or negative.
    #[error("no sustainable edge: Dol cost ≥ marginal competitor cost")]
    NoSustainableEdge,

    /// Mandate constraints are infeasible given current inputs.
    #[error("infeasible mandate: denominator ≤ 0 in capacity ceiling")]
    InfeasibleMandate,

    /// The OLS design matrix X'X is singular or near-singular (pivot < 1e-14).
    ///
    /// Added in Phase 2a for `adf_test` (Gauss-Jordan pivot check).
    #[error("singular or near-singular matrix in regression")]
    SingularMatrix,

    /// OLS regression failed due to degenerate input (e.g. zero variance).
    ///
    /// Added in Phase 2a for `fit_ou` (sxx ≤ 0) and `adf_test` (var_beta ≤ 0).
    #[error("regression failed: degenerate input (zero variance or zero SE)")]
    RegressionFailed,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annualized_rate_round_trip() {
        let h = HourlyRate(0.001);
        let a = AnnualizedRate::from_hourly(h);
        assert!((a.0 - 8.76).abs() < 1e-12);
        let back = a.to_hourly();
        assert!((back.0 - h.0).abs() < 1e-15);
    }

    #[test]
    fn venue_class_mapping() {
        assert_eq!(Venue::Pacifica.class(), VenueClass::P);
        assert_eq!(Venue::Hyperliquid.class(), VenueClass::P);
        assert_eq!(Venue::Lighter.class(), VenueClass::P);
        assert_eq!(Venue::Backpack.class(), VenueClass::M);
    }

    #[test]
    fn ou_params_half_life() {
        let ou = OuParams {
            mu: AnnualizedRate(0.10),
            theta_ou: HourlyRate(0.01),
            sigma_ou: Dimensionless(0.002),
        };
        // half_life = ln(2) / 0.01 ≈ 69.315 hours
        assert!((ou.half_life().0 - std::f64::consts::LN_2 / 0.01).abs() < 1e-12);
    }

    #[test]
    fn ou_params_stationary_std() {
        let ou = OuParams {
            mu: AnnualizedRate(0.10),
            theta_ou: HourlyRate(0.01),
            sigma_ou: Dimensionless(0.002),
        };
        // stationary_std = 0.002 / sqrt(2 * 0.01) = 0.002 / sqrt(0.02)
        let expected = 0.002 / (0.02_f64).sqrt();
        assert!((ou.stationary_std().0 - expected).abs() < 1e-15);
    }

    #[test]
    fn mandate_default_cuts_sum_to_one() {
        let m = Mandate::default();
        let total = m.cut_customer.0 + m.cut_buffer.0 + m.cut_reserve.0;
        assert!((total - 1.0).abs() < 1e-15);
    }
}
