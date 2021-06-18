mod contribution;
mod frequency;

use self::frequency::Frequency;
use chrono::{Date, Utc};
use contribution::Contribution;
use std::{convert::TryFrom, ops::Deref};
use thiserror::Error;

// This represents the number of decimal places that a currency can validly express.
// I.e. multiplying by 100 shifts the decimal place 2 significant figures, allowing us to
// round the number.
// @todo Support the full range of currency precisions specified in ISO 4217.
const CURRENCY_PRECISION: f64 = 100.0;

// This represents the maximum number of days that a time span can be.
// NOTE! If you change this const, make sure you also update the `ExcessiveDateRange`
// error message.
const MAX_DAYS: u32 = (325.25 * 10.0) as u32; // 10 years

// Internal representation of a currency value.
// Given the limitations of representing floating point numbers in binary, we instead
// represent currency values (i.e. f64 to n decimal places) as integers. For example,
// AUD2.75 becomes 275.
#[derive(Debug, PartialEq)]
struct CurrencyValue(i64);

#[derive(Error, Debug)]
pub enum CurrencyError {
    #[error("currency values cannot have more than 2 decimal places")]
    CurrencyPrecision,
}

/// The model for a future transaction or group of transactions.
///
/// `TransactionModel`s are the building blocks of a budget, and are used to track
/// revenues, expenses and savings over time. These models are also used to calculate the
/// affordability of a user's finances in perpetuity.
pub struct TransactionModel {
    matcher: TransactionMatcher,
    value: TransactionValue,
    contribution: Contribution,
    ameliorations: Option<Vec<Contribution>>,
    frequency: Frequency,
    // This is the date that the contributions were calculated from. If we lose this date
    // then we won't be able to recalculate our contributions accurately later.
    calculation_date: Date<Utc>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
}

#[derive(Error, Debug, PartialEq)]
pub enum TransactionModelError {
    #[error("date ranges greater than 10 years are unsupported")]
    ExcessiveDateRange,
}

/// Matches one or more transactions
#[derive(Default)]
pub struct TransactionMatcher {
    category: Option<String>,
    description: Option<Vec<String>>,
}

/// The value of a modelled transaction
pub enum TransactionValue {
    Fixed(f64),
    Variable(f64, f64), // Lower bound, upper bound
}

impl TryFrom<f64> for CurrencyValue {
    type Error = CurrencyError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        // Convert a floating point currency value with n decimal places to an int with
        // zero decimal places.
        let int_value = f64::trunc(value * CURRENCY_PRECISION);

        if value != int_value / CURRENCY_PRECISION {
            return Err(CurrencyError::CurrencyPrecision);
        }

        Ok(CurrencyValue(int_value as i64))
    }
}

impl From<CurrencyValue> for f64 {
    fn from(value: CurrencyValue) -> f64 {
        value.0 as f64 / CURRENCY_PRECISION
    }
}

impl Deref for CurrencyValue {
    type Target = i64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TransactionModel {
    pub fn new(
        matcher: TransactionMatcher,
        value: TransactionValue,
        frequency: Frequency,
        start_date: Date<Utc>,
        end_date: Option<Date<Utc>>,
    ) -> Result<Self, TransactionModelError> {
        // Ensure we don't have any date ranges > 10 years
        if let Some(end) = end_date {
            let duration = end - start_date;
            if duration.num_days() > MAX_DAYS as i64 {
                return Err(TransactionModelError::ExcessiveDateRange);
            }
        }

        unimplemented!();
    }
}

impl TransactionMatcher {
    pub fn with_category<S: Into<String>>(&mut self, category: S) -> &mut Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_description<S: Into<String>>(&mut self, description: S) -> &mut Self {
        // XXX Option::get_or_insert_default would make this slightly more succinct
        // self.description.get_or_insert_default().push(description);
        // https://github.com/rust-lang/rust/issues/82901
        self.description
            .get_or_insert(Vec::new())
            .push(description.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn new_transaction_model_excessive_date_range() {
        let result = TransactionModel::new(
            TransactionMatcher::default(),
            TransactionValue::Fixed(0.0),
            Frequency::Once,
            Utc.ymd(2000, 1, 1),
            Some(Utc.ymd(3000, 1, 2)),
        );
        assert_eq!(
            result.err(),
            Some(TransactionModelError::ExcessiveDateRange)
        );
    }

    #[test]
    fn transaction_matcher_with_category_and_descriptions() {
        let mut matcher = TransactionMatcher::default();
        matcher
            .with_category("abc")
            .with_description("def")
            .with_description("ghi");
        assert_eq!(matcher.category, Some("abc".into()));
        assert_eq!(matcher.description, Some(vec!["def".into(), "ghi".into()]));
    }
}
