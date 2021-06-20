use super::{CurrencyValue, Frequency};
use chrono::{Date, Duration, Utc};
use thiserror::Error;

/// The daily amount to contribute to an upcoming payment
#[derive(Debug)]
pub struct Contribution {
    regular: f64,
    last: Option<f64>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
}

#[derive(Error, Debug, Eq, PartialEq)]
pub enum ContributionError {
    #[error("the start date occurs in the past")]
    HistoricalStartDate,
}

impl Contribution {
    /// Create a new `Contribution`
    pub fn new(
        regular: f64,
        last: Option<f64>,
        start_date: Date<Utc>,
        end_date: Option<Date<Utc>>,
    ) -> Self {
        Contribution {
            regular,
            last,
            start_date,
            end_date,
        }
    }

    /// Returns whether this `Contribution` has expired.
    /// If `end_date` is not set, this `Contribution` will never expire.
    pub fn has_expired(&self) -> bool {
        match self.end_date {
            Some(date) => date < Utc::today(),
            None => false,
        }
    }
}

/// Returns a tuple of the regular contribution and any required onboarding amelioration.
/// For example, if I have a new weekly payment schedule which starts in 3 days, but the
/// first payment is tomorrow, I need an 'onboarding' amelioration to cover the payments
/// we haven't been saving for.
pub(super) fn calculate(
    value: CurrencyValue,
    frequency: Frequency,
    mut start_date: Date<Utc>,
    mut end_date: Option<Date<Utc>>,
    mut contributions: Vec<Contribution>,
    now: Option<Date<Utc>>, // This allows overriding the current time for testing
) -> Result<Vec<Contribution>, ContributionError> {
    let now = now.unwrap_or(Utc::today());

    // We don't allow contributions that start in the past
    if now >= start_date {
        return Err(ContributionError::HistoricalStartDate);
    }

    // Get payment dates from `Frequency` for start and end dates
    let payments = frequency.get_payment_dates(start_date, end_date);

    // Get period length
    let length = if let Frequency::Once = frequency {
        end_date = Some(start_date);
        start_date = now;
        *end_date.as_ref().unwrap() - start_date
    } else if let Some(end) = end_date.as_mut() {
        // Adjust start and end dates to maximise time available
        start_date = now;
        *end = *payments.last().unwrap();

        let mut length = *end - start_date;

        // @todo Adjust start date (per "Contribution start date algorithm" in Notes)

        length
    } else {
        frequency.get_period_length()
    };

    // Calculate contribution amounts
    let (regular, last) = calculate_for_duration(value, length);

    // Add this `Contribution` to the chain
    contributions.push(Contribution {
        regular: regular.into(),
        last: last.map(|l| l.into()),
        start_date,
        end_date,
    });

    // @todo For amelioration:
    // ---
    // IF there is an end date:
    //      Calculate period length from now => start date
    // ELSE:
    //      Calculate period length from original start date => start_date
    // Is period length > 0
    //      Loop over fn until start date - now == 0

    Ok(contributions)

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

/// Adjust the start and end dates to appropriate values for the given `Frequency`.
///
/// In order to make the period even, we may have to shift the start date forward.
/// For example, say we have payments of $1 with a 3 day frequency. Thus we have
/// payments on the first day and the fourth day, and our daily contribution is
/// $0.50. If our start date lands on day 1, we will have only contributed $0.50
/// towards that expense, where we need $1. Therefore, we need to shift our start
/// date to day 2 in order that we can balance our contributions and payments.
///
/// We may also need to adjust the end date where that end date falls after the last
/// payment. Otherwise when calculating daily contributions, we would include days
/// that don't contribute to any payment, resulting in a shortfall on the last n
/// payments.
pub fn align_dates(start: &mut Date<Utc>, end: Option<&mut Date<Utc>>) {}

// Calculate the contribution amounts for a value and duration
fn calculate_for_duration(
    value: CurrencyValue,
    duration: Duration,
) -> (CurrencyValue, Option<CurrencyValue>) {
    // PROBLEM - If a payment is set for a sufficiently long time away, or for a very small amount, our contributions
    // may end up being rounded down to zero, i.e. $0.001 => $0.00 rounded.
    // SOLUTION - Detect zero contributions and adjust start date until contribution > 0

    // Calculate the regular contribution
    // Note that non-floating point integers automatically round down, which
    // could lead to exaggerated final payments. Thus we convert to a float,
    // then round manually.
    let regular = (*value as f64 / duration.num_days() as f64).round() as i64;

    // Given the rounding of the regular contribution, we may need a
    // different final contribution to handle the rounding error.
    let last = if regular * duration.num_days() != *value {
        Some(CurrencyValue(*value - regular * (duration.num_days() - 1)))
    } else {
        None
    };

    // The regular and last payments must equal the value, or our maths is
    // fundamentally broken!
    match last {
        Some(ref l) => assert_eq!(*value, regular * (duration.num_days() - 1) + **l),
        None => assert_eq!(*value, regular * duration.num_days()),
    }

    (CurrencyValue(regular), last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use std::convert::TryInto;

    #[test]
    fn calculate_for_duration_exact() {
        let (regular, last) = calculate_for_duration(9f64.try_into().unwrap(), Duration::days(4));

        assert_eq!(*regular, 225);
        assert_eq!(last, None);
    }

    #[test]
    fn calculate_for_duration_rounddown() {
        let (regular, last) =
            calculate_for_duration(9.57f64.try_into().unwrap(), Duration::days(4));

        assert_eq!(*regular, 239);
        assert_eq!(last.map(|l| *l), Some(240));
    }

    #[test]
    fn calculate_for_duration_roundup() {
        let (regular, last) =
            calculate_for_duration(11.1f64.try_into().unwrap(), Duration::days(4));

        assert_eq!(*regular, 278);
        assert_eq!(last.map(|l| *l), Some(276));
    }

    #[test]
    fn calculate_contribution_historical() {
        let result = calculate(
            1f64.try_into().unwrap(),
            Frequency::Once,
            Utc.ymd(2000, 4, 1),
            None,
            Vec::new(),
            Some(Utc.ymd(2000, 4, 2)),
        );

        assert_eq!(result.err(), Some(ContributionError::HistoricalStartDate));
    }

    #[test]
    fn contribution_has_expired() {
        let c = Contribution::new(1.0, None, Utc::today(), Some(Utc::today().pred()));
        assert!(c.has_expired());
    }

    #[test]
    fn contribution_has_not_expired() {
        let c = Contribution::new(1.0, None, Utc::today(), Some(Utc::today()));
        assert!(!c.has_expired());
    }

    #[test]
    fn contribution_no_expiry() {
        let c = Contribution::new(1.0, None, Utc::today(), None);
        assert!(!c.has_expired());
    }
}
