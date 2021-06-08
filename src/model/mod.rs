mod contribution;

use chrono::{DateTime, Utc};
use contribution::Contribution;
use std::{convert::TryFrom, ops::Deref};
use thiserror::Error;

// This represents the number of decimal places that a currency can validly express.
// I.e. multiplying by 100 shifts the decimal place 2 significant figures, allowing us to
// round the number.
// @todo Support the full range of currency precisions specified in ISO 4217.
const CURRENCY_PRECISION: f64 = 100.0;

// Internal representation of a currency value.
// Given the limitations of representing floating point numbers in binary, we instead
// represent currency values (i.e. f64 to n decimal places) as integers. For example,
// AUD2.75 becomes 275.
#[derive(Debug)]
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
    frequency: TransactionFrequency,
    start_date: DateTime<Utc>,
    end_date: Option<DateTime<Utc>>,
}

/// Matches one or more transactions
struct TransactionMatcher {
    category: Option<String>,
    description: Option<Vec<String>>,
}

/// The value of a modelled transaction
enum TransactionValue {
    Fixed(f64),
    Variable(f64, f64), // Lower bound, upper bound
}

/// Records the recurrence of a transaction
enum TransactionFrequency {
    Once,
    Daily(u32),                             // Every n days
    Weekly(u32, [u8; 7]),                   // Every n weeks on x days (Monday = 1, Sunday = 7)
    MonthlyDate(u32, [u8; 31]),             // Every n months each date
    MonthlyDay(u32, u8, FrequencyMonthDay), // Every n months, on the nth (First = 1, Fifth = 5, Last = 0) day
    // Every n years in x months (January = 1, December = 12) on the nth (First = 1, Fifth = 5, Last = 0) day
    Yearly(u32, [u8; 12], Option<u8>, Option<FrequencyMonthDay>),
}

enum FrequencyMonthDay {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
    Day,     // The nth day of the month
    Weekday, // The nth week day (Mon - Fri)
    Weekend, // The nth weekend day (Sat - Sun)
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

#[cfg(test)]
mod tests {
    use super::*;
}
