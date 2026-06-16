use crate::models::{ProjectStatus, TaskContext, TaskStatus};
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy)]
pub struct ScoringWeights {
    pub priority_step: i32,
    pub project_priority_step: i32,
    pub pinned: i32,
    pub overdue_base: i32,
    pub overdue_per_day: i32,
    pub due_within_hour: i32,
    pub due_today: i32,
    pub due_tomorrow: i32,
    pub in_progress: i32,
    pub ready: i32,
    pub blocked: i32,
    pub waiting: i32,
    pub paused_project: i32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            priority_step: 12,
            project_priority_step: 3,
            pinned: 100,
            overdue_base: 90,
            overdue_per_day: 8,
            due_within_hour: 60,
            due_today: 40,
            due_tomorrow: 20,
            in_progress: 55,
            ready: 15,
            blocked: -45,
            waiting: -35,
            paused_project: -50,
        }
    }
}

/// Highest (most urgent) selectable priority. Priorities run P1..P5 where a
/// **lower number is more urgent**, so P1 is the highest priority and P5 the
/// lowest/backlog.
pub const HIGHEST_PRIORITY: i32 = 1;
/// Lowest (least urgent) selectable priority.
pub const LOWEST_PRIORITY: i32 = 5;

/// Convert a 1–5 priority (P1 highest) into an urgency rank where a higher
/// number means more urgent: P1 → 5, P3 → 3, P5 → 1. Scoring multiplies this
/// rank (not the raw priority) so a P1 task always outscores a P5 task. Stray
/// values are clamped into the valid range.
pub fn priority_rank(priority: i32) -> i32 {
    LOWEST_PRIORITY + 1 - priority.clamp(HIGHEST_PRIORITY, LOWEST_PRIORITY)
}

pub fn score_task(task: &TaskContext, now: DateTime<Utc>, weights: ScoringWeights) -> i32 {
    let mut score = priority_rank(task.task.priority) * weights.priority_step
        + priority_rank(task.project_priority) * weights.project_priority_step;
    if task.task.pinned {
        score += weights.pinned;
    }
    score += match task.task.status {
        TaskStatus::InProgress => weights.in_progress,
        TaskStatus::Ready => weights.ready,
        TaskStatus::Blocked => weights.blocked,
        TaskStatus::Waiting => weights.waiting,
        _ => 0,
    };
    if task.project_status == ProjectStatus::Paused {
        score += weights.paused_project;
    }
    if let Some(due) = task.task.due_at {
        let until = due - now;
        score += if until < Duration::zero() {
            weights.overdue_base + (-until.num_days() as i32 * weights.overdue_per_day)
        } else if until <= Duration::hours(1) {
            weights.due_within_hour
        } else if until <= Duration::days(1) {
            weights.due_today
        } else if until <= Duration::days(2) {
            weights.due_tomorrow
        } else {
            0
        };
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProjectType, Task};

    fn context() -> TaskContext {
        let now = Utc::now();
        TaskContext {
            task: Task {
                id: "task".into(),
                project_id: "project".into(),
                title: "Test".into(),
                description: None,
                status: TaskStatus::Ready,
                priority: 3,
                due_at: None,
                scheduled_at: None,
                started_at: None,
                completed_at: None,
                estimated_minutes: Some(30),
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
    fn pinned_overdue_in_progress_gets_major_boost() {
        let now = Utc::now();
        let baseline = score_task(&context(), now, ScoringWeights::default());
        let mut urgent = context();
        urgent.task.pinned = true;
        urgent.task.status = TaskStatus::InProgress;
        urgent.task.due_at = Some(now - Duration::days(1));
        assert!(score_task(&urgent, now, ScoringWeights::default()) > baseline + 200);
    }

    #[test]
    fn p1_outranks_p5() {
        let now = Utc::now();
        let mut highest = context();
        highest.task.priority = 1;
        let mut lowest = context();
        lowest.task.priority = 5;
        assert!(
            score_task(&highest, now, ScoringWeights::default())
                > score_task(&lowest, now, ScoringWeights::default()),
            "P1 must score higher than P5"
        );
    }

    #[test]
    fn blocked_paused_work_is_penalized() {
        let now = Utc::now();
        let baseline = score_task(&context(), now, ScoringWeights::default());
        let mut paused = context();
        paused.task.status = TaskStatus::Blocked;
        paused.project_status = ProjectStatus::Paused;
        assert!(score_task(&paused, now, ScoringWeights::default()) < baseline);
    }
}
