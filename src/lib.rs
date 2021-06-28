mod contribution;
mod frequency;
mod transaction;

pub use transaction::Transaction;

use std::{convert::TryFrom, ops::Deref};
use thiserror::Error;

// struct Transaction {
//     amount: f32,
//     account_from: String,
//     account_to: String,
//     date: String,
//     description: Option<String>,
// }

// This represents the number of decimal places that a currency can validly express.
// I.e. multiplying by 100 shifts the decimal place 2 significant figures, allowing us to
// round the number.
// @todo Support the full range of currency precisions specified in ISO 4217.
const CURRENCY_PRECISION: f64 = 100.0;

// Internal representation of a currency value.
// Given the limitations of representing floating point numbers in binary, we instead
// represent currency values (i.e. f64 to n decimal places) as integers. For example,
// AUD2.75 becomes 275.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurrencyValue(i64);

#[derive(Error, Debug)]
pub enum CurrencyError {
    #[error("currency values cannot have more than 2 decimal places")]
    CurrencyPrecision,
}

/// Matches one or more transactions
#[derive(Default)]
pub struct TransactionMatcher {
    category: Option<String>,
    description: Option<Vec<String>>,
}

/// The value of a modelled transaction
pub enum TransactionValue {
    Fixed(CurrencyValue),
    Variable(CurrencyValue, CurrencyValue), // Lower bound, upper bound
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

impl TransactionMatcher {
    pub fn with_category<S: Into<String>>(&mut self, category: S) -> &mut Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_description<S: Into<String>>(&mut self, description: S) -> &mut Self {
        self.description
            .get_or_insert(Vec::new())
            .push(description.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
