# Router/Execution Plane

This module implements the router and execution plane for the Numi Sui Stack aggregator, as described in the main README.

## Components

### RouteSelector (`selector.rs`)
Selects optimal execution routes based on price-of-execution, which includes:
- L2 price from venues
- Expected fill slippage
- Gas costs
- Latency penalties (higher for shared-object routes)
- Venue failure risk

### ValidatorSelector (`validator.rs`)
Tracks validator performance using EWMA (Exponentially Weighted Moving Average) of effects times. Selects the best validator based on:
- Observed effects latency
- Health status
- Staleness of statistics

### ExecutionEngine (`execution.rs`)
Compiles routes to PTBs, signs transactions, and submits them with:
- Idempotent retry logic (tracks transaction digests)
- Automatic validator selection
- Support for both gRPC and JSON-RPC execution
- Effects time tracking for validator selection

### Routes (`routes.rs`)
Defines route types and scoring:
- `DeepBookSingle`: Single-leg order on DeepBook
- `MultiVenueSplit`: Multi-venue routes (future)
- `CancelReplace`: Cancel and replace chains (future)
- `FlashLoanArb`: Flash-loan backed arbitrage (future)

## Usage Example

```rust
use ultra_aggr::router::{ExecutionEngine, RouteSelector, ValidatorSelector};
use ultra_aggr::venues::adapter::LimitReq;

// Initialize components
let validator_selector = Arc::new(ValidatorSelector::default());
validator_selector.register("https://fullnode.mainnet.sui.io:443").await;

let route_selector = RouteSelector::new(
    Some(deepbook_adapter.clone()),
    100,  // base_latency_ms (fast-path)
    400,  // shared_object_latency_ms (consensus)
);

let execution_engine = Arc::new(ExecutionEngine::new(
    Some(deepbook_adapter.clone()),
    grpc_clients,
    jsonrpc_client,
    validator_selector.clone(),
    secret_key_hex,
    use_grpc_execute,
));

// Create a limit order request
let order_req = LimitReq {
    pool: "SUI_USDC".to_string(),
    price: 1.5,
    quantity: 100.0,
    is_bid: true,
    client_order_id: "12345".to_string(),
    pay_with_deep: false,
    expiration_ms: None,
};

// Select optimal route
let selection = route_selector.select_route(&order_req).await?;
let best_plan = selection.best_plan();

// Execute the route
let result = execution_engine.execute(best_plan).await?;
println!("Executed with digest: {}", result.digest);
println!("Effects time: {}ms", result.effects_time_ms);
```

## Fast-Path Optimization

The router penalizes routes that require shared-object contention unless price improvement offsets expected latency. This aligns with Sui's execution model:
- Owned-object transactions skip consensus (fast-path)
- Shared-object transactions require consensus (Mysticeti v2)

## Idempotent Retries

The execution engine tracks transaction digests to prevent duplicate execution. If a transaction is retried with the same digest, it will be skipped.

## Validator Selection

The validator selector uses EWMA to track effects times (submission to effects observed). Validators are selected based on:
1. Health status (must be healthy)
2. Minimum observations (default: 5)
3. Staleness threshold (default: 5 minutes)
4. Lowest EWMA latency

