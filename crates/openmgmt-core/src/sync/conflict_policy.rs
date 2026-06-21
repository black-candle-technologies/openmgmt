use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictPolicy {
    pub organization: EntityConflictPolicy,
    pub project: EntityConflictPolicy,
    pub task: TaskConflictPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityConflictPolicy {
    pub normal_update: FieldMergeStrategy,
    pub archive_vs_update: ArchiveConflictStrategy,
    pub restore_behavior: RestoreConflictStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskConflictPolicy {
    pub normal_update: FieldMergeStrategy,
    pub status_update: StatusConflictStrategy,
    pub terminal_status_behavior: TerminalStatusConflictStrategy,
    pub archive_vs_update: ArchiveConflictStrategy,
    pub restore_behavior: RestoreConflictStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldMergeStrategy {
    LastWriteWinsWholeEntity,
    LastWriteWinsPerField,
    RecordConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveConflictStrategy {
    ArchiveWins,
    LastWriteWins,
    RecordConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreConflictStrategy {
    ExplicitRestoreOnly,
    LastWriteWins,
    RecordConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusConflictStrategy {
    ServerOrderWins,
    TerminalStatusProtected,
    RecordConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatusConflictStrategy {
    ProtectDoneCanceledArchived,
    ServerOrderWins,
    RecordConflict,
}

impl Default for EntityConflictPolicy {
    fn default() -> Self {
        Self {
            normal_update: FieldMergeStrategy::LastWriteWinsWholeEntity,
            archive_vs_update: ArchiveConflictStrategy::ArchiveWins,
            restore_behavior: RestoreConflictStrategy::ExplicitRestoreOnly,
        }
    }
}

impl Default for TaskConflictPolicy {
    fn default() -> Self {
        Self {
            normal_update: FieldMergeStrategy::LastWriteWinsWholeEntity,
            status_update: StatusConflictStrategy::ServerOrderWins,
            terminal_status_behavior: TerminalStatusConflictStrategy::ProtectDoneCanceledArchived,
            archive_vs_update: ArchiveConflictStrategy::ArchiveWins,
            restore_behavior: RestoreConflictStrategy::ExplicitRestoreOnly,
        }
    }
}
