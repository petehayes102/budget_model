use chrono::{Date, Datelike, Duration, LocalResult, TimeZone, Utc};

/// This constant represents the shortest number of days that is guaranteed to be
/// consistent. It is used to smooth periods that span months or years. Both units
/// contain inconsistent numbers of days, therefore a full 4 year period is required.
pub(super) const MACRO_PERIOD: u32 = (365.25 * 4.0) as u32;

// These are tedious arrays to aid the lookup of month lengths. Unfortunately the
// `chrono` library does not give us helpers for this.
const MONTH_LENGTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const MONTH_LENGTHS_LEAP: [u32; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

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
            // Note that we add 1 to days as a "2 day" frequency means "every second day".
            // Thus, days 1 and 3. This actually a 3 day period, when you include the
            // start day.
            Frequency::Daily(days) => Duration::days((days + 1) as i64),
            Frequency::Weekly(weeks, _) => Duration::weeks(weeks as i64),
            Frequency::MonthlyDate(months, _) | Frequency::MonthlyDay(months, _, _) => {
                // The period length must be units of `MACRO_PERIOD` in order to handle
                // leap years.
                to_macro_periods(months as f32 * 365.25 / 12.0)
            }
            Frequency::Yearly(years, _, _, _) => {
                // The period length must be units of `MACRO_PERIOD` in order to handle
                // leap years.
                to_macro_periods(years as f32 * 365.25)
            }
        }
    }

    pub fn get_payment_dates(&self, start: Date<Utc>, end: Option<Date<Utc>>) -> Vec<Date<Utc>> {
        // Set period end to yesterday + period length. We do this to prevent the
        // inclusion of the first day of next period in this period. For example, Monday
        // + 1 week = Monday. However the _end_ of this period is *Sunday*.
        let period_end = start.pred() + self.get_period_length();
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
                    Duration::weeks(1)
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
            Frequency::MonthlyDay(months, nth, ref day) => {
                // Calculate the month integers for the period
                let month_list = get_months_for_interval(months, start, end);

                let mut dates = Vec::new();

                // Loop over months and years to calculate all payment dates
                for (m, y) in month_list.iter() {
                    if let Some(d) = day.get_date(*y, *m, nth) {
                        if d >= start && d <= end {
                            dates.push(d);
                        }
                    }
                }

                dates
            }
            Frequency::Yearly(years, ref months, nth, ref day) => {
                let mut dates = Vec::new();

                for year in start.year()..=end.year() {
                    // Only include years that match the recursion
                    if (year - start.year()) % years as i32 == 0 {
                        for m in months {
                            // 'nth day' recursion is optional. If the user hasn't
                            // defined these params, use the start date's day.
                            if let Some(nth) = nth {
                                let day = day
                                    .as_ref()
                                    .expect("`day` was not set for Frequency::Yearly");
                                if let Some(d) = day.get_date(year, *m, nth) {
                                    if d >= start && d <= end {
                                        dates.push(d);
                                    }
                                }
                            } else {
                                // Make sure we don't try to instantiate an invalid date,
                                // e.g. "31 June".
                                if let LocalResult::Single(date) =
                                    Utc.ymd_opt(year, *m, start.day())
                                {
                                    if date >= start && date <= end {
                                        dates.push(date);
                                    }
                                }
                            }
                        }
                    }
                }

                dates
            }
        }
    }
}

impl FrequencyMonthDay {
    fn get_day_of_week(&self) -> u32 {
        match *self {
            FrequencyMonthDay::Monday => 1,
            FrequencyMonthDay::Tuesday => 2,
            FrequencyMonthDay::Wednesday => 3,
            FrequencyMonthDay::Thursday => 4,
            FrequencyMonthDay::Friday => 5,
            FrequencyMonthDay::Saturday => 6,
            FrequencyMonthDay::Sunday => 7,

            // There are no good defaults for these, so set them to something nonsensical
            FrequencyMonthDay::Day => 0,
            FrequencyMonthDay::Weekday => 0,
            FrequencyMonthDay::Weekend => 0,
        }
    }

    fn get_date(&self, year: i32, month: u32, mut nth: u32) -> Option<Date<Utc>> {
        let date = Utc.ymd(year, month, 1);
        let weekday = date.weekday().number_from_monday();
        let seek_last = nth == 0;

        // Get length of month, accounting for leap years
        let length = if date.year() % 4 == 0 {
            MONTH_LENGTHS_LEAP[date.month0() as usize]
        } else {
            MONTH_LENGTHS[date.month0() as usize]
        };

        // The maximum length for 4 weeks is 28 days. If there is surplus, this means
        // that some days can accommodate a 5th recursion, if they occur early enough
        // in the month.
        let max_nth = if self.get_day_of_week() <= length - 28 {
            5
        } else {
            4
        };

        // Handle 'last' nth, which is represented by a 0
        if seek_last {
            nth = max_nth;
        }
        // Handle invalid nth (i.e. where nth places the day in the next month)
        else if nth > max_nth {
            return None;
        }

        // Subtract one as we are already on day/week 1
        let week_interval = Duration::weeks(nth as i64 - 1);
        let day_interval = Duration::days(nth as i64 - 1);

        let d = match *self {
            FrequencyMonthDay::Monday
            | FrequencyMonthDay::Tuesday
            | FrequencyMonthDay::Wednesday
            | FrequencyMonthDay::Thursday
            | FrequencyMonthDay::Friday
            | FrequencyMonthDay::Saturday
            | FrequencyMonthDay::Sunday => {
                increment_to_weekday(date, self.get_day_of_week(), Duration::weeks(1))
                    + week_interval
            }
            FrequencyMonthDay::Day => {
                if seek_last {
                    // This will never fail as we check the month length in advance
                    date.with_day(length).unwrap()
                } else {
                    date + day_interval
                }
            }
            FrequencyMonthDay::Weekday if !seek_last => {
                // Account for day offset. If the first day of the month is Monday, the
                // offset will be zero. Otherwise it will track the number of days offset
                // from Monday the first date is.
                let offset = weekday - 1;

                // If this month date spans the weekend, increment by weekend length (i.e. 2)
                if nth + offset > 5 {
                    nth += 2;
                }

                date.with_day(nth).unwrap()
            }
            FrequencyMonthDay::Weekday if seek_last => {
                // 28 days is the minimum month length and exactly 4 weeks from the
                // first day. Thus `weekday` for day 28 = `weekday` for day 7. In order
                // to get an offset that tells us how to find the final day, subtract by
                // 29 instead, which is = to `weekday` for day 1.
                let mut offset = length as i32 - 29;
                let mut last_weekday = weekday as i32 + offset;

                // Adjust for week length
                if last_weekday > 7 {
                    last_weekday -= 7;
                }

                // Adjust for weekend
                if last_weekday > 5 {
                    offset -= last_weekday - 5;
                }

                date.with_day((29 + offset) as u32).unwrap()
            }
            FrequencyMonthDay::Weekend if !seek_last => {
                // Calculate the number of days 'offset' the month is. I.e. how many
                // non-weekend days occur before our first weekend.
                let offset = if weekday == 7 { 0 } else { 6 - weekday };

                // If the month start on a day <= Saturday, we have a full first weekend.
                // However the `weekdays` formula uses `nth / 2` to determine whether a
                // set of weekdays lies between the previous weekend and this one. If we
                // have a full first weekend, the formula would give us a date for nth=2
                // of 7. Thus we subtract 1 in order that the formula doesn't apply any
                // weekdays to the first full weekend. However, if the weekend starts on
                // Sunday, this behaviour is desirable, so increment by 1.
                let nth_adjusted = nth - 1 + weekday / 7;

                // Calculate how many sets of weekdays sit between each weekend pair.
                // This is important because the 3rd, 4th and 5th weekend days are
                // separated by 1 or 2 sets of weekdays.
                let weekdays = nth_adjusted / 2 * 5;

                // Add all the components together to get our date.
                let month_date = nth + offset + weekdays;

                date.with_day(month_date).unwrap()
            }
            FrequencyMonthDay::Weekend if seek_last => {
                // 28 days is the minimum month length and exactly 4 weeks from the
                // first day. Thus `weekday` for day 28 = `weekday` for day 7. In order
                // to get an offset that tells us how to find the final day, subtract by
                // 29 instead, which is = to `weekday` for day 1.
                let mut offset = length as i32 - 29;
                let mut last_weekday = weekday as i32 + offset;

                // Adjust for week length
                if last_weekday > 7 {
                    last_weekday -= 7;
                }

                // Adjust for weekdays
                if last_weekday < 6 {
                    offset -= last_weekday;
                }

                println!("Offset: {}, Lask wkd: {}", offset, last_weekday);

                date.with_day((29 + offset) as u32).unwrap()
            }
            _ => unreachable!(),
        };

        Some(d)
    }
}

// Where we recurse over months or years, we have to handle different period lengths. For
// example, January has 31 days, February has 28 days (but 29 on a leap year), and April
// has 30 days. In order to calculate a single daily contribution that handles all this
// variability, we amortise the recursion over a 4 year period. This guarantees that we
// have a consistent number of days, irrespective of months or leap years.
fn to_macro_periods(days: f32) -> Duration {
    let num_macro = (days / MACRO_PERIOD as f32).ceil() as i64;
    Duration::days(num_macro * MACRO_PERIOD as i64)
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
        assert_eq!(freq.get_period_length(), Duration::days(5));
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
    fn get_period_length_monthly_date_gt_macro() {
        let freq = Frequency::MonthlyDate(49, vec![1]);
        assert_eq!(
            freq.get_period_length(),
            Duration::days(MACRO_PERIOD as i64 * 2)
        );
    }

    #[test]
    fn get_period_length_monthly_day_gt_macro() {
        let freq = Frequency::MonthlyDay(49, 1, FrequencyMonthDay::Day);
        assert_eq!(
            freq.get_period_length(),
            Duration::days(MACRO_PERIOD as i64 * 2)
        );
    }

    #[test]
    fn get_period_length_yearly_gt_macro() {
        let freq = Frequency::Yearly(5, vec![1], None, None);
        assert_eq!(
            freq.get_period_length(),
            Duration::days(MACRO_PERIOD as i64 * 2)
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
    fn get_date_today() {
        let frequency = FrequencyMonthDay::Saturday;
        let new_date = Utc.ymd(2000, 4, 1);
        assert_eq!(frequency.get_date(2000, 4, 1), Some(new_date));
    }

    #[test]
    fn get_date_next_week() {
        let frequency = FrequencyMonthDay::Monday;
        let new_date = Utc.ymd(2000, 4, 3);
        assert_eq!(frequency.get_date(2000, 4, 1), Some(new_date));
    }

    #[test]
    fn get_date_nth_week() {
        let frequency = FrequencyMonthDay::Monday;
        let new_date = Utc.ymd(2000, 4, 17);
        assert_eq!(frequency.get_date(2000, 4, 3), Some(new_date));
    }

    #[test]
    fn get_date_day() {
        let frequency = FrequencyMonthDay::Day;
        let new_date = Utc.ymd(2000, 4, 4);
        assert_eq!(frequency.get_date(2000, 4, 4), Some(new_date));
    }

    #[test]
    fn get_date_weekday_today() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 5, 1);
        assert_eq!(frequency.get_date(2000, 5, 1), Some(new_date));
    }

    #[test]
    fn get_date_weekday_fourth() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 3, 6);
        assert_eq!(frequency.get_date(2000, 3, 4), Some(new_date));
    }

    #[test]
    fn get_date_weekday_fifth() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 2, 7);
        assert_eq!(frequency.get_date(2000, 2, 5), Some(new_date));
    }

    #[test]
    fn get_date_weekday_weekend() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 1, 3);
        assert_eq!(frequency.get_date(2000, 1, 1), Some(new_date));
    }

    #[test]
    fn get_date_weekend_today() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 4, 1);
        assert_eq!(frequency.get_date(2000, 4, 1), Some(new_date));
    }

    #[test]
    fn get_date_weekend_first() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 5, 6);
        assert_eq!(frequency.get_date(2000, 5, 1), Some(new_date));
    }

    #[test]
    fn get_date_weekend_second() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 10, 7);
        assert_eq!(frequency.get_date(2000, 10, 2), Some(new_date));
    }

    #[test]
    fn get_date_last_friday() {
        let frequency = FrequencyMonthDay::Friday;
        let new_date = Utc.ymd(2000, 4, 28);
        assert_eq!(frequency.get_date(2000, 4, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_day() {
        let frequency = FrequencyMonthDay::Day;
        let new_date = Utc.ymd(2000, 4, 30);
        assert_eq!(frequency.get_date(2000, 4, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekday_fri() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 4, 28);
        assert_eq!(frequency.get_date(2000, 4, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekday_wed() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 5, 31);
        assert_eq!(frequency.get_date(2000, 5, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekday_jan() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 1, 31);
        assert_eq!(frequency.get_date(2000, 1, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekday_feb() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2001, 2, 28);
        assert_eq!(frequency.get_date(2001, 2, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekday_oct() {
        let frequency = FrequencyMonthDay::Weekday;
        let new_date = Utc.ymd(2000, 10, 31);
        assert_eq!(frequency.get_date(2000, 10, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekend_today() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 9, 30);
        assert_eq!(frequency.get_date(2000, 9, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekend_jan() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 1, 30);
        assert_eq!(frequency.get_date(2000, 1, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekend_may() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 5, 28);
        assert_eq!(frequency.get_date(2000, 5, 0), Some(new_date));
    }

    #[test]
    fn get_date_last_weekend_oct() {
        let frequency = FrequencyMonthDay::Weekend;
        let new_date = Utc.ymd(2000, 10, 29);
        assert_eq!(frequency.get_date(2000, 10, 0), Some(new_date));
    }

    #[test]
    fn get_date_invalid_date() {
        let frequency = FrequencyMonthDay::Friday;
        assert_eq!(frequency.get_date(2000, 4, 5), None);
    }

    #[test]
    fn get_payment_dates_daily_end_date() {
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

        assert_eq!(frequency.get_payment_dates(start, Some(end)), days);
    }

    #[test]
    fn get_payment_dates_daily_no_end_date() {
        let frequency = Frequency::Daily(10);
        let start = Utc.ymd(2000, 4, 1);
        let days = vec![start, Utc.ymd(2000, 4, 11)];

        assert_eq!(frequency.get_payment_dates(start, None), days);
    }

    #[test]
    fn get_payment_dates_weekly_end_date() {
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

        assert_eq!(frequency.get_payment_dates(start, Some(end)), dates);
    }

    #[test]
    fn get_payment_dates_weekly_no_end_date() {
        let frequency = Frequency::Weekly(3, vec![1, 2, 3, 5]);
        let start = Utc.ymd(2000, 4, 4);
        let dates = vec![
            Utc.ymd(2000, 4, 4),
            Utc.ymd(2000, 4, 5),
            Utc.ymd(2000, 4, 7),
            Utc.ymd(2000, 4, 24),
        ];

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }

    #[test]
    fn get_payment_dates_monthly_dates_end_date() {
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

        assert_eq!(frequency.get_payment_dates(start, Some(end)), dates);
    }

    #[test]
    fn get_payment_dates_monthly_dates_no_end_date() {
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

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }

    #[test]
    fn get_payment_dates_monthly_dates_period_end() {
        let frequency = Frequency::MonthlyDate(12, vec![1]);
        let start = Utc.ymd(2000, 4, 1);
        let dates = vec![
            Utc.ymd(2000, 4, 1),
            Utc.ymd(2001, 4, 1),
            Utc.ymd(2002, 4, 1),
            Utc.ymd(2003, 4, 1),
        ];

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }

    #[test]
    fn get_payment_dates_yearly_nth_end_date() {
        let frequency = Frequency::Yearly(2, vec![1, 2], Some(0), Some(FrequencyMonthDay::Weekend));
        let start = Utc.ymd(2000, 1, 1);
        let end = Utc.ymd(2008, 2, 1);
        let dates = vec![
            Utc.ymd(2000, 1, 30),
            Utc.ymd(2000, 2, 27),
            Utc.ymd(2002, 1, 27),
            Utc.ymd(2002, 2, 24),
            Utc.ymd(2004, 1, 31),
            Utc.ymd(2004, 2, 29),
            Utc.ymd(2006, 1, 29),
            Utc.ymd(2006, 2, 26),
            Utc.ymd(2008, 1, 27),
        ];

        assert_eq!(frequency.get_payment_dates(start, Some(end)), dates);
    }

    #[test]
    fn get_payment_dates_yearly_nth_no_end_date() {
        let frequency = Frequency::Yearly(2, vec![1, 2], Some(0), Some(FrequencyMonthDay::Weekend));
        let start = Utc.ymd(2000, 1, 1);
        let dates = vec![
            Utc.ymd(2000, 1, 30),
            Utc.ymd(2000, 2, 27),
            Utc.ymd(2002, 1, 27),
            Utc.ymd(2002, 2, 24),
        ];

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }

    #[test]
    fn get_payment_dates_yearly_end_date() {
        let frequency = Frequency::Yearly(2, vec![1, 2], None, None);
        let start = Utc.ymd(2000, 1, 29);
        let end = Utc.ymd(2008, 2, 1);
        let dates = vec![
            Utc.ymd(2000, 1, 29),
            Utc.ymd(2000, 2, 29),
            Utc.ymd(2002, 1, 29),
            Utc.ymd(2004, 1, 29),
            Utc.ymd(2004, 2, 29),
            Utc.ymd(2006, 1, 29),
            Utc.ymd(2008, 1, 29),
        ];

        assert_eq!(frequency.get_payment_dates(start, Some(end)), dates);
    }

    #[test]
    fn get_payment_dates_yearly_no_end_date() {
        let frequency = Frequency::Yearly(2, vec![1, 2], None, None);
        let start = Utc.ymd(2000, 1, 29);
        let dates = vec![
            Utc.ymd(2000, 1, 29),
            Utc.ymd(2000, 2, 29),
            Utc.ymd(2002, 1, 29),
        ];

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }

    #[test]
    fn get_payment_dates_yearly_period_end() {
        let frequency = Frequency::Yearly(1, vec![1], None, None);
        let start = Utc.ymd(2000, 1, 1);
        let dates = vec![
            Utc.ymd(2000, 1, 1),
            Utc.ymd(2001, 1, 1),
            Utc.ymd(2002, 1, 1),
            Utc.ymd(2003, 1, 1),
        ];

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }

    #[test]
    fn get_payment_dates_yearly_odd_years() {
        let frequency = Frequency::Yearly(2, vec![1, 2], None, None);
        let start = Utc.ymd(2001, 2, 1);
        let dates = vec![
            Utc.ymd(2001, 2, 1),
            Utc.ymd(2003, 1, 1),
            Utc.ymd(2003, 2, 1),
            Utc.ymd(2005, 1, 1),
        ];

        assert_eq!(frequency.get_payment_dates(start, None), dates);
    }
}
