use crate::models::{CalendarBlock, CalendarBlockStatus, RecurrenceRule};
use chrono::{DateTime, Datelike, Duration, Months, Utc, Weekday};

/// Increment scheduled blocks snap to when dragging / resizing, in minutes.
pub const SCHEDULE_SNAP_MINUTES: i64 = 15;
/// Shortest scheduled block a resize may produce, in minutes.
pub const MIN_BLOCK_MINUTES: i64 = 15;

/// Which edge of a scheduled block a resize drag is moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockEdge {
    /// The top handle: moves the start time (earlier when dragged up).
    Start,
    /// The bottom handle: moves the end time (later when dragged down).
    End,
}

/// Snap a minute delta to the nearest [`SCHEDULE_SNAP_MINUTES`] increment.
pub fn snap_to_increment(minutes: i64) -> i64 {
    let step = SCHEDULE_SNAP_MINUTES.max(1);
    ((minutes as f64) / step as f64).round() as i64 * step
}

/// Apply a resize drag to a `[start, end)` block by moving one edge by
/// `delta_minutes` (snapped to [`SCHEDULE_SNAP_MINUTES`]). The block is kept at
/// least [`MIN_BLOCK_MINUTES`] long and the dragged edge never crosses the other,
/// so an invalid (end ≤ start) range can never be produced. The task/block
/// identity is the caller's concern and is left untouched.
pub fn resize_block(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    edge: BlockEdge,
    delta_minutes: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let snapped = Duration::minutes(snap_to_increment(delta_minutes));
    let min = Duration::minutes(MIN_BLOCK_MINUTES);
    match edge {
        BlockEdge::Start => {
            let new_start = (start + snapped).min(end - min);
            (new_start, end)
        }
        BlockEdge::End => {
            let new_end = (end + snapped).max(start + min);
            (start, new_end)
        }
    }
}

/// Move a `[start, end)` block by `delta_minutes` (snapped to
/// [`SCHEDULE_SNAP_MINUTES`]), preserving its duration. Used by the timeline's
/// drag-to-move so a moved block keeps the same length.
pub fn move_block(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    delta_minutes: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let snapped = Duration::minutes(snap_to_increment(delta_minutes));
    (start + snapped, end + snapped)
}

/// Convert a vertical pixel delta on the day timeline into a minute delta, given
/// the pixels-per-hour the timeline renders at. The result is **not** snapped —
/// callers pass it through [`resize_block`]/[`move_block`], which snap. Sharing
/// this conversion keeps the drag preview and the commit using identical math.
pub fn pixels_to_minutes(delta_px: f64, hour_px: f64) -> i64 {
    if hour_px <= 0.0 {
        return 0;
    }
    (delta_px / hour_px * 60.0).round() as i64
}

/// Pixel `(top, height)` for a scheduled block on a vertical day timeline that
/// begins at `day_start_hour` (local) and draws `hour_px` pixels per hour.
/// `start_min`/`end_min` are minutes from local midnight, so a 5–7 PM block
/// (`1020`–`1260`) spans `2 * hour_px`. Height is floored at `min_px` so very
/// short blocks stay readable, and `top` is clamped to the visible range.
pub fn block_pixel_layout(
    start_min: i64,
    end_min: i64,
    day_start_hour: i64,
    hour_px: f64,
    min_px: f64,
) -> (f64, f64) {
    let day_start_min = day_start_hour * 60;
    let top = ((start_min - day_start_min) as f64 / 60.0 * hour_px).max(0.0);
    let height = (((end_min - start_min).max(0) as f64) / 60.0 * hour_px).max(min_px);
    (top, height)
}

/// Pack overlapping `[start, end)` intervals (minutes) into side-by-side columns
/// so a day timeline can render concurrent blocks without hiding one behind the
/// other. Returns, in the input order, each interval's `(column, column_count)`
/// where `column_count` is the number of columns its overlap cluster needs — the
/// UI renders width `1/column_count` at offset `column/column_count`.
///
/// ponytail: greedy first-free-column packing, O(n²) over a day's blocks; that is
/// plenty for a personal schedule. Swap for interval-graph colouring only if days
/// ever hold hundreds of overlapping blocks.
pub fn layout_columns(intervals: &[(i64, i64)]) -> Vec<(usize, usize)> {
    let n = intervals.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| intervals[i]);

    let mut column = vec![0usize; n];
    let mut cluster_columns = vec![1usize; n];
    let mut cluster: Vec<usize> = Vec::new();
    // Per-column end time of the last block placed in it, within this cluster.
    let mut column_ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;

    for &i in &order {
        let (start, end) = intervals[i];
        if start >= cluster_end && !cluster.is_empty() {
            let cols = column_ends.len();
            for &c in &cluster {
                cluster_columns[c] = cols;
            }
            cluster.clear();
            column_ends.clear();
            cluster_end = i64::MIN;
        }
        let slot = column_ends.iter().position(|&end| end <= start);
        let col = match slot {
            Some(c) => {
                column_ends[c] = end;
                c
            }
            None => {
                column_ends.push(end);
                column_ends.len() - 1
            }
        };
        column[i] = col;
        cluster.push(i);
        cluster_end = cluster_end.max(end);
    }
    if !cluster.is_empty() {
        let cols = column_ends.len();
        for &c in &cluster {
            cluster_columns[c] = cols;
        }
    }
    (0..n).map(|i| (column[i], cluster_columns[i])).collect()
}

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
                | CalendarBlockStatus::OnHold
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
        let on_hold = block("on-hold", "On Hold Block", CalendarBlockStatus::OnHold);

        let ics = generate_schedule_ics(&[planned, moved, skipped, canceled, on_hold]);
        assert!(ics.contains("Planned Block"));
        assert!(!ics.contains("Moved Block"));
        assert!(!ics.contains("Skipped Block"));
        assert!(!ics.contains("Canceled Block"));
        assert!(!ics.contains("On Hold Block"));
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 19, hour, minute, 0).unwrap()
    }

    #[test]
    fn resize_bottom_extends_end_only() {
        // 5–6 PM, drag the bottom edge down an hour → 5–7 PM, start unchanged.
        let (start, end) = resize_block(at(17, 0), at(18, 0), BlockEdge::End, 60);
        assert_eq!(start, at(17, 0));
        assert_eq!(end, at(19, 0));
    }

    #[test]
    fn resize_top_moves_start_earlier_only() {
        // 5–7 PM, drag the top edge up an hour → 4–7 PM, end unchanged.
        let (start, end) = resize_block(at(17, 0), at(19, 0), BlockEdge::Start, -60);
        assert_eq!(start, at(16, 0));
        assert_eq!(end, at(19, 0));
    }

    #[test]
    fn resize_cannot_invert_or_go_below_minimum() {
        // Dragging the end far up keeps at least MIN_BLOCK_MINUTES after the start.
        let (start, end) = resize_block(at(17, 0), at(19, 0), BlockEdge::End, -600);
        assert_eq!(start, at(17, 0));
        assert_eq!(end, at(17, 0) + Duration::minutes(MIN_BLOCK_MINUTES));
        // Dragging the start far down keeps at least MIN_BLOCK_MINUTES before the end.
        let (start, end) = resize_block(at(17, 0), at(19, 0), BlockEdge::Start, 600);
        assert_eq!(end, at(19, 0));
        assert_eq!(start, at(19, 0) - Duration::minutes(MIN_BLOCK_MINUTES));
    }

    #[test]
    fn resize_snaps_to_increment() {
        // A 20-minute drag snaps to 15; a 23-minute drag snaps to 30.
        assert_eq!(snap_to_increment(20), 15);
        assert_eq!(snap_to_increment(23), 30);
        let (_, end) = resize_block(at(17, 0), at(18, 0), BlockEdge::End, 23);
        assert_eq!(end, at(18, 30));
    }

    #[test]
    fn move_preserves_duration_and_snaps() {
        // A 23-minute drag snaps to 30; both edges shift, duration unchanged.
        let (start, end) = move_block(at(17, 0), at(18, 30), 23);
        assert_eq!(start, at(17, 30));
        assert_eq!(end, at(19, 0));
        assert_eq!((end - start).num_minutes(), 90);
    }

    #[test]
    fn pixels_to_minutes_converts_against_hour_height() {
        assert_eq!(pixels_to_minutes(56.0, 56.0), 60);
        assert_eq!(pixels_to_minutes(28.0, 56.0), 30);
        assert_eq!(pixels_to_minutes(-56.0, 56.0), -60);
        // Degenerate hour height never divides by zero.
        assert_eq!(pixels_to_minutes(10.0, 0.0), 0);
    }

    #[test]
    fn block_layout_spans_real_duration() {
        // 5–7 PM on an 8 AM-start timeline at 56px/hour: top = 9h, height = 2h.
        let (top, height) = block_pixel_layout(17 * 60, 19 * 60, 8, 56.0, 30.0);
        assert_eq!(top, 9.0 * 56.0);
        assert_eq!(height, 2.0 * 56.0);
        // A 15-minute block is floored to the minimum readable height.
        let (_, short) = block_pixel_layout(17 * 60, 17 * 60 + 15, 8, 56.0, 30.0);
        assert_eq!(short, 30.0);
    }

    #[test]
    fn columns_pack_overlaps_and_reset_between_clusters() {
        // Two overlapping blocks share a 2-column cluster; a later, separate block
        // starts a fresh single-column cluster.
        let layout = layout_columns(&[(540, 660), (600, 720), (780, 840)]);
        assert_eq!(layout, vec![(0, 2), (1, 2), (0, 1)]);
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
