use super::{CurrencyValue, TransactionFrequency};
use chrono::{Date, Utc};
use thiserror::Error;

/// The daily amount to contribute to an upcoming payment
#[derive(Debug)]
pub struct Contribution {
    regular: f64,
    last: Option<f64>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
}

#[derive(Error, Debug)]
pub enum ContributionError {
    #[error("the start date occurs in the past")]
    HistoricalStartDate,
}

impl Contribution {
    pub(super) fn calculate(
        value: CurrencyValue,
        frequency: TransactionFrequency,
        start_date: Date<Utc>,
        end_date: Option<Date<Utc>>,
        now: Option<Date<Utc>>, // This arg is for testing
    ) -> Result<Self, ContributionError> {
        // Every n days
        // Every n weeks on x days (Monday = 1, Sunday = 7)
        // Every n months each date
        // Every n months, on the nth (First = 1, Fifth = 5, Last = 0) day
        // Every n years in x months (January = 1, December = 12)[ on the nth (First = 1, Fifth = 5, Last = 0) day]
        let now = now.unwrap_or(Utc::today());

        // We don't allow contributions that start in the past
        if now >= start_date {
            return Err(ContributionError::HistoricalStartDate);
        }

        match frequency {
            TransactionFrequency::Once => {
                // Determine the difference between now and the start date
                let duration = start_date - now;

                // Calculate the regular contribution
                // Note that non-floating point integers automatically round down, which
                // could lead to exaggerated final payments. Thus we convert to a float,
                // then round manually.
                let regular = (*value as f64 / duration.num_days() as f64).round() as i64;

                let last = if regular * duration.num_days() != *value {
                    Some(*value - regular * (duration.num_days() - 1))
                } else {
                    None
                };

                return Ok(Self {
                    regular: CurrencyValue(regular).into(),
                    last: last.map(|l| CurrencyValue(l).into()),
                    start_date,
                    end_date,
                });
            }
            _ => unimplemented!(),
        }

        // Is it a once off?
        // Is there an end date?
        // Determine the period length
        // Is there an uneven period?
        //      Determine shortest macro period
        // Determine start date
        // How many payments are between now and start date?
        //      Calculate as for end date

        // --------------------------------------
        // Thoughts:
        //
        // PROBLEM - Some frequencies mean that certain periods are skipped (e.g. "30th of each month" skips February).
        // ? SOLUTION - When calculating monthly expenses/affordability, take frequency into account as well as contributions.
        //   PROBLEM - What is a standard period (as above example)?
        //
        // PROBLEM - Standard contributions assume that payments are able to be covered prior to period starting.
        // ? SOLUTION - Spend previous period saving for current period. This might be suboptimal where period are protracted and
        //              include ample time to save for payments during period.
        // SOLUTION - Calculate even contributions for period, then modify the start date so that all payments are covered by the time the
        //            last payment is made. Any initial shortfall is made up by an amelioration.
        //
        // PROBLEM - this model assumes we can save contributions after the last payment is made. For example, a contribution
        //               every week on Mon and Thurs would assume that Fri, Sat and Sun are contributing towards the Mon and Thurs.
        //               We can't have contributions for past payments!
        // SOLUTION - Calculate from end of last period to beginning of next, e.g. for a weekly payment on Monday and Saturday, our
        //            period starts on Sunday and ends on Saturday.
        //
        // - if end date, then calculate number of payments between now and end, multiply by value, divide by number of days = std contribution
        // - if no end,
        //   INITIAL CONTRIBUTION
        //   - if now < 1 period away (e.g. bi-monthly recurring expense, with the first payment in 10 days),
        //     then calculate number of days until first expense = initial contribution
        //   - if now > 1 period away, then set start date to exactly 1 period before first expense
        //
        //   STD CONTRIBUTION
        //   - for each period (e.g. every 3 days = 3 day period), calculate number of payments, calculate days in period, divide total payments by days in period = std contribution,
        //     then set start date of std contribution to day after end of previous period. If date is prior to today (tomorrow?) then set an amelioration to make up shortfall.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::convert::TryInto;

    #[test]
    fn calculate_contribution_once_exact() {
        let contribution = Contribution::calculate(
            9f64.try_into().unwrap(),
            TransactionFrequency::Once,
            Utc.ymd(2000, 4, 5),
            None,
            Some(Utc.ymd(2000, 4, 1)),
        )
        .unwrap();

        assert_eq!(contribution.regular, 2.25);
        assert_eq!(contribution.last, None);
    }

    #[test]
    fn calculate_contribution_once_rounddown() {
        let contribution = Contribution::calculate(
            9.57f64.try_into().unwrap(),
            TransactionFrequency::Once,
            Utc.ymd(2000, 4, 5),
            None,
            Some(Utc.ymd(2000, 4, 1)),
        )
        .unwrap();

        assert_eq!(contribution.regular, 2.39);
        assert_eq!(contribution.last, Some(2.4));
    }

    #[test]
    fn calculate_contribution_once_roundup() {
        let contribution = Contribution::calculate(
            11.1f64.try_into().unwrap(),
            TransactionFrequency::Once,
            Utc.ymd(2000, 4, 5),
            None,
            Some(Utc.ymd(2000, 4, 1)),
        )
        .unwrap();

        assert_eq!(contribution.regular, 2.78);
        assert_eq!(contribution.last, Some(2.76));
    }
}
