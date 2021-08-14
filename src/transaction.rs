use crate::{
    contribution::{calculate, Contribution, ContributionError},
    frequency::Frequency,
    TransactionValue, CURRENCY_PRECISION,
};
use chrono::{Date, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

/// The representation of a future transaction.
///
/// `TransactionModel`s are the building blocks of a budget, and are used to track
/// revenues, expenses and savings over time. These models are also used to calculate the
/// affordability of a user's finances in perpetuity.
#[derive(Debug)]
pub struct TransactionModel {
    value: TransactionValue,
    contributions: Vec<Contribution>,
    frequency: Frequency,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
}

#[derive(Error, Debug, PartialEq)]
pub enum TransactionError {
    #[error("could not calculate contributions: {0}")]
    Contribution(#[from] ContributionError),
    #[error("currency values cannot have more than 2 decimal places: {0}")]
    CurrencyPrecision(Decimal),
}

impl TransactionModel {
    /// Create a new `Transaction`.
    ///
    /// This function captures a `calculation_date`, which represents the date that this
    /// `Transaction` was first calculated. This is important for recreating past
    /// `Transaction`s accurately, as a `Transaction`'s contributions towards a future
    /// payment will often begin on the day that the `Transaction` is calculated.
    pub fn new(
        value: TransactionValue,
        frequency: Frequency,
        start_date: Date<Utc>,
        end_date: Option<Date<Utc>>,
        calculation_date: Option<Date<Utc>>,
    ) -> Result<Self, TransactionError> {
        let v = match value {
            TransactionValue::Fixed(v) => v,
            TransactionValue::Variable(_, v) => v,
        };

        // Check that we have a valid currency value
        if v.round_dp(CURRENCY_PRECISION) != v {
            return Err(TransactionError::CurrencyPrecision(v));
        }

        let now = calculation_date.unwrap_or_else(Utc::today);
        let contributions = calculate(v, &frequency, start_date, end_date, now)?;

        Ok(TransactionModel {
            value,
            contributions,
            frequency,
            start_date,
            end_date,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use rust_decimal_macros::dec;

    #[test]
    fn new_transaction_ok() {
        let start_date = Utc::today();
        let result = TransactionModel::new(
            TransactionValue::Fixed(dec!(0.01)),
            Frequency::Weekly(1, vec![start_date.weekday().number_from_monday()]),
            start_date,
            None,
            None,
        );
        assert!(result.is_ok());
    }
}
