use crate::{
    models::{BoardState, ScoredTask, TaskContext, TaskStatus},
    scoring::{ScoringWeights, score_task},
};
use chrono::{DateTime, Duration, Utc};
use std::cmp::Ordering;

/// A scored task's effective scheduled start (explicit block start, else the
/// legacy single `scheduled_at`), used to order the upcoming sections by time.
fn scheduled_start(task: &ScoredTask) -> Option<DateTime<Utc>> {
    task.context
        .task
        .scheduled_start_at
        .or(task.context.task.scheduled_at)
}

/// Ordering for the "upcoming" sections (Up Next, Later Today): scheduled tasks
/// come first, earliest scheduled time wins, and urgency (higher first) breaks
/// ties — including between two unscheduled tasks, so P1 still outranks P5.
fn upcoming_cmp(a: &ScoredTask, b: &ScoredTask) -> Ordering {
    match (scheduled_start(a), scheduled_start(b)) {
        (Some(a_start), Some(b_start)) => a_start
            .cmp(&b_start)
            .then_with(|| b.urgency_score.cmp(&a.urgency_score)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b.urgency_score.cmp(&a.urgency_score),
    }
}

pub fn build_board(tasks: Vec<TaskContext>, now: DateTime<Utc>) -> BoardState {
    let mut board = BoardState {
        generated_at: now,
        ..Default::default()
    };

    for context in tasks {
        let status = context.task.status;
        if status == TaskStatus::Canceled {
            continue;
        }
        if status == TaskStatus::Done {
            if context
                .task
                .completed_at
                .is_some_and(|at| at.date_naive() == now.date_naive())
            {
                board.done_today.push(scored(context, now));
            }
            continue;
        }
        let scheduled_start = context
            .task
            .scheduled_start_at
            .or(context.task.scheduled_at);
        let scheduled_end = context.task.scheduled_end_at;
        let scheduled_block_active = scheduled_start.is_some_and(|start| start <= now)
            && scheduled_end.is_some_and(|end| now < end);
        let scheduled_block_elapsed = scheduled_end.is_some_and(|end| end <= now);
        let legacy_schedule_due =
            scheduled_end.is_none() && scheduled_start.is_some_and(|start| start <= now);
        let scheduled_later_today = scheduled_start
            .is_some_and(|start| start > now && start.date_naive() == now.date_naive());

        if matches!(status, TaskStatus::Blocked | TaskStatus::Waiting) {
            board.waiting_blocked.push(scored(context, now));
        } else if context.task.due_at.is_some_and(|at| at < now) || scheduled_block_elapsed {
            board.overdue.push(scored(context, now));
        } else if status == TaskStatus::InProgress || scheduled_block_active || legacy_schedule_due
        {
            board.now.push(scored(context, now));
        } else if scheduled_later_today {
            board.later_today.push(scored(context, now));
        } else if context
            .task
            .due_at
            .is_some_and(|at| at - now <= Duration::days(1))
        {
            board.due_soon.push(scored(context, now));
        } else {
            board.next_up.push(scored(context, now));
        }
    }

    // Most columns rank by raw urgency. The upcoming sections instead lead with
    // scheduled work in time order (see `upcoming_cmp`) so a task with a clear
    // upcoming time is never buried behind a higher-priority unscheduled task.
    for column in [
        &mut board.now,
        &mut board.due_soon,
        &mut board.waiting_blocked,
        &mut board.overdue,
        &mut board.done_today,
    ] {
        column.sort_by(|a, b| b.urgency_score.cmp(&a.urgency_score));
    }
    board.next_up.sort_by(upcoming_cmp);
    board.later_today.sort_by(upcoming_cmp);
    board
}

fn scored(context: TaskContext, now: DateTime<Utc>) -> ScoredTask {
    let urgency_score = score_task(&context, now, ScoringWeights::default());
    ScoredTask {
        context,
        urgency_score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProjectStatus, ProjectType, Task};
    use chrono::TimeZone;

    fn task(id: &str, status: TaskStatus, now: DateTime<Utc>) -> TaskContext {
        TaskContext {
            task: Task {
                id: id.into(),
                project_id: "project".into(),
                title: id.into(),
                description: None,
                status,
                priority: 3,
                due_at: None,
                scheduled_at: None,
                scheduled_start_at: None,
                scheduled_end_at: None,
                deadline_at: None,
                reminder_at: None,
                recurrence_rule: None,
                recurrence_anchor_at: None,
                recurrence_timezone: None,
                calendar_block_id: None,
                started_at: None,
                completed_at: None,
                estimated_minutes: None,
                time_limit_minutes: None,
                pinned: false,
                blocked_reason: None,
                tags: vec![],
                created_at: now,
                updated_at: now,
            },
            project_name: "Project".into(),
            project_type: ProjectType::Software,
            project_status: ProjectStatus::Active,
            project_priority: 3,
            organization_name: "Personal".into(),
            organization_color: None,
        }
    }

    #[test]
    fn groups_fixed_states_and_filters_terminal_work() {
        let now = Utc::now();
        let mut overdue = task("overdue", TaskStatus::Ready, now);
        overdue.task.due_at = Some(now - Duration::hours(1));
        let mut done_today = task("done-today", TaskStatus::Done, now);
        done_today.task.completed_at = Some(now);
        let mut done_before = task("done-before", TaskStatus::Done, now);
        done_before.task.completed_at = Some(now - Duration::days(1));
        let board = build_board(
            vec![
                overdue,
                task("blocked", TaskStatus::Blocked, now),
                done_today,
                done_before,
                task("canceled", TaskStatus::Canceled, now),
            ],
            now,
        );
        assert_eq!(board.overdue[0].context.task.id, "overdue");
        assert_eq!(board.waiting_blocked[0].context.task.id, "blocked");
        assert_eq!(board.done_today.len(), 1);
        assert_eq!(board.done_today[0].context.task.id, "done-today");
    }

    #[test]
    fn sorts_columns_by_score() {
        let now = Utc::now();
        let normal = task("normal", TaskStatus::Ready, now);
        let mut pinned = task("pinned", TaskStatus::Ready, now);
        pinned.task.priority = 1;
        pinned.task.pinned = true;
        let board = build_board(vec![normal, pinned], now);
        assert_eq!(board.next_up[0].context.task.id, "pinned");
    }

    /// A scheduled task lands in the right urgency column based on where the clock
    /// sits relative to its time block: active → NOW, future-today → Later Today,
    /// elapsed/unfinished → Overdue, completed → Done Today.
    #[test]
    fn scheduled_tasks_group_by_their_time_block() {
        let now = Utc
            .with_ymd_and_hms(2026, 6, 19, 12, 0, 0)
            .single()
            .unwrap();

        let mut active = task("active", TaskStatus::Scheduled, now);
        active.task.scheduled_start_at = Some(now - Duration::minutes(5));
        active.task.scheduled_end_at = Some(now + Duration::minutes(25));

        let mut later = task("later", TaskStatus::Scheduled, now);
        later.task.scheduled_start_at = Some(now + Duration::hours(2));
        later.task.scheduled_end_at = Some(now + Duration::hours(3));

        let mut elapsed = task("elapsed", TaskStatus::Scheduled, now);
        elapsed.task.scheduled_start_at = Some(now - Duration::hours(2));
        elapsed.task.scheduled_end_at = Some(now - Duration::hours(1));

        let mut exact_end = task("exact-end", TaskStatus::Scheduled, now);
        exact_end.task.scheduled_start_at = Some(now - Duration::hours(1));
        exact_end.task.scheduled_end_at = Some(now);

        let mut done = task("done", TaskStatus::Done, now);
        done.task.scheduled_start_at = Some(now - Duration::hours(2));
        done.task.scheduled_end_at = Some(now - Duration::hours(1));
        done.task.completed_at = Some(now);

        let mut tomorrow = task("tomorrow", TaskStatus::Scheduled, now);
        tomorrow.task.scheduled_start_at = Some(now + Duration::days(1));
        tomorrow.task.scheduled_end_at = Some(now + Duration::days(1) + Duration::hours(1));

        let board = build_board(vec![active, later, elapsed, exact_end, done, tomorrow], now);
        assert!(board.now.iter().any(|t| t.context.task.id == "active"));
        assert!(
            board
                .later_today
                .iter()
                .any(|t| t.context.task.id == "later")
        );
        assert!(board.overdue.iter().any(|t| t.context.task.id == "elapsed"));
        assert!(
            board
                .overdue
                .iter()
                .any(|t| t.context.task.id == "exact-end")
        );
        assert!(board.done_today.iter().any(|t| t.context.task.id == "done"));
        assert!(
            !board
                .later_today
                .iter()
                .any(|t| t.context.task.id == "tomorrow")
        );
    }

    /// Within a single urgency column (here NOW), a P1 task must outrank a P5 one.
    #[test]
    fn p1_outranks_p5_within_now_column() {
        let now = Utc::now();
        let mut p5 = task("p5", TaskStatus::InProgress, now);
        p5.task.priority = 5;
        let mut p1 = task("p1", TaskStatus::InProgress, now);
        p1.task.priority = 1;
        // Insert lowest-priority first so ordering can only come from scoring.
        let board = build_board(vec![p5, p1], now);
        assert_eq!(board.now[0].context.task.id, "p1");
        assert_eq!(board.now[1].context.task.id, "p5");
    }

    /// Up Next leads with scheduled work in time order, then ranks the rest by
    /// priority/urgency: a scheduled 9 AM P3 beats a scheduled 10 AM P1, both beat
    /// an unscheduled P1, and unscheduled P1 still beats unscheduled P5.
    #[test]
    fn up_next_orders_scheduled_by_time_then_priority() {
        let now = Utc
            .with_ymd_and_hms(2026, 6, 19, 12, 0, 0)
            .single()
            .unwrap();
        let nine = Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).single().unwrap();
        let ten = Utc
            .with_ymd_and_hms(2026, 6, 20, 10, 0, 0)
            .single()
            .unwrap();

        let mut sched_9_p3 = task("sched-9-p3", TaskStatus::Scheduled, now);
        sched_9_p3.task.priority = 3;
        sched_9_p3.task.scheduled_start_at = Some(nine);
        sched_9_p3.task.scheduled_end_at = Some(nine + Duration::hours(1));

        let mut sched_10_p1 = task("sched-10-p1", TaskStatus::Scheduled, now);
        sched_10_p1.task.priority = 1;
        sched_10_p1.task.scheduled_start_at = Some(ten);
        sched_10_p1.task.scheduled_end_at = Some(ten + Duration::hours(1));

        let mut unsched_p1 = task("unsched-p1", TaskStatus::Ready, now);
        unsched_p1.task.priority = 1;
        let mut unsched_p5 = task("unsched-p5", TaskStatus::Ready, now);
        unsched_p5.task.priority = 5;

        // Inserted out of final order so ordering can only come from the sort.
        let board = build_board(vec![unsched_p5, sched_10_p1, unsched_p1, sched_9_p3], now);
        let order: Vec<&str> = board
            .next_up
            .iter()
            .map(|t| t.context.task.id.as_str())
            .collect();
        assert_eq!(
            order,
            vec!["sched-9-p3", "sched-10-p1", "unsched-p1", "unsched-p5"]
        );
    }

    /// Two tasks scheduled at the same time fall back to priority/urgency: P1 first.
    #[test]
    fn up_next_breaks_scheduled_ties_by_priority() {
        let now = Utc
            .with_ymd_and_hms(2026, 6, 19, 12, 0, 0)
            .single()
            .unwrap();
        let nine = Utc.with_ymd_and_hms(2026, 6, 20, 9, 0, 0).single().unwrap();

        let mut tie_p3 = task("tie-p3", TaskStatus::Scheduled, now);
        tie_p3.task.priority = 3;
        tie_p3.task.scheduled_start_at = Some(nine);
        tie_p3.task.scheduled_end_at = Some(nine + Duration::hours(1));
        let mut tie_p1 = task("tie-p1", TaskStatus::Scheduled, now);
        tie_p1.task.priority = 1;
        tie_p1.task.scheduled_start_at = Some(nine);
        tie_p1.task.scheduled_end_at = Some(nine + Duration::hours(1));

        let board = build_board(vec![tie_p3, tie_p1], now);
        assert_eq!(board.next_up[0].context.task.id, "tie-p1");
        assert_eq!(board.next_up[1].context.task.id, "tie-p3");
    }
}
