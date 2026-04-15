# Aurora-Ω Appendix F — α-Cascade Scoring Rule Strict Propriety

**Status:** Formal proof + empirical validation spec for Aurora-Ω Proposition 7 (referenced in `aurora-omega-spec.md` §20.3).
**Audience:** Anyone implementing or auditing `strategy/forecast_scoring.py` or its Rust port.
**References:**
- Gneiting, T. (2011). *Making and Evaluating Point Forecasts.* JASA 106(494), 746–762.
- Gneiting, T. & Raftery, A. (2007). *Strictly Proper Scoring Rules, Prediction, and Estimation.* JASA 102(477), 359–378.
- Savage, L. J. (1971). *Elicitation of Personal Probabilities and Expectations.* JASA 66(336), 783–801.

---

## F.0 Why this appendix exists

Aurora-Ω §20 introduces a forecast scoring rule

$$S(\hat X, X) = -\sum_{\ell=0}^{L_{\max}} w_\ell \sum_k |\hat x_k - x_k|^{\alpha_0 + \ell\eta}$$

that feeds a *tail-estimate-deterioration red flag* into the FSM. For this flag to be principled rather than ad hoc, the scoring rule must satisfy **strict propriety**: the expected score (under the true data-generating distribution $P$) must be *uniquely* maximized at the true $P$-optimal point forecast. Otherwise the flag can be gamed by a predictor that emits a biased-but-score-maximizing constant.

The theorem below states the exact conditions under which $S$ is strictly proper, and the proof reduces the claim to two elementary lemmas plus a Gneiting 2011 consistency fact.

---

## F.1 Setup and notation

Let $X$ be a real-valued random variable with distribution $P$. Let $\hat x \in \mathbb R$ denote a point forecast, and consider the **α-power loss**

$$\mathcal L_\alpha(\hat x, x) := |\hat x - x|^\alpha, \qquad \alpha > 0.$$

A scoring rule $s(\hat x, x)$ is **negatively oriented** here (higher is better; we take negatives so the runtime maximizes). A scoring rule is **strictly consistent** for a functional $T: \mathcal P \to \mathbb R$ on a class $\mathcal P$ of distributions if, for every $P \in \mathcal P$,

$$\mathbb E_{X \sim P}[s(T(P), X)] \ge \mathbb E_{X \sim P}[s(\hat x, X)] \quad \forall \hat x$$

with equality iff $\hat x = T(P)$. When this holds, $T(P)$ is the unique maximizer in expectation.

Define the cascade grid

$$\alpha_\ell := \alpha_0 + \ell \eta, \qquad \ell = 0, 1, \ldots, L_{\max}$$

with $\alpha_0 > 0$ and $\eta > 0$, so $\alpha_0 < \alpha_1 < \cdots < \alpha_{L_{\max}}$.

Let $\mathcal P^\alpha$ denote the class of distributions with finite $\alpha$-th absolute moment: $\mathbb E_{X \sim P}[|X|^\alpha] < \infty$. Assume

$$P \in \bigcap_{\ell=0}^{L_{\max}} \mathcal P^{\alpha_\ell} \quad \text{(assumption M)}$$

so all cascade moments exist.

For a vector-valued point forecast $\hat X = (\hat x_1, \ldots, \hat x_n)$ of an n-dimensional observation vector $X = (x_1, \ldots, x_n)$ with joint distribution $P$, define the **α-cascade scoring rule**

$$S(\hat X, X) := -\sum_{\ell=0}^{L_{\max}} w_\ell \sum_{k=1}^{n} |\hat x_k - x_k|^{\alpha_\ell}$$

with weights $w_\ell \ge 0$, $\sum_\ell w_\ell = 1$.

**Note on the functional targeted.** For a scalar $X$, the minimizer of $\hat x \mapsto \mathbb E[|\hat x - X|^\alpha]$ is
- the median of $P$ when $\alpha = 1$,
- the mean of $P$ when $\alpha = 2$,
- for general $\alpha > 1$ it is a *unique* value $T_\alpha(P)$ (strict convexity of $|\cdot|^\alpha$ in the forecast argument).
- for $\alpha < 1$ it can be set-valued (non-convex); we exclude this case by assuming $\alpha_0 \ge 1$ in the practical cascade. With $\alpha_0 = 1$ and $\eta > 0$ all $\ell \ge 1$ terms are strictly convex.

This matters because the cascade optimizer is the **unique joint argmin** over all $\alpha_\ell$ terms only when each individual term has a unique minimum. Assumption $\alpha_0 \ge 1$ with $w_0 < 1$ (so at least one $\alpha_\ell > 1$ term is active) guarantees overall strict convexity.

---

## F.2 Lemma 1 — Single-α strict consistency (Gneiting 2011)

**Lemma 1.** *For any $\alpha > 1$ and any $P \in \mathcal P^\alpha$, the scoring rule $s_\alpha(\hat x, x) := -|\hat x - x|^\alpha$ is strictly consistent for the functional*

$$T_\alpha(P) := \arg\min_{c \in \mathbb R} \mathbb E_{X \sim P}[|c - X|^\alpha]$$

*and this functional is single-valued.*

**Proof.** The map $c \mapsto g(c) := \mathbb E[|c - X|^\alpha]$ is finite under $P \in \mathcal P^\alpha$ (Jensen: $|c - X|^\alpha \le 2^{\alpha - 1}(|c|^\alpha + |X|^\alpha)$). It is strictly convex because $c \mapsto |c - x|^\alpha$ is strictly convex for $\alpha > 1$ (standard calculus: second derivative $\alpha(\alpha - 1)|c - x|^{\alpha - 2} > 0$ for $c \ne x$, and strict convexity in the distributional sense on any interval not concentrated at a single $x$), and the expectation of strictly convex functions is strictly convex as long as $P$ is not a point mass (in which case the functional is trivially single-valued at the atom). Strictly convex finite functions on $\mathbb R$ attain their infimum at a **unique** point; hence $T_\alpha(P)$ is single-valued. Strict consistency follows by definition: $g(T_\alpha(P)) < g(\hat x)$ for all $\hat x \ne T_\alpha(P)$, so $\mathbb E[s_\alpha(T_\alpha(P), X)] > \mathbb E[s_\alpha(\hat x, X)]$. $\qed$

**Remark.** This is a special case of Theorem 8 in Gneiting (2011), which shows that the class of consistent scoring functions for a strictly convex Bregman-divergence-generated functional is characterized by the underlying convex generator. For $\alpha$-power loss, the generator is $\phi(c) = |c|^\alpha / (\alpha(\alpha-1))$ for $\alpha > 1$.

---

## F.3 Lemma 2 — Positive-weighted sum preservation

**Lemma 2.** *Let $s_1, s_2, \ldots, s_m$ be scoring rules, each strictly consistent for the same functional $T$ on the same class $\mathcal P$. Let $w_1, \ldots, w_m \ge 0$ with at least one $w_i > 0$. Then*

$$s(\hat x, x) := \sum_{i=1}^m w_i \, s_i(\hat x, x)$$

*is strictly consistent for $T$ on $\mathcal P$.*

**Proof.** For any $P \in \mathcal P$ and any $\hat x$,

$$\mathbb E_P[s(\hat x, X)] = \sum_i w_i \mathbb E_P[s_i(\hat x, X)] \le \sum_i w_i \mathbb E_P[s_i(T(P), X)] = \mathbb E_P[s(T(P), X)]$$

with equality iff each term with $w_i > 0$ achieves equality, which by strict consistency of $s_i$ requires $\hat x = T(P)$. $\qed$

**Caveat for the cascade case.** Different $\alpha_\ell$ in the cascade generally target *different* functionals $T_{\alpha_\ell}(P)$, so Lemma 2 in its plain form does not directly apply across different $\alpha$'s — it only combines rules sharing a single functional target. The cascade rule is therefore strictly proper for a **joint vector functional**, not a single scalar one. We formalize this below.

---

## F.4 Theorem (Aurora-Ω Proposition 7)

**Theorem.** *Let Assumption M hold ($P$ has finite moments of order $\alpha_\ell$ for all $\ell \in \{0, \ldots, L_{\max}\}$). Assume $\alpha_0 \ge 1$ and at least one $\ell$ has $\alpha_\ell > 1$ with $w_\ell > 0$ (equivalently, $w_0 < 1$ when $\alpha_0 = 1$). Assume $w_\ell \ge 0$ for all $\ell$ with $\sum_\ell w_\ell = 1$. Consider the scoring rule*

$$S(\hat X, X) = -\sum_{\ell = 0}^{L_{\max}} w_\ell \sum_{k=1}^n |\hat x_k - x_k|^{\alpha_\ell}.$$

*Then for each coordinate $k$, $S$ is strictly consistent for the functional*

$$T_k(P) := \arg\min_{c \in \mathbb R} \sum_{\ell : w_\ell > 0} w_\ell \, \mathbb E_{X_k \sim P_k}[|c - X_k|^{\alpha_\ell}]$$

*where $P_k$ is the marginal of coordinate $k$. The functional $T_k(P)$ is single-valued under the assumptions, and $S$ is jointly strictly maximized in expectation at $\hat X^* = (T_1(P), \ldots, T_n(P))$.*

**Proof.**

*Step 1: coordinate separability.* The cascade loss is additive across coordinates,

$$\mathbb E_P[-S(\hat X, X)] = \sum_{k=1}^n \underbrace{\sum_{\ell} w_\ell \, \mathbb E_{P_k}[|\hat x_k - X_k|^{\alpha_\ell}]}_{=: h_k(\hat x_k)},$$

so minimizing the expected negative score in $\hat X$ decomposes into $n$ independent scalar minimizations of $h_k$.

*Step 2: per-coordinate strict convexity.* Each summand $c \mapsto \mathbb E_{P_k}[|c - X_k|^{\alpha_\ell}]$ is:
- convex for $\alpha_\ell = 1$,
- **strictly** convex for $\alpha_\ell > 1$ (by the argument in Lemma 1, provided $P_k$ is not a point mass).

Since $w_\ell \ge 0$ and at least one $\ell$ with $\alpha_\ell > 1$ has $w_\ell > 0$, the weighted sum $h_k$ has at least one strictly convex summand. A sum of convex functions with at least one strictly convex term is itself strictly convex (the strict inequality from the strict summand propagates). Hence $h_k$ is strictly convex.

If $P_k$ is a point mass at some $x_k^*$, then $h_k$ is trivially minimized uniquely at $c = x_k^*$ (all terms equal zero there).

*Step 3: uniqueness of the joint minimizer.* A strictly convex finite function on $\mathbb R$ attains its minimum at a unique point. So each $h_k$ has a unique minimizer $T_k(P)$, and the joint vector minimizer is $\hat X^* = (T_1(P), \ldots, T_n(P))$ — unique.

*Step 4: strict propriety.* For any $\hat X \ne \hat X^*$, at least one coordinate $k$ has $\hat x_k \ne T_k(P)$, and for that coordinate $h_k(\hat x_k) > h_k(T_k(P))$ (strict inequality by strict convexity). Summing across $k$,

$$\mathbb E_P[-S(\hat X, X)] > \mathbb E_P[-S(\hat X^*, X)],$$

i.e., $\mathbb E_P[S(\hat X^*, X)] > \mathbb E_P[S(\hat X, X)]$. $\qed$

**Functional interpretation.** $T_k(P)$ is a convex combination of the $\alpha_\ell$-functionals of $P_k$ — *not* the mean, not the median, but a weighted M-estimator that balances median-like sensitivity ($\alpha_\ell = 1$ terms) with mean-like sensitivity ($\alpha_\ell = 2$ terms) and higher-moment sensitivity ($\alpha_\ell > 2$). Intuitively, as higher-$\alpha$ weights grow, $T_k$ shifts from median-robust toward max-error-hard.

---

## F.5 Corollaries — cascade limits

### F.5.1 L1 limit ($\ell = 0$, $\alpha_0 = 1$)

At $\ell = 0$ with $\alpha_0 = 1$, the score reduces to the L1 loss $-|\hat x_k - x_k|$. This is consistent for the **median** of $P_k$ (Laplace 1774; standard). The cascade's lowest tier acts as a median tracker.

### F.5.2 Quadratic tier ($\alpha_\ell = 2$)

When $\alpha_\ell = 2$ (i.e., $\ell = 2$ with $\eta = 0.5$, or $\ell = 1$ with $\eta = 1$), the term $-|\hat x - x|^2$ is the negative squared error, strictly consistent for the **mean** of $P_k$.

### F.5.3 Hard-threshold limit ($\ell \to L_{\max}$, $\alpha_{L_{\max}} \gg 1$)

As $\alpha \to \infty$, $|\hat x - x|^\alpha$ on $[0, 1]$-normalized residuals concentrates all mass on the maximum, so the scoring rule approaches $-\max_k |\hat x_k - x_k|$ (after rescaling). This is the **hard-threshold / $L^\infty$ detector**: it fires when *any* single residual breaches a threshold. The cascade interpolates continuously from median-soft ($\alpha_0 = 1$) to max-hard ($\alpha \to \infty$), which is the unification of the "L-1 / L-2 / L-3 limit modes" and the "hard-threshold limit" noted in the aurora-omega-spec audit.

### F.5.4 Choice of weights

The default cascade $\alpha = \{1.0, 1.5, 2.0, 2.5, 3.0\}$ with uniform weights $w_\ell = 0.2$ is chosen to balance: (i) robustness to outliers (heavy $\alpha_0 = 1$ weight), (ii) coverage of the standard Gaussian mean case ($\alpha = 2$), (iii) sensitivity to upper-tail blowouts ($\alpha = 3$), while keeping all moments existence-friendly for typical heavy-tailed funding data (which generally have $\approx 3$-$4$ finite moments).

---

## F.6 Red-flag trigger as a consistency property

The tail-estimate red flag

$$\Delta S_t := S_t - \bar S_{[t-W, t]} < -\theta_S \cdot \sigma_S$$

fires when the running α-cascade score has dropped $\theta_S$ baseline-window standard deviations below its own trailing mean. Since $S$ is strictly proper under the theorem, a stable unbiased predictor targeting $T_k(P)$ achieves a constant expected $S$ under a fixed $P$, and any material drop in $S$ implies *either* (a) the predictor has drifted away from $T_k(P)$, *or* (b) the underlying $P$ has changed in a way that shifts the $\alpha$-functionals. Both are legitimate reasons to enter Robust mode. The flag cannot be gamed by a biased-constant predictor because strict propriety forbids that predictor from achieving the same expected score as the $P$-optimal one.

**Caveat.** Strict propriety is a *population* property (it holds under $\mathbb E_P[\cdot]$). The empirical rolling-window version $\Delta S_t$ has finite-sample noise. The $2\sigma_S$ gate is a heuristic that trades false-positive rate against detection delay — it is not claimed to be optimal. Principled alternatives (e.g. a sequential CUSUM on $S_t$, or a conformal change-point test) are future work.

---

## F.7 Empirical validation (runtime sanity test)

`scripts/validate_aurora_omega.py` implements the following propriety check:

**Test F.7.a (strict-propriety on Gaussian mixture).**
1. Draw $N = 10^4$ samples from $P = 0.7 \mathcal N(0, 1) + 0.3 \mathcal N(3, 2^2)$.
2. Compute $h(c) = \sum_\ell w_\ell \, \hat{\mathbb E}[|c - X|^{\alpha_\ell}]$ on a grid of $c$.
3. Find the numerical argmin $c^*$.
4. Assert $h(c^* \pm \varepsilon) > h(c^*)$ for $\varepsilon = 0.01, 0.1, 1.0$ (monotone strict).
5. Assert the map $c \mapsto h(c)$ is strictly convex on the grid (finite-diff second-derivative $> 0$).

**Test F.7.b (per-α limit check).**
For single-$\alpha$ cascades with $L_{\max} = 0$:
- $\alpha_0 = 1$: assert $c^* \approx \text{median}(P)$ (within 0.05).
- $\alpha_0 = 2$: assert $c^* \approx \text{mean}(P)$ (within 0.05).

**Test F.7.c (cascade differs from both).**
For the default cascade $\{1, 1.5, 2, 2.5, 3\}$ with uniform weights, assert $c^*$ lies strictly between median($P$) and mean($P$) for a skewed $P$, confirming that the cascade tracks a distinct M-estimator rather than one of the degenerate limits.

**Test F.7.d (non-gamability of constant predictor).**
Generate 1000 candidate constant predictors $\hat c \in [c^* - 3, c^* + 3]$. Verify $S(c^*) > S(\hat c)$ for all $\hat c \ne c^*$. (Sanity check of the strict inequality in expectation on this specific $P$.)

**Test F.7.e (weight edge cases).**
- All weight on $\ell = 0$ ($w_0 = 1$, $\alpha_0 = 1$): score consistent for median — verify.
- All weight on largest $\ell$ ($w_{L_{\max}} = 1$, $\alpha_{L_{\max}} = 3$): score consistent for the unique $\alpha=3$ M-estimator — verify uniqueness via strict convexity sweep.

All tests must pass with tolerance $10^{-2}$ on continuous parameters and exact on convexity sign.

---

## F.8 What is NOT proven here

1. **Finite-sample rate.** We give no finite-sample bound on how quickly the empirical $\hat h(c)$ concentrates around the population $h(c)$. A Maurer-Pontil empirical Bernstein bound (cf. `strategy/frontier.py`) could be applied but is deferred — the rolling-window heuristic does not rely on a specific rate.

2. **Robust baseline when $P$ changes.** The rolling baseline mean/std assumes approximate stationarity over the window $W$. Under rapid regime change, both baseline and current score shift simultaneously, and $\Delta S_t$ can underreact. An adaptive-forgetting baseline or CUSUM variant is listed as future work in §F.6.

3. **Joint distributional propriety.** The theorem proves strict propriety for point forecasts against a joint vector observation via coordinate decomposition. It does NOT claim strict propriety for full probabilistic forecasts (which would require the Gneiting-Raftery framework on density forecasts). Aurora-Ω's forecast layer emits point predictions, so this restriction is adequate for the engine.

4. **α = ∞ as a limit rule.** §F.5.3 describes $L^\infty$ as a formal limit but the proof chain uses finite $\alpha_\ell$. Near-infinite $\alpha$ in practice runs into floating-point overflow; the implementation caps the cascade at $\alpha = \alpha_0 + L_{\max} \eta \le 3$ by default to keep numerical stability.

---

## F.9 Implementation cross-reference

| Section | Python reference | Rust target |
|---|---|---|
| F.1 setup, α-cascade definition | `strategy/forecast_scoring.py::CascadeConfig`, `alpha_grid` | `bot-strategy-v3/forecast/forecast_scoring.rs::CascadeConfig` |
| F.4 theorem (score computation) | `cascade_score(residuals, cfg)` | `cascade_score(&[f64], &CascadeConfig)` |
| F.6 red flag trigger | `tail_deterioration_flag`, `BaselineRing` | `TailFlag::evaluate`, `BaselineRing` |
| F.7 empirical validation | `scripts/validate_aurora_omega.py::test_alpha_cascade_propriety` | `tests/parity/forecast_scoring_parity.rs` |

**End of Appendix F.**

---

# Appendix B — Empirical contraction measurement for the self-correcting map (Lemma S3)

**Relation to spec:** Aurora-Ω §24.3. Supports the claim in §24.2 that hard-clip and Banach contraction are separate guarantees.

## B.1 Setup

Let $\theta_t \in \mathbb R$ be a scalar parameter updated by

$$\theta_{t+1} = \text{clip}(\mathcal T(\theta_t), [-\Delta_{\max}, \Delta_{\max}]; \theta_t)$$

where $\mathcal T(\theta) = (\lambda \mathbb E[R \mid \theta] + \beta u(\theta)) / (\beta + \lambda)$ and the clip is applied to the step $\mathcal T(\theta_t) - \theta_t$ rather than to $\mathcal T(\theta_t)$ itself.

The runtime does **not** observe $\mathcal T$ directly — it only has access to the realized trajectory $\{\theta_t\}$ and to its own configuration. Lipschitz of $\mathcal T$ is a population property; we need an empirical proxy.

## B.2 Lemma S3 — Empirical Lipschitz upper bound

Let $\hat L_W(t) := \max_{s \in [t - W, t]} |\theta_{s+1} - \theta_s|_\infty$ over a window of $W$ consecutive ticks. Then:

1. $\hat L_W(t)$ is a conservative empirical upper bound on the **realized** step magnitude over the window.
2. If the clip is not binding ($\hat L_W(t) < \Delta_{\max}$ strictly), the map $\mathcal T$ produced at least one unclipped step of magnitude $\hat L_W(t)$, so $|\mathcal T(\theta_s) - \theta_s| \ge \hat L_W(t)$ for some $s$ in the window.
3. Statement (2) is **not** a Lipschitz bound on $\mathcal T$; it is a bound on *one realized step*. To convert to a Lipschitz estimate we would need two steps from the same $\theta$ values, which the runtime never sees (trajectories don't repeat).

## B.3 Runtime procedure

1. Track $\{\theta_t\}$ in `fsm_controller` state.
2. Every $W$ ticks, compute $\hat L_W$ via `empirical_lipschitz_estimate(theta_history, t_history, window=W)`.
3. Emit $\hat L_W / \Delta_{\max}$ as telemetry. This ratio $\in [0, 1]$ tells the operator how much of the clip budget is being used.
4. If $\hat L_W / \Delta_{\max} > 0.9$ persistently, the clip is binding and the adapter is in a regime where the hard clip dominates $\mathcal T$. The operator should investigate: either $L_{\mathcal T}$ has grown (external shock), or the reward estimate $\mathbb E[R \mid \theta]$ has become unstable.

## B.4 What this does NOT prove

- $\hat L_W$ does not upper-bound $L_{\mathcal T}$ in the population sense.
- A small $\hat L_W$ does not imply convergence — it only implies the realized sequence is moving slowly.
- A large $\hat L_W$ does not imply divergence — it could reflect a genuine regime change that the adapter is legitimately tracking.

The lemma's role is purely diagnostic. It gives the operator a **stability-monitoring metric**, not a correctness proof.

**End of Appendix B.**

---

# Appendix C — CVaR budget Latin-Hypercube derivation

**Relation to spec:** Aurora-Ω §28. Replaces the provisional "CVaR_99 ≤ 76k" single-number bound.

## C.1 Motivation

A single-number CVaR bound is operationally brittle: (a) it hides the parameter uncertainty that drives tail behavior, (b) it forces a binary halt/continue decision with no middle ground, (c) it cannot be verified short-term because tail events are rare by definition. The replacement is a three-tier budget derived from a scenario envelope.

## C.2 Method

1. **Parameter envelope.** Define plausible ranges for every calibration parameter that touches the loss tail: per-portfolio deployed notional, active pair count, adverse-drift bound $r_*$, Beta posterior $(a, b)$, fallback mixture weights (Exp fraction + Pareto $\alpha$), IOC failure probability, toxicity breach probability, and basis-blowout triplet (frequency, correlation fraction, shock magnitude).

2. **Latin Hypercube sampling.** Draw $N$ scenarios from the product of those ranges using LHS with independent per-dimension strata. LHS gives better space-filling coverage than Monte Carlo at equal $N$ (see McKay-Beckman-Conover 1979).

3. **Per-scenario simulation.** For each scenario, simulate $T$ ticks of portfolio-level loss. Each tick's loss is a sum of three routine components (adverse-selection, fallback execution cost, toxicity breach) aggregated across $n_{\text{pairs}}$ independent legs, plus a rare correlated basis-blowout component that fires with the scenario's `basis_shock_prob` and hits `basis_corr_frac` of the portfolio with magnitude `basis_shock_mag`.

4. **Tail aggregation.** For each scenario, compute $\text{CVaR}_\alpha$ via Rockafellar-Uryasev (`cvar_ru`). This gives $N$ CVaR samples.

5. **Tier derivation.** The three budget tiers are the (p50, p85, p95) quantiles of the CVaR sample distribution:
   - $\text{budget} := \text{p50}$ — typical regime; below this, the bot runs unrestricted.
   - $\text{warning} := \text{p85}$ — upper-normal tail; 85% of regimes stay below.
   - $\text{halt} := \text{p95}$ — catastrophic tail; 95% of regimes stay below.

## C.3 Results (100,000-scenario production run)

Derived in `scripts/validate_risk_budget.py` with `n_scenarios=100000`, `n_losses=400`, portfolio notional $100k–$1.5M, seed=20260415. This run replaces the earlier 2,000-scenario bootstrap and is the current calibration of record pending Phase 1 live data.

**CVaR_95 (fast guard):**

| quantile | value |
|---|---|
| p50 (budget) | $965.12 |
| p75 | $3,131.30 |
| p85 (warning) | $5,093.26 |
| p90 | $6,656.32 |
| p95 (halt) | $9,489.69 |
| p99 | $17,540.98 |
| max | $55,048.41 |

Derived tiers (rounded to round numbers close to LHS quantiles):

| tier | threshold |
|---|---|
| budget (p50) | $1,000 |
| warning (p85) | $5,100 |
| halt (p95) | $9,500 |

**CVaR_99 (deep guard):**

| quantile | value |
|---|---|
| p50 (budget) | $1,483.09 |
| p75 | $13,344.32 |
| p85 (warning) | $22,762.86 |
| p90 | $30,377.07 |
| p95 (halt) | $44,366.89 |
| p99 | $84,231.16 |
| max | $256,354.71 |

Derived tiers:

| tier | threshold |
|---|---|
| budget (p50) | $1,500 |
| warning (p85) | $23,000 |
| halt (p95) | $44,000 |

Implemented as `DEFAULT_BUDGET_95` and `DEFAULT_BUDGET_99` in `strategy/risk_stack.py`. **PROVISIONAL** — Phase 1 live data must replace the parameter envelope with empirical quantiles.

### C.3.1 Shift vs earlier 2K bootstrap

The 100K run moved CVaR_99 budget (p50) by −26% relative to the earlier 2K estimate ($2,000 → $1,483). All other tiers shifted within ±12%. The CVaR_99 budget shift is attributable to the 2K run's sampling noise at the center of the CVaR distribution; at 100K scenarios the p50 estimate has standard error on the order of $10, well below the rounding precision. The other tiers (CVaR_95 across all three quantiles, CVaR_99 warning and halt) were effectively converged already at 2K; the CVaR_99 budget (p50) was the one quantile that needed the larger sample to stabilize.

### C.3.2 Why not 1M scenarios

A 1M run at the current simulator throughput (~10ms per scenario) would take ~6 hours and yield p95 estimates to ~0.02% relative precision — effectively indistinguishable from 100K in operational terms. The marginal value of 1M over 100K is below the granularity at which these tiers are consumed (the bot rounds to nearest $100-$1000 anyway). We stop at 100K pending Phase 1 real-data calibration, which will dominate any remaining LHS noise.

## C.4 Limitations

1. **LHS quality vs. scenario size.** With $N = 2000$ scenarios the tier estimates have sampling noise of order $1/\sqrt{N}$ on the quantile. Production calibration should use $N \ge 10^5$ to tighten.

2. **Envelope coverage.** The bounds are synthetic — they don't come from measured Aurora-Ω live data (which doesn't exist yet). In particular, the basis-blowout parameters (prob, corr, magnitude) are the dominant tail driver and are the most uncertain. Phase 1 operators must observe real venue-correlation events and re-calibrate.

3. **Per-tick i.i.d. assumption.** The simulator treats ticks as independent. Real loss processes exhibit clustering (Hawkes-style, per `strategy/frontier.py`). A more refined derivation would use a clustered point process for the basis-shock component.

4. **Portfolio-scaling linearity.** The budget scales approximately linearly with deployed notional. At different AUM levels, the tiers should be scaled proportionally.

## C.5 Integration with spec §28

The results above are the source for the `BudgetTable` defaults in `strategy/risk_stack.py`. The spec §28 table is a copy of §C.3. When this appendix is re-run with a larger scenario count or a Phase 1-updated envelope, both §28 and `risk_stack.py` defaults must be updated together.

**End of Appendix C.**

---

# Appendix D — OU Stationary Tail Rate Function

**Relation to spec:** Supporting result for §21 risk stack + §28 budget. Provides an analytical CVaR upper bound for OU-driven losses that **may replace the LHS numerical bound in Appendix C once Phase 1 calibration delivers $(\kappa, \theta, \sigma)$ for observed basis spreads**. Until then, Appendix C's LHS-derived `BudgetTable` is the operational source; this appendix is documentation-only and forward-looking.

## D.1 Setup

Let $F$ be the stationary marginal of an Ornstein–Uhlenbeck process

$$dF_t = \kappa(\theta - F_t)\,dt + \sigma\,dW_t$$

so $F_\infty \sim \mathcal N(\theta,\,\sigma^2/(2\kappa))$. Write $s^2 := \sigma^2/(2\kappa)$ for the stationary variance.

## D.2 MGF of the stationary marginal

For a Gaussian $\mathcal N(\mu, v)$ the log-MGF is $\lambda\mu + \lambda^2 v/2$. Substituting $\mu = \theta$ and $v = \sigma^2/(2\kappa)$:

$$\psi(\lambda) := \log E[e^{\lambda F_\infty}] = \lambda\theta + \frac{\lambda^2 \sigma^2}{4\kappa}.$$

This is **finite for all $\lambda \in \mathbb R$** (Gaussian is sub-exponential), so Gärtner–Ellis applies at the single-time level without any time-scaling limit.

## D.3 Rate function

The Legendre–Fenchel dual of $\psi$ gives the Cramér rate function:

$$\Lambda^*(x) := \sup_{\lambda \in \mathbb R}\bigl\{\lambda x - \psi(\lambda)\bigr\} = \sup_{\lambda}\Bigl\{\lambda(x - \theta) - \tfrac{\lambda^2\sigma^2}{4\kappa}\Bigr\}.$$

First-order condition: $x - \theta - \lambda \sigma^2/(2\kappa) = 0$, so $\lambda^* = 2\kappa(x-\theta)/\sigma^2$. Substituting back:

$$\Lambda^*(x) = \lambda^*(x-\theta) - \frac{(\lambda^*)^2 \sigma^2}{4\kappa} = \frac{2\kappa(x-\theta)^2}{\sigma^2} - \frac{\kappa(x-\theta)^2}{\sigma^2} = \boxed{\frac{\kappa(x-\theta)^2}{\sigma^2}}.$$

This is simply the Gaussian tail exponent for $\mathcal N(\theta, s^2)$ with $s^2 = \sigma^2/(2\kappa)$:
$$\frac{(x-\theta)^2}{2 s^2} = \frac{(x-\theta)^2}{2 \cdot \sigma^2/(2\kappa)} = \frac{\kappa(x-\theta)^2}{\sigma^2}.$$

**Important note on derivation.** A common error is to try to derive $\Lambda^*$ via a Gärtner–Ellis *time-scaling limit* $\lim_{t \to 0} (1/t)\, \psi(t\lambda)$ on OU. That limit is *degenerate* (equals $\lambda\theta$) because the stationary MGF is quadratic in $\lambda$ with no $t$-dependence, so $(1/t)\cdot t^2\lambda^2\cdot\text{const} \to 0$. The correct derivation is the **single-time** Gaussian-tail computation above — no time scaling involved.

## D.4 Tail bound

By Markov + Legendre:

$$P(F_\infty > x) \le \exp(-\Lambda^*(x)) = \exp\!\left(-\frac{\kappa(x-\theta)^2}{\sigma^2}\right).$$

This is equivalent (up to polynomial prefactor) to the exact Gaussian tail
$$P(F_\infty > x) = \Phi^c\!\left(\frac{x-\theta}{s}\right),\qquad s^2 = \frac{\sigma^2}{2\kappa}.$$

## D.5 CVaR bound via the rate function

A Chernoff-style CVaR upper bound uses the rate function to control the tail integral:

$$CVaR_\alpha(F_\infty) \le \inf_{c > \theta}\left\{c + \frac{1}{1 - \alpha} \cdot E\!\left[(F_\infty - c)\mathbf 1\{F_\infty > c\}\right]\right\}.$$

For Gaussian $F_\infty$, the right-hand side has a closed form using $\phi$ (PDF) and $\Phi^c$ (upper tail). A standard tight bound using the rate function directly:

$$E\!\left[(F_\infty - c)\mathbf 1\{F_\infty > c\}\right] \le \frac{s^2}{c - \theta} \cdot \exp\!\left(-\frac{(c-\theta)^2}{2s^2}\right) = \frac{\sigma^2/(2\kappa)}{c-\theta} \cdot \exp(-\Lambda^*(c))$$

valid for $c > \theta$, following from the standard tail-integration identity $\int_c^\infty (f-c)\,\phi((f-\theta)/s)\,df/s \le s \cdot \phi((c-\theta)/s)$ after dividing by $(c-\theta)/s$.

**Tight Gaussian CVaR (exact, for comparison).** The operational bound is the exact formula
$$CVaR_\alpha(F_\infty) = \theta + s \cdot \frac{\phi(\Phi^{-1}(\alpha))}{1-\alpha} = \theta + \sqrt{\frac{\sigma^2}{2\kappa}} \cdot \frac{\phi(\Phi^{-1}(\alpha))}{1-\alpha}.$$

At $\alpha = 0.99$: $\phi(\Phi^{-1}(0.99)) \approx 0.0267$, so
$$CVaR_{0.99}(F_\infty) \approx \theta + 2.67 \cdot \sqrt{\frac{\sigma^2}{2\kappa}}.$$

## D.6 Parameter trade-off

The stationary variance $s^2 = \sigma^2/(2\kappa)$ couples $\sigma$ and $\kappa$. Two regimes with **identical** $\sigma^2/\kappa$ produce identical tail behavior:
$$CVaR_{0.99}^{(1)} \approx \theta + 2.67 \sqrt{\frac{\sigma_1^2}{2\kappa_1}} = \theta + 2.67 \sqrt{\frac{\sigma_2^2}{2\kappa_2}} = CVaR_{0.99}^{(2)} \iff \frac{\sigma_1^2}{\kappa_1} = \frac{\sigma_2^2}{\kappa_2}.$$

Operational consequence: when calibrating $(\hat\kappa, \hat\sigma)$ from live data, a **joint confidence band** on the ratio $\sigma^2/\kappa$ is more useful than separate bands on $\sigma$ and $\kappa$, because risk budget depends only on this ratio.

## D.7 Integration with Appendix C

Appendix C's LHS-derived `DEFAULT_BUDGET_99 = (budget=$1,500, warning=$23,000, halt=$44,000)` is a **portfolio-level** bound that folds in basis-blowout tail, IOC failures, partial-fill variance, and toxicity breach. Appendix D's OU bound is a **per-pair basis-spread** bound that assumes Gaussian dynamics and no regime transition.

The two are **complementary, not competitors**:
- Appendix C: conservative, numerical, captures fat-tail and jump components.
- Appendix D: tight, analytical, applicable once OU calibration exists.

When Phase 1 live data delivers $(\hat\kappa, \hat\theta, \hat\sigma)$ for each active symbol, the per-pair OU CVaR can be computed via D.5 and **aggregated up to portfolio level** for comparison with the LHS bound. If the OU analytical bound is tighter than the LHS bound on the same portfolio, the operator may elect to use the OU bound as the `DEFAULT_BUDGET_99` source. Until then, Appendix C's LHS numbers are authoritative.

## D.8 What this appendix is NOT

- **Not** a replacement for Appendix C at v1.1.2. Data-free; requires Phase 1 calibration before use.
- **Not** a large-deviation result for ergodic averages $(1/t)\int_0^t F_s\,ds$ — that requires a different derivation (Donsker–Varadhan for empirical measure) and has a different rate function.
- **Not** applicable to cross-venue basis blowout — that is a regime-transition event, not a Gaussian tail event. Appendix C's LHS with basis-shock correlation is the correct tool for that.

**End of Appendix D.**


