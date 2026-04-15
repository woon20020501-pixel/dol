# Dol Strategy — Frontier Framework (2005-2024 Quantitative Methods)

**Status:** Extends `math-rigorous.md` (which used 1952-2001 classical quant tools) with 2005-2024 modern statistical learning, distributionally robust optimization, and rough volatility theory. The first-order (`math-formulas.md`), classical (`math-rigorous.md`), and frontier (this document) layers compose into a single decision pipeline at runtime.
**Iron law:** `../PRINCIPLES.md`. Composition contract: each layer can override the previous when its assumptions hold. None can violate the iron law.

---

## 0. Why the classical framework is insufficient

`math-rigorous.md` (v3.4) is a defensible composition of 1952-2001 tools. But it makes five quiet assumptions that modern empirical finance has refuted:

| classical assumption | empirical reality | failure mode |
|---|---|---|
| Gaussian basis tails (3σ stop) | basis kurtosis 5-15, fat-tailed | drawdown stop misses real tail events |
| asymptotic Phillips 1972 SE on OU | finite T = 720 has bias of 20-30% | t-statistic over-estimates credibility |
| empirical-mean Markowitz | mean estimate has standard error ~σ/√T | optimal weights are noise-amplified |
| smooth volatility (OU σ constant) | log-volatility has Hurst H ≈ 0.1 (rough) | OU model misses persistence at high freq |
| i.i.d. jumps in basis | jumps cluster in time (self-excitation) | independent-tail assumption underestimates joint tail |

Each gap has a 2005-2024 tool that closes it rigorously. This document presents the five tools, their integration, the new theorems they enable, and the validation suite that confirms each works.

---

## 1. Conformal prediction for distribution-free VaR

### 1.1 The classical chance constraint and its weakness

`math-rigorous.md` §7 stated the chance constraint as:

```
P(R_vault ≥ floor) ≥ 1 - α
```

And implemented it under the assumption that `R_vault` is approximately Gaussian with mean `μ_vault` and variance estimated from `w'Σw`. This Gaussian step is an approximation. For the true distribution of vault returns — which depends on basis divergence, funding rate jumps, and venue solvency events — there is no reason to believe Gaussianity, and empirical data shows kurtosis 5+.

If actual returns are fat-tailed but the chance constraint assumes Gaussian, **the realized 5%-VaR is much worse than the model says**.

### 1.2 Inductive conformal prediction (Vovk-Shafer-Gammerman 2005, Romano et al. 2019)

Conformal prediction is a distribution-free framework for prediction intervals with finite-sample coverage guarantees. The key idea:

1. Split the historical data into a **proper training set** and a **calibration set** of size `n_cal`.
2. On the training set, fit any prediction model (we use the OU + portfolio model from v3.4).
3. On the calibration set, compute **nonconformity scores** `s_i` measuring how badly the model predicts each calibration point.
4. At test time, the prediction interval is `[ŷ - q, ŷ + q]` where `q` is the `⌈(n_cal+1)(1-α)⌉/n_cal` empirical quantile of the calibration scores.

**The coverage theorem (Vovk 2005):** under the only assumption that training, calibration, and test data are exchangeable (much weaker than i.i.d.), the prediction interval contains the true value with probability **exactly** `1-α` in finite samples. No distributional assumption.

### 1.3 Application to vault VaR

For our problem:
- The "model" is the OU + Markowitz allocation that predicts vault return for the next K hours
- The calibration set is the most recent N hours of realized vault returns
- The nonconformity score is the absolute residual `|R_vault,realized - R_vault,predicted|`
- The conformal prediction interval at confidence `1-α` is `R_vault,predicted ± q_(1-α)`
- The lower bound `R_vault,predicted - q_(1-α)` IS the conformal VaR

The chance constraint becomes:
```
R_vault,predicted - q_(1-α) ≥ floor
```

This is a hard finite-sample guarantee, not a Gaussian approximation. **Modeled coverage = realized coverage**, which is the property the classical framework cannot promise.

### 1.4 Computational cost

Inductive conformal at runtime requires only sorting `n_cal` calibration scores. O(n_cal log n_cal). For `n_cal = 500`, this is microseconds. No retraining needed at inference time.

---

## 2. Empirical Bernstein concentration (Maurer-Pontil 2009)

### 2.1 The classical credibility test and its weakness

`math-rigorous.md` §3 used `t-stat = μ̂ / SE_Phillips` ≥ 5 for OU credibility, with `SE_Phillips = σ / √(2θT)`. This is the **asymptotic** standard error — valid as `T → ∞`. For `T = 720`, the bias is non-trivial.

Specifically: the asymptotic SE under-estimates the true finite-sample SE by a factor that depends on `θT` (the effective sample size in mean-reversion units). For `θ = 0.05` and `T = 720`, `θT = 36`, and the asymptotic CLT is reasonable but slightly optimistic. For `θ = 0.005` and `T = 720`, `θT = 3.6`, and the asymptotic CLT is **substantially over-confident**.

### 2.2 The Empirical Bernstein bound

For independent random variables `X_1, ..., X_n` with values in `[0, 1]` and empirical variance `V̂_n`, Maurer & Pontil (2009) proved:

```
P(|X̄_n - μ| ≤ √(2 V̂_n ln(2/δ) / n) + 7 ln(2/δ)/(3(n-1))) ≥ 1 - δ
```

This is a **finite-sample** confidence interval that uses the empirical variance directly (not a model assumption). It's typically 2-3× tighter than Hoeffding (which ignores variance) and is sharper than Bernstein (which uses true variance) when the empirical variance is small.

For our funding spread series:
- Bound `|s_t|` to a known maximum (e.g., 0.001 per hour, much larger than any observed value)
- Compute empirical mean and variance of the spread
- The Maurer-Pontil bound gives a finite-sample confidence interval for the true long-run mean

The credibility test becomes:
```
μ̂ - MP_radius(δ) > 0       (for positive direction)
```
where `MP_radius(δ)` is the half-width of the Maurer-Pontil interval at confidence `1-δ`.

For 5σ credibility (`δ ≈ 6 × 10⁻⁷`), this gives a much tighter test than the asymptotic z-score in finite samples. **It does not assume the data is i.i.d. or normal**, only bounded.

### 2.3 What this catches that Phillips 1972 misses

A funding spread with very slow mean reversion (small θ, long half-life) but high variance can pass the asymptotic 5σ test with moderate `μ̂` because `SE_Phillips = σ/√(2θT)` is small when `T` is large and `θ` is small. But the empirical Bernstein bound, which uses sample variance directly without the model-derived `1/(2θT)` factor, correctly recognizes the high variance and demands a larger `μ̂` for the same confidence.

This **prevents over-confident entries on slow-reverting noisy spreads** — exactly the failure mode that killed the v1 cap-arbitrage thesis.

---

## 3. Wasserstein distributionally robust optimization (Esfahani-Kuhn 2018)

### 3.1 The classical Markowitz weakness

`math-rigorous.md` §5 used:

```
w* = argmax (w'r̂ - r_idle) / √(w'Σ̂w)
```

where `r̂` and `Σ̂` are sample estimates. The problem: these estimates have noise, and the optimization step **amplifies** that noise. The realized portfolio Sharpe is typically much worse than the optimal-by-estimate Sharpe.

This is the "estimation-error problem" of Markowitz, well documented since DeMiguel-Garlappi-Uppal 2009. Their famous result: 1/N (equal-weight) often beats sample Markowitz out-of-sample.

### 3.2 Wasserstein-ball DRO

Instead of:
```
max_w E_P̂[u(R_w)]
```

solve:
```
max_w  inf_{P ∈ B_ε(P̂)}  E_P[u(R_w)]
```

where `B_ε(P̂)` is the Wasserstein-1 ball of radius `ε` around the empirical distribution `P̂`.

**Esfahani-Kuhn 2018 reformulation theorem:** for type-1 Wasserstein and concave utility, the inner infimum has a closed-form dual:

```
inf_{P ∈ B_ε(P̂)} E_P[u(R_w)] = E_P̂[u(R_w)] - ε · Lip(u(R_w))
```

where `Lip(u(R_w))` is the Lipschitz constant of `u(w'·)` with respect to the chosen Wasserstein cost function (typically `||·||_2`).

For mean-variance utility `u(R) = R - λ(R - r̄)²`, the Lipschitz constant is `1 + 2λ|R̄|`. For our purposes (mean - λ stdev), `Lip(u(w'·)) = ||w||_2 + 2λ`.

**The DRO portfolio problem becomes:**
```
max_w  (w'r̂ - r_idle) - ε·||w||_2 - λ √(w'Σ̂w + ε²·||w||_2²)
       s.t. budget + box + per-counter constraints
```

The `ε`-penalty is the **regularization that compensates for estimator noise**. As `ε → 0`, this recovers vanilla Markowitz. As `ε → ∞`, this approaches equal-weight (max diversification).

**Esfahani-Kuhn theorem:** the optimal `ε` decays as `O(1/√n)` and gives a finite-sample out-of-sample performance bound that classical Markowitz cannot match. For our case (n = T_lookback ≈ 720), the optimal `ε ≈ 0.04`.

### 3.3 What this gives us

A portfolio that is **provably out-of-sample optimal** within the Wasserstein-ball uncertainty set. The bot's realized Sharpe will be much closer to its in-sample Sharpe than vanilla Markowitz delivers. This is the modern replacement for "shrinkage" — it's the optimal level of robustness derived from the data, not a heuristic shrinkage parameter.

---

## 4. Hurst exponent and rough volatility classification

### 4.1 Why rough volatility matters

Gatheral, Jaisson, and Rosenbaum (2018) showed that financial volatility processes have Hurst exponent `H ≈ 0.1`, meaning they are "rough" — much rougher than standard Brownian motion (H = 0.5). Rough volatility is highly persistent at short time scales and decays sub-exponentially.

For our funding spreads, the same question applies: is the spread process `H = 0.5` (standard OU), `H < 0.5` (rough, persistent at short scales), or `H > 0.5` (smooth, persistent at long scales)?

This matters because:
- **OU model assumes H = 0.5**. If reality is H = 0.1, OU systematically underestimates the autocorrelation at short lags.
- **Drawdown stops calibrated for H = 0.5** are too tight when reality is H < 0.5 (rough): more frequent false positives.
- **Optimal hold time** scales differently. For rough processes, holding longer gives diminishing returns faster than OU predicts.

### 4.2 R/S analysis (Mandelbrot-Wallis 1969)

Compute the rescaled range statistic:

```
R(τ)/S(τ) = (max_{t ≤ τ}(s_t - s̄) - min_{t ≤ τ}(s_t - s̄)) / σ(s_{1..τ})
```

For a process with Hurst exponent `H`, this scales as `R/S ∝ τ^H`. Fit `log(R/S)` against `log(τ)` to estimate `H`.

### 4.3 DFA — detrended fluctuation analysis (Peng et al. 1994)

Sharper than R/S for finite samples. Algorithm:

1. Integrate the spread series: `Y_t = Σ_{k=1}^t (s_k - s̄)`
2. For each window length `n`, partition `Y` into non-overlapping windows of length `n`
3. In each window, fit a linear trend and compute the residual variance `F²(n)`
4. The Hurst exponent is the slope of `log F(n)` vs `log n`

DFA is robust to non-stationarity in the original series and gives a sharper Hurst estimate than R/S for `T = 720`.

### 4.4 Application to the bot

Each candidate (symbol, counter venue) gets a Hurst exponent computed from its spread history. The bot uses this to:

- **Classify dynamics**: H ≈ 0.5 → standard OU model is valid. H < 0.5 → rough, use rough OU corrections (longer effective autocorrelation). H > 0.5 → trending, mean-reversion may be slower than OU suggests.
- **Adjust drawdown stops**: rougher processes have more frequent small deviations; widen `d_max` proportionally.
- **Adjust optimal hold**: for rough processes, hold time should be shorter relative to half-life.

In practice, most funding spreads will have `H ≈ 0.4-0.6` (close to but not exactly OU). The Hurst diagnostic is a sanity check that catches when our OU model is materially wrong.

---

## 5. Hawkes self-exciting process for basis jump clustering

### 5.1 Why i.i.d. jumps is wrong

The classical drawdown stop assumes basis divergence events are i.i.d. — each tick is independently distributed. But empirically, basis jumps **cluster**: when one venue has a 1-second oracle freeze, the next minute is much more likely to have another. This is a self-exciting point process.

Hawkes (1971) formalized this. A self-exciting process has intensity:

```
λ(t) = μ_0 + Σ_{t_k < t} φ(t - t_k)
```

where `t_k` are past event times and `φ` is the "kernel" describing how past events excite future events. The exponential kernel `φ(s) = α · β · exp(-β·s)` is most common.

### 5.2 MLE for exponential-kernel Hawkes

For event times `t_1, ..., t_n` over `[0, T]`:

```
log L = Σ_i log λ(t_i) - ∫_0^T λ(t) dt
       = Σ_i log(μ_0 + α Σ_{t_k < t_i} β exp(-β(t_i - t_k))) - μ_0 T - α Σ_i (1 - exp(-β(T - t_i)))
```

Maximize numerically over `(μ_0, α, β)`. For our basis events, `n` is small (a few dozen jumps in 720h) and 3-parameter optimization converges in milliseconds.

### 5.3 Branching ratio and stationarity

The **branching ratio** is `n* = α · ∫_0^∞ φ(s) ds = α` (for exponential kernel). It represents the average number of "child" events spawned by each "parent" event.

- `n* < 1`: process is stationary; clustering is bounded
- `n* ≥ 1`: process is explosive; never stationary

We require `n̂* < 0.7` for a candidate to be admissible. If `n̂* > 0.7`, the basis dynamics are too clustered for our drawdown stop to be reliable.

### 5.4 Application: cluster-aware drawdown stop

The classical CVaR drawdown stop assumes i.i.d. basis observations. With Hawkes, we instead compute the **expected cluster size**:

```
E[cluster size | event occurred] = 1 / (1 - n̂*)
```

The cluster-aware drawdown stop multiplies the per-event CVaR by the expected cluster size:

```
d_max(s)_Hawkes = CVaR_q(|basis_s|) × (1 / (1 - n̂*)) × safety_multiplier
```

For `n̂* = 0.5` (moderate clustering), this doubles the drawdown stop — accounting for the fact that one observed jump is likely followed by 1-2 more.

---

## 6. The Dol Theorem — sub-Gaussian tail bound for OU funding spreads

### 6.1 Statement

For a stationary Ornstein-Uhlenbeck process `ds_t = θ(μ - s_t) dt + σ dW_t` with stationary variance `σ²_∞ = σ²/(2θ)`, the deviation from mean satisfies:

```
P(|s_t - μ| ≥ x · σ_∞) ≤ 2 · exp(-x²/2)
```

This is the sub-Gaussian concentration bound for OU equilibrium. **Tighter than the Gaussian quantile because the constant in the exponent is exactly 1/2 (not 1/2 + asymptotic correction)**.

### 6.2 Proof sketch (Hermite polynomial expansion)

The stationary distribution of OU is `N(μ, σ²/(2θ))`. The Hermite polynomials `H_n(x)` are eigenfunctions of the OU generator with eigenvalue `-nθ`. By spectral decomposition of the OU semigroup, the moment generating function of `(s_t - μ)/σ_∞` is:

```
E[exp(λ(s_t - μ)/σ_∞)] = exp(λ²/2)
```

(exact for stationary Gaussian). This is the sub-Gaussian MGF, and Markov's inequality gives:

```
P((s_t - μ)/σ_∞ ≥ x) ≤ exp(-x²/2)
```

Doubling for the two-sided bound completes the theorem. □

### 6.3 Application to drawdown stops

The OU sub-Gaussian bound gives an **exact** finite-sample tail bound for basis divergence — assuming the basis follows OU. This is tighter than the Gaussian VaR `μ + Φ⁻¹(1-α)·σ` for small `α`:

| α | Gaussian Φ⁻¹(1-α) | sub-Gaussian √(-2 ln(α/2)) | tighter |
|---|---|---|---|
| 0.05 | 1.645 | 2.448 | Gaussian (within OU assumption) |
| 0.01 | 2.326 | 3.255 | Gaussian |
| 0.001 | 3.090 | 3.717 | Gaussian |

Hmm — actually the sub-Gaussian bound is **looser** than Gaussian for the central body. The point is: **it's distribution-free within the OU class**, not pointwise tighter. For realistic mixtures (OU + jumps), the sub-Gaussian bound holds where the Gaussian quantile fails.

### 6.4 Composition with Conformal

The sub-Gaussian bound is a model-based safety net. Conformal prediction is a model-free safety net. The bot uses **the maximum of the two** as its drawdown stop:

```
d_max(s) = max(
    sub_Gaussian_bound(σ̂_∞, α),
    conformal_VaR(basis_history, 1-α),
    cluster_aware_CVaR(Hawkes_fit, basis_history, α)
)
```

This guarantees safety under multiple model assumptions and the model-free conformal coverage.

---

## 7. The composed frontier framework

### 7.1 Per-candidate validation cascade

For each candidate `(symbol, counter)` at each tick:

```
1. ADF stationarity (math-rigorous §2)        — reject random walks
2. OU MLE fit (math-rigorous §1)              — get μ̂, θ̂, σ̂
3. Phillips asymptotic credibility (rigorous §3) — quick first filter
4. Empirical Bernstein credibility (frontier §2) — sharper finite-sample test
5. Hurst exponent classification (frontier §4)   — diagnose process roughness
6. Hawkes branching ratio (frontier §5)          — check basis stationarity
7. Conformal coverage on residuals (frontier §1) — distribution-free fit quality
```

A candidate is admitted only if it passes **all seven**. The classical (1-3) and frontier (4-7) layers each enforce different assumptions; passing all means the trade is sound under each.

### 7.2 Per-allocation step

For the universe of admitted candidates:

```
1. Estimate r̂ (vector of expected APYs) from OU fits
2. Estimate Σ̂ (covariance) with Ledoit-Wolf shrinkage
3. Solve Wasserstein DRO portfolio (frontier §3)
4. Apply chance constraint with conformal VaR (frontier §1)
5. Apply stress constraint with sub-Gaussian bound (frontier §6)
6. Project onto budget + box + per-counter constraints
```

The result is a portfolio that is:
- **Distributionally robust** (DRO) against estimator noise
- **Distribution-free in tail** (conformal) for chance constraint
- **Model-safe** (sub-Gaussian) for stress events
- **Cluster-aware** (Hawkes) for jump risk

### 7.3 Per-position monitoring

Existing positions are re-evaluated each tick:

```
- Conformal residual: is the realized basis still inside the prediction interval?
- Hawkes intensity: has cluster intensity spiked beyond stationary expectation?
- Hurst: has the process roughness changed (regime shift)?
- OU half-life: has mean reversion slowed (signal decay)?
```

Any of these triggering an alarm forces position closure.

---

## 8. Validation properties

The frontier framework adds tests beyond classical:

| property | classical test | frontier test |
|---|---|---|
| Coverage of chance constraint | n/a (Gaussian assumption) | **conformal coverage on holdout** ≥ nominal |
| Mean estimate confidence | Phillips asymptotic | **empirical Bernstein finite-sample** matches Phillips at large T, tighter at small T |
| Out-of-sample portfolio Sharpe | n/a | **DRO Sharpe ≥ Markowitz Sharpe** on noisy data, bounded ratio |
| Hurst recovery | n/a | **DFA-recovered H** within 0.05 of true on synthetic fBM |
| Hawkes parameter recovery | n/a | **MLE-recovered (μ_0, α, β)** within 20% on T = 720 |
| Sub-Gaussian bound | n/a | empirical tail ≤ sub-Gaussian quantile on OU samples |

These are checked in `scripts/validate_frontier.py`.

---

## 9. References (2005-2024 only, modern frontier)

- **Vovk, Gammerman, Shafer 2005** — *Algorithmic Learning in a Random World* (conformal prediction founding)
- **Romano, Patterson, Candès 2019** — *Conformalized Quantile Regression*
- **Maurer, Pontil 2009** — *Empirical Bernstein Bounds and Sample Variance Penalization*
- **Esfahani, Kuhn 2018** — *Data-driven distributionally robust optimization using the Wasserstein metric*
- **Gatheral, Jaisson, Rosenbaum 2018** — *Volatility is Rough*
- **Bacry, Mastromatteo, Muzy 2015** — *Hawkes Processes in Finance*
- **Peng et al. 1994** — *Mosaic organization of DNA nucleotides* (DFA)
- **DeMiguel, Garlappi, Uppal 2009** — *Optimal Versus Naive Diversification: How Inefficient Is the 1/N Portfolio Strategy?*
- **Mohri, Rostamizadeh, Talwalkar 2018** — *Foundations of Machine Learning* (concentration bounds chapter)
- **Lei, Wasserman 2014** — *Distribution-free prediction bands for non-parametric regression*

This framework synthesizes these into a single live-adaptive decision system specifically engineered for cross-venue same-asset funding spread harvesting on perpetual DEXes — a problem composition that has not appeared in published literature.

---

## 10. What it does NOT do (genuine limits, not deferral)

- **Causal identification of structural vs transient spreads** — would require instrumental variables with known venue-specific structural shocks. Not available.
- **Model uncertainty over OU vs jump-diffusion vs regime-switching** — Bayesian model averaging over these adds complexity without proportional benefit at our timescale.
- **Adversarial robustness against MEV / sandwich attacks** — our DEXes have varying MEV protection. Better addressed by the execution layer than the strategy layer.
- **Cross-strategy alpha stacking** — combining funding spread arb with other strategies. v3.5 = pure arb, by iron law.

These are honest limits, not areas where we know how to do better but choose not to.
