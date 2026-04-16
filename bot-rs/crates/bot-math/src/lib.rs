//! `bot-math` — Pure math layer for the Dol v4 framework.
//!
//! All functions are deterministic, side-effect-free, and `tokio`/`rayon`-free.
//! No `unwrap()` in library code; recoverable errors return `Result<_, FrameworkError>`.
//! No `f64::mul_add` (FMA), no SIMD intrinsics, no `-ffast-math`-equivalent flags.
//!
//! Module layout mirrors the spec Part D sections:
//!   - `phi`      : D.1  absorption function
//!   - `ou`       : D.2  OU time-averaged spread
//!   - `impact`   : D.3  effective spread with impact
//!   - `breakeven`: D.4  break-even hold time
//!   - `optimum`  : D.5  interior optimum w*, n*, T*
//!   - `leverage` : D.6–D.7  critical AUM, Bernstein bound
//!   - `mfg`      : D.8  MFG free-entry equilibrium
//!   - `routing`  : D.9  mandate cap routing
//!   - `cost`     : D.10 Model C round-trip cost

pub mod breakeven;
pub mod cost;
pub mod impact;
pub mod leverage;
pub mod mfg;
pub mod optimum;
pub mod ou;
pub mod phi;
pub mod routing;

// Re-export all public items at crate root for ergonomic use.
pub use breakeven::{break_even_hold_at_mean, break_even_hold_fixed_point};
pub use cost::{round_trip_cost_model_c, slippage};
pub use impact::effective_spread_with_impact;
pub use leverage::{bernstein_leverage_bound, critical_aum};
pub use mfg::{capacity_ceiling, dol_sustainable_flow_per_pair, mfg_competitor_count};
pub use optimum::{optimal_margin_fraction, optimal_notional, optimal_trading_contribution};
pub use ou::ou_time_averaged_spread;
pub use phi::{phi, phi_derivative};
pub use routing::{cap_routing, mandate_floor};
