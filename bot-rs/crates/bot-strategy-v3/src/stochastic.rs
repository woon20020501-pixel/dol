//! `stochastic.rs` — Rigorous statistical models for funding spread dynamics.
//!
//! Faithful port of Python `strategy/stochastic.py`. Each function mirrors the
//! Python reduction order exactly (no FMA, no rayon, no reordering).
//!
//! # Functions
//!
//! - [`fit_ou`]            — OU MLE via AR(1) OLS regression
//! - [`adf_test`]          — Augmented Dickey-Fuller unit-root test
//! - [`cvar_drawdown_stop`]— Empirical CVaR for drawdown stops
//! - [`expected_residual_income`] — Integral of expected OU spread over hold period
//! - [`fit_drift`]         — Drift-model fit for persistent (H > 0.7) spreads

use bot_types::FrameworkError;
use serde::{Deserialize, Serialize};

// ===========================================================================
// §1 — Ornstein-Uhlenbeck MLE fit
// ===========================================================================

/// Result of fitting the OU process  ds = θ(μ - s)dt + σ dW  to discrete data.
///
/// Field names and units match Python `stochastic.OUFit` exactly (raw per-hour units).
/// The fixture `fit_ou.json` stores these raw values and is the parity target.
///
/// NOTE: The `sigma` field here is the raw Python-scale sigma (`[series_units / √hour]`).
/// To obtain the `OuParams.sigma_ou` (hybrid `[AnnualizedRate / √hour]`) used elsewhere in
/// the framework, multiply by 8760 — see the spec Part C.3 
/// The fixture stores the raw value (pre-scaling), so parity tests compare `sigma` directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitOuOutput {
    /// Number of observations.
    pub n_obs: usize,
    /// AR(1) intercept: â.
    pub a: f64,
    /// AR(1) slope coefficient: b̂ = e^(-θ Δt).
    pub b: f64,
    /// AR(1) residual std: σ̂_ε.
    pub sigma_eps: f64,
    /// OU long-run mean μ̂ (same units as the input series).
    pub mu: f64,
    /// OU mean-reversion rate θ̂ (per hour, i.e. 1/hour).
    pub theta: f64,
    /// OU diffusion volatility σ̂ (raw `[series_units / √hour]`; multiply by 8760 for
    /// the hybrid `AnnualizedRate / √hour` convention used in `OuParams`).
    pub sigma: f64,
    /// Standard error of b̂.
    pub se_b: f64,
    /// Standard error of μ̂ (OU asymptotic Phillips 1972).
    pub se_mu: f64,
    /// Standard error of θ̂.
    pub se_theta: f64,
    /// Half-life in hours: ln(2) / θ̂.
    pub half_life_h: f64,
    /// Bayesian credibility: t = μ̂ / SE(μ̂).
    pub t_statistic: f64,
}

impl FitOuOutput {
    /// True when the AR(1) regression produced a degenerate fit (b outside
    /// (0, 1)), meaning OU mean-reversion is not detected. Downstream code
    /// MUST check this before using `mu`, `sigma`, or `theta` — those fields
    /// contain NaN/0.0/Inf in the degenerate case (Python parity constraint).
    #[inline]
    pub fn is_degenerate(&self) -> bool {
        self.b >= 1.0 || self.b <= 0.0
    }
}

/// Fit OU process to a series of observations spaced `dt_hours` apart.
///
/// Input series is a slice of `(timestamp_ms, rate)` pairs; only the rate values
/// are used (timestamps are ignored; the caller must ensure uniform spacing).
///
/// Returns `Err(FrameworkError::InsufficientHistory)` if `n < 30` (matching Python).
/// Returns `Err(FrameworkError::RegressionFailed)` if the regression is degenerate
/// (all identical values → zero variance).
///
/// # Python parity
/// Mirrors `stochastic.fit_ou` exactly including:
/// - Left-fold sum ordering
/// - `sigma_eps_sq = sse / max(n_reg - 2, 1)`
/// - Degenerate-b branch returning NaN/Inf placeholders
/// - SE(μ̂) from Phillips 1972 asymptotic formula
pub fn fit_ou(series: &[(i64, f64)], dt_hours: f64) -> Result<FitOuOutput, FrameworkError> {
    let n = series.len();
    if n < 30 {
        return Err(FrameworkError::InsufficientHistory);
    }

    // Python: x = s[:-1], y = s[1:]
    let x: Vec<f64> = series[..n - 1].iter().map(|&(_, v)| v).collect();
    let y: Vec<f64> = series[1..].iter().map(|&(_, v)| v).collect();
    let n_reg = x.len(); // == n - 1

    // Left-fold sums (Python uses `sum(...)` which is left-fold)
    let sum_x: f64 = x.iter().sum();
    let sum_y: f64 = y.iter().sum();
    let mean_x = sum_x / n_reg as f64;
    let mean_y = sum_y / n_reg as f64;

    let sxx: f64 = x.iter().map(|&xi| (xi - mean_x) * (xi - mean_x)).sum();
    let sxy: f64 = x
        .iter()
        .zip(y.iter())
        .map(|(&xi, &yi)| (xi - mean_x) * (yi - mean_y))
        .sum();

    if sxx <= 0.0 {
        return Err(FrameworkError::RegressionFailed);
    }

    let b = sxy / sxx;
    let a = mean_y - b * mean_x;

    // Residual standard error
    // Python: residuals = [y[i] - a - b*x[i] for i in range(n_reg)]
    let residuals: Vec<f64> = (0..n_reg).map(|i| y[i] - a - b * x[i]).collect();
    let sse: f64 = residuals.iter().map(|&r| r * r).sum();
    let sigma_eps_sq = sse / std::cmp::max(n_reg as i64 - 2, 1) as f64;
    let sigma_eps = sigma_eps_sq.sqrt();

    let se_b = if sxx > 0.0 {
        sigma_eps / sxx.sqrt()
    } else {
        f64::INFINITY
    };

    // Python: if b >= 1.0 or b <= 0.0: return degenerate OUFit
    if b >= 1.0 || b <= 0.0 {
        return Ok(FitOuOutput {
            n_obs: n,
            a,
            b,
            sigma_eps,
            mu: f64::NAN,
            theta: 0.0,
            sigma: f64::NAN,
            se_b,
            se_mu: f64::INFINITY,
            se_theta: f64::INFINITY,
            half_life_h: f64::INFINITY,
            t_statistic: 0.0,
        });
    }

    // Python: theta = -log(b) / dt; mu = a / (1 - b)
    let theta = -b.ln() / dt_hours;
    let mu = a / (1.0 - b);
    // Python: sigma = sqrt(sigma_eps_sq * 2 * theta / (1 - b^2))
    let sigma = (sigma_eps_sq * 2.0 * theta / (1.0 - b * b)).sqrt();
    let half_life = if theta > 0.0 {
        std::f64::consts::LN_2 / theta
    } else {
        f64::INFINITY
    };

    // OU asymptotic SE of μ (Phillips 1972): Var(μ̂) = σ² / (2θ · T · Δt)
    // where T = n (sample size) and σ is OU diffusion volatility
    let t_total_h = n as f64 * dt_hours;
    let se_mu = if theta > 0.0 {
        sigma / (2.0 * theta * t_total_h).sqrt()
    } else {
        f64::INFINITY
    };
    // Python: se_theta = se_b / max(b, 1e-18)
    let se_theta = se_b / b.max(1e-18);

    // Python: t_stat = mu / se_mu if se_mu > 0 else inf
    let t_stat = if se_mu > 0.0 {
        mu / se_mu
    } else {
        f64::INFINITY
    };

    Ok(FitOuOutput {
        n_obs: n,
        a,
        b,
        sigma_eps,
        mu,
        theta,
        sigma,
        se_b,
        se_mu,
        se_theta,
        half_life_h: half_life,
        t_statistic: t_stat,
    })
}

// ===========================================================================
// §2 — Augmented Dickey-Fuller stationarity test
// ===========================================================================

/// MacKinnon (1996) critical values for ADF with constant only, no trend.
const ADF_CRITICAL_1PCT: f64 = -3.43;
const ADF_CRITICAL_5PCT: f64 = -2.86;
const ADF_CRITICAL_10PCT: f64 = -2.57;

/// MacKinnon (1996) critical values for ADF without constant.
const ADF_NC_CRITICAL_1PCT: f64 = -2.567;
const ADF_NC_CRITICAL_5PCT: f64 = -1.941;
#[allow(dead_code)]
const ADF_NC_CRITICAL_10PCT: f64 = -1.616;

/// Result of the Augmented Dickey-Fuller unit-root test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdfResult {
    /// ADF t-statistic.
    pub statistic: f64,
    /// Approximate p-value (rough MacKinnon bracket, not exact).
    pub p_value_estimate: f64,
    /// Number of augmentation lags used (Schwert's rule).
    pub n_lags: usize,
    /// Critical value at 5% significance level.
    pub critical_5pct: f64,
    /// Whether the test rejects the unit-root null at 5% level.
    pub rejects_unit_root: bool,
}

/// Augmented Dickey-Fuller test on series `s`.
///
/// H₀: unit root (random walk). Reject H₀ → mean-reverting → OU model valid.
///
/// Returns `Err(FrameworkError::InsufficientHistory)` if `T < 50` or if the
/// design matrix is too small after lag trimming (`n_lagged < 30`).
///
/// Returns `Err(FrameworkError::SingularMatrix)` if the normal equations are
/// singular (degenerate input).
///
/// # Python parity
/// Mirrors `stochastic.adf_test(s, with_constant=True)` exactly:
/// - Lag selection: `p = max(1, floor((T-1)^(1/3)))`
/// - Design: `Δs_{i+1} = α + β s_i + γ₁ Δs_i + … + γ_p Δs_{i-p+1} + ε`
/// - OLS via Gauss-Jordan with partial pivoting (matches `_solve_with_inverse`)
/// - Critical values: MacKinnon 1996, constant-only model
pub fn adf_test(series: &[f64]) -> Result<AdfResult, FrameworkError> {
    adf_test_inner(series, true)
}

/// Internal: ADF test with optional constant term (mirrors Python's `with_constant` param).
fn adf_test_inner(s: &[f64], with_constant: bool) -> Result<AdfResult, FrameworkError> {
    let big_t = s.len();
    if big_t < 50 {
        return Err(FrameworkError::InsufficientHistory);
    }

    // Schwert's rule: p = max(1, floor((T-1)^(1/3)))
    let p = std::cmp::max(1, ((big_t - 1) as f64).powf(1.0 / 3.0) as usize);

    // diffs: Δs_t = s[t] - s[t-1] for t = 1..T
    // NOTE: Python computes `diffs = [s[i]-s[i-1] for i in range(1, T)]` here but never
    // actually references it in the row builder (all differences are re-computed inline
    // as `s[i-k+1] - s[i-k]`). We omit the dead variable.

    let n_lagged = big_t - p - 1;
    if n_lagged < 30 {
        return Err(FrameworkError::InsufficientHistory);
    }

    // Build design matrix and target vector.
    // Python: for i in range(p, T-1): Δs_{i+1} = α + β s_i + γ_1 Δs_i + ... + γ_p Δs_{i-p+1}
    let mut y_vec: Vec<f64> = Vec::with_capacity(big_t - p - 1);
    let mut rows: Vec<Vec<f64>> = Vec::with_capacity(big_t - p - 1);

    for i in p..big_t - 1 {
        // Python: Y.append(s[i+1] - s[i])
        y_vec.push(s[i + 1] - s[i]);
        let mut row: Vec<f64> = Vec::new();
        if with_constant {
            row.push(1.0);
        }
        // Python: row.append(s[i])  — coefficient β on s_{i}
        row.push(s[i]);
        // Python: for k in range(1, p+1): row.append(s[i-k+1] - s[i-k])
        for k in 1..=p {
            row.push(s[i - k + 1] - s[i - k]);
        }
        rows.push(row);
    }

    let n_rows = rows.len();
    let n_cols = rows[0].len();

    // Build X'X and X'Y by left-fold (Python loops in row order)
    let mut xtx: Vec<Vec<f64>> = vec![vec![0.0; n_cols]; n_cols];
    let mut xty: Vec<f64> = vec![0.0; n_cols];

    for i in 0..n_rows {
        let row = &rows[i];
        let yi = y_vec[i];
        for a in 0..n_cols {
            xty[a] += row[a] * yi;
            for b_idx in 0..n_cols {
                xtx[a][b_idx] += row[a] * row[b_idx];
            }
        }
    }

    // Solve via Gauss-Jordan with partial pivoting (mirrors Python `_solve_with_inverse`)
    let (beta_hat, xtx_inv) =
        solve_with_inverse(&xtx, &xty).map_err(|_| FrameworkError::SingularMatrix)?;

    // Residuals and degrees of freedom
    let residuals: Vec<f64> = (0..n_rows)
        .map(|i| {
            let pred: f64 = rows[i]
                .iter()
                .zip(beta_hat.iter())
                .map(|(&r, &b)| r * b)
                .sum();
            y_vec[i] - pred
        })
        .collect();
    let sse: f64 = residuals.iter().map(|&r| r * r).sum();
    let df = n_rows as i64 - n_cols as i64;
    if df <= 0 {
        return Err(FrameworkError::InsufficientHistory);
    }
    let sigma2 = sse / df as f64;

    // ADF statistic = β̂ / SE(β̂); β is coefficient on s_i
    // Python: beta_idx = 1 if with_constant else 0
    let beta_idx = if with_constant { 1 } else { 0 };
    let var_beta = sigma2 * xtx_inv[beta_idx][beta_idx];
    if var_beta <= 0.0 {
        return Err(FrameworkError::RegressionFailed);
    }
    let se_beta = var_beta.sqrt();
    if se_beta == 0.0 {
        return Err(FrameworkError::RegressionFailed);
    }
    let adf_stat = beta_hat[beta_idx] / se_beta;

    let (cv1, cv5) = if with_constant {
        (ADF_CRITICAL_1PCT, ADF_CRITICAL_5PCT)
    } else {
        (ADF_NC_CRITICAL_1PCT, ADF_NC_CRITICAL_5PCT)
    };

    let rejects = adf_stat < cv5;

    // P-value approximation (rough bracket, mirrors Python exactly)
    let p_value = if adf_stat < cv1 {
        0.005
    } else if adf_stat < cv5 {
        0.025
    } else if adf_stat < ADF_CRITICAL_10PCT {
        0.075
    } else {
        0.5
    };

    Ok(AdfResult {
        statistic: adf_stat,
        p_value_estimate: p_value,
        n_lags: p,
        critical_5pct: cv5,
        rejects_unit_root: rejects,
    })
}

/// Solve `Ax = b` for small symmetric positive-definite A using Gauss-Jordan
/// elimination with partial pivoting.
///
/// Returns `(solution_x, A_inverse)`.
///
/// Faithful port of Python `stochastic._solve_with_inverse`. Panics replaced with
/// `Err` so callers can map to `FrameworkError::SingularMatrix`.
// The inner loops use `j` to index two rows simultaneously (aug[r][j] and aug[col][j]),
// which cannot be replaced with a single iterator without unsafe borrow-splitting.
#[allow(clippy::needless_range_loop)]
fn solve_with_inverse(
    a: &[Vec<f64>],
    b: &[f64],
) -> Result<(Vec<f64>, Vec<Vec<f64>>), &'static str> {
    let n = a.len();
    // Augmented matrix: [A | I | b]  — total width = 2n + 1
    // Python: aug = [[...A_row..., ...identity_row..., b_i] for ...]
    let mut aug: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let mut row = a[i].clone();
            for j in 0..n {
                row.push(if i == j { 1.0 } else { 0.0 });
            }
            row.push(b[i]);
            row
        })
        .collect();

    let width = 2 * n + 1;

    // Forward elimination with partial pivoting (mirrors Python exactly)
    for col in 0..n {
        // Find pivot row
        let mut pivot_row = col;
        for r in col + 1..n {
            if aug[r][col].abs() > aug[pivot_row][col].abs() {
                pivot_row = r;
            }
        }
        if aug[pivot_row][col].abs() < 1e-14 {
            return Err("singular matrix");
        }
        if pivot_row != col {
            aug.swap(col, pivot_row);
        }
        let pivot = aug[col][col];
        for j in 0..width {
            aug[col][j] /= pivot;
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let factor = aug[r][col];
            for j in 0..width {
                aug[r][j] -= factor * aug[col][j];
            }
        }
    }

    // Extract inverse (columns n..2n) and solution (column 2n)
    let inv: Vec<Vec<f64>> = aug.iter().map(|row| row[n..2 * n].to_vec()).collect();
    let sol: Vec<f64> = aug.iter().map(|row| row[2 * n]).collect();
    Ok((sol, inv))
}

// ===========================================================================
// §6 — Empirical CVaR for drawdown stops
// ===========================================================================

/// Drawdown stop derived from the upper tail of |basis divergence|.
///
/// Returns absolute drawdown threshold (fraction of notional).
///
/// Python: `stochastic.cvar_drawdown_stop(basis_history, q=0.01, safety_multiplier=2.0, min_history=100)`.
///
/// Bootstrap fallback `0.005` is returned when `len(basis_history) < min_history`.
///
/// # Arguments
/// - `basis_series`: slice of `(timestamp_ms, basis_value)` pairs; only values used.
/// - `alpha`: tail quantile (Python `q`). E.g. 0.01 → top 1% of |basis|.
/// - `multiplier`: safety multiplier (Python `safety_multiplier`). E.g. 2.0.
pub fn cvar_drawdown_stop(basis_series: &[(i64, f64)], alpha: f64, multiplier: f64) -> f64 {
    const MIN_HISTORY: usize = 100;
    if basis_series.len() < MIN_HISTORY {
        return 0.005; // bootstrap fallback (Python constant)
    }
    // Python: abs_basis = [abs(x) for x in basis_history]
    let mut abs_basis: Vec<f64> = basis_series.iter().map(|&(_, v)| v.abs()).collect();
    let tail = upper_tail_mean(&mut abs_basis, alpha);
    tail * multiplier
}

/// E[X | X ≥ (1-q)-quantile] — upper tail mean on a mutable sorted-descending copy.
///
/// Python: `upper_tail_mean(samples, q=0.05)`.
/// `k = max(1, floor(q * n))` items from the top.
fn upper_tail_mean(samples: &mut [f64], q: f64) -> f64 {
    let n = samples.len();
    if n == 0 {
        return 0.0;
    }
    // Python: sorted(samples, reverse=True)
    samples.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    // Python: k = max(1, int(math.floor(q * n)))
    let k = std::cmp::max(1, (q * n as f64).floor() as usize);
    // Python: statistics.mean(s[:k])
    let mean: f64 = samples[..k].iter().sum::<f64>() / k as f64;
    mean
}

// ===========================================================================
// §4 — Optimal hold via OU expected residual income
// ===========================================================================

/// ∫₀^τ E[sign(d)·s_{t+u}] du — total expected income over hold period.
///
/// For θ > 0 (OU mean-reversion regime): closed-form integral.
/// For θ ≤ 0 (drift-persistent regime, H > 0.7): income = direction · μ · hold_h.
///
/// # Arguments
/// - `ou`: OU fit parameters. `mu` and `current_spread` must be in the same units.
/// - `current_spread`: current observed spread s_now.
/// - `hold`: planning horizon in hours.
/// - `direction`: +1 for long spread, -1 for short.
///
/// # Python parity
/// Mirrors `stochastic.expected_residual_income(s_now, mu, theta, hold_h, direction)` exactly.
pub fn expected_residual_income(
    s_now: f64,
    mu: f64,
    theta: f64,
    hold_h: f64,
    direction: i32,
) -> f64 {
    let dir = direction as f64;
    if theta <= 0.0 {
        // Drift regime: no mean reversion
        return dir * mu * hold_h;
    }
    // Python: drift_term = direction * mu * hold_h
    //         decay_term = direction * (s_now - mu) * (1 - exp(-theta * hold_h)) / theta
    let drift_term = dir * mu * hold_h;
    let decay_term = dir * (s_now - mu) * (1.0 - (-theta * hold_h).exp()) / theta;
    drift_term + decay_term
}

// ===========================================================================
// §1b — Drift-model fit
// ===========================================================================

/// Drift-model fit output. Reuses the same field layout as `FitOuOutput` with
/// `theta = 0`, `b = 1.0`, `half_life_h = inf` to allow downstream code to
/// distinguish OU (theta > 0) from drift (theta = 0) regimes.
///
/// This struct has the same fields as `FitOuOutput`. A separate type is used
/// for documentation clarity and future divergence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitDriftOutput {
    pub n_obs: usize,
    /// μ̂ = sample mean = `a` (drift intercept).
    pub a: f64,
    /// b = 1.0 always (no mean reversion).
    pub b: f64,
    /// σ̂_ε = sample std.
    pub sigma_eps: f64,
    /// μ̂ = sample mean.
    pub mu: f64,
    /// θ = 0.0 always (drift regime).
    pub theta: f64,
    /// σ = σ_ε (same as sigma_eps for drift model, Python `fit_drift.sigma`).
    pub sigma: f64,
    /// SE(b̂) = ∞ (undefined for random walk model).
    pub se_b: f64,
    /// SE(μ̂) = σ_ε / √n (iid approximation).
    pub se_mu: f64,
    /// SE(θ̂) = ∞.
    pub se_theta: f64,
    /// Half-life = ∞ (no mean reversion).
    pub half_life_h: f64,
    /// t = μ̂ / SE(μ̂).
    pub t_statistic: f64,
}

/// Drift-model fit for persistent (H > 0.7) spreads that are NOT mean-reverting.
///
/// Model: `s_t = μ + ε_t`, where ε_t are treated as i.i.d. N(0, σ²_ε).
/// The t-statistic is an optimistic sanity check (not a valid p-value with H ≈ 0.9).
///
/// Returns `Err(FrameworkError::InsufficientHistory)` if `n < 30`.
/// Returns `Err(FrameworkError::RegressionFailed)` if σ_ε ≤ 0 (all identical values).
///
/// # Python parity
/// Mirrors `stochastic.fit_drift` exactly, including:
/// - `var_s = sum((v - mean_s)^2 for v in s) / max(n - 1, 1)`
/// - `se_mu = sigma_eps / sqrt(n)` (iid, optimistic for H > 0.5)
pub fn fit_drift(series: &[(i64, f64)], _dt_hours: f64) -> Result<FitDriftOutput, FrameworkError> {
    let n = series.len();
    if n < 30 {
        return Err(FrameworkError::InsufficientHistory);
    }
    let s: Vec<f64> = series.iter().map(|&(_, v)| v).collect();
    // Python: mean_s = sum(s) / n
    let mean_s: f64 = s.iter().sum::<f64>() / n as f64;
    // Python: var_s = sum((v - mean_s)**2 for v in s) / max(n - 1, 1)
    let var_s: f64 = s.iter().map(|&v| (v - mean_s) * (v - mean_s)).sum::<f64>()
        / std::cmp::max(n as i64 - 1, 1) as f64;
    let sigma_eps = var_s.sqrt();
    if sigma_eps <= 0.0 {
        return Err(FrameworkError::RegressionFailed);
    }
    // Python: se_mu = sigma_eps / sqrt(n)
    let se_mu = sigma_eps / (n as f64).sqrt();
    // Python: t_stat = mean_s / se_mu if se_mu > 0 else inf
    let t_stat = if se_mu > 0.0 {
        mean_s / se_mu
    } else {
        f64::INFINITY
    };

    Ok(FitDriftOutput {
        n_obs: n,
        a: mean_s,
        b: 1.0,
        sigma_eps,
        mu: mean_s,
        theta: 0.0,
        sigma: sigma_eps,
        se_b: f64::INFINITY,
        se_mu,
        se_theta: f64::INFINITY,
        half_life_h: f64::INFINITY,
        t_statistic: t_stat,
    })
}

// ===========================================================================
// Internal unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build (ts, value) pairs with dummy timestamps
    fn to_series(vals: &[f64]) -> Vec<(i64, f64)> {
        vals.iter()
            .enumerate()
            .map(|(i, &v)| (i as i64, v))
            .collect()
    }

    // -----------------------------------------------------------------------
    // fit_ou — edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn fit_ou_insufficient_history() {
        let short: Vec<(i64, f64)> = (0..29).map(|i| (i as i64, 0.0001_f64)).collect();
        assert_eq!(
            fit_ou(&short, 1.0).unwrap_err(),
            FrameworkError::InsufficientHistory
        );
    }

    #[test]
    fn fit_ou_degenerate_constant_series() {
        // All-constant series: due to float accumulation, sxx may be a tiny positive
        // number rather than exactly 0, so fit_ou may return Ok with degenerate values
        // (NaN mu, b ≈ 1, theta ≈ 0) rather than Err — both are acceptable.
        // Python also hits the degenerate-b branch for this case.
        // We just verify it doesn't panic and either returns an error or a degenerate fit.
        let constant: Vec<(i64, f64)> = (0..50).map(|i| (i as i64, 0.0001_f64)).collect();
        match fit_ou(&constant, 1.0) {
            Err(_) => { /* RegressionFailed is acceptable */ }
            Ok(r) => {
                // Degenerate fit: mu and sigma should be NaN (b out of (0, 1) range)
                assert!(
                    r.mu.is_nan() || r.mu.is_infinite() || r.theta == 0.0,
                    "constant series should produce degenerate fit, got mu={}, theta={}",
                    r.mu,
                    r.theta
                );
            }
        }
    }

    #[test]
    fn fit_ou_basic_sanity() {
        // Synthetic OU: mu=0.0001, theta=0.1, dt=1
        // Just check it returns Ok and theta > 0, |mu| > 0
        let sample = generate_ou_sample(0.0001, 0.1, 0.0001, 200, 1.0, 0.0, 42);
        let series = to_series(&sample);
        let result = fit_ou(&series, 1.0).unwrap();
        assert!(result.theta > 0.0, "theta should be positive for OU");
        assert!(result.mu.is_finite(), "mu should be finite");
        assert!(result.sigma > 0.0, "sigma should be positive");
    }

    // -----------------------------------------------------------------------
    // fit_drift — internal sanity tests with known slope/intercept
    // -----------------------------------------------------------------------

    /// Generate a nearly-constant-drift series: s_t = mu + noise
    fn noisy_mean_series(mu: f64, sigma: f64, n: usize, seed: u64) -> Vec<(i64, f64)> {
        // LCG PRNG (deterministic, no external crate)
        let mut state = seed;
        let lcg = |s: &mut u64| -> f64 {
            *s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            // Box-Muller transform
            let u1 = (*s >> 11) as f64 / (1u64 << 53) as f64;
            *s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u2 = (*s >> 11) as f64 / (1u64 << 53) as f64;

            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        };
        (0..n)
            .map(|i| {
                let noise = lcg(&mut state) * sigma;
                (i as i64, mu + noise)
            })
            .collect()
    }

    #[test]
    fn fit_drift_recovers_mean_positive() {
        // mu = 0.001, sigma = 0.0001, n = 1000
        // With n=1000 and SNR = 0.001/0.0001 = 10, t-stat should be large
        let series = noisy_mean_series(0.001, 0.0001, 1000, 1);
        let result = fit_drift(&series, 1.0).unwrap();
        assert!(
            (result.mu - 0.001).abs() < 0.00002,
            "mu recovery off: got {}, expected ~0.001",
            result.mu
        );
        assert!(
            result.t_statistic > 30.0,
            "t-stat should be large for high-SNR signal, got {}",
            result.t_statistic
        );
        assert_eq!(result.theta, 0.0, "drift regime must have theta=0");
        assert_eq!(result.b, 1.0, "drift regime must have b=1");
        assert!(
            result.half_life_h.is_infinite(),
            "drift regime half_life must be inf"
        );
    }

    #[test]
    fn fit_drift_recovers_mean_negative() {
        // mu = -0.0005, sigma = 0.00005, n = 500
        let series = noisy_mean_series(-0.0005, 0.00005, 500, 2);
        let result = fit_drift(&series, 1.0).unwrap();
        assert!(
            (result.mu - (-0.0005)).abs() < 0.00001,
            "mu recovery off: got {}, expected ~-0.0005",
            result.mu
        );
        assert!(
            result.t_statistic < -20.0,
            "t-stat should be large negative for negative signal, got {}",
            result.t_statistic
        );
    }

    #[test]
    fn fit_drift_recovers_mean_near_zero() {
        // mu ≈ 0.0002, sigma = 0.0001, n = 200
        let series = noisy_mean_series(0.0002, 0.0001, 200, 3);
        let result = fit_drift(&series, 1.0).unwrap();
        // Allow ±3σ/√n tolerance: 3 * 0.0001 / sqrt(200) ≈ 0.0000212
        let tol = 3.0 * 0.0001 / (200.0_f64).sqrt();
        assert!(
            (result.mu - 0.0002).abs() < tol * 3.0,
            "mu recovery off: got {}, expected ~0.0002, tol {}",
            result.mu,
            tol
        );
    }

    #[test]
    fn fit_drift_insufficient_history() {
        let short: Vec<(i64, f64)> = (0..29)
            .map(|i| (i as i64, 0.001_f64 + i as f64 * 1e-6))
            .collect();
        assert_eq!(
            fit_drift(&short, 1.0).unwrap_err(),
            FrameworkError::InsufficientHistory
        );
    }

    // -----------------------------------------------------------------------
    // cvar_drawdown_stop — edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn cvar_bootstrap_fallback() {
        // len < 100 → 0.005
        let short: Vec<(i64, f64)> = (0..50).map(|i| (i as i64, 0.001)).collect();
        assert_eq!(cvar_drawdown_stop(&short, 0.01, 2.0), 0.005);
    }

    #[test]
    fn cvar_empty_fallback() {
        assert_eq!(cvar_drawdown_stop(&[], 0.01, 2.0), 0.005);
    }

    #[test]
    fn cvar_positive_result() {
        // 200 identical values of 0.01 → upper 1% mean = 0.01, × 2 = 0.02
        let series: Vec<(i64, f64)> = (0..200).map(|i| (i as i64, 0.01_f64)).collect();
        let result = cvar_drawdown_stop(&series, 0.01, 2.0);
        assert!((result - 0.02).abs() < 1e-12, "got {}", result);
    }

    // -----------------------------------------------------------------------
    // expected_residual_income — edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn eri_drift_regime() {
        // theta <= 0 → direction * mu * hold_h
        let result = expected_residual_income(0.001, 0.0001, 0.0, 168.0, 1);
        assert!((result - 0.0001 * 168.0).abs() < 1e-15);
    }

    #[test]
    fn eri_ou_zero_spread_equals_mu_hold() {
        // When s_now == mu, decay_term = 0, result = direction * mu * hold_h
        let mu = 0.001;
        let theta = 0.1;
        let hold = 20.0;
        let result = expected_residual_income(mu, mu, theta, hold, 1);
        let expected = mu * hold;
        assert!(
            (result - expected).abs() < 1e-14,
            "got {}, expected {}",
            result,
            expected
        );
    }

    // -----------------------------------------------------------------------
    // adf_test — basic smoke
    // -----------------------------------------------------------------------

    #[test]
    fn adf_insufficient_history() {
        let short: Vec<f64> = (0..49).map(|i| i as f64 * 0.001).collect();
        assert_eq!(
            adf_test(&short).unwrap_err(),
            FrameworkError::InsufficientHistory
        );
    }

    // -----------------------------------------------------------------------
    // Helpers for internal tests
    // -----------------------------------------------------------------------

    /// Generate OU sample using Python's exact generator:
    /// next = mu*(1-b) + b*prev + gauss(0, sigma_eps)
    /// with Python's random.Random(seed) Mersenne Twister via Box-Muller.
    ///
    /// NOTE: This Rust reimplementation uses a simple LCG so results won't
    /// match Python's Mersenne Twister. Only used for structural sanity tests,
    /// not for parity verification (parity tests use fixtures from the spec).
    fn generate_ou_sample(
        mu: f64,
        theta: f64,
        sigma: f64,
        t: usize,
        dt: f64,
        x0: f64,
        seed: u64,
    ) -> Vec<f64> {
        let b = (-theta * dt).exp();
        let sigma_eps = sigma * ((1.0 - b * b) / (2.0 * theta)).sqrt();
        let mut state = seed;
        let mut gauss = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u1 = (state >> 11) as f64 / (1u64 << 53) as f64;
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u2 = (state >> 11) as f64 / (1u64 << 53) as f64;
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        };
        let mut out = vec![x0];
        for _ in 0..t - 1 {
            let next = mu * (1.0 - b) + b * out[out.len() - 1] + gauss() * sigma_eps;
            out.push(next);
        }
        out
    }
}
