use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NewDispatch {
    pub task_id: String,
    pub project_id: String,
    pub repo: String,
    pub branch: String,
    pub prompt: String,
    pub provider: String,
    pub model: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Event {
    pub state: String,
    pub at: String,
    pub note: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Dispatch {
    pub dispatch_id: String,
    #[serde(flatten)]
    pub input: NewDispatch,
    #[serde(skip_serializing)]
    pub owner_sub: String,
    pub state: String,
    pub pi_job_id: Option<String>,
    pub pr_url: Option<String>,
    pub activity: String,
    pub created_at: String,
    pub updated_at: String,
    pub events: Vec<Event>,
}
pub struct Registry(Mutex<Connection>);
impl Registry {
    pub fn open(path: &str) -> Result<Self> {
        let c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS dispatches (dispatch_id TEXT PRIMARY KEY, owner_sub TEXT NOT NULL, task_id TEXT NOT NULL, project_id TEXT NOT NULL, repo TEXT NOT NULL, branch TEXT NOT NULL, prompt TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL, state TEXT NOT NULL, pi_job_id TEXT, pr_url TEXT, activity TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS dispatch_owner_task ON dispatches(owner_sub,task_id,created_at);
            CREATE TABLE IF NOT EXISTS dispatch_events (id INTEGER PRIMARY KEY, dispatch_id TEXT NOT NULL REFERENCES dispatches(dispatch_id), state TEXT NOT NULL, at TEXT NOT NULL, note TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS dispatch_jobs (dispatch_id TEXT NOT NULL REFERENCES dispatches(dispatch_id), pi_job_id TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('initial','continuation')), PRIMARY KEY(dispatch_id,pi_job_id));")?;
        Ok(Self(Mutex::new(c)))
    }
    pub fn create(&self, owner: String, input: NewDispatch) -> Result<Dispatch> {
        let now = Utc::now().to_rfc3339();
        let d = Dispatch {
            dispatch_id: uuid::Uuid::new_v4().to_string(),
            input,
            owner_sub: owner,
            state: "dispatched".into(),
            pi_job_id: None,
            pr_url: None,
            activity: String::new(),
            created_at: now.clone(),
            updated_at: now,
            events: vec![],
        };
        let mut c = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("registry lock"))?;
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO dispatches VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,NULL,?11,?12,?12)",
            params![
                d.dispatch_id,
                d.owner_sub,
                d.input.task_id,
                d.input.project_id,
                d.input.repo,
                d.input.branch,
                d.input.prompt,
                d.input.provider,
                d.input.model,
                d.state,
                d.activity,
                d.created_at
            ],
        )?;
        tx.execute("INSERT INTO dispatch_events(dispatch_id,state,at,note) VALUES (?1,?2,?3,'Dispatch requested')", params![d.dispatch_id,d.state,d.created_at])?;
        tx.commit()?;
        Ok(d)
    }
    pub fn save(
        &self,
        d: &mut Dispatch,
        state: &str,
        note: &str,
        job: Option<(&str, &str)>,
    ) -> Result<()> {
        let mut c = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("registry lock"))?;
        let tx = c.transaction()?;
        d.updated_at = Utc::now().to_rfc3339();
        if d.state != state || job.is_some() {
            tx.execute(
                "INSERT INTO dispatch_events(dispatch_id,state,at,note) VALUES (?1,?2,?3,?4)",
                params![d.dispatch_id, state, d.updated_at, note],
            )?;
        }
        if let Some((id, kind)) = job {
            tx.execute(
                "INSERT INTO dispatch_jobs VALUES (?1,?2,?3)",
                params![d.dispatch_id, id, kind],
            )?;
            d.pi_job_id = Some(id.into());
        }
        tx.execute("UPDATE dispatches SET state=?2, pi_job_id=?3, pr_url=?4, activity=?5, updated_at=?6 WHERE dispatch_id=?1",params![d.dispatch_id,state,d.pi_job_id,d.pr_url,d.activity,d.updated_at])?;
        tx.commit()?;
        d.state = state.into();
        Ok(())
    }
    pub fn all(&self) -> Result<Vec<Dispatch>> {
        self.query("ORDER BY created_at DESC, rowid DESC", [])
    }
    fn query(&self, clause: &str, params: impl rusqlite::Params) -> Result<Vec<Dispatch>> {
        let c = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("registry lock"))?;
        let mut stmt = c.prepare(&format!("SELECT dispatch_id,owner_sub,task_id,project_id,repo,branch,prompt,provider,model,state,pi_job_id,pr_url,activity,created_at,updated_at FROM dispatches {clause}"))?;
        let rows = stmt.query_map(params, |r| {
            Ok(Dispatch {
                dispatch_id: r.get(0)?,
                owner_sub: r.get(1)?,
                input: NewDispatch {
                    task_id: r.get(2)?,
                    project_id: r.get(3)?,
                    repo: r.get(4)?,
                    branch: r.get(5)?,
                    prompt: r.get(6)?,
                    provider: r.get(7)?,
                    model: r.get(8)?,
                },
                state: r.get(9)?,
                pi_job_id: r.get(10)?,
                pr_url: r.get(11)?,
                activity: r.get(12)?,
                created_at: r.get(13)?,
                updated_at: r.get(14)?,
                events: vec![],
            })
        })?;
        let mut result = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for d in &mut result {
            let mut events = c.prepare(
                "SELECT state,at,note FROM dispatch_events WHERE dispatch_id=?1 ORDER BY id",
            )?;
            d.events = events
                .query_map([&d.dispatch_id], |r| {
                    Ok(Event {
                        state: r.get(0)?,
                        at: r.get(1)?,
                        note: r.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(result)
    }
    pub fn owned(&self, id: &str, owner: &str) -> Result<Dispatch> {
        self.query("WHERE dispatch_id=?1 AND owner_sub=?2", params![id, owner])?
            .into_iter()
            .next()
            .context("dispatch not found")
    }
    pub fn latest_by_task(&self, owner: &str, task: &str) -> Result<Option<Dispatch>> {
        Ok(self
            .query(
                "WHERE owner_sub=?1 AND task_id=?2 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![owner, task],
            )?
            .into_iter()
            .next())
    }
}
