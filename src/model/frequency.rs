use chrono::{Date, Datelike, Duration, LocalResult, TimeZone, Utc};

/// This constant represents the shortest number of days that is guaranteed to be
/// consistent. It is used to smooth periods that span months or years. Both units
/// contain inconsistent numbers of days, therefore a full 4 year period is required.
pub(super) const MACRO_PERIOD: u32 = (365.25 * 4.0) as u32;

/// Records the recurrence of a transaction
pub enum Frequency {
    Once,
    Daily(u32),                              // Every n days
    Weekly(u32, Vec<u32>),                   // Every n weeks on x days (Monday = 1, Sunday = 7)
    MonthlyDate(u32, Vec<u32>),              // Every n months each date
    MonthlyDay(u32, u32, FrequencyMonthDay), // Every n months, on the nth (First = 1, Fifth = 5, Last = 0) day
    // Every n years in x months (January = 1, December = 12) on the nth (First = 1, Fifth = 5, Last = 0) day
    Yearly(u32, Vec<u32>, Option<u32>, Option<FrequencyMonthDay>),
}

pub enum FrequencyMonthDay {
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

impl Frequency {
    pub fn get_period_length(&self) -> Duration {
        match *self {
            Frequency::Once => Duration::days(1),
            Frequency::Daily(days) => Duration::days(days as i64),
            Frequency::Weekly(weeks, _) => Duration::weeks(weeks as i64),
            // Months and years (i.e. leap years) have differing lengths, so assuming no
            // end date, any recursion involving months and years is inherently uneven.
            // Thus we use the MACRO_PERIOD const to smooth the variability.
            Frequency::MonthlyDate(_, _)
            | Frequency::MonthlyDay(_, _, _)
            | Frequency::Yearly(_, _, _, _) => Duration::days(MACRO_PERIOD as i64),
        }
    }

    pub fn get_payment_days(&self, start: Date<Utc>, end: Option<Date<Utc>>) -> Vec<Date<Utc>> {
        let period_end = start + self.get_period_length();
        let end = end.unwrap_or_else(|| period_end);

        match *self {
            Frequency::Once => vec![start.to_owned()],
            Frequency::Daily(days) => {
                let interval = Duration::days(days as i64);
                get_dates_for_interval(interval, start, end)
            }
            Frequency::Weekly(weeks, ref days) => {
                let interval = Duration::weeks(weeks as i64);

                // If start date's day (e.g. Saturday) is greater than all `days`, the
                // period will start next week. Otherwise, the period starts this week,
                // so the interval should be the `Frequency`'s interval.
                // For example, if `start` is a Saturday and we repeat on Tuesdays and
                // Fridays, it is reasonable to assume that the user intended for the
                // period to start on the following Tuesday. However if `start` is a
                // Wednesday, the period starts on the proceeding Friday and the first
                // Tuesday recursion is in n weeks (i.e. the `Frequency`'s interval).
                let start_weekday = start.weekday().number_from_monday();
                let max_day = *days.iter().max().expect("Frequency::Weekly days is empty");
                let weekday_interval = if start_weekday > max_day {
                    Duration::days(7)
                } else {
                    interval
                };

                // Convert a vector of day integers to dates, then for each date, get
                // every repetition of that date for the interval.
                let mut dates: Vec<Date<Utc>> = days
                    .iter()
                    .map(|day| increment_to_weekday(start, *day, weekday_interval))
                    .map(|date| get_dates_for_interval(interval, date, end))
                    .reduce(|mut a, mut b| {
                        a.append(&mut b);
                        a
                    })
                    .expect("Frequency::Weekly days is empty");

                // As multiple Date vectors are merged, their contents will be out of
                // order. The consumer will reasonably expect an ordered vector, so let's
                // be neighbourly.
                dates.sort_unstable();
                dates
            }
            Frequency::MonthlyDate(months, ref month_dates) => {
                // Calculate the month integers for the period
                let month_list = get_months_for_interval(months, start, end);

                let mut dates = Vec::new();

                // Loop over days, months and years to calculate all payment dates
                for (m, y) in month_list.iter() {
                    for d in month_dates {
                        if let LocalResult::Single(date) = Utc.ymd_opt(*y, *m, *d) {
                            if date >= start && date <= end {
                                dates.push(date);
                            }
                        }
                    }
                }

                // We loop over days before months/years, so we need to reshuffle.
                dates.sort_unstable();
                dates
            }
            _ => unimplemented!(),
        }
    }
}

// Get dates at given interval from start until end date
fn get_dates_for_interval(interval: Duration, start: Date<Utc>, end: Date<Utc>) -> Vec<Date<Utc>> {
    let mut payments = vec![start];
    let mut next = start;

    // XXX There's likely a smarter way to do this than mindless recursion!
    loop {
        next = next + interval;
        if next <= end {
            payments.push(next);
        } else {
            break;
        }
    }

    payments
}

// Get list of month integers at given interval from start until end date
fn get_months_for_interval(interval: u32, start: Date<Utc>, end: Date<Utc>) -> Vec<(u32, i32)> {
    let mut month = start.month();
    let mut year = start.year();
    let mut months = vec![(month, year)];

    // Loop until we reach the end month in that year
    while year < end.year() || month + interval <= end.month() {
        month += interval;

        // Reset month each year
        if month > 12 {
            month -= 12;
            year += 1;
        }

        months.push((month, year));
    }

    months
}

// Adjust date to match the given week day
// Note that this function will always go forwards in time, even if the closest day is
// yesterday. This is to ensure that we don't ever create payments before the start date.
fn increment_to_weekday(date: Date<Utc>, mut day: u32, interval: Duration) -> Date<Utc> {
    let weekday = date.weekday().number_from_monday();

    // If desired `day` is less than `date`'s week day, increment day to the next
    // interval so that we always go forward in time.
    if day < weekday {
        day += interval.num_days() as u32;
    }

    // Increment date to match closest week day
    let seek_duration = Duration::days((day - weekday) as i64);
    date + seek_duration
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn get_period_length_once() {
        let freq = Frequency::Once;
        assert_eq!(freq.get_period_length(), Duration::days(1));
    }

    #[test]
    fn get_period_length_daily() {
        let freq = Frequency::Daily(4);
        assert_eq!(freq.get_period_length(), Duration::days(4));
    }

    #[test]
    fn get_period_length_weekly() {
        let freq = Frequency::Weekly(5, vec![2, 3]);
        assert_eq!(freq.get_period_length(), Duration::weeks(5));
    }

    #[test]
    fn get_period_length_monthly_date() {
        let freq = Frequency::MonthlyDate(3, vec![1, 2]);
        assert_eq!(
            freq.get_period_length(),
            Duration::days(MACRO_PERIOD as i64)
        );
    }

    #[test]
    fn get_period_length_monthly_day() {
        let freq = Frequency::MonthlyDay(3, 1, FrequencyMonthDay::Day);
        assert_eq!(
            freq.get_period_length(),
            Duration::days(MACRO_PERIOD as i64)
        );
    }

    #[test]
    fn get_period_length_yearly() {
        let freq = Frequency::Yearly(3, vec![1], None, None);
        assert_eq!(
            freq.get_period_length(),
            Duration::days(MACRO_PERIOD as i64)
        );
    }

    #[test]
    fn get_dates_for_interval_5_days() {
        let interval = Duration::days(5);
        let start = Utc.ymd(2000, 4, 1);
        let end = Utc.ymd(2000, 4, 10);
        let days = vec![start, Utc.ymd(2000, 4, 6)];

        assert_eq!(get_dates_for_interval(interval, start, end), days);
    }

    #[test]
    fn get_months_for_interval_same_year() {
        let interval = 2;
        let start = Utc.ymd(2000, 4, 1);
        let end = Utc.ymd(2000, 9, 1);
        let months = vec![(4, 2000), (6, 2000), (8, 2000)];

        assert_eq!(get_months_for_interval(interval, start, end), months);
    }

    #[test]
    fn get_months_for_interval_5_years() {
        let interval = 7;
        let start = Utc.ymd(2000, 4, 1);
        let end = Utc.ymd(2005, 9, 1);
        let months = vec![
            (4, 2000),
            (11, 2000),
            (6, 2001),
            (1, 2002),
            (8, 2002),
            (3, 2003),
            (10, 2003),
            (5, 2004),
            (12, 2004),
            (7, 2005),
        ];

        assert_eq!(get_months_for_interval(interval, start, end), months);
    }

    #[test]
    fn increment_to_weekday_equal() {
        let date = Utc.ymd(2000, 4, 1); // Saturday
        assert_eq!(increment_to_weekday(date, 6, Duration::weeks(1)), date);
    }

    #[test]
    fn increment_to_weekday_greater() {
        let date = Utc.ymd(2000, 4, 1); // Saturday
        assert_eq!(
            increment_to_weekday(date, 7, Duration::weeks(1)),
            date.succ()
        );
    }

    #[test]
    fn increment_to_weekday_less() {
        let date = Utc.ymd(2000, 4, 1); // Saturday
        assert_eq!(
            increment_to_weekday(date, 2, Duration::weeks(2)),
            Utc.ymd(2000, 4, 11)
        );
    }

    #[test]
    fn get_payment_days_daily_end_date() {
        let frequency = Frequency::Daily(10);
        let start = Utc.ymd(2000, 4, 1);
        let end = Utc.ymd(2000, 5, 28);
        let days = vec![
            start,
            Utc.ymd(2000, 4, 11),
            Utc.ymd(2000, 4, 21),
            Utc.ymd(2000, 5, 1),
            Utc.ymd(2000, 5, 11),
            Utc.ymd(2000, 5, 21),
        ];

        assert_eq!(frequency.get_payment_days(start, Some(end)), days);
    }

    #[test]
    fn get_payment_days_daily_no_end_date() {
        let frequency = Frequency::Daily(10);
        let start = Utc.ymd(2000, 4, 1);
        let days = vec![start, Utc.ymd(2000, 4, 11)];

        assert_eq!(frequency.get_payment_days(start, None), days);
    }

    #[test]
    fn get_payment_days_weekly_end_date() {
        let frequency = Frequency::Weekly(3, vec![1, 3, 5]);
        let start = Utc.ymd(2000, 4, 4);
        let end = Utc.ymd(2000, 6, 30);
        let dates = vec![
            Utc.ymd(2000, 4, 5),
            Utc.ymd(2000, 4, 7),
            Utc.ymd(2000, 4, 24),
            Utc.ymd(2000, 4, 26),
            Utc.ymd(2000, 4, 28),
            Utc.ymd(2000, 5, 15),
            Utc.ymd(2000, 5, 17),
            Utc.ymd(2000, 5, 19),
            Utc.ymd(2000, 6, 5),
            Utc.ymd(2000, 6, 7),
            Utc.ymd(2000, 6, 9),
            Utc.ymd(2000, 6, 26),
            Utc.ymd(2000, 6, 28),
            Utc.ymd(2000, 6, 30),
        ];

        assert_eq!(frequency.get_payment_days(start, Some(end)), dates);
    }

    #[test]
    fn get_payment_days_weekly_no_end_date() {
        let frequency = Frequency::Weekly(3, vec![1, 3, 5]);
        let start = Utc.ymd(2000, 4, 4);
        let dates = vec![
            Utc.ymd(2000, 4, 5),
            Utc.ymd(2000, 4, 7),
            Utc.ymd(2000, 4, 24),
        ];

        assert_eq!(frequency.get_payment_days(start, None), dates);
    }

    #[test]
    fn get_payment_days_monthly_dates_end_date() {
        let frequency = Frequency::MonthlyDate(6, vec![27, 28, 29, 30, 31]);
        let start = Utc.ymd(1999, 8, 29);
        let end = Utc.ymd(2004, 4, 30);
        let dates = vec![
            Utc.ymd(1999, 8, 29),
            Utc.ymd(1999, 8, 30),
            Utc.ymd(1999, 8, 31),
            Utc.ymd(2000, 2, 27),
            Utc.ymd(2000, 2, 28),
            Utc.ymd(2000, 2, 29),
            Utc.ymd(2000, 8, 27),
            Utc.ymd(2000, 8, 28),
            Utc.ymd(2000, 8, 29),
            Utc.ymd(2000, 8, 30),
            Utc.ymd(2000, 8, 31),
            Utc.ymd(2001, 2, 27),
            Utc.ymd(2001, 2, 28),
            Utc.ymd(2001, 8, 27),
            Utc.ymd(2001, 8, 28),
            Utc.ymd(2001, 8, 29),
            Utc.ymd(2001, 8, 30),
            Utc.ymd(2001, 8, 31),
            Utc.ymd(2002, 2, 27),
            Utc.ymd(2002, 2, 28),
            Utc.ymd(2002, 8, 27),
            Utc.ymd(2002, 8, 28),
            Utc.ymd(2002, 8, 29),
            Utc.ymd(2002, 8, 30),
            Utc.ymd(2002, 8, 31),
            Utc.ymd(2003, 2, 27),
            Utc.ymd(2003, 2, 28),
            Utc.ymd(2003, 8, 27),
            Utc.ymd(2003, 8, 28),
            Utc.ymd(2003, 8, 29),
            Utc.ymd(2003, 8, 30),
            Utc.ymd(2003, 8, 31),
            Utc.ymd(2004, 2, 27),
            Utc.ymd(2004, 2, 28),
            Utc.ymd(2004, 2, 29),
        ];

        assert_eq!(frequency.get_payment_days(start, Some(end)), dates);
    }

    #[test]
    fn get_payment_days_monthly_dates_no_end_date() {
        let frequency = Frequency::MonthlyDate(9, vec![10, 31]);
        let start = Utc.ymd(2000, 4, 1);
        let dates = vec![
            Utc.ymd(2000, 4, 10),
            Utc.ymd(2001, 1, 10),
            Utc.ymd(2001, 1, 31),
            Utc.ymd(2001, 10, 10),
            Utc.ymd(2001, 10, 31),
            Utc.ymd(2002, 7, 10),
            Utc.ymd(2002, 7, 31),
            Utc.ymd(2003, 4, 10),
            Utc.ymd(2004, 1, 10),
            Utc.ymd(2004, 1, 31),
        ];

        assert_eq!(frequency.get_payment_days(start, None), dates);
    }
}
