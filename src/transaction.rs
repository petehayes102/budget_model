use crate::{
    contribution::{calculate, Contribution, ContributionError},
    frequency::Frequency,
    TransactionMatcher, TransactionValue, CURRENCY_PRECISION,
};
use chrono::{Date, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

// This represents the maximum number of days that a time span can be.
// NOTE! If you change this const, make sure you also update the `ExcessiveDateRange`
// error message.
const MAX_DAYS: u32 = (325.25 * 10.0) as u32; // 10 years

/// The model for a future transaction or group of transactions.
///
/// `Transaction`s are the building blocks of a budget, and are used to track
/// revenues, expenses and savings over time. These models are also used to calculate the
/// affordability of a user's finances in perpetuity.
pub struct Transaction {
    matcher: TransactionMatcher,
    value: TransactionValue,
    contributions: Vec<Contribution>,
    frequency: Frequency,
    // This is the date that the contributions were calculated from. If we lose this date
    // then we won't be able to recalculate our contributions accurately later.
    calculation_date: Date<Utc>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
}

#[derive(Error, Debug, PartialEq)]
pub enum TransactionError {
    #[error("could not calculate contributions: {0}")]
    Contribution(#[from] ContributionError),
    #[error("currency values cannot have more than 2 decimal places: {0}")]
    CurrencyPrecision(Decimal),
    #[error("date ranges greater than 10 years are unsupported")]
    ExcessiveDateRange,
}

impl Transaction {
    pub fn new(
        matcher: TransactionMatcher,
        value: TransactionValue,
        frequency: Frequency,
        start_date: Date<Utc>,
        end_date: Option<Date<Utc>>,
    ) -> Result<Self, TransactionError> {
        // Ensure we don't have any date ranges > 10 years
        if let Some(end) = end_date {
            let duration = end - start_date;
            if duration.num_days() > MAX_DAYS as i64 {
                return Err(TransactionError::ExcessiveDateRange);
            }
        }

        let v = match value {
            TransactionValue::Fixed(v) => v,
            TransactionValue::Variable(_, v) => v,
        };

        // Check that we have a valid currency value
        if v.round_dp(CURRENCY_PRECISION) != v {
            return Err(TransactionError::CurrencyPrecision(v));
        }

        let contributions = calculate(v, &frequency, start_date, end_date, None)?;

        Ok(Transaction {
            matcher,
            value,
            contributions,
            frequency,
            calculation_date: Utc::today(),
            start_date,
            end_date,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone};
    use rust_decimal_macros::dec;

    #[test]
    fn new_transaction_excessive_date_range() {
        let result = Transaction::new(
            TransactionMatcher::default(),
            TransactionValue::Fixed(dec!(0.0)),
            Frequency::Once,
            Utc.ymd(2000, 1, 1),
            Some(Utc.ymd(3000, 1, 2)),
        );
        assert_eq!(result.err(), Some(TransactionError::ExcessiveDateRange));
    }

    #[test]
    fn new_transaction_ok() {
        let _ = env_logger::builder().is_test(true).try_init();

        let start_date = Utc::today();
        let result = Transaction::new(
            TransactionMatcher::default(),
            TransactionValue::Fixed(dec!(0.01)),
            Frequency::Weekly(1, vec![start_date.weekday().number_from_monday()]),
            start_date,
            None,
        );
        assert!(result.is_ok());
    }
}
