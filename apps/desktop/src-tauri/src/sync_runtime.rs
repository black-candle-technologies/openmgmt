use openmgmt_core::{Database, SyncConnectionState, SyncSettings, SyncSettingsPatch, SyncStatus};
use openmgmt_sync_client::{SyncClientError, SyncClientResult, SyncOnceResult};
use std::{
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, mpsc, watch},
    time::MissedTickBehavior,
};

const STARTUP_SYNC_DELAY: Duration = Duration::from_secs(3);
const PERIODIC_SYNC_INTERVAL: Duration = Duration::from_secs(5 * 60);
const MUTATION_SYNC_DEBOUNCE: Duration = Duration::from_secs(10);
const INITIAL_FAILURE_BACKOFF: Duration = Duration::from_secs(30);
const MAX_FAILURE_BACKOFF: Duration = Duration::from_secs(10 * 60);
pub const SYNC_ALREADY_RUNNING_ERROR: &str = "sync is already running";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncReadiness {
    Disabled,
    NotConfigured,
    Ready,
}

pub fn sync_readiness(settings: &SyncSettings) -> SyncReadiness {
    if !settings.enabled {
        SyncReadiness::Disabled
    } else if settings.server_url.is_none() {
        SyncReadiness::NotConfigured
    } else {
        SyncReadiness::Ready
    }
}

pub fn patch_changes_server_url(patch: &SyncSettingsPatch) -> bool {
    patch.server_url.is_some()
}

#[derive(Debug, Default)]
pub struct SyncScheduler {
    pending: bool,
    current_backoff: Option<Duration>,
    backoff_until: Option<Instant>,
    mutation_deadline: Option<Instant>,
    mutation_task_scheduled: bool,
    retry_scheduled: bool,
}

impl SyncScheduler {
    pub fn mark_pending(&mut self) {
        self.pending = true;
    }

    pub fn take_pending(&mut self) -> bool {
        let pending = self.pending;
        self.pending = false;
        pending
    }

    pub fn record_failure(&mut self, now: Instant) {
        let next_backoff = self
            .current_backoff
            .map(|duration| (duration * 2).min(MAX_FAILURE_BACKOFF))
            .unwrap_or(INITIAL_FAILURE_BACKOFF);
        self.current_backoff = Some(next_backoff);
        self.backoff_until = Some(now + next_backoff);
        self.retry_scheduled = false;
    }

    pub fn record_success(&mut self) {
        self.current_backoff = None;
        self.backoff_until = None;
        self.retry_scheduled = false;
    }

    pub fn reset_backoff(&mut self) {
        self.record_success();
    }

    pub fn backoff_remaining(&self, now: Instant) -> Option<Duration> {
        self.backoff_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
    }

    pub fn schedule_mutation(&mut self, now: Instant, debounce: Duration) {
        self.mutation_deadline = Some(now + debounce);
    }

    pub fn mutation_deadline(&self) -> Option<Instant> {
        self.mutation_deadline
    }

    fn clear_mutation_deadline(&mut self) {
        self.mutation_deadline = None;
    }

    fn mark_mutation_task_scheduled(&mut self) -> bool {
        if self.mutation_task_scheduled {
            false
        } else {
            self.mutation_task_scheduled = true;
            true
        }
    }

    fn clear_mutation_task_scheduled(&mut self) {
        self.mutation_task_scheduled = false;
    }

    fn mark_retry_scheduled(&mut self) -> bool {
        if self.retry_scheduled {
            false
        } else {
            self.retry_scheduled = true;
            true
        }
    }

    fn clear_retry_scheduled(&mut self) {
        self.retry_scheduled = false;
    }
}

#[derive(Debug, Clone, Copy)]
enum BackgroundSyncTrigger {
    Startup,
    Periodic,
    Mutation,
    SettingsChanged,
    Pending,
    BackoffExpired,
}

#[derive(Clone)]
pub struct SyncRuntime {
    inner: Arc<SyncRuntimeInner>,
}

struct SyncRuntimeInner {
    database: Database,
    sync_lock: Mutex<()>,
    syncing: AtomicBool,
    scheduler: StdMutex<SyncScheduler>,
    trigger_tx: mpsc::UnboundedSender<BackgroundSyncTrigger>,
    shutdown_tx: watch::Sender<bool>,
}

impl SyncRuntime {
    pub fn new(database: Database) -> Self {
        let (trigger_tx, trigger_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let inner = Arc::new(SyncRuntimeInner {
            database,
            sync_lock: Mutex::new(()),
            syncing: AtomicBool::new(false),
            scheduler: StdMutex::new(SyncScheduler::default()),
            trigger_tx,
            shutdown_tx,
        });
        Self::spawn_background_loop(inner.clone(), trigger_rx, shutdown_rx);
        Self { inner }
    }

    pub fn trigger_startup_sync(&self) {
        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(STARTUP_SYNC_DELAY).await;
            tracing::info!("startup background sync scheduled");
            inner.send_trigger(BackgroundSyncTrigger::Startup);
        });
    }

    pub fn trigger_mutation_sync(&self) {
        let should_spawn = self.with_scheduler(|scheduler| {
            scheduler.schedule_mutation(Instant::now(), MUTATION_SYNC_DEBOUNCE);
            scheduler.mark_mutation_task_scheduled()
        });
        tracing::info!("mutation-triggered sync scheduled");
        if should_spawn {
            Self::spawn_mutation_debounce(self.inner.clone());
        }
    }

    pub fn trigger_settings_sync(&self) {
        self.inner
            .send_trigger(BackgroundSyncTrigger::SettingsChanged);
    }

    pub fn reset_backoff(&self) {
        self.with_scheduler(|scheduler| scheduler.reset_backoff());
    }

    pub fn shutdown(&self) {
        let _ = self.inner.shutdown_tx.send(true);
    }

    pub fn is_syncing(&self) -> bool {
        self.inner.syncing.load(Ordering::SeqCst)
    }

    pub fn with_runtime_status(&self, mut status: SyncStatus) -> SyncStatus {
        if self.is_syncing() {
            status.state = SyncConnectionState::Syncing;
        }
        status
    }

    pub async fn sync_now(&self) -> SyncClientResult<SyncOnceResult> {
        let _sync_guard = self
            .inner
            .sync_lock
            .try_lock()
            .map_err(|_| SyncClientError::Other(SYNC_ALREADY_RUNNING_ERROR.into()))?;
        let _running = RunningFlag::new(&self.inner.syncing);
        tracing::info!("manual sync started");
        let result = openmgmt_sync_client::sync_once(&self.inner.database).await;
        self.record_sync_result(&result);
        match &result {
            Ok(_) => tracing::info!("manual sync succeeded"),
            Err(error) if is_skip_error(error) => tracing::info!(%error, "manual sync skipped"),
            Err(error) => tracing::error!(%error, "manual sync failed"),
        }
        self.trigger_pending_after_sync();
        result
    }

    fn spawn_background_loop(
        inner: Arc<SyncRuntimeInner>,
        mut trigger_rx: mpsc::UnboundedReceiver<BackgroundSyncTrigger>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(PERIODIC_SYNC_INTERVAL);
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            tracing::info!("background sync runtime shutting down");
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        tracing::info!("periodic background sync scheduled");
                        Self::handle_background_trigger(inner.clone(), BackgroundSyncTrigger::Periodic).await;
                    }
                    trigger = trigger_rx.recv() => {
                        let Some(trigger) = trigger else { break; };
                        Self::handle_background_trigger(inner.clone(), trigger).await;
                    }
                }
            }
        });
    }

    fn spawn_mutation_debounce(inner: Arc<SyncRuntimeInner>) {
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(deadline) =
                    inner.with_scheduler(|scheduler| scheduler.mutation_deadline())
                else {
                    inner.with_scheduler(|scheduler| scheduler.clear_mutation_task_scheduled());
                    break;
                };
                let now = Instant::now();
                if let Some(remaining) = deadline.checked_duration_since(now) {
                    tokio::time::sleep(remaining).await;
                    continue;
                }
                inner.with_scheduler(|scheduler| {
                    scheduler.clear_mutation_deadline();
                    scheduler.clear_mutation_task_scheduled();
                });
                inner.send_trigger(BackgroundSyncTrigger::Mutation);
                break;
            }
        });
    }

    async fn handle_background_trigger(
        inner: Arc<SyncRuntimeInner>,
        trigger: BackgroundSyncTrigger,
    ) {
        let settings = match inner.database.get_sync_settings() {
            Ok(settings) => settings,
            Err(error) => {
                tracing::error!(%error, ?trigger, "background sync skipped because settings could not be loaded");
                return;
            }
        };
        match sync_readiness(&settings) {
            SyncReadiness::Disabled => {
                tracing::info!(?trigger, "background sync skipped because sync is disabled");
                return;
            }
            SyncReadiness::NotConfigured => {
                tracing::info!(
                    ?trigger,
                    "background sync skipped because sync is not configured"
                );
                return;
            }
            SyncReadiness::Ready => {}
        }

        let now = Instant::now();
        if let Some(remaining) = inner.with_scheduler(|scheduler| scheduler.backoff_remaining(now))
        {
            tracing::info!(
                ?trigger,
                backoff_seconds = remaining.as_secs(),
                "background sync backoff active"
            );
            inner.with_scheduler(|scheduler| scheduler.mark_pending());
            Self::schedule_after_backoff(inner, remaining);
            return;
        }

        let _sync_guard = match inner.sync_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::info!(
                    ?trigger,
                    "background sync coalesced because another sync is running"
                );
                inner.with_scheduler(|scheduler| scheduler.mark_pending());
                return;
            }
        };
        let _running = RunningFlag::new(&inner.syncing);
        tracing::info!(?trigger, "background sync started");
        let result = openmgmt_sync_client::sync_once(&inner.database).await;
        Self {
            inner: inner.clone(),
        }
        .record_sync_result(&result);
        match &result {
            Ok(_) => tracing::info!(?trigger, "background sync succeeded"),
            Err(error) if is_skip_error(error) => {
                tracing::info!(%error, ?trigger, "background sync skipped")
            }
            Err(error) => tracing::error!(%error, ?trigger, "background sync failed"),
        }
        drop(_running);
        drop(_sync_guard);
        Self { inner }.trigger_pending_after_sync();
    }

    fn schedule_after_backoff(inner: Arc<SyncRuntimeInner>, remaining: Duration) {
        let should_spawn = inner.with_scheduler(|scheduler| scheduler.mark_retry_scheduled());
        if !should_spawn {
            return;
        }
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(remaining).await;
            inner.with_scheduler(|scheduler| scheduler.clear_retry_scheduled());
            inner.send_trigger(BackgroundSyncTrigger::BackoffExpired);
        });
    }

    fn record_sync_result(&self, result: &SyncClientResult<SyncOnceResult>) {
        self.with_scheduler(|scheduler| match result {
            Ok(_) => scheduler.record_success(),
            Err(error) if is_skip_error(error) => {}
            Err(_) => scheduler.record_failure(Instant::now()),
        });
    }

    fn trigger_pending_after_sync(&self) {
        let pending = self.with_scheduler(|scheduler| scheduler.take_pending());
        if pending {
            self.inner.send_trigger(BackgroundSyncTrigger::Pending);
        }
    }

    fn with_scheduler<T>(&self, f: impl FnOnce(&mut SyncScheduler) -> T) -> T {
        self.inner.with_scheduler(f)
    }
}

impl Drop for SyncRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl SyncRuntimeInner {
    fn send_trigger(&self, trigger: BackgroundSyncTrigger) {
        let _ = self.trigger_tx.send(trigger);
    }

    fn with_scheduler<T>(&self, f: impl FnOnce(&mut SyncScheduler) -> T) -> T {
        let mut scheduler = self.scheduler.lock().expect("sync scheduler lock poisoned");
        f(&mut scheduler)
    }
}

struct RunningFlag<'a> {
    syncing: &'a AtomicBool,
}

impl<'a> RunningFlag<'a> {
    fn new(syncing: &'a AtomicBool) -> Self {
        syncing.store(true, Ordering::SeqCst);
        Self { syncing }
    }
}

impl Drop for RunningFlag<'_> {
    fn drop(&mut self) {
        self.syncing.store(false, Ordering::SeqCst);
    }
}

fn is_skip_error(error: &SyncClientError) -> bool {
    matches!(
        error,
        SyncClientError::Disabled | SyncClientError::NotConfigured
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmgmt_core::{Database, SyncSettings, SyncSettingsPatch};
    use std::time::{Duration, Instant};

    #[test]
    fn overlapping_syncs_are_prevented_by_shared_runtime_lock() {
        let (trigger_tx, _trigger_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let inner = SyncRuntimeInner {
            database: Database::in_memory().unwrap(),
            sync_lock: Mutex::new(()),
            syncing: AtomicBool::new(false),
            scheduler: StdMutex::new(SyncScheduler::default()),
            trigger_tx,
            shutdown_tx,
        };
        let _guard = inner.sync_lock.try_lock().expect("first sync starts");

        assert!(inner.sync_lock.try_lock().is_err());
    }

    #[test]
    fn multiple_background_triggers_coalesce_into_one_pending_sync() {
        let mut scheduler = SyncScheduler::default();

        scheduler.mark_pending();
        scheduler.mark_pending();
        scheduler.mark_pending();

        assert!(scheduler.take_pending());
        assert!(!scheduler.take_pending());
    }

    #[test]
    fn failed_sync_increases_backoff_up_to_maximum() {
        let mut scheduler = SyncScheduler::default();
        let now = Instant::now();

        scheduler.record_failure(now);
        assert_eq!(
            scheduler.backoff_remaining(now),
            Some(Duration::from_secs(30))
        );

        scheduler.record_failure(now + Duration::from_secs(30));
        assert_eq!(
            scheduler.backoff_remaining(now + Duration::from_secs(30)),
            Some(Duration::from_secs(60))
        );

        for offset in [90, 210, 450, 930, 1530] {
            scheduler.record_failure(now + Duration::from_secs(offset));
        }

        assert_eq!(
            scheduler.backoff_remaining(now + Duration::from_secs(1530)),
            Some(Duration::from_secs(600))
        );
    }

    #[test]
    fn successful_sync_resets_backoff() {
        let mut scheduler = SyncScheduler::default();
        let now = Instant::now();

        scheduler.record_failure(now);
        scheduler.record_success();

        assert_eq!(scheduler.backoff_remaining(now), None);
    }

    #[test]
    fn disabled_sync_is_skipped() {
        let settings = SyncSettings::default();

        assert_eq!(sync_readiness(&settings), SyncReadiness::Disabled);
    }

    #[test]
    fn enabled_but_not_configured_sync_is_skipped() {
        let settings = SyncSettings {
            enabled: true,
            ..Default::default()
        };

        assert_eq!(sync_readiness(&settings), SyncReadiness::NotConfigured);
    }

    #[test]
    fn configured_enabled_sync_is_ready() {
        let settings = SyncSettings {
            enabled: true,
            server_url: Some("http://127.0.0.1:8787".into()),
            ..Default::default()
        };

        assert_eq!(sync_readiness(&settings), SyncReadiness::Ready);
    }

    #[test]
    fn mutation_trigger_schedules_debounced_sync_and_extends_window() {
        let mut scheduler = SyncScheduler::default();
        let now = Instant::now();
        let debounce = Duration::from_secs(10);

        scheduler.schedule_mutation(now, debounce);
        assert_eq!(scheduler.mutation_deadline(), Some(now + debounce));

        scheduler.schedule_mutation(now + Duration::from_secs(4), debounce);
        assert_eq!(
            scheduler.mutation_deadline(),
            Some(now + Duration::from_secs(14))
        );
    }

    #[test]
    fn server_url_changes_are_detected_for_backoff_reset() {
        let patch = SyncSettingsPatch {
            server_url: Some(Some("http://127.0.0.1:8787".into())),
            ..Default::default()
        };

        assert!(patch_changes_server_url(&patch));
        assert!(!patch_changes_server_url(&SyncSettingsPatch::default()));
    }
}
