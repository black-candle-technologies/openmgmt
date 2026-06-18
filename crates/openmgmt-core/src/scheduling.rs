use crate::models::{CalendarBlock, CalendarBlockStatus, RecurrenceRule};
use chrono::{DateTime, Datelike, Duration, Months, Utc, Weekday};

pub fn next_recurrence_at(
    rule: RecurrenceRule,
    occurrence: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match rule {
        RecurrenceRule::None => None,
        RecurrenceRule::Daily => Some(occurrence + Duration::days(1)),
        RecurrenceRule::Weekdays => {
            let mut next = occurrence + Duration::days(1);
            while matches!(next.weekday(), Weekday::Sat | Weekday::Sun) {
                next += Duration::days(1);
            }
            Some(next)
        }
        RecurrenceRule::Weekly => Some(occurrence + Duration::weeks(1)),
        RecurrenceRule::Monthly => occurrence.checked_add_months(Months::new(1)),
    }
}

pub fn generate_schedule_ics(blocks: &[CalendarBlock]) -> String {
    let mut output = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//OpenMgmt//Scheduling Core//EN\r\nCALSCALE:GREGORIAN\r\n",
    );
    for block in blocks.iter().filter(|block| {
        !matches!(
            block.status,
            CalendarBlockStatus::Canceled | CalendarBlockStatus::Skipped
        )
    }) {
        output.push_str("BEGIN:VEVENT\r\n");
        output.push_str(&format!("UID:{}@openmgmt\r\n", ics_escape(&block.id)));
        output.push_str(&format!(
            "DTSTAMP:{}\r\n",
            block.updated_at.format("%Y%m%dT%H%M%SZ")
        ));
        output.push_str(&format!(
            "DTSTART:{}\r\n",
            block.start_at.format("%Y%m%dT%H%M%SZ")
        ));
        output.push_str(&format!(
            "DTEND:{}\r\n",
            block.end_at.format("%Y%m%dT%H%M%SZ")
        ));
        output.push_str(&format!("SUMMARY:{}\r\n", ics_escape(&block.title)));
        if let Some(description) = &block.description {
            output.push_str(&format!("DESCRIPTION:{}\r\n", ics_escape(description)));
        }
        output.push_str(&format!("STATUS:{}\r\n", ics_status(block.status)));
        output.push_str("END:VEVENT\r\n");
    }
    output.push_str("END:VCALENDAR\r\n");
    output
}

fn ics_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

fn ics_status(status: CalendarBlockStatus) -> &'static str {
    match status {
        CalendarBlockStatus::Completed => "COMPLETED",
        CalendarBlockStatus::Canceled => "CANCELLED",
        _ => "CONFIRMED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[test]
    fn recurrence_supports_daily_weekly_monthly_and_weekdays() {
        let friday = Utc.with_ymd_and_hms(2026, 6, 19, 9, 30, 0).unwrap();
        assert_eq!(
            next_recurrence_at(RecurrenceRule::Daily, friday).unwrap(),
            friday + Duration::days(1)
        );
        assert_eq!(
            next_recurrence_at(RecurrenceRule::Weekly, friday).unwrap(),
            friday + Duration::weeks(1)
        );
        let weekday = next_recurrence_at(RecurrenceRule::Weekdays, friday).unwrap();
        assert_eq!(weekday.weekday(), Weekday::Mon);
        assert_eq!(weekday.hour(), 9);

        let january = Utc.with_ymd_and_hms(2026, 1, 15, 9, 30, 0).unwrap();
        let monthly = next_recurrence_at(RecurrenceRule::Monthly, january).unwrap();
        assert_eq!(monthly.month(), 2);
        assert_eq!(monthly.day(), 15);
    }
}
