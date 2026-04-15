# Dol Strategy — Rigorous Stochastic & Portfolio Framework

**Status:** Extends `math-formulas.md` (the closed-form first-order framework) with second-order stochastic models, Bayesian credibility tests, optimal stopping theory, and constrained portfolio optimization. Every claim here is mathematically auditable and survives standard quant-finance rigor checks.
**Iron law:** `../PRINCIPLES.md`. This document is the rigorous statistical implementation.

---

## 0. Why the first-order framework is insufficient

`math-formulas.md` builds every safety gate as a closed-form function of live point estimates: rolling mean μ̂, rolling stddev σ̂, sign-counting persistence p̂, fixed Z multipliers, equal-weighted portfolio. This is correct as a first approximation but it has seven specific weaknesses that a rigorous framework must close:

| weakness | first-order | rigorous |
|---|---|---|
| funding spread dynamics | rolling mean, no model | Ornstein-Uhlenbeck stochastic process with MLE fit |
| persistence test | sign-counting against random-walk null | Bayesian P(μ > 0 \| data) via OU posterior |
| stationarity assumption | implicit | Augmented Dickey-Fuller test on each candidate |
| min hold derivation | ratio gate at point estimate | optimal stopping via OU expected residual income |
| portfolio allocation | equal weight, ignores correlation | mean-variance Markowitz with cross-pair covariance |
| drawdown stop | 3σ Gaussian assumption | empirical CVaR with EVT tail correction |
| mandate compliance | deterministic point check | chance constraint: P(R_vault ≥ floor) ≥ 1−ε |

Each closes with rigorous statistical machinery below. Notation throughout: `s_t` = funding spread at time t, `Δt` = 1 hour (the funding tick interval), `T` = lookback length in hours.

---

## 1. Funding spread as Ornstein-Uhlenbeck process

### 1.1 The model

For each candidate (symbol, counter venue), let the per-hour funding spread be:

```
s_t = f_counter(t) - f_pacifica(t)
```

A mean-reverting funding spread is well-modeled as an Ornstein-Uhlenbeck (OU) process:

```
ds_t = θ (μ - s_t) dt + σ dW_t
```

where:
- `μ` = long-run mean of the spread (in per-hour units)
- `θ > 0` = mean-reversion rate (1/hours); larger θ = faster reversion
- `σ` = volatility of innovations (per-hour scale)
- `W_t` = standard Brownian motion

The OU model captures three stylized facts of perp funding spreads:
1. spreads cluster around a long-run mean (mean reversion)
2. they have continuous drift between funding ticks (Brownian diffusion)
3. they are persistent on multi-hour timescales but bounded on multi-day timescales

### 1.2 Closed-form discrete dynamics

For discrete observations with interval Δt = 1 hour, the OU process has an exact discretization:

```
s_(t+Δt) = a + b · s_t + ε_t
```

where:
- `b = e^(-θΔt)` = autoregressive coefficient
- `a = μ(1 - b)` = intercept
- `ε_t ~ N(0, σ_ε²)` with `σ_ε² = σ²(1 - b²)/(2θ)`

This is an AR(1) process. We can fit it by ordinary least squares regression.

### 1.3 MLE estimators (closed form)

Given T hours of observations `s_1, s_2, …, s_T`, run the OLS regression `s_(i+1) = a + b·s_i + ε_i`:

```
b̂ = (Σ (s_i − s̄_x)(s_(i+1) − s̄_y)) / (Σ (s_i − s̄_x)²)
â = s̄_y − b̂ s̄_x
σ̂_ε² = (1/(T−2)) Σ (s_(i+1) − â − b̂ s_i)²
```

Then recover the OU parameters:

```
θ̂ = -ln(b̂) / Δt
μ̂ = â / (1 − b̂)
σ̂² = σ̂_ε² · 2θ̂ / (1 − b̂²)
```

with standard errors:

```
SE(b̂) = σ̂_ε / √(Σ (s_i − s̄_x)²)
SE(μ̂) ≈ σ̂_ε / √((1−b̂)² · T · Var(s))
SE(θ̂) ≈ SE(b̂) / b̂
```

These give us not just point estimates but full uncertainty quantification.

### 1.4 Half-life

The natural time scale of the process — the time for a deviation from the mean to decay to half its initial value — is:

```
t_½ = ln(2) / θ̂
```

For `θ̂ = 0.05/hour`, `t_½ ≈ 14 hours`. For `θ̂ = 0.005/hour`, `t_½ ≈ 139 hours ≈ 5.8 days`. This is the BIOLOGICAL hold time of the position — try to exit before t_½ and you've barely captured the signal; hold for >> 3·t_½ and you're entering territory where the process forgets its current state.

---

## 2. Augmented Dickey-Fuller stationarity test

### 2.1 Why this matters

OU model assumes the spread is stationary (mean-reverting). If a spread is actually a random walk (`s_(t+1) = s_t + ε`), then there is NO mean to revert to and any "signal" is artifact. We must test for stationarity before trusting an OU fit.

The Augmented Dickey-Fuller (ADF) test has null hypothesis `H_0: unit root` (random walk) vs alternative `H_1: stationary` (OU-like). Reject H_0 → the process is mean-reverting → OU model valid.

### 2.2 ADF test statistic

The ADF regression on first differences:

```
Δs_t = α + β s_(t-1) + Σ_{k=1}^{p} γ_k Δs_(t-k) + ε_t
```

with augmentation lags `p = ⌊(T-1)^(1/3)⌋` (Schwert's rule).

Test statistic:

```
ADF = β̂ / SE(β̂)
```

Under H_0 (unit root), ADF has a non-standard distribution with critical values from MacKinnon (1996):

```
critical values (no constant):
  1% level: -2.567
  5% level: -1.941
  10% level: -1.616
```

Reject H_0 (and accept OU-like) when `ADF < critical_value`. Use the 5% critical value -1.941 as default.

### 2.3 Implementation

For T = 720 observations and p = ⌊(719)^(1/3)⌋ = 8 lags, the ADF regression is straightforward. Python: implement directly with numpy; no need for statsmodels.

A candidate enters the universe only if its spread series passes the ADF test at the 5% level. Random-walk spreads are silently dropped — preventing the v3.3 trap where STRK's 91.6% sign-dominance was actually statistical noise around a near-zero mean.

---

## 3. Bayesian credibility of mean-reversion

### 3.1 Replacing sign-counting persistence

The first-order framework used `p̂ = fraction of past hours where sign(s) = sign(mean)` and required `p̂ ≥ 0.70`. This conflates sign of point estimate with confidence in the directional claim. The rigorous test is:

```
P(μ > 0 | observations) ≥ 1 − ε_credibility
```

or symmetrically `P(μ < 0 | observations) ≥ 1 − ε`, depending on direction.

### 3.2 The posterior

With OU fit, the MLE μ̂ is asymptotically normal: `μ̂ ~ N(μ, SE(μ̂)²)`. Under a flat prior on μ, the posterior is also normal centered at μ̂ with the same standard error. The credibility statement becomes:

```
P(μ > 0 | data) = Φ(μ̂ / SE(μ̂))
```

where Φ is the standard normal CDF.

For 5σ credibility (ε ≈ 3·10⁻⁷):

```
μ̂ / SE(μ̂) ≥ 5
```

This is the t-statistic of the OU mean estimate. It replaces the ad-hoc sign-counting threshold with a proper hypothesis test.

### 3.3 Comparison with v3.3

| candidate | v3.3 says | rigorous says |
|---|---|---|
| stable strong signal: μ̂ = 30%, SE = 3% | p̂ ≈ 0.97, pass 0.70 floor | t = 10, pass 5σ, **enter** |
| weak but consistent: μ̂ = 5%, SE = 1.5% | p̂ ≈ 0.85, pass 0.70 floor | t = 3.3, **fail 5σ, reject** |
| noisy bid signal: μ̂ = 30%, SE = 12% | p̂ ≈ 0.85 (because mean is well above zero), pass | t = 2.5, **fail 5σ, reject** |

The rigorous test correctly rejects the second and third candidates that v3.3 would have accepted.

---

## 4. Optimal hold time via OU expected residual income

### 4.1 The stopping problem

A position has been entered with current spread `s_t` (signed in our favor). The expected income over future hold time τ is:

```
J(s_t, τ) = E[ ∫_t^{t+τ} sign(direction) · s_u du - c | s_t ]
```

For the OU process:

```
E[s_u | s_t] = μ + (s_t - μ) e^(-θ(u-t))
```

Integrating:

```
J(s_t, τ) = sign(d) · [μ τ + (s_t - μ) (1 - e^(-θτ))/θ] - c
```

### 4.2 The optimal τ*

Differentiate with respect to τ and set to zero:

```
dJ/dτ = sign(d) · [μ + (s_t - μ) e^(-θτ)] = 0
```

The marginal income at time τ is `s_(t+τ)`. The optimal exit is when expected marginal income first crosses zero (or some threshold):

```
sign(d) · E[s_(t+τ*)] = 0
```

For mean-reverting OU with `μ` of the favorable sign, the marginal income is always positive in expectation — we should hold "forever" (or until a separate exit condition fires: drawdown, persistence break, signal decay, withdrawal pressure).

For mean-reverting OU with μ of UNFAVORABLE sign (we entered on a deviation), the marginal income decays toward μ, and we should exit when it reaches some risk-adjusted threshold. The optimal closed-form τ*:

```
τ* = (1/θ) · ln((s_t - μ)/threshold_residual_per_hour - μ/threshold + 1)
```

simplifies, in the standard "exit when expected marginal income < 0" rule, to "exit when sign(d) · E[s_u] reaches zero" — which under OU with unfavorable μ happens at `u = t + (1/θ) · ln((s_t - μ)/(0 - μ))`.

### 4.3 Practical rule

The bot uses a simpler decision: at each tick, compute the expected residual income over the next half-life:

```
J_(t,t_½) = sign(d) · [μ · t_½ + (s_t - μ) (1 - e^(-1·ln 2))/θ]
          = sign(d) · [μ · t_½ + 0.5 · (s_t - μ)/θ]
```

If `J_(t,t_½) > c_marginal_holding`, hold. Otherwise exit.

The minimum hold (commitment) is `t_½ / 2` (half a half-life — aggressive but bounded). This replaces the v3.3 fixed `H_min = 168h` with a half-life-driven minimum that adapts per pair to the actual mean-reversion speed.

---

## 5. Cross-pair covariance and Markowitz allocation

### 5.1 Why equal-weight is wrong

Equal-weight allocation across N candidates ignores:
1. variance differences between pairs (a noisy pair adds variance per dollar)
2. correlation between pairs (basis blowup on venue X affects all pairs hedging on X)
3. relative expected return differences

The mean-variance optimal allocation extracts more Sharpe per unit of capital.

### 5.2 The covariance matrix

For N candidate pairs, define:

```
Σ_ij = Cov(R_i, R_j)
```

where `R_i` is the per-hour signed return of pair i. Estimate from rolling history:

```
R_i,t = sign(d_i) · (counter_funding_i,t - pacifica_funding_i,t)
Σ̂_ij = (1/(T-1)) · Σ_t (R_i,t - R̄_i)(R_j,t - R̄_j)
```

Off-diagonal covariance arises mainly through shared counter venues. If pairs i and j both use Backpack as counter, their basis P&L is correlated through Backpack's local oracle.

### 5.3 The Markowitz problem

Maximize portfolio Sharpe subject to the iron-law constraints:

```
max  (w'r - r_idle) / √(w'Σw)
s.t. Σ w_i = (1 - α)              [budget = trading bucket fraction of AUM]
     0 ≤ w_i ≤ m_pos              [per-symbol concentration cap]
     Σ_{i: counter(i)=v} w_i ≤ m_counter   [per-counter venue cap]
```

This is a quadratic program. For modest N (≤30), it solves in milliseconds.

Closed-form solution (without inequality constraints) is:

```
w* ∝ Σ^(-1) (r - r_idle · 1)
```

normalized so `Σ w_i* = (1 - α)`. With inequality constraints active, we project onto the feasible polytope using a sequential quadratic programming step.

### 5.4 Equivalence with Kelly criterion

For log-utility (long-term geometric growth maximization), the optimal weights are exactly:

```
w_Kelly = (1/γ) · Σ^(-1) (r - r_idle · 1)
```

where `γ` is the relative risk aversion coefficient. Setting `γ = 1` recovers the unconstrained Markowitz tangency portfolio. Setting `γ > 1` gives "fractional Kelly" — half-Kelly (γ=2) is standard for institutional risk control because full Kelly has too much variance.

The rigorous framework uses **fractional Kelly with γ = 2** by default.

---

## 6. CVaR-based drawdown stop with EVT tail

### 6.1 Why Gaussian fails

Basis divergence is heavy-tailed. Empirical kurtosis is typically 5-15 (vs Gaussian 3). A 3σ Gaussian threshold underestimates the tail by 2-4×.

### 6.2 Empirical CVaR

For a position size `n`, the per-hour basis P&L is `n · δ_t` where δ is the basis divergence. Define:

```
VaR_q(δ) = q-th quantile of empirical basis distribution (q=0.05 typical)
CVaR_q(δ) = E[δ | δ ≤ VaR_q]
```

Sample CVaR from rolling history:

```
CVaR_q = (1/k) Σ {smallest k = ⌊q·T⌋ basis values}
```

The drawdown stop becomes:

```
d_max(s) = -CVaR_0.01(δ_s) · safety_multiplier
```

with safety_multiplier = 2 (covers tail beyond the 1% quantile).

### 6.3 EVT tail correction

When the empirical sample doesn't have enough tail observations (T < 1000), fit a Generalized Pareto Distribution (GPD) to the lowest 10% of observations:

```
F(x; ξ, β) = 1 - (1 + ξ x/β)^(-1/ξ)
```

Then compute VaR and CVaR analytically from the GPD parameters. This extrapolates beyond the empirical sample for safety.

For most pairs at our timescales, empirical CVaR is sufficient. EVT is a backup for thin-history candidates.

---

## 7. Chance-constrained mandate compliance

### 7.1 The rigorous mandate

Replace the deterministic "vault APY ≥ floor" with:

```
P(R_vault > floor | currently chosen allocation) ≥ 1 - ε_mandate
```

For `ε_mandate = 0.05`, we require 95% confidence the vault clears the floor.

### 7.2 The portfolio return distribution

Under our model, vault return is:

```
R_vault = α · r_idle + (1-α) · w' R_pair_basket
```

Mean: `α r_idle + (1-α) w' r̂`
Variance: `(1-α)² w' Σ̂ w`

Assuming approximate normality (Central Limit Theorem on basket return), the 5th percentile of the vault return is:

```
R_vault_5% = α r_idle + (1-α) [w' r̂ - 1.645 · √(w' Σ̂ w)]
```

The mandate floor compliance becomes:

```
α r_idle + (1-α) [w' r̂ - 1.645 √(w' Σ̂ w)] ≥ floor
```

This is a SECOND-ORDER cone constraint — solvable jointly with the Markowitz problem in §5.3.

### 7.3 Implication for α

The rigorous α is no longer a closed-form quotient but the smallest α (≥ α_floor) such that the chance-constrained portfolio problem is feasible. If signals are good (high mean, low variance), α can be high. If signals are noisy, α drops to soak the floor.

### 7.4 Stress floor

As an additional safeguard, also require:

```
R_vault_1% ≥ floor - 0.02   (100bps tolerance below floor at 1% confidence)
```

This ensures even the 1-in-100 day clears 6% gross (vs 8% target). The bot will not enter an allocation that violates this.

---

## 8. Composition: the rigorous decision rule

At each funding tick, the bot runs the following pipeline. **Every step uses live data; no constants other than mandate and Z thresholds.**

```
RIGOROUS_DECISION(inputs, mandate):
    1. for each (s, v_c) pair from inputs.funding_history:
         2. ADF_stat, ADF_pvalue = adf_test(spread_series(s, v_c))
         3. if ADF_pvalue > 0.05: skip [random walk, no signal]
         4. (μ̂, θ̂, σ̂, SE_μ) = ou_fit(spread_series(s, v_c))
         5. half_life = ln(2) / θ̂
         6. if half_life < 4h or half_life > 1000h: skip [implausible dynamics]
         7. t_stat = μ̂ / SE_μ
         8. if |t_stat| < 5: skip [insufficient credibility]
         9. CVaR_basis = empirical_cvar(basis_history(s), q=0.01)
        10. d_max(s) = -2 · CVaR_basis
        11. record candidate (s, v_c, μ̂, σ̂, half_life, t_stat, d_max)
    
   12. r̂ = vector of expected per-pair APY (μ̂ × 8760)
   13. Σ̂ = covariance matrix from rolling spread returns
   14. solve Markowitz QP:
         max  (w'r̂ - r_idle) / √(w'Σ̂w)
         s.t. Σ w_i = (1-α)
              w_i ∈ [0, m_pos_dynamic]
              Σ_{counter=v} w_i ≤ m_counter_dynamic
              chance constraint: α r_idle + (1-α)[w'r̂ - 1.645 √(w'Σ̂w)] ≥ vault_floor
              stress constraint: same with 2.326 (1% z-score) ≥ vault_floor - 0.02
   15. if QP infeasible: stay all-idle, alert operator (degenerate market)
   16. let w*, α* = optimal solution
   17. for each pair i with w_i* > 0:
         emit signal: enter pair i with notional = w_i* · A · L / 2
   18. for each existing position p:
         compute expected residual income J_(t,t_½)
         compute current basis drawdown vs d_max(symbol(p))
         re-evaluate ADF and t-stat
         if any of: J < 0, drawdown > d_max, t-stat < 4 → forced close
```

This is the rigorous framework. Every step is mathematically justified, every parameter is data-driven, and every decision has uncertainty quantification baked in.

---

## 9. Validation properties

A correct implementation of the rigorous framework must satisfy:

| property | check |
|---|---|
| ADF rejection rate on truly stationary OU | empirical type-I error close to 5% |
| ADF acceptance rate on random walk | should be close to 5% (i.e. correctly rejecting random walks 95% of the time) |
| OU fit recovery | when given a known-θ OU sample, MLE should recover θ within 10% on T=720 |
| t-statistic distribution | under H_0 (μ=0), t-stat should be ~ N(0,1); empirical 99th percentile should be ≈ 2.33 |
| Markowitz monotonicity | adding a positive-Sharpe pair to the universe should not decrease portfolio Sharpe |
| chance constraint binding | when solver returns optimal w, the chance constraint should be active or strict |
| mandate compliance under monte carlo | simulating 10000 random vault paths from the chosen allocation, ≥ 95% should clear the floor |

These tests are implemented in `scripts/validate_rigorous.py`.

---

## 10. References (the bar this framework is held to)

- **Vasicek 1977** — original OU process for mean-reverting interest rates
- **Dickey & Fuller 1979** — original DF test
- **MacKinnon 1996** — modern critical values for ADF
- **Markowitz 1952** — mean-variance portfolio selection
- **Kelly 1956** — log-utility growth optimization
- **Rockafellar & Uryasev 2000** — conditional value-at-risk for portfolio optimization
- **Charnes & Cooper 1959** — chance constrained programming
- **Almgren & Chriss 2001** — square-root market impact (used in slippage estimator)
- **Embrechts, Klüppelberg, Mikosch 1997** — extreme value theory for fat tails

Every formula in §1-§8 appears in this canonical literature. The Dol framework is not inventing new math — it is composing standard rigorous machinery into a live-adaptive decision system.

---

## 11. What is NOT here (and why)

These are intentionally deferred:

- **Stochastic volatility (Heston-class)**: the basis volatility σ is itself a random process, but the drawdown CVaR captures the relevant tail without requiring full stochastic-vol modeling. Add only if Phase 1 dry run shows σ regime-shifts disrupt the strategy.
- **Hidden Markov regime switching**: useful for adapting to "calm vs stressed" regimes. Phase 2 enhancement.
- **Continuous-time HJB Bellman equation**: full DP solution. Discrete approximation suffices for hourly funding ticks.
- **Game-theoretic competition modeling**: the bot doesn't model other arb desks compressing the spread. Phase 2 if needed.
- **Jump-diffusion**: basis can have jumps (venue freezes). Drawdown stop catches them; explicit jump model adds complexity for marginal gain.
- **Higher-frequency execution alpha**: this strategy is hourly. Sub-minute execution alpha is out of scope.

Each of these is mathematically interesting but adds modeling risk without proportional Sharpe improvement in our specific setting.
