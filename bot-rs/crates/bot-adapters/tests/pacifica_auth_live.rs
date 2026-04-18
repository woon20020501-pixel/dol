//! Live integration test for `PacificaAuthenticatedAdapter`.
//!
//! ALL tests in this file are `#[ignore]` — they require real Pacifica
//! API credentials and network access. Run manually:
//!
//! ```bash
//! cargo test -p bot-adapters --test pacifica_auth_live -- --ignored --nocapture
//! ```
//!
//! Set `PACIFICA_API_KEY` and `PACIFICA_BUILDER_CODE` before running.
//!
//! **Endpoint path status:**
//! The primary account endpoint is `GET /api/v1/account?account=<builder_code>`
//! with header `X-API-Key`. This is consistent with the public endpoint used
//! by `PacificaRest::get_balance`. The adapter falls back to `/account/info`
//! and `/user` if the primary returns 404. If all three fail, the test will
//! surface the HTTP status so the correct path can be diagnosed.
//!
//! The builder endpoint is `GET /api/v1/builder/program?builder_code=<code>`.
//! If this path is wrong, the test will print the HTTP status.
//!
//! **No API key is ever logged.** Only builder_code and balance are printed.

use bot_adapters::pacifica_auth::PacificaAuthenticatedAdapter;
use bot_adapters::venue::VenueAdapter;
use bot_types::Venue;

fn make_adapter() -> PacificaAuthenticatedAdapter {
    PacificaAuthenticatedAdapter::from_env()
        .expect("PACIFICA_API_KEY and PACIFICA_BUILDER_CODE must be set to run live tests")
}

/// Fetch account info from the live Pacifica authenticated API.
///
/// Asserts that the response parses into `AccountInfo` and that key
/// fields are plausibly shaped. Does NOT assert exact balance values.
#[tokio::test]
#[ignore = "requires PACIFICA_API_KEY + PACIFICA_BUILDER_CODE + network"]
async fn live_fetch_account_info_parses() {
    let adapter = make_adapter();

    let info = adapter
        .fetch_account_info()
        .await
        .expect("fetch_account_info should succeed against live Pacifica");

    // Balance must be non-negative (zero is valid for a new account).
    assert!(
        info.balance_usd >= 0.0,
        "balance_usd must be non-negative: {}",
        info.balance_usd
    );
    assert!(
        info.margin_available_usd >= 0.0,
        "margin_available_usd must be non-negative"
    );

    // Log safe fields (NOT the API key).
    eprintln!("--- Pacifica authenticated account info ---");
    eprintln!("  account:              {}", info.account);
    eprintln!("  balance_usd:          {:.6}", info.balance_usd);
    eprintln!("  margin_available_usd: {:.6}", info.margin_available_usd);
    eprintln!("  margin_locked_usd:    {:.6}", info.margin_locked_usd);
    eprintln!("  open_positions_count: {}", info.open_positions_count);
    eprintln!("  builder_code (safe):  {}", adapter.builder_code());
}

/// Fetch builder program status from the live Pacifica API.
#[tokio::test]
#[ignore = "requires PACIFICA_API_KEY + PACIFICA_BUILDER_CODE + network"]
async fn live_fetch_builder_status_parses() {
    let adapter = make_adapter();

    let status = adapter
        .fetch_builder_status()
        .await
        .expect("fetch_builder_status should succeed against live Pacifica");

    // builder_code in the response should match the one we sent.
    assert_eq!(
        status.builder_code,
        adapter.builder_code(),
        "builder_code in response must match the one configured"
    );

    eprintln!("--- Pacifica builder program status ---");
    eprintln!("  builder_code:       {}", status.builder_code);
    eprintln!("  registered:         {}", status.registered);
    eprintln!("  fee_tier:           {}", status.fee_tier);
    eprintln!("  rebate_accrued_usd: {:.6}", status.rebate_accrued_usd);
    eprintln!("  since:              {:?}", status.since);
}

/// Verify that the authenticated adapter correctly delegates `fetch_snapshot`
/// to the inner read-only adapter (market data still works).
#[tokio::test]
#[ignore = "requires PACIFICA_API_KEY + PACIFICA_BUILDER_CODE + network"]
async fn live_authenticated_adapter_delegates_snapshot() {
    let adapter = make_adapter();
    assert_eq!(adapter.venue(), Venue::Pacifica);

    let snap = adapter
        .fetch_snapshot("BTC")
        .await
        .expect("fetch_snapshot must work through authenticated adapter");

    assert_eq!(snap.venue, Venue::Pacifica);
    assert_eq!(snap.symbol, "BTC");
    assert!(snap.mid_price > 0.0, "mid_price must be positive");

    eprintln!("--- Authenticated adapter BTC snapshot ---");
    eprintln!("  mid_price: {:.2}", snap.mid_price);
    eprintln!("  builder_code (safe): {}", adapter.builder_code());
}
