// Quantization module for DeepBook pool constraints
// This file handles price and size quantization according to pool parameters
//
// Numan Thabit 2025 Nov

use anyhow::{ensure, Result};

#[derive(Debug, Clone)]
pub struct PoolParams {
    /// Tick size expressed in quote units per base unit.
    pub tick_size: f64,
    /// Lot size expressed in base units.
    pub lot_size: f64,
    /// Minimum order size expressed in base units.
    pub min_size: f64,
}

pub fn quantize_price(price: f64, tick_size: f64) -> Result<f64> {
    ensure!(
        tick_size.is_finite() && tick_size > 0.0,
        "tick size must be positive"
    );
    ensure!(
        price.is_finite() && price > 0.0,
        "price must be positive and finite"
    );
    let steps = (price / tick_size).floor();
    ensure!(
        steps >= 1.0,
        "price {price} is below minimum tick {tick_size}"
    );
    Ok(steps * tick_size)
}

pub fn quantize_size(quantity: f64, lot_size: f64, min_size: f64) -> Result<f64> {
    ensure!(
        lot_size.is_finite() && lot_size > 0.0,
        "lot size must be positive"
    );
    ensure!(
        min_size.is_finite() && min_size > 0.0,
        "min size must be positive"
    );
    ensure!(
        quantity.is_finite() && quantity >= min_size,
        "quantity {quantity} below minimum size {min_size}"
    );
    let steps = (quantity / lot_size).floor();
    ensure!(
        steps >= 1.0,
        "quantity {quantity} insufficient for lot size {lot_size}"
    );
    Ok(steps * lot_size)
}
