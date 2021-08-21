use std::collections::HashMap;

use crate::{
    contribution::{calculate, Contribution, ContributionError},
    frequency::Frequency,
    CURRENCY_PRECISION,
};
use chrono::{Date, Utc};
use log::{debug, trace};
use rust_decimal::Decimal;
use thiserror::Error;

/// The representation of a future transaction.
///
/// `TransactionModel`s are the building blocks of a budget, and are used to track
/// revenues, expenses and savings over time. These models are also used to calculate the
/// affordability of a user's finances in perpetuity.
#[derive(Debug)]
pub struct TransactionModel {
    value: Decimal,
    contributions: Vec<Contribution>,
    frequency: Frequency,
}

#[derive(Error, Debug, PartialEq)]
pub enum TransactionError {
    #[error("could not calculate contributions: {0}")]
    Contribution(#[from] ContributionError),
    #[error("currency values cannot have more than 2 decimal places: {0}")]
    CurrencyPrecision(Decimal),
}

/// The result of an affordability calculation. See [`is_affordable`] for details.
#[derive(PartialEq, Eq, Debug)]
pub enum AffordabilityResult {
    /// The balance of transactions is less than zero
    Deficit(Vec<Date<Utc>>),
    /// The balance of transactions equals zero
    Balanced,
    /// The balance of transactions is greater than zero
    Surplus(Vec<Date<Utc>>),
}

#[derive(Debug)]
enum ContributionSign<'a> {
    Positive(&'a Contribution),
    Negative(&'a Contribution),
}

impl TransactionModel {
    /// Create a new `Transaction`.
    ///
    /// This function captures a `calculation_date`, which represents the date that this
    /// `Transaction` was first calculated. This is important for recreating past
    /// `Transaction`s accurately, as a `Transaction`'s contributions towards a future
    /// payment will often begin on the day that the `Transaction` is calculated.
    pub fn new(
        value: Decimal,
        frequency: Frequency,
        start_date: Date<Utc>,
        end_date: Option<Date<Utc>>,
        calculation_date: Option<Date<Utc>>,
    ) -> Result<Self, TransactionError> {
        // Check that we have a valid currency value
        if value.round_dp(CURRENCY_PRECISION) != value {
            return Err(TransactionError::CurrencyPrecision(value));
        }

        let now = calculation_date.unwrap_or_else(Utc::today);
        let contributions = calculate(value, &frequency, start_date, end_date, now)?;

        Ok(TransactionModel {
            value,
            contributions,
            frequency,
        })
    }
}

impl<'a> ContributionSign<'a> {
    pub fn regular_or_last(&self, date: Date<Utc>) -> Option<Decimal> {
        match self {
            ContributionSign::Positive(c) => c.regular_or_last(date),
            &ContributionSign::Negative(c) => {
                c.regular_or_last(date).map(|i| i * Decimal::NEGATIVE_ONE)
            }
        }
    }

    pub fn start_date(&self) -> Date<Utc> {
        self.inner().start_date()
    }

    pub fn period_end(&self, date: Option<Date<Utc>>) -> Date<Utc> {
        self.inner().period_end(date)
    }

    fn inner(&self) -> &Contribution {
        match self {
            ContributionSign::Positive(c) => c,
            ContributionSign::Negative(c) => c,
        }
    }
}

/// Calculate whether a collection of revenue, expense and savings transactions are
/// sustainable in perpetuity.
///
/// In other words, calculate whether our expense and savings transactions will ever
/// cause us to lose money over time.
///
/// The affordability equation is: `revenue = savings + expenses`.
///
/// Counterintuitively, the optimal affordability result is where you have a zero balance
/// rather than a surplus (i.e. `AffordabilityResult::Balanced`). This is optimal because
/// you have perfectly allocated all revenues to either expenses or savings. No amounts
/// will be left untracked.
pub fn is_affordable(
    revenues: Option<&[TransactionModel]>,
    expenses: Option<&[TransactionModel]>,
    savings: Option<&[TransactionModel]>,
) -> AffordabilityResult {
    debug!("calculating affordability");

    // let mut day_totals = HashMap::new();
    let mut contributions = Vec::new();

    // Extract revenue contributions
    let r = revenues.unwrap_or_default();
    r.iter()
        .flat_map(|t| &t.contributions)
        .map(ContributionSign::Positive)
        .for_each(|c| contributions.push(c));

    // Extract expense contributions
    let e = expenses.unwrap_or_default();
    e.iter()
        .flat_map(|t| &t.contributions)
        .map(ContributionSign::Negative)
        .for_each(|c| contributions.push(c));

    // Extract savings contributions
    let s = savings.unwrap_or_default();
    s.iter()
        .flat_map(|t| &t.contributions)
        .map(ContributionSign::Negative)
        .for_each(|c| contributions.push(c));

    let day_totals = acc_daily_contributions(&contributions);

    // Accumulate surplus dates
    let surplus: Vec<Date<Utc>> = day_totals
        .iter()
        .filter(|(_, dec)| **dec > Decimal::ZERO)
        .map(|(date, _)| *date)
        .collect();

    // Accumulate deficit dates
    let deficit: Vec<Date<Utc>> = day_totals
        .iter()
        .filter(|(_, dec)| **dec < Decimal::ZERO)
        .map(|(date, _)| *date)
        .collect();

    if !deficit.is_empty() {
        AffordabilityResult::Deficit(deficit)
    } else if !surplus.is_empty() {
        AffordabilityResult::Surplus(surplus)
    } else {
        AffordabilityResult::Balanced
    }
}

fn acc_daily_contributions(contributions: &[ContributionSign]) -> HashMap<Date<Utc>, Decimal> {
    let mut day_totals = HashMap::new();

    trace!("provided contributions: {:?}", contributions);

    if let Some(min) = contributions.iter().min_by_key(|c| c.start_date()) {
        // If we have a min, we must have a max, so unwrapping is safe
        let max = contributions
            .iter()
            .max_by_key(|c| c.period_end(None))
            .unwrap();

        trace!(
            "contribution date range is from {} to {}",
            min.start_date(),
            max.period_end(None)
        );

        let mut date = min.start_date();
        while date <= max.period_end(None) {
            // Calculate total for date
            let total = contributions
                .iter()
                .filter_map(|c| c.regular_or_last(date))
                .fold(Decimal::ZERO, |total, value| total + value);

            trace!(
                "accumulating date {}: current {} + new {} = {}",
                date,
                day_totals.get(&date).unwrap_or(&Decimal::ZERO),
                total,
                day_totals.get(&date).unwrap_or(&Decimal::ZERO) + total
            );

            // Insert/update total
            day_totals
                .entry(date)
                .and_modify(|mut d| d += total)
                .or_insert(total);

            date = date.succ();
        }
    }

    day_totals
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone};
    use rust_decimal_macros::dec;

    #[test]
    fn new_transaction_ok() {
        let start_date = Utc::today();
        let result = TransactionModel::new(
            dec!(0.01),
            Frequency::Weekly(1, vec![start_date.weekday().number_from_monday()]),
            start_date,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn is_affordable_balanced() {
        let _ = env_logger::builder().is_test(true).try_init();
        let today = Utc.ymd(2000, 4, 1);
        let start = Utc.ymd(2000, 4, 7);

        let revenues = vec![TransactionModel::new(
            dec!(14),
            Frequency::Weekly(1, vec![4]),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let expenses =
            vec![
                TransactionModel::new(dec!(1), Frequency::Daily(1), start, None, Some(today))
                    .unwrap(),
            ];

        let savings =
            vec![
                TransactionModel::new(dec!(1), Frequency::Daily(1), start, None, Some(today))
                    .unwrap(),
            ];

        assert_eq!(
            is_affordable(Some(&revenues), Some(&expenses), Some(&savings)),
            AffordabilityResult::Balanced
        );
    }

    #[test]
    fn is_affordable_deficit() {
        let _ = env_logger::builder().is_test(true).try_init();
        let today = Utc.ymd(2000, 4, 1);
        let start = Utc.ymd(2000, 4, 7);

        let revenues = vec![TransactionModel::new(
            dec!(14),
            Frequency::Weekly(1, vec![4]),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let expenses =
            vec![
                TransactionModel::new(dec!(1), Frequency::Daily(1), start, None, Some(today))
                    .unwrap(),
            ];

        let savings = vec![
            TransactionModel::new(dec!(1), Frequency::Daily(1), start, None, Some(today)).unwrap(),
            TransactionModel::new(dec!(2), Frequency::Once, start, None, Some(start)).unwrap(),
        ];

        assert_eq!(
            is_affordable(Some(&revenues), Some(&expenses), Some(&savings)),
            AffordabilityResult::Deficit(vec![start])
        );
    }

    #[test]
    fn is_affordable_surplus() {
        let _ = env_logger::builder().is_test(true).try_init();
        let today = Utc.ymd(2000, 4, 1);
        let start = Utc.ymd(2000, 4, 7);

        let revenues = vec![TransactionModel::new(
            dec!(14),
            Frequency::Weekly(1, vec![4]),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let expenses = vec![TransactionModel::new(
            dec!(1),
            Frequency::Daily(1),
            start.succ(),
            None,
            Some(today),
        )
        .unwrap()];

        let savings =
            vec![
                TransactionModel::new(dec!(1), Frequency::Daily(1), start, None, Some(today))
                    .unwrap(),
            ];

        assert_eq!(
            is_affordable(Some(&revenues), Some(&expenses), Some(&savings)),
            AffordabilityResult::Surplus(vec![start])
        );
    }
}
