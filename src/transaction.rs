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
    min_value: Option<Decimal>,
    contributions: Vec<Contribution>,
    frequency: Frequency,
}

/// Errors encountered whilst working with [`TransactionModel`]s.
#[derive(Error, Debug, PartialEq)]
pub enum TransactionError {
    #[error("could not calculate contributions: {0}")]
    Contribution(#[from] ContributionError),
    #[error("currency values cannot have more than 2 decimal places: {0}")]
    CurrencyPrecision(Decimal),
    #[error("no contributions could be calculated for this model")]
    EmptyContributions,
}

/// The result of an affordability calculation. See [`is_affordable`] for details.
#[derive(PartialEq, Eq, Debug)]
pub enum AffordabilityResult {
    /// The balance of transactions is less than zero for the given dates (Deficit[0]).
    /// There may also be one or more surplus days, which are included in Deficit[1].
    Deficit(Vec<Date<Utc>>, Vec<Date<Utc>>),
    /// The balance of transactions equals zero
    Balanced,
    /// The balance of transactions is greater than zero
    Surplus(Vec<Date<Utc>>),
}

// Used internally to determine whether a contribution should be added to or subtracted
// from a daily total. For example, revenues are `Positive` and should be added, whilst
// expenses are `Negative` and should be subtracted.
#[derive(Debug)]
enum ContributionSign<'a> {
    Positive(&'a Contribution),
    Negative(&'a Contribution),
}

impl TransactionModel {
    /// Create a new `TransactionModel`.
    ///
    /// This function captures a `calculation_date`, which represents the date that this
    /// `TransactionModel` was first calculated. This is important for recreating past
    /// `TransactionModel`s accurately, as a `TransactionModel`'s contributions
    /// towards a future payment will often begin on the day that the
    /// `TransactionModel` is calculated.
    pub fn new(
        value: Decimal,
        min_value: Option<Decimal>,
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

        if contributions.is_empty() {
            return Err(TransactionError::EmptyContributions);
        }

        Ok(TransactionModel {
            value,
            min_value,
            contributions,
            frequency,
        })
    }

    /// Whether this `TransactionModel` can be ameliorated.
    pub fn can_ameliorate(&self) -> bool {
        self.min_value.is_some()
    }

    /// Attempt to create a new `TransactionModel` that gets as close as it can to the
    /// target payment value.
    ///
    /// This function is used in situations where actual transactions exceed the model
    /// and we need to remedy (hence _ameliorate_) the position. We attempt to reduce the
    /// model's expenditure to a target value that will account for the discrepancy
    /// between the model and actual transactions. The `TransactionModel` to be
    /// ameliorated must have a minimum value that it can safely be reduced to, otherwise
    /// this function will return `None`.
    ///
    /// In cases where amelioration will only affect a part of the model's duration, a
    /// second `TransactionModel` is returned to represent the surplus duration. For
    /// example, say we have a `TransactionModel` that repeats once a week in perpetuity.
    /// On a given week, the actual transaction exceeds the model by 50%. We could then
    /// ameliorate this model over 2 weeks by 25% to cover the shortfall. This function
    /// would return the ameliorated transaction as well as a 'normal' transaction to
    /// represent the original recurring payment.
    pub fn ameliorate(
        &mut self,
        mut target: Decimal,
        mut start_date: Date<Utc>,
        mut end_date: Date<Utc>,
    ) -> Option<(
        Result<TransactionModel, TransactionError>,
        Option<Result<TransactionModel, TransactionError>>,
    )> {
        // Don't attempt to ameliorate a transaction that has no minimum value
        if !self.can_ameliorate() {
            return None;
        }

        // If the target value is lower than the minimum amount this transaction can be
        // reduced to, set the target to the minimum value.
        if Some(target) < self.min_value {
            target = self.min_value.unwrap();
        }

        // If the start date is less than the minimum `Contribution` for this transaction
        // then trim it.
        let self_start_date = self.get_start_date();
        if self_start_date.is_some() && Some(start_date) < self_start_date {
            start_date = self_start_date.unwrap();
        }

        // If the end date is greater than the maximum `Contribution` for this
        // transaction then trim it.
        let self_period_end = self.get_period_end(Some(end_date));
        if self_period_end.is_some() && Some(end_date) > self_period_end {
            end_date = self_period_end.unwrap();
        }

        // Create an ameliorated transaction
        let ameliorated = TransactionModel::new(
            target,
            self.min_value,
            self.frequency.clone(),
            start_date,
            Some(end_date),
            Some(start_date),
        );

        // Cache the actual end date to avoid multiple calls to fn
        let self_end_date = self.get_end_date();

        // If the amelioration end date is less than this transaction's end date, create
        // a new `TransactionModel` to represent the rest of the period.
        let restarted = if self_end_date.is_none() || Some(end_date) < self_end_date {
            let new_start = end_date.succ();
            Some(TransactionModel::new(
                self.value,
                self.min_value,
                self.frequency.clone(),
                new_start,
                self_end_date,
                Some(new_start),
            ))
        } else {
            None
        };

        // Curtail current transaction, ending the day before this transaction starts
        self.set_end_date(start_date.pred());

        Some((ameliorated, restarted))
    }

    fn get_start_date(&self) -> Option<Date<Utc>> {
        self.contributions
            .iter()
            .min_by_key(|c| c.get_start_date())
            .map(|c| c.get_start_date())
    }

    fn get_end_date(&self) -> Option<Date<Utc>> {
        self.contributions
            .iter()
            .max_by_key(|c| c.get_start_date())
            .and_then(|c| c.get_end_date())
    }

    fn set_end_date(&mut self, end_date: Date<Utc>) {
        // Delete any contributions that start after the end date
        self.contributions.retain(|c| c.get_start_date() < end_date);

        // Get any last payment for this contribution so we can calculate surplus
        // contributions from that date.
        // Note that `get_start_date().unwrap()` is safe where there are 1 or more
        // contributions.
        if !self.contributions.is_empty() {
            let last_payment = self
                .frequency
                .get_payment_dates(self.get_start_date().unwrap(), Some(end_date))
                .last()
                .map(|d| *d);

            // Find the contribution that overlaps the end date, then shorten it.
            // Note that as contributions for a model are sequential and non-overlapping, we
            // are guaranteed to only ever deal with one contribution that overlaps the new
            // end date.
            self.contributions
                .iter_mut()
                .find(|c| c.get_period_end(Some(end_date)) > end_date)
                .and_then(|c| Some(c.set_end_date(end_date, last_payment)));
        }
    }

    fn get_period_end(&self, date: Option<Date<Utc>>) -> Option<Date<Utc>> {
        self.contributions
            .iter()
            .max_by_key(|c| c.get_start_date())
            .map(|c| c.get_period_end(date))
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

    pub fn get_start_date(&self) -> Date<Utc> {
        self.inner().get_start_date()
    }

    pub fn get_period_end(&self, date: Option<Date<Utc>>) -> Date<Utc> {
        self.inner().get_period_end(date)
    }

    fn inner(&self) -> &Contribution {
        match self {
            ContributionSign::Positive(c) => c,
            ContributionSign::Negative(c) => c,
        }
    }
}

/// Calculate whether a collection of revenue, expense and savings [`TransactionModel`]s
/// are sustainable in perpetuity.
///
/// In other words, calculate whether our expense and savings [`TransactionModel`]s will
/// ever cause us to lose money over time.
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

    // Accumulate totals for each day we have contributions for
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
        AffordabilityResult::Deficit(deficit, surplus)
    } else if !surplus.is_empty() {
        AffordabilityResult::Surplus(surplus)
    } else {
        AffordabilityResult::Balanced
    }
}

fn acc_daily_contributions(contributions: &[ContributionSign]) -> HashMap<Date<Utc>, Decimal> {
    let mut day_totals = HashMap::new();

    trace!("provided contributions: {:?}", contributions);

    if let Some(min) = contributions.iter().min_by_key(|c| c.get_start_date()) {
        // If we have a min, we must have a max, so unwrapping is safe
        let max = contributions
            .iter()
            .max_by_key(|c| c.get_period_end(None))
            .unwrap();

        trace!(
            "contribution date range is from {} to {}",
            min.get_start_date(),
            max.get_period_end(None)
        );

        let mut date = min.get_start_date();
        while date <= max.get_period_end(None) {
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
            None,
            Frequency::Weekly(1, vec![start_date.weekday().number_from_monday()]),
            start_date,
            None,
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn transaction_ameliorate_no_min() {
        let start_date = Utc::today();
        let mut trans =
            TransactionModel::new(dec!(10), None, Frequency::Daily(2), start_date, None, None)
                .unwrap();
        assert!(trans.ameliorate(dec!(6), start_date, start_date).is_none());
    }

    #[test]
    fn transaction_ameliorate_oob() {
        let mut trans = TransactionModel::new(
            dec!(10),
            Some(dec!(5)),
            Frequency::Daily(2),
            Utc.ymd(2000, 4, 1),
            Some(Utc.ymd(2000, 4, 5)),
            Some(Utc.ymd(2000, 4, 1)),
        )
        .unwrap();
        let result = trans.ameliorate(dec!(2), Utc.ymd(2000, 3, 18), Utc.ymd(2000, 6, 1));
        let (t1, t2) = result.expect("Result should contain a tuple");
        let t1 = t1.expect("Failed to create TransactionModel");

        assert_eq!(trans.get_period_end(None), None);
        assert_eq!(t1.value, dec!(5));
        assert_eq!(t1.min_value, Some(dec!(5)));
        assert_eq!(t1.get_start_date(), Some(Utc.ymd(2000, 4, 1)));
        assert_eq!(t1.get_period_end(None), Some(Utc.ymd(2000, 4, 5)));
        assert!(t2.is_none());
    }

    #[test]
    fn transaction_ameliorate_no_restart() {
        let mut trans = TransactionModel::new(
            dec!(10),
            Some(dec!(5)),
            Frequency::Daily(2),
            Utc.ymd(2000, 4, 1),
            Some(Utc.ymd(2000, 6, 1)),
            Some(Utc.ymd(2000, 3, 15)),
        )
        .unwrap();
        let result = trans.ameliorate(dec!(6), Utc.ymd(2000, 5, 18), Utc.ymd(2000, 6, 1));
        let (t1, t2) = result.expect("Result should contain a tuple");
        let t1 = t1.expect("Failed to create TransactionModel");

        assert_eq!(trans.get_period_end(None), Some(Utc.ymd(2000, 5, 17)));
        assert_eq!(t1.value, dec!(6));
        assert_eq!(t1.min_value, Some(dec!(5)));
        assert_eq!(t1.get_start_date(), Some(Utc.ymd(2000, 5, 18)));
        assert_eq!(t1.get_period_end(None), Some(Utc.ymd(2000, 5, 30)));
        assert!(t2.is_none());
    }

    #[test]
    fn transaction_ameliorate_with_restart_fixed() {
        let mut trans = TransactionModel::new(
            dec!(10),
            Some(dec!(5)),
            Frequency::Daily(2),
            Utc.ymd(2000, 4, 1),
            Some(Utc.ymd(2000, 6, 1)),
            Some(Utc.ymd(2000, 3, 15)),
        )
        .unwrap();
        let result = trans.ameliorate(dec!(6), Utc.ymd(2000, 5, 1), Utc.ymd(2000, 5, 18));
        let (t1, t2) = result.expect("Result should contain a tuple");
        let t1 = t1.expect("Failed to create TransactionModel");
        let t2 = t2
            .expect("Tuple should contain 2 TransactionModels")
            .expect("Failed to create TransactionModel");

        assert_eq!(trans.get_period_end(None), Some(Utc.ymd(2000, 4, 30)));
        assert_eq!(t1.value, dec!(6));
        assert_eq!(t1.min_value, Some(dec!(5)));
        assert_eq!(t1.get_start_date(), Some(Utc.ymd(2000, 5, 1)));
        assert_eq!(t1.get_period_end(None), Some(Utc.ymd(2000, 5, 17)));
        assert_eq!(t2.value, dec!(10));
        assert_eq!(t2.min_value, Some(dec!(5)));
        assert_eq!(t2.get_start_date(), Some(Utc.ymd(2000, 5, 19)));
        assert_eq!(t2.get_period_end(None), Some(Utc.ymd(2000, 5, 31)));
    }

    #[test]
    fn transaction_ameliorate_with_restart_infinite() {
        let mut trans = TransactionModel::new(
            dec!(10),
            Some(dec!(5)),
            Frequency::Daily(2),
            Utc.ymd(2000, 4, 1),
            None,
            Some(Utc.ymd(2000, 3, 15)),
        )
        .unwrap();
        let result = trans.ameliorate(dec!(6), Utc.ymd(2000, 5, 1), Utc.ymd(2000, 5, 18));
        let (t1, t2) = result.expect("Result should contain a tuple");
        let t1 = t1.expect("Failed to create TransactionModel");
        let t2 = t2
            .expect("Tuple should contain 2 TransactionModels")
            .expect("Failed to create TransactionModel");

        assert_eq!(trans.get_period_end(None), Some(Utc.ymd(2000, 4, 30)));
        assert_eq!(t1.value, dec!(6));
        assert_eq!(t1.min_value, Some(dec!(5)));
        assert_eq!(t1.get_start_date(), Some(Utc.ymd(2000, 5, 1)));
        assert_eq!(t1.get_period_end(None), Some(Utc.ymd(2000, 5, 17)));
        assert_eq!(t2.value, dec!(10));
        assert_eq!(t2.min_value, Some(dec!(5)));
        assert_eq!(t2.get_start_date(), Some(Utc.ymd(2000, 5, 19)));
        assert_eq!(
            t2.get_period_end(Some(Utc.ymd(2000, 6, 20))),
            Some(Utc.ymd(2000, 6, 20))
        );
    }

    #[test]
    fn is_affordable_balanced() {
        let today = Utc.ymd(2000, 4, 1);
        let start = Utc.ymd(2000, 4, 7);

        let revenues = vec![TransactionModel::new(
            dec!(14),
            None,
            Frequency::Weekly(1, vec![4]),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let expenses = vec![TransactionModel::new(
            dec!(1),
            None,
            Frequency::Daily(1),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let savings = vec![TransactionModel::new(
            dec!(1),
            None,
            Frequency::Daily(1),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        assert_eq!(
            is_affordable(Some(&revenues), Some(&expenses), Some(&savings)),
            AffordabilityResult::Balanced
        );
    }

    #[test]
    fn is_affordable_deficit() {
        let today = Utc.ymd(2000, 4, 1);
        let start = Utc.ymd(2000, 4, 7);

        let revenues = vec![TransactionModel::new(
            dec!(14),
            None,
            Frequency::Weekly(1, vec![4]),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let expenses = vec![TransactionModel::new(
            dec!(1),
            None,
            Frequency::Daily(1),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let savings = vec![
            TransactionModel::new(dec!(1), None, Frequency::Daily(1), start, None, Some(today))
                .unwrap(),
            TransactionModel::new(dec!(2), None, Frequency::Once, start, None, Some(start))
                .unwrap(),
        ];

        assert_eq!(
            is_affordable(Some(&revenues), Some(&expenses), Some(&savings)),
            AffordabilityResult::Deficit(vec![start], Vec::new())
        );
    }

    #[test]
    fn is_affordable_surplus() {
        let today = Utc.ymd(2000, 4, 1);
        let start = Utc.ymd(2000, 4, 7);

        let revenues = vec![TransactionModel::new(
            dec!(14),
            None,
            Frequency::Weekly(1, vec![4]),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        let expenses = vec![TransactionModel::new(
            dec!(1),
            None,
            Frequency::Daily(1),
            start.succ(),
            None,
            Some(today),
        )
        .unwrap()];

        let savings = vec![TransactionModel::new(
            dec!(1),
            None,
            Frequency::Daily(1),
            start,
            None,
            Some(today),
        )
        .unwrap()];

        assert_eq!(
            is_affordable(Some(&revenues), Some(&expenses), Some(&savings)),
            AffordabilityResult::Surplus(vec![start])
        );
    }
}
