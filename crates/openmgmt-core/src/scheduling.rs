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
            CalendarBlockStatus::Canceled
                | CalendarBlockStatus::Skipped
                | CalendarBlockStatus::Moved
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
        .replace("\r\n", "\n")
        .replace('\r', "\n")
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
    use crate::models::CalendarBlockSource;
    use chrono::{TimeZone, Timelike};

    fn block(id: &str, title: &str, status: CalendarBlockStatus) -> CalendarBlock {
        let start = Utc.with_ymd_and_hms(2026, 6, 19, 9, 0, 0).unwrap();
        CalendarBlock {
            id: id.into(),
            task_id: None,
            project_id: None,
            organization_id: None,
            title: title.into(),
            description: None,
            start_at: start,
            end_at: start + Duration::hours(1),
            timezone: None,
            source: CalendarBlockSource::OpenMgmt,
            external_id: None,
            status,
            created_at: start,
            updated_at: start,
        }
    }

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

    #[test]
    fn monthly_recurrence_clamps_month_end_dates() {
        let jan_31_2025 = Utc.with_ymd_and_hms(2025, 1, 31, 9, 30, 0).unwrap();
        let feb_2025 = next_recurrence_at(RecurrenceRule::Monthly, jan_31_2025).unwrap();
        assert_eq!(
            feb_2025,
            Utc.with_ymd_and_hms(2025, 2, 28, 9, 30, 0).unwrap()
        );

        let jan_31_2024 = Utc.with_ymd_and_hms(2024, 1, 31, 9, 30, 0).unwrap();
        let feb_2024 = next_recurrence_at(RecurrenceRule::Monthly, jan_31_2024).unwrap();
        assert_eq!(
            feb_2024,
            Utc.with_ymd_and_hms(2024, 2, 29, 9, 30, 0).unwrap()
        );
    }

    #[test]
    fn ics_export_excludes_inactive_blocks() {
        let planned = block("planned", "Planned Block", CalendarBlockStatus::Planned);
        let moved = block("moved", "Moved Block", CalendarBlockStatus::Moved);
        let skipped = block("skipped", "Skipped Block", CalendarBlockStatus::Skipped);
        let canceled = block("canceled", "Canceled Block", CalendarBlockStatus::Canceled);

        let ics = generate_schedule_ics(&[planned, moved, skipped, canceled]);
        assert!(ics.contains("Planned Block"));
        assert!(!ics.contains("Moved Block"));
        assert!(!ics.contains("Skipped Block"));
        assert!(!ics.contains("Canceled Block"));
    }

    #[test]
    fn ics_export_escapes_carriage_returns() {
        let mut block = block(
            "escaped",
            "Title\r\nCRLF\rCR\nLF,semi;slash\\",
            CalendarBlockStatus::Planned,
        );
        block.description = Some("Description\r\nCRLF\rCR\nLF".into());

        let ics = generate_schedule_ics(&[block]);
        let summary = ics
            .split("\r\n")
            .find(|line| line.starts_with("SUMMARY:"))
            .unwrap();
        let description = ics
            .split("\r\n")
            .find(|line| line.starts_with("DESCRIPTION:"))
            .unwrap();

        assert!(summary.contains("Title\\nCRLF\\nCR\\nLF\\,semi\\;slash\\\\"));
        assert!(description.contains("Description\\nCRLF\\nCR\\nLF"));
        assert!(!summary.contains('\r'));
        assert!(!description.contains('\r'));
    }
}
