//! Workspace pages. Each module owns one top-level section of the app.

mod board;
mod dashboard;
mod organizations;
mod project_detail;
mod projects;
mod tasks;
mod today;

pub use board::BoardPage;
pub use dashboard::Dashboard;
pub use organizations::OrganizationsPage;
pub use project_detail::ProjectDetailPage;
pub use projects::ProjectsPage;
pub use tasks::TasksPage;
pub use today::TodayPage;
