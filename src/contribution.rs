use crate::{frequency::Frequency, CurrencyValue};
use chrono::{Date, Duration, Utc};
use thiserror::Error;

/// The daily amount to contribute to an upcoming payment
#[derive(Debug, PartialEq)]
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
    #[error("there are no payments for this contribution")]
    NoPayments,
    #[error("the date '{2}' is beyond the range {0} - {1}")]
    PaymentOutOfBounds(Date<Utc>, Date<Utc>, Date<Utc>), // start, end, oob
}

// impl Contribution {
//     /// Returns whether this `Contribution` has expired.
//     /// If `end_date` is not set, this `Contribution` will never expire.
//     pub fn has_expired(&self) -> bool {
//         match self.end_date {
//             Some(date) => date < Utc::today(),
//             None => false,
//         }
//     }
// }

/// Returns a tuple of the regular contribution and any required onboarding amelioration.
/// For example, if I have a new weekly payment schedule which starts in 3 days, but the
/// first payment is tomorrow, I need an 'onboarding' amelioration to cover the payments
/// we haven't been saving for.
pub(super) fn calculate(
    value: &CurrencyValue,
    frequency: &Frequency,
    mut start_date: Date<Utc>,
    mut end_date: Option<Date<Utc>>,
    now: Option<Date<Utc>>, // This allows overriding the current time for testing
) -> Result<Vec<Contribution>, ContributionError> {
    let now = now.unwrap_or(Utc::today());

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
    while payments.len() > 0 {
        // This sucks but we've got to create a clone of payments here.
        // `naive_contribution` mutates this vector, which will make it impossible to
        // track payments earlier than the start date, which can be incremented for fit.
        let payments_c = payments.clone();

        // Create a new Contribution from naive dates
        let contribution = naive_contribution(value, &frequency, payments_c, start_date, end_date)?;

        // Remove all payments covered by above contribution
        payments.retain(|p| *p < contribution.start_date);

        // Set end_date to the current start_date. This allows us to measure the
        // difference between the user-defined start date and the adjusted start date.
        // If there is a difference, we should setup a new contribution.
        end_date = Some(contribution.start_date);

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
    value: &CurrencyValue,
    frequency: &Frequency,
    mut payments: Vec<Date<Utc>>,
    start_date: Date<Utc>,
    end_date: Option<Date<Utc>>,
) -> Result<Contribution, ContributionError> {
    // If there aren't any payments, there shouldn't be a `Contribution`
    if payments.is_empty() {
        return Err(ContributionError::NoPayments);
    }

    // Setup a fixed end date that is either the user defined date, or the last day of
    // the period. Note we need to subtract 1 day as we want the *last* day of the period
    // rather than the first day of the next period.
    let period_end =
        end_date.unwrap_or(start_date + frequency.get_period_length() - Duration::days(1));

    // Define period length
    // Note we add a day here as we are calculating length, not difference
    let length = period_end - start_date + Duration::days(1);

    // For these algorithms to work, we need an ordered list
    payments.sort_unstable();

    // Make sure we never process payments beyond the start and end bounds
    if payments.first() < Some(&start_date) {
        return Err(ContributionError::PaymentOutOfBounds(
            start_date,
            period_end,
            *payments.first().unwrap(),
        ));
    } else if let Some(end) = end_date {
        if payments.last() > Some(&end) {
            return Err(ContributionError::PaymentOutOfBounds(
                start_date,
                end,
                *payments.last().unwrap(),
            ));
        }
    }

    // Calculate contribution amounts
    let total = CurrencyValue(**value * payments.len() as i64);
    let (regular, last) = calculate_for_duration(&total, length);

    // If final contribution equals zero, it means that we've accumulated money too
    // quickly. Advancing by a day will help to push contributions back so we accumulate
    // money slower.
    if *regular == 0 || last.as_ref().map(|l| **l) == Some(0) {
        if payments.first() == Some(&start_date) {
            let first = payments.remove(0);

            // If there is no end date, we have an infinitely recurring period. This means
            // that the first payment will now become the last payment. Hence we need to move
            // it!
            if end_date.is_none() {
                payments.push(first + length);
            }
        }

        return naive_contribution(value, frequency, payments, start_date.succ(), end_date);
    }

    // If every day in period has a payment, we don't need to modify the start date
    if payments.len() as i64 == length.num_days() {
        return Ok(Contribution {
            regular: regular.into(),
            last: last.map(|l| l.into()),
            start_date,
            end_date,
        });
    }

    // If there is no payment on the last day, we will end up accumulating contributions
    // after the last payment. This means that we won't actually cover the last payment
    // until after it's been made. Ladies and gentlemen, we cannot stand for this!
    if *payments.last().unwrap() < period_end {
        // If an end date exists, shift it forward to the last payment
        if end_date.is_some() {
            let last = *payments.last().unwrap();

            return naive_contribution(value, frequency, payments, start_date, Some(last));
        }
        // If no end date exists, rotate the period to the second payment
        else {
            let first = payments.remove(0);
            let new_start = first.succ();
            payments.push(first + length);

            return naive_contribution(value, frequency, payments, new_start, None);
        }
    }

    // If there is a payment on the start day, the contribution must equal the payment.
    // Otherwise we won't be able to cover today's payment! However as not every day in
    // the period has a payment, we will end up with a surplus. Thus today cannot be the
    // start date.
    if payments.first().unwrap() == &start_date {
        let first = payments.remove(0);

        // If there is no end date, we have an infinitely recurring period. This means
        // that the first payment will now become the last payment. Hence we need to move
        // it!
        if end_date.is_none() {
            payments.push(first + length);
        }

        return naive_contribution(value, frequency, payments, start_date.succ(), end_date);
    }

    // Loop through each payment to ensure that contributions keep up with the payments.
    // If not, shift the start date and try again.
    let mut acc = *regular;
    let mut prev_payment = &start_date;
    for payment in payments.iter() {
        let duration = *payment - *prev_payment;

        // Calculate current accumulated balance
        acc += duration.num_days() * *regular - **value;

        // If this is the last payment, swap the regular contribution for the final one
        if Some(payment) == payments.last() {
            if let Some(l) = last.as_ref() {
                acc += **l - *regular;
            }
        }

        // If we go below zero, this contribution and start_date combination are not
        // sustainable.
        if acc < 0 {
            break;
        }

        prev_payment = payment;
    }

    // Handle broken contribution by iterating the start date until we find a sustainable
    // date. Note that we don't shift any payment dates here. If we had a payment today,
    // it would be caught by earlier start date checks and handled there.
    if acc < 0 {
        naive_contribution(value, frequency, payments, start_date.succ(), end_date)
    } else {
        Ok(Contribution {
            regular: regular.into(),
            last: last.map(|l| l.into()),
            start_date,
            end_date,
        })
    }
}

// Calculate the contribution amounts for a value and duration
fn calculate_for_duration(
    value: &CurrencyValue,
    duration: Duration,
) -> (CurrencyValue, Option<CurrencyValue>) {
    // Calculate the regular contribution
    // Note that non-floating point integers automatically round down, which
    // could lead to exaggerated final payments. Thus we convert to a float,
    // then round manually.
    let regular = (**value as f64 / duration.num_days() as f64).round() as i64;

    // Given the rounding of the regular contribution, we may need a
    // different final contribution to handle the rounding error.
    let last = if regular * duration.num_days() != **value {
        Some(CurrencyValue(**value - regular * (duration.num_days() - 1)))
    } else {
        None
    };

    // The regular and last payments must equal the value, or our maths is
    // fundamentally broken!
    match last {
        Some(ref l) => assert_eq!(**value, regular * (duration.num_days() - 1) + **l),
        None => assert_eq!(**value, regular * duration.num_days()),
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
        let (regular, last) = calculate_for_duration(&9f64.try_into().unwrap(), Duration::days(4));

        assert_eq!(*regular, 225);
        assert_eq!(last, None);
    }

    #[test]
    fn calculate_for_duration_rounddown() {
        let (regular, last) =
            calculate_for_duration(&9.57f64.try_into().unwrap(), Duration::days(4));

        assert_eq!(*regular, 239);
        assert_eq!(last.map(|l| *l), Some(240));
    }

    #[test]
    fn calculate_for_duration_roundup() {
        let (regular, last) =
            calculate_for_duration(&11.1f64.try_into().unwrap(), Duration::days(4));

        assert_eq!(*regular, 278);
        assert_eq!(last.map(|l| *l), Some(276));
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
    fn naive_contribution_start_oob() {
        let start = Utc.ymd(2000, 1, 2);
        let payment = Utc.ymd(2000, 1, 1);
        let payments = vec![payment];
        let contribution =
            naive_contribution(&CurrencyValue(100), &Frequency::Once, payments, start, None);
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
            &CurrencyValue(100),
            &Frequency::Once,
            payments,
            start,
            Some(end),
        );
        assert_eq!(
            contribution.err(),
            Some(ContributionError::PaymentOutOfBounds(start, end, payment))
        );
    }

    #[test]
    fn naive_contribution_empty_payments() {
        let contribution = naive_contribution(
            &CurrencyValue(100),
            &Frequency::Once,
            Vec::new(),
            Utc.ymd(2000, 1, 2),
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
            &CurrencyValue(100),
            &Frequency::Daily(1),
            payments,
            start,
            Some(end),
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 1.0,
                last: None,
                start_date: start,
                end_date: Some(end),
            })
        );
    }

    #[test]
    fn naive_contribution_start_with_end() {
        let start = Utc.ymd(2000, 1, 1);
        let end = Utc.ymd(2000, 1, 3);
        let payments = vec![start, end];
        let contribution = naive_contribution(
            &CurrencyValue(100),
            &Frequency::Daily(2),
            payments,
            start,
            Some(end),
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.5,
                last: None,
                start_date: start.succ(),
                end_date: Some(end),
            })
        );
    }

    #[test]
    fn naive_contribution_start_no_end() {
        let start = Utc.ymd(2000, 1, 1);
        let end = Utc.ymd(2000, 1, 3);
        let payments = vec![start, end];
        let contribution = naive_contribution(
            &CurrencyValue(100),
            &Frequency::Daily(2),
            payments,
            start,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.67,
                last: Some(0.66),
                start_date: start.succ(),
                end_date: None,
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
            &CurrencyValue(100),
            &Frequency::Daily(1),
            payments,
            start,
            Some(end),
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 1.0,
                last: None,
                start_date: start,
                end_date: Some(pay_end),
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
            &CurrencyValue(100),
            &Frequency::Weekly(1, vec![3, 4, 5]),
            payments,
            start,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.43,
                last: Some(0.42),
                start_date: Utc.ymd(2000, 1, 8),
                end_date: None,
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
            &CurrencyValue(100),
            &Frequency::Weekly(1, vec![1, 2, 4, 5]),
            payments,
            start,
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.57,
                last: Some(0.58),
                start_date: Utc.ymd(2000, 4, 8),
                end_date: None,
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
            &CurrencyValue(100),
            &Frequency::Weekly(1, vec![2, 4, 5, 7]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.57,
                last: Some(0.58),
                start_date: Utc.ymd(2000, 4, 8),
                end_date: None,
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
            &CurrencyValue(100),
            &Frequency::Weekly(1, vec![1, 2, 3, 4, 7]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.71,
                last: Some(0.74),
                start_date: Utc.ymd(2000, 4, 7),
                end_date: None,
            })
        );
    }

    #[test]
    fn naive_contribution_pattern4() {
        let payments = vec![Utc.ymd(2000, 4, 4), Utc.ymd(2000, 4, 8)];
        let contribution = naive_contribution(
            &CurrencyValue(100),
            &Frequency::Weekly(1, vec![2, 6]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.29,
                last: Some(0.26),
                start_date: Utc.ymd(2000, 4, 5),
                end_date: None,
            })
        );
    }

    #[test]
    fn naive_contribution_pattern5() {
        let payments = vec![Utc.ymd(2000, 4, 6), Utc.ymd(2000, 4, 9)];
        let contribution = naive_contribution(
            &CurrencyValue(100),
            &Frequency::Weekly(1, vec![4, 6]),
            payments,
            Utc.ymd(2000, 4, 3),
            None,
        );
        assert_eq!(
            contribution,
            Ok(Contribution {
                regular: 0.29,
                last: Some(0.26),
                start_date: Utc.ymd(2000, 4, 3),
                end_date: None,
            })
        );
    }

    #[test]
    fn calculate_historical_error() {
        let result = calculate(
            &1f64.try_into().unwrap(),
            &Frequency::Once,
            Utc.ymd(2000, 4, 1),
            None,
            Some(Utc.ymd(2000, 4, 2)),
        );

        assert_eq!(result.err(), Some(ContributionError::HistoricalStartDate));
    }

    #[test]
    fn calculate_once() {
        let contributions = calculate(
            &CurrencyValue(100),
            &Frequency::Once,
            Utc.ymd(2000, 4, 2),
            None,
            Some(Utc.ymd(2000, 4, 1)),
        );
        assert_eq!(
            contributions,
            Ok(vec![Contribution {
                regular: 0.5,
                last: None,
                start_date: Utc.ymd(2000, 4, 1),
                end_date: Some(Utc.ymd(2000, 4, 2))
            }])
        );
    }

    #[test]
    fn calculate_daily_no_end() {
        let contributions = calculate(
            &CurrencyValue(100),
            &Frequency::Daily(2),
            Utc.ymd(2000, 4, 2),
            None,
            Some(Utc.ymd(2000, 4, 1)),
        );
        assert_eq!(
            contributions,
            Ok(vec![
                Contribution {
                    regular: 1.0,
                    last: None,
                    start_date: Utc.ymd(2000, 4, 2),
                    end_date: Some(Utc.ymd(2000, 4, 2))
                },
                Contribution {
                    regular: 0.67,
                    last: Some(0.66),
                    start_date: Utc.ymd(2000, 4, 3),
                    end_date: None
                }
            ])
        );
    }

    #[test]
    fn calculate_daily_end_today() {
        let contributions = calculate(
            &CurrencyValue(100),
            &Frequency::Daily(2),
            Utc.ymd(2000, 4, 2),
            Some(Utc.ymd(2000, 4, 4)),
            Some(Utc.ymd(2000, 4, 2)),
        );
        assert_eq!(
            contributions,
            Ok(vec![
                Contribution {
                    regular: 1.0,
                    last: None,
                    start_date: Utc.ymd(2000, 4, 2),
                    end_date: Some(Utc.ymd(2000, 4, 2))
                },
                Contribution {
                    regular: 0.5,
                    last: None,
                    start_date: Utc.ymd(2000, 4, 3),
                    end_date: Some(Utc.ymd(2000, 4, 4)),
                }
            ])
        );
    }

    #[test]
    fn calculate_daily_end_yesterday() {
        let contributions = calculate(
            &CurrencyValue(100),
            &Frequency::Daily(2),
            Utc.ymd(2000, 4, 2),
            Some(Utc.ymd(2000, 4, 4)),
            Some(Utc.ymd(2000, 4, 1)),
        );
        assert_eq!(
            contributions,
            Ok(vec![Contribution {
                regular: 0.5,
                last: None,
                start_date: Utc.ymd(2000, 4, 1),
                end_date: Some(Utc.ymd(2000, 4, 4)),
            }])
        );
    }

    #[test]
    fn calculate_approaching_zero() {
        let contributions = calculate(
            &CurrencyValue(1),
            &Frequency::Once,
            Utc.ymd(2000, 4, 3),
            None,
            Some(Utc.ymd(2000, 4, 1)),
        );
        assert_eq!(
            contributions,
            Ok(vec![Contribution {
                regular: 0.01,
                last: None,
                start_date: Utc.ymd(2000, 4, 3),
                end_date: Some(Utc.ymd(2000, 4, 3)),
            }])
        );
    }
}
