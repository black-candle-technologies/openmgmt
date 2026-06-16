//! Workspace pages. Each module owns one top-level section of the app.

mod board;
mod daily_ops;
mod dashboard;
mod organizations;
mod project_detail;
mod projects;
mod settings;
mod sync;
mod tasks;
mod today;

pub use board::BoardPage;
pub use daily_ops::DailyOpsPage;
pub use dashboard::Dashboard;
pub use organizations::OrganizationsPage;
pub use project_detail::ProjectDetailPage;
pub use projects::ProjectsPage;
pub use settings::SettingsPage;
pub use sync::SyncPage;
pub use tasks::TasksPage;
pub use today::TodayPage;
