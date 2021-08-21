use crate::frequency::Frequency;
use chrono::{Date, Duration, Utc};
use log::{debug, error, trace};
use rust_decimal::Decimal;
use thiserror::Error;

/// The daily amount to contribute to an upcoming payment
#[derive(Debug, PartialEq)]
pub struct Contribution {
    regular: Decimal,
    last: Option<Decimal>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
    period_length: Duration,
}

#[derive(Error, Debug, Eq, PartialEq)]
pub enum ContributionError {
    #[error("the start date occurs in the past")]
    HistoricalStartDate,
    #[error("there are no payments for this contribution")]
    NoPayments,
    #[error("the date '{2}' is beyond the range {0} - {1}")]
    PaymentOutOfBounds(Date<Utc>, Date<Utc>, Date<Utc>), // start, end, oob
    #[error("contribution is approaching zero")]
    ApproachingZero,
    #[error("could not resolve contribution to cover all payments")]
    Unresolvable,
    #[error("too much recursion trying to find a sustainable contribution")]
    TooMuchRecursion,
}

impl Contribution {
    /// Returns whether this `Contribution` has expired.
    /// If `end_date` is not set, this `Contribution` will never expire.
    // pub fn has_expired(&self) -> bool {
    //     match self.end_date {
    //         Some(date) => date < Utc::today(),
    //         None => false,
    //     }
    // }

    pub fn regular_or_last(&self, date: Date<Utc>) -> Option<Decimal> {
        let period_end = self.period_end(Some(date));
        if date >= self.start_date {
            if date == period_end {
                return self.last.or(Some(self.regular));
            } else if date < period_end {
                return Some(self.regular);
            }
        }
        None
    }

    pub fn start_date(&self) -> Date<Utc> {
        self.start_date
    }

    pub fn period_end(&self, date: Option<Date<Utc>>) -> Date<Utc> {
        let date = date.unwrap_or(self.start_date);
        self.end_date.unwrap_or_else(|| {
            // If there's no end date, roll forward to the next period end for `date`.
            // We do this by finding the nearest multiple of `period_length`.
            let diff = date - self.start_date + Duration::days(1);
            let rem = Duration::days(diff.num_days() % self.period_length.num_days());

            // If the given `date` is exactly divisible by our period length, we are
            // already at the end of the period, so don't attempt to roll forward.
            if rem.is_zero() {
                date
            } else {
                date + self.period_length - rem
            }
        })
    }
}

/// Returns a tuple of the regular contribution and any required onboarding amelioration.
/// For example, if I have a new weekly payment schedule which starts in 3 days, but the
/// first payment is tomorrow, I need an 'onboarding' amelioration to cover the payments
/// we haven't been saving for.
pub(super) fn calculate(
    value: Decimal,
    frequency: &Frequency,
    mut start_date: Date<Utc>,
    mut end_date: Option<Date<Utc>>,
    // Fixing the current date allows us to recalculate the contribution repeatably
    now: Date<Utc>,
) -> Result<Vec<Contribution>, ContributionError> {
    // We don't allow contributions that start in the past
    if now > start_date {
        return Err(ContributionError::HistoricalStartDate);
    }

    // Get payment dates from `Frequency` for start and end dates
    let mut payments = frequency.get_payment_dates(start_date, end_date);

    // If we have a fixed period, adjust start and end dates to maximise time available
    // for contributions. This approach doesn't work for repeating payments where there
    // is no end date; if our periods are not even, we will accumulate a surplus when we
    // actually need to break even.
    if let Frequency::Once = frequency {
        end_date = Some(start_date);
        start_date = now;
    } else if let Some(end) = end_date.as_mut() {
        start_date = now;
        *end = *payments.last().unwrap();
    }

    let mut contributions = Vec::new();

    // Recurse over `naive_contribution` until we have contributions for every payment
    while !payments.is_empty() {
        // This sucks but we've got to create a clone of payments here.
        // `naive_contribution` mutates this vector, which will make it impossible to
        // track payments earlier than the start date, which can be incremented for fit.
        let payments_c = payments.clone();

        // Create a new Contribution from naive dates
        let contribution =
            naive_contribution(value, frequency, payments_c, start_date, end_date, None)?;

        // Remove all payments covered by above contribution
        payments.retain(|p| *p < contribution.start_date);

        // Set start_date to `now`. This allows us to make the most of any lead time in
        // contributing to the initial payment/s.
        start_date = now;

        // Set end_date to the day before the current start date. This allows us to
        // measure the difference between the user-defined start date and the adjusted
        // start date. If there is a difference, we should setup a new contribution.
        end_date = Some(contribution.start_date.pred());

        // Insert this contribution at the beginning of the vector. Each successive loop
        // operates on an earlier contribution, so this keeps the vector ordered
        // correctly.
        contributions.insert(0, contribution);
    }

    Ok(contributions)
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
fn naive_contribution(
    payment: Decimal,
    frequency: &Frequency,
    mut payment_dates: Vec<Date<Utc>>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
    mut final_date: Option<Date<Utc>>,
) -> Result<Contribution, ContributionError> {
    // If the start date goes beyond the final date, abort to prevent infinite loop
    if let Some(f_date) = final_date {
        if start_date > f_date {
            return Err(ContributionError::TooMuchRecursion);
        }
    }

    debug!("evaluating contribution for {}", frequency);

    // If there aren't any payments, there shouldn't be a contribution
    if payment_dates.is_empty() {
        error!("no payment dates provided for this contribution");
        return Err(ContributionError::NoPayments);
    }

    // Setup a fixed end date that is either the user defined date, or the last day of
    // the period. Note we need to subtract 1 day as we want the *last* day of the period
    // rather than the first day of the next period.
    let period_end =
        end_date.unwrap_or(start_date + frequency.get_period_length() - Duration::days(1));

    // Calculate period length, which is not necessarily the frequency length if an end
    // date is provided.
    // We increment by 1 day as we want to include the start date in our contributions.
    let period_length = period_end - start_date + Duration::days(1);

    debug!(
        "assuming contribution starts on {} and {}",
        start_date,
        match end_date {
            Some(e) => format!("ends on {}", e),
            None => format!("the period ends on {}", period_end),
        }
    );

    debug!("assuming payments on these dates: {:?}", payment_dates);

    // Assign final date to prevent infinite looping. This date represents the end of the
    // unmodified contribution period. Going beyond this date means that we have tested
    // every date in the contribution and can't find a sustainable solution. Further
    // recursion would be a waste of time and very likely result in infinite looping.
    if final_date.is_none() {
        debug!("setting recursion limit date of {}", period_end);
        final_date = Some(period_end);
    }

    // For these algorithms to work, we need an ordered list
    payment_dates.sort_unstable();

    // Make sure we never process payments beyond the start and end bounds
    if payment_dates.first() < Some(&start_date) {
        error!("the first payment occurs before the start date");

        return Err(ContributionError::PaymentOutOfBounds(
            start_date,
            period_end,
            *payment_dates.first().unwrap(),
        ));
    } else if let Some(end) = end_date {
        if payment_dates.last() > Some(&end) {
            error!("the last payment occurs after the end date");

            return Err(ContributionError::PaymentOutOfBounds(
                start_date,
                end,
                *payment_dates.last().unwrap(),
            ));
        }
    }

    // If every day in period has a payment, this contribution is valid
    if payment_dates.len() as i64 == period_length.num_days() {
        let c = Contribution {
            regular: payment,
            last: None,
            start_date,
            end_date,
            period_length,
        };

        debug!(
            "every day in this period has a payment - finalise contribution: {:?}",
            c
        );

        return Ok(c);
    }

    // Calculate contribution amounts
    let (regular, last) =
        calculate_for_duration(payment, payment_dates.len() as u64, period_length)?;

    debug!(
        "assuming the contribution has a regular value of ${}{}",
        regular,
        match last {
            Some(l) => format!(" and a final value of ${}", l),
            None => String::new(),
        }
    );

    // If there is no payment on the last day, we will end up accumulating contributions
    // after the last payment. This means that we won't actually cover the last payment
    // until after it's been made.
    if *payment_dates.last().unwrap() < period_end {
        // If an end date exists, shift it forward to the last payment
        if end_date.is_some() {
            let last = *payment_dates.last().unwrap();

            debug!(
                "there must be a payment on the last day - shift end date to the final payment date: {}",
                last
            );

            return naive_contribution(
                payment,
                frequency,
                payment_dates,
                start_date,
                Some(last),
                final_date,
            );
        }
        // If no end date exists, rotate the period to the second payment
        else {
            let first = payment_dates.remove(0);
            payment_dates.push(first + period_length);

            debug!(
                "there must be a payment on the last day - move first payment ({}) to the end ({})",
                first,
                first + period_length
            );

            return naive_contribution(
                payment,
                frequency,
                payment_dates,
                first.succ(),
                None,
                final_date,
            );
        }
    }

    // If there is a payment on the start day, the contribution must equal the payment.
    // Otherwise we won't be able to cover today's payment! However as not every day in
    // the period has a payment, we will end up with a surplus. Thus we need to move the
    // first payment to the end of the period.
    if payment_dates.first().unwrap() == &start_date {
        debug!("there must not be a payment on the first day - skip today for this contribution");

        let first = payment_dates.remove(0);

        // If there is no end date, we have an infinitely recurring period. This means
        // that the first payment will now become the last payment. Hence we need to move
        // it!
        if end_date.is_none() {
            payment_dates.push(first + period_length);
        }

        return naive_contribution(
            payment,
            frequency,
            payment_dates,
            start_date.succ(),
            end_date,
            final_date,
        );
    }

    // Ensure that the duration between each payment is sufficient for accumulated
    // contributions to cover it.
    // XXX Hack rounding to 23 decimal places - only solves rounding error for narrow test cases
    let min_length = ((payment - last.unwrap_or(regular)) / regular + Decimal::ONE).round_dp(23);
    // Note we set lag_date to the day before start date, so that the duration
    // calculations will be correct. Every subsequent calculation is from payment date to
    // payment date, i.e. from the last day of the previous payment to the last day of
    // the next. If we set lag_date to start_date, we are erroneously reducing the
    // duration by 1 day.
    let mut lag_date = start_date.pred();
    let mut first_positive_date = None;
    let mut cumulative_delta = Decimal::ZERO;
    for date in payment_dates.iter() {
        // Calculate number of days between payments
        let duration = *date - lag_date;
        let days = Decimal::from(duration.num_days());

        // Calculate contribution length delta
        let delta = days - min_length;

        trace!(
            "contribution delta from {} to {}: {} - {} = {} (cumulative: {})",
            lag_date,
            date,
            days,
            min_length,
            delta,
            cumulative_delta + delta
        );

        // Cache first positive date if we need to wind forward
        if delta <= Decimal::ZERO {
            first_positive_date = None;
        } else if first_positive_date.is_none() {
            // Use `lag_date` here as we measure at the *end* of each payment
            first_positive_date = Some(lag_date);
        }

        // If the cumulative delta turns from negative to positive, the lag date is a
        // good candidate for a sustainable contribution.
        if cumulative_delta < Decimal::ZERO && cumulative_delta + delta >= Decimal::ZERO {
            break;
        }

        // Update tracking vars
        lag_date = *date;
        cumulative_delta += delta;
    }

    // Handle a suggested new start date
    if let Some(date) = first_positive_date {
        debug!(
            "the contribution does not cover all payments; choosing next viable start date: {}",
            date.succ()
        );

        // Shift prior payment dates
        while payment_dates.first() <= Some(&date) {
            let first = payment_dates.remove(0);
            // Only append date if we have an infinite recursion
            if end_date.is_none() {
                payment_dates.push(first + period_length);
            }
        }

        return naive_contribution(
            payment,
            frequency,
            payment_dates,
            date.succ(), // Shift forward 1 day as this date is for a payment
            end_date,
            final_date,
        );
    }
    // If our contribution ends with a negative delta and there's no suggested start
    // date, the last thing we can try is reducing the number of payments until we reach
    // a sustainable point...or we run out of payments.
    else if cumulative_delta < Decimal::ZERO {
        if !payment_dates.is_empty() {
            let first = payment_dates.remove(0);

            debug!(
                "the contribution does not cover all payments; remove the first payment: {}",
                first
            );

            return naive_contribution(
                payment,
                frequency,
                payment_dates,
                first.succ(), // Shift forward 1 day as this date is for a payment
                end_date,
                final_date,
            );
        } else {
            return Err(ContributionError::Unresolvable);
        }
    }

    let contribution = Contribution {
        regular,
        last,
        start_date,
        end_date,
        period_length,
    };
    debug!("finalising contribution: {:?}", contribution);

    Ok(contribution)
}

// Calculate the contribution amounts for a value and duration
fn calculate_for_duration(
    payment: Decimal,
    num_payments: u64,
    duration: Duration,
) -> Result<(Decimal, Option<Decimal>), ContributionError> {
    // Calculate total payments
    let num_pay_dec = Decimal::from(num_payments);
    let total = payment * num_pay_dec;

    // Number of days in contribution
    let days = Decimal::from(duration.num_days());

    trace!(
        "calculating contribution for {} payments of {} (total={}) over {} days",
        num_pay_dec,
        payment,
        total,
        days
    );

    // Calculate the regular contribution
    let regular = total / days;

    // Given the potential rounding of the regular contribution, we may need a separate
    // final contribution to handle the rounding error.
    let last = if regular * days != total {
        Some(total - regular * (days - Decimal::ONE))
    } else {
        None
    };

    // Don't attempt to process contribution amounts that are so small we can't represent
    // them.
    if regular == Decimal::ZERO || last == Some(Decimal::ZERO) {
        error!(
            "The {} contribution amount equals zero",
            if regular == Decimal::ZERO {
                "regular"
            } else {
                "last"
            }
        );
        return Err(ContributionError::ApproachingZero);
    }

    // The regular and last payments must equal the value, or our maths is
    // fundamentally broken!
    match last {
        Some(ref l) => assert_eq!(total, regular * (days - Decimal::ONE) + l),
        None => assert_eq!(total, regular * days),
    }

    Ok((regular, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use rust_decimal_macros::dec;

    #[test]
    fn calculate_for_duration_exact() {
        let (regular, last) = calculate_for_duration(dec!(4.5), 2, Duration::days(4)).unwrap();

        assert_eq!(regular, dec!(2.25));
        assert_eq!(last, None);
    }

    #[test]
    fn calculate_for_duration_rounding() {
        let (regular, last) = calculate_for_duration(dec!(0.01), 1, Duration::days(365)).unwrap();

        assert_eq!(regular, dec!(0.000027397260273972602739726));
        assert_eq!(last, Some(dec!(0.000027397260273972602739736)));
    }

    // #[test]
    // fn contribution_has_expired() {
    //     let c = Contribution {
    //         regular: 1.0,
    //         last: None,
    //         start_date: Utc::today(),
    //         end_date: Some(Utc::today().pred()),
    //     };
    //     assert!(c.has_expired());
    // }

    // #[test]
    // fn contribution_has_not_expired() {
    //     let c = Contribution {
    //         regular: 1.0,
    //         last: None,
    //         start_date: Utc::today(),
    //         end_date: Some(Utc::today()),
    //     };
    //     assert!(!c.has_expired());
    // }

    // #[test]
    // fn contribution_no_expiry() {
    //     let c = Contribution {
    //         regular: 1.0,
    //         last: None,
    //         start_date: Utc::today(),
    //         end_date: None,
    //     };
    //     assert!(!c.has_expired());
    // }

    #[test]
    fn contribution_regular_or_last_regular() {
        let c = Contribution {
            regular: dec!(1),
            last: None,
            start_date: Utc.ymd(2000, 4, 1),
            end_date: None,
            period_length: Duration::days(2),
        };
        assert_eq!(c.regular_or_last(Utc.ymd(2000, 4, 2)), Some(dec!(1)));
    }

    #[test]
    fn contribution_regular_or_last_last() {
        let c = Contribution {
            regular: dec!(1),
            last: Some(dec!(2)),
            start_date: Utc.ymd(2000, 4, 1),
            end_date: None,
            period_length: Duration::days(2),
        };
        assert_eq!(c.regular_or_last(Utc.ymd(2000, 4, 4)), Some(dec!(2)));
    }

    #[test]
    fn contribution_regular_or_last_before() {
        let c = Contribution {
            regular: dec!(1),
            last: None,
            start_date: Utc.ymd(2000, 4, 1),
            end_date: None,
            period_length: Duration::days(1),
        };
        assert!(c.regular_or_last(Utc.ymd(2000, 3, 31)).is_none());
    }

    #[test]
    fn contribution_regular_or_last_after() {
        let c = Contribution {
            regular: dec!(1),
            last: None,
            start_date: Utc.ymd(2000, 4, 1),
            end_date: Some(Utc.ymd(2000, 4, 3)),
            period_length: Duration::days(2),
        };
        assert!(c.regular_or_last(Utc.ymd(2000, 4, 4)).is_none());
    }

    #[test]
    fn contribution_period_end_no_date() {
        let c = Contribution {
            regular: dec!(1),
            last: None,
            start_date: Utc.ymd(2000, 4, 1),
            end_date: None,
            period_length: Duration::days(2),
        };
        assert_eq!(c.period_end(None), Utc.ymd(2000, 4, 2));
    }

    #[test]
    fn contribution_period_end_with_end_date() {
        let c = Contribution {
            regular: dec!(1),
            last: None,
            start_date: Utc.ymd(2000, 4, 1),
            end_date: Some(Utc.ymd(2000, 4, 2)),
            period_length: Duration::days(2),
        };
        assert_eq!(c.period_end(Some(Utc.ymd(2000, 4, 2))), Utc.ymd(2000, 4, 2));
    }

    #[test]
    fn contribution_period_end_next_week() {
        let c = Contribution {
            regular: dec!(1),
            last: None,
            start_date: Utc.ymd(2000, 4, 1),
            end_date: None,
            period_length: Duration::days(2),
        };
        assert_eq!(c.period_end(Some(Utc.ymd(2000, 4, 7))), Utc.ymd(2000, 4, 8));
    }

    #[test]
    fn naive_contribution_start_oob() {
        let start = Utc.ymd(2000, 1, 2);
        let payment = Utc.ymd(2000, 1, 1);
        let payments = vec![payment];
        let contribution =
            naive_contribution(Decimal::ONE, &Frequency::Once, payments, start, None, None);
        assert_eq!(
            contribution.err(),
            Some(ContributionError::PaymentOutOfBounds(
                start,
                Utc.ymd(2000, 1, 2),
                payment
            ))
        );
    }

    #[test]
    fn naive_contribution_end_oob() {
        let start = Utc.ymd(2000, 1, 2);
        let end = Utc.ymd(2000, 1, 3);
        let payment = Utc.ymd(2000, 1, 4);
        let payments = vec![payment];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Once,
            payments,
            start,
            Some(end),
            None,
        );
        assert_eq!(
            contribution.err(),
            Some(ContributionError::PaymentOutOfBounds(start, end, payment))
        );
    }

    #[test]
    fn naive_contribution_empty_payments() {
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Once,
            Vec::new(),
            Utc.ymd(2000, 1, 2),
            None,
            None,
        );
        assert_eq!(contribution.err(), Some(ContributionError::NoPayments));
    }

    #[test]
    fn naive_contribution_daily() {
        let start = Utc.ymd(2000, 1, 1);
        let end = Utc.ymd(2000, 1, 5);
        let payments = vec![
            start,
            Utc.ymd(2000, 1, 2),
            Utc.ymd(2000, 1, 3),
            Utc.ymd(2000, 1, 4),
            end,
        ];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Daily(1),
            payments,
            start,
            Some(end),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: Decimal::ONE,
                last: None,
                start_date: start,
                end_date: Some(end),
                period_length: Duration::days(5)
            })
        );
    }

    #[test]
    fn naive_contribution_start_with_end() {
        let start = Utc.ymd(2000, 1, 1);
        let end = Utc.ymd(2000, 1, 3);
        let payments = vec![start, end];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Daily(2),
            payments,
            start,
            Some(end),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.5),
                last: None,
                start_date: start.succ(),
                end_date: Some(end),
                period_length: Duration::days(2)
            })
        );
    }

    #[test]
    fn naive_contribution_start_no_end() {
        let start = Utc.ymd(2000, 1, 1);
        let payments = vec![start];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Daily(2),
            payments,
            start,
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.5),
                last: None,
                start_date: start.succ(),
                end_date: None,
                period_length: Duration::days(2)
            })
        );
    }

    #[test]
    fn naive_contribution_end_with_date() {
        let start = Utc.ymd(2000, 1, 1);
        let end = Utc.ymd(2000, 1, 3);
        let pay_end = Utc.ymd(2000, 1, 2);
        let payments = vec![start, pay_end];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Daily(1),
            payments,
            start,
            Some(end),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: Decimal::ONE,
                last: None,
                start_date: start,
                end_date: Some(pay_end),
                period_length: Duration::days(2)
            })
        );
    }

    #[test]
    fn naive_contribution_end_no_date() {
        let start = Utc.ymd(2000, 1, 3);
        let payments = vec![
            Utc.ymd(2000, 1, 5),
            Utc.ymd(2000, 1, 6),
            Utc.ymd(2000, 1, 7),
        ];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![3, 4, 5]),
            payments,
            start,
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.4285714285714285714285714286),
                last: Some(dec!(0.4285714285714285714285714284)),
                start_date: Utc.ymd(2000, 1, 8),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern1() {
        let start = Utc.ymd(2000, 4, 3);
        let payments = vec![
            start,
            Utc.ymd(2000, 4, 4),
            Utc.ymd(2000, 4, 6),
            Utc.ymd(2000, 4, 7),
        ];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![1, 2, 4, 5]),
            payments,
            start,
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.5714285714285714285714285714),
                last: Some(dec!(0.5714285714285714285714285716)),
                start_date: Utc.ymd(2000, 4, 8),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern2() {
        let payments = vec![
            Utc.ymd(2000, 4, 4),
            Utc.ymd(2000, 4, 6),
            Utc.ymd(2000, 4, 7),
            Utc.ymd(2000, 4, 9),
        ];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![2, 4, 5, 7]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.5714285714285714285714285714),
                last: Some(dec!(0.5714285714285714285714285716)),
                start_date: Utc.ymd(2000, 4, 8),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern3() {
        let payments = vec![
            Utc.ymd(2000, 4, 3),
            Utc.ymd(2000, 4, 4),
            Utc.ymd(2000, 4, 5),
            Utc.ymd(2000, 4, 6),
            Utc.ymd(2000, 4, 9),
        ];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![1, 2, 3, 4, 7]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.7142857142857142857142857143),
                last: Some(dec!(0.7142857142857142857142857142)),
                start_date: Utc.ymd(2000, 4, 7),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern4() {
        let payments = vec![Utc.ymd(2000, 4, 4), Utc.ymd(2000, 4, 8)];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![2, 6]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.2857142857142857142857142857),
                last: Some(dec!(0.2857142857142857142857142858)),
                start_date: Utc.ymd(2000, 4, 5),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern5() {
        let payments = vec![Utc.ymd(2000, 4, 6), Utc.ymd(2000, 4, 9)];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![4, 6]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.2857142857142857142857142857),
                last: Some(dec!(0.2857142857142857142857142858)),
                start_date: Utc.ymd(2000, 4, 3),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern6() {
        let payments = vec![Utc.ymd(2021, 7, 2)];
        let contribution = naive_contribution(
            dec!(0.01),
            &Frequency::Weekly(1, vec![5]),
            payments,
            Utc.ymd(2021, 7, 2),
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.0014285714285714285714285714),
                last: Some(dec!(0.0014285714285714285714285716)),
                start_date: Utc.ymd(2021, 7, 3),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn naive_contribution_pattern7() {
        let payments = vec![Utc.ymd(2021, 7, 2)];
        let contribution = naive_contribution(
            Decimal::ONE,
            &Frequency::Weekly(1, vec![5]),
            payments,
            Utc.ymd(2021, 7, 2),
            None,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: dec!(0.1428571428571428571428571429),
                last: Some(dec!(0.1428571428571428571428571426)),
                start_date: Utc.ymd(2021, 7, 3),
                end_date: None,
                period_length: Duration::days(7)
            })
        );
    }

    #[test]
    fn calculate_historical_error() {
        let result = calculate(
            Decimal::ONE,
            &Frequency::Once,
            Utc.ymd(2000, 4, 1),
            None,
            Utc.ymd(2000, 4, 2),
        );

        assert_eq!(result.err(), Some(ContributionError::HistoricalStartDate));
    }

    #[test]
    fn calculate_once() {
        let contributions = calculate(
            Decimal::ONE,
            &Frequency::Once,
            Utc.ymd(2000, 4, 2),
            None,
            Utc.ymd(2000, 4, 1),
        );
        assert_eq!(
            contributions,
            Ok(vec![Contribution {
                regular: dec!(0.5),
                last: None,
                start_date: Utc.ymd(2000, 4, 1),
                end_date: Some(Utc.ymd(2000, 4, 2)),
                period_length: Duration::days(2)
            }])
        );
    }

    #[test]
    fn calculate_daily_no_end() {
        let contributions = calculate(
            Decimal::ONE,
            &Frequency::Daily(2),
            Utc.ymd(2000, 4, 2),
            None,
            Utc.ymd(2000, 4, 1),
        );
        assert_eq!(
            contributions,
            Ok(vec![
                Contribution {
                    regular: dec!(0.5),
                    last: None,
                    start_date: Utc.ymd(2000, 4, 1),
                    end_date: Some(Utc.ymd(2000, 4, 2)),
                    period_length: Duration::days(2)
                },
                Contribution {
                    regular: dec!(0.5),
                    last: None,
                    start_date: Utc.ymd(2000, 4, 3),
                    end_date: None,
                    period_length: Duration::days(2)
                }
            ])
        );
    }

    #[test]
    fn calculate_daily_end_today() {
        let contributions = calculate(
            Decimal::ONE,
            &Frequency::Daily(2),
            Utc.ymd(2000, 4, 2),
            Some(Utc.ymd(2000, 4, 4)),
            Utc.ymd(2000, 4, 2),
        );
        assert_eq!(
            contributions,
            Ok(vec![
                Contribution {
                    regular: Decimal::ONE,
                    last: None,
                    start_date: Utc.ymd(2000, 4, 2),
                    end_date: Some(Utc.ymd(2000, 4, 2)),
                    period_length: Duration::days(1)
                },
                Contribution {
                    regular: dec!(0.5),
                    last: None,
                    start_date: Utc.ymd(2000, 4, 3),
                    end_date: Some(Utc.ymd(2000, 4, 4)),
                    period_length: Duration::days(2)
                }
            ])
        );
    }

    #[test]
    fn calculate_daily_end_yesterday() {
        let contributions = calculate(
            Decimal::ONE,
            &Frequency::Daily(2),
            Utc.ymd(2000, 4, 2),
            Some(Utc.ymd(2000, 4, 4)),
            Utc.ymd(2000, 4, 1),
        );
        assert_eq!(
            contributions,
            Ok(vec![Contribution {
                regular: dec!(0.5),
                last: None,
                start_date: Utc.ymd(2000, 4, 1),
                end_date: Some(Utc.ymd(2000, 4, 4)),
                period_length: Duration::days(4)
            }])
        );
    }

    #[test]
    fn calculate_approaching_zero() {
        let contributions = calculate(
            dec!(0.01),
            &Frequency::Once,
            Utc.ymd(2000, 4, 3),
            None,
            Utc.ymd(2000, 4, 1),
        );
        assert_eq!(
            contributions,
            Ok(vec![Contribution {
                regular: dec!(0.0033333333333333333333333333),
                last: Some(dec!(0.0033333333333333333333333334)),
                start_date: Utc.ymd(2000, 4, 1),
                end_date: Some(Utc.ymd(2000, 4, 3)),
                period_length: Duration::days(3)
            }])
        );
    }

    #[test]
    fn calculate_small_payment_biannually() {
        let contributions = calculate(
            dec!(5.0),
            &Frequency::Yearly(1, vec![2, 8], None, None),
            Utc.ymd(2000, 1, 1),
            None,
            Utc.ymd(2000, 1, 1),
        );
        assert_eq!(
            contributions,
            Ok(vec![
                Contribution {
                    regular: dec!(0.15625),
                    last: None,
                    start_date: Utc.ymd(2000, 01, 01),
                    end_date: Some(Utc.ymd(2000, 02, 01)),
                    period_length: Duration::days(32)
                },
                Contribution {
                    regular: dec!(0.0274725274725274725274725275),
                    last: Some(dec!(0.0274725274725274725274725225)),
                    start_date: Utc.ymd(2000, 02, 02),
                    end_date: Some(Utc.ymd(2000, 08, 01)),
                    period_length: Duration::days(182)
                },
                Contribution {
                    regular: dec!(0.0273972602739726027397260274),
                    last: Some(dec!(0.027397260273972602739726024)),
                    start_date: Utc.ymd(2000, 08, 02),
                    end_date: Some(Utc.ymd(2003, 08, 01)),
                    period_length: Duration::days(1095)
                },
                Contribution {
                    regular: dec!(0.0273785078713210130047912389),
                    last: Some(dec!(0.027378507871321013004791206)),
                    start_date: Utc.ymd(2003, 08, 02),
                    end_date: None,
                    period_length: Duration::days(1461)
                }
            ])
        );
    }

    #[test]
    fn calculate_small_payment_biennially() {
        let contributions = calculate(
            dec!(5.0),
            &Frequency::Yearly(2, vec![2, 8], None, None),
            Utc.ymd(2000, 1, 1),
            None,
            Utc.ymd(2000, 1, 1),
        );
        assert_eq!(
            contributions,
            Ok(vec![
                Contribution {
                    regular: dec!(0.15625),
                    last: None,
                    start_date: Utc.ymd(2000, 01, 01),
                    end_date: Some(Utc.ymd(2000, 02, 01)),
                    period_length: Duration::days(32)
                },
                Contribution {
                    regular: dec!(0.0274725274725274725274725275),
                    last: Some(dec!(0.0274725274725274725274725225)),
                    start_date: Utc.ymd(2000, 02, 02),
                    end_date: Some(Utc.ymd(2000, 08, 01)),
                    period_length: Duration::days(182)
                },
                Contribution {
                    regular: dec!(0.0136986301369863013698630137),
                    last: Some(dec!(0.013698630136986301369863013)),
                    start_date: Utc.ymd(2000, 08, 02),
                    end_date: Some(Utc.ymd(2002, 08, 01)),
                    period_length: Duration::days(730)
                },
                Contribution {
                    regular: dec!(0.0136892539356605065023956194),
                    last: Some(dec!(0.013689253935660506502395676)),
                    start_date: Utc.ymd(2002, 08, 02),
                    end_date: None,
                    period_length: Duration::days(1461)
                }
            ])
        );
    }
}
