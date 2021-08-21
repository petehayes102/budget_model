mod contribution;
mod frequency;
mod transaction;

pub use contribution::ContributionError;
pub use frequency::{Frequency, FrequencyMonthDay};
pub use transaction::{is_affordable, AffordabilityResult, TransactionError, TransactionModel};

// This represents the number of decimal places that a currency can validly express.
// @todo Support the full range of currency precisions specified in ISO 4217.
const CURRENCY_PRECISION: u32 = 2;
