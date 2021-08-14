mod contribution;
mod frequency;
mod transaction;

use rust_decimal::Decimal;
pub use transaction::TransactionModel;

// This represents the number of decimal places that a currency can validly express.
// @todo Support the full range of currency precisions specified in ISO 4217.
const CURRENCY_PRECISION: u32 = 2;

/// The value of a modelled transaction
#[derive(Debug)]
pub enum TransactionValue {
    Fixed(Decimal),
    Variable(Decimal, Decimal), // Lower bound, upper bound
}
