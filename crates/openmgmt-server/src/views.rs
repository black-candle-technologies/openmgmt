//! Read-only presentation API, gated independently from the sync device-token protocol.
use crate::{
    state::AppState,
    store::{ServerStore, StoreError},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use openmgmt_protocol::AccountAuthError;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Debug)]
pub struct ViewError(StatusCode, &'static str);
impl IntoResponse for ViewError {
    fn into_response(self) -> Response {
        (
            self.0,
            [(header::CACHE_CONTROL, "private, no-store")],
            Json(serde_json::json!({"error":{"code":self.0.as_u16(),"message":self.1}})),
        )
            .into_response()
    }
}
impl From<StoreError> for ViewError {
    fn from(_: StoreError) -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "OpenMGMT data is unavailable.",
        )
    }
}
impl From<rusqlite::Error> for ViewError {
    fn from(_: rusqlite::Error) -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            "OpenMGMT data is unavailable.",
        )
    }
}
type Result<T> = std::result::Result<T, ViewError>;
fn bad() -> ViewError {
    ViewError(StatusCode::BAD_REQUEST, "Invalid query or cursor.")
}
fn response(value: impl Serialize) -> Response {
    (
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::VARY, "Authorization"),
        ],
        Json(value),
    )
        .into_response()
}

async fn account(
    state: &AppState,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<String> {
    let auth = state.account_auth.as_ref().ok_or(ViewError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Account-authenticated views are not enabled.",
    ))?;
    let bearer = bearer.ok_or(ViewError(StatusCode::UNAUTHORIZED, "Sign in required."))?;
    if bearer.token().len() > 4096 {
        return Err(bad());
    }
    let identity = auth
        .validate(bearer.token())
        .await
        .map_err(|error| match error {
            AccountAuthError::InvalidToken => {
                ViewError(StatusCode::UNAUTHORIZED, "Sign in required.")
            }
            AccountAuthError::EmailUnverified => {
                ViewError(StatusCode::FORBIDDEN, "Verified account required.")
            }
            AccountAuthError::Issuer(_) => ViewError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Account verification is unavailable.",
            ),
        })?;
    // Do not inherit the legacy identity-only/full-access default from the sync API.
    if !identity.email_verified
        || !identity
            .scope
            .split_whitespace()
            .any(|scope| scope == "openmgmt:tasks:read")
    {
        return Err(ViewError(
            StatusCode::FORBIDDEN,
            "OpenMGMT read permission required.",
        ));
    }
    if identity.user_id.is_empty() {
        return Err(ViewError(StatusCode::UNAUTHORIZED, "Sign in required."));
    }
    Ok(identity.user_id)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ViewQuery {
    #[serde(default = "task_kind")]
    pub kind: String,
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default = "active_filter")]
    pub filter_id: String,
    #[serde(default = "updated_sort")]
    pub sort_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default = "page_limit")]
    pub limit: u32,
}
fn task_kind() -> String {
    "task".into()
}
fn active_filter() -> String {
    "active".into()
}
fn updated_sort() -> String {
    "updated_desc".into()
}
fn page_limit() -> u32 {
    25
}
impl Default for ViewQuery {
    fn default() -> Self {
        Self {
            kind: task_kind(),
            q: None,
            filter_id: active_filter(),
            sort_id: updated_sort(),
            cursor: None,
            limit: page_limit(),
        }
    }
}
impl ViewQuery {
    fn validate(&mut self) -> Result<()> {
        if !matches!(self.kind.as_str(), "task" | "project")
            || !matches!(self.filter_id.as_str(), "active" | "archived" | "all")
            || !matches!(self.sort_id.as_str(), "updated_desc" | "title_asc")
            || !(1..=50).contains(&self.limit)
            || self.q.as_ref().is_some_and(|q| q.len() > 200)
            || self.cursor.as_ref().is_some_and(|q| q.len() > 4096)
        {
            return Err(bad());
        }
        self.q = self
            .q
            .take()
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());
        Ok(())
    }
}

#[derive(Serialize)]
struct Header {
    title: String,
    subtitle: String,
}
#[derive(Serialize)]
struct EmptyState {
    title: String,
    message: String,
}
#[derive(Serialize)]
struct OptionView {
    id: &'static str,
    label: &'static str,
}
#[derive(Debug, Serialize)]
pub struct ItemRow {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub status_label: String,
    pub route: String,
}
#[derive(Serialize)]
pub struct PageView {
    schema_version: u32,
    screen: Header,
    revision: String,
    pub rows: Vec<ItemRow>,
    filters: Vec<OptionView>,
    sorts: Vec<OptionView>,
    selected_filter_id: String,
    selected_sort_id: String,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    empty_state: Option<EmptyState>,
}
#[derive(Serialize)]
struct Field {
    label: String,
    value: String,
}
#[derive(Serialize)]
struct DetailView {
    schema_version: u32,
    screen: Header,
    revision: String,
    fields: Vec<Field>,
    actions: Vec<String>,
}
#[derive(Serialize)]
struct Section {
    id: String,
    title: String,
    component: &'static str,
    items: Vec<ItemRow>,
}
#[derive(Serialize)]
struct Metric {
    id: &'static str,
    label: &'static str,
    value: String,
    tone: &'static str,
    action: &'static str,
}
#[derive(Serialize)]
struct Overview {
    schema_version: u32,
    service_id: &'static str,
    screen: Header,
    availability: &'static str,
    metrics: Vec<Metric>,
    sections: Vec<Section>,
    capabilities: Vec<&'static str>,
    revision: String,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    account: String,
    query: ViewQuery,
    revision: i64,
    key: String,
    id: String,
}

fn revision(connection: &Connection, account: &str) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(revision),0) FROM mobile_projection WHERE account_id=?1",
        [account],
        |row| row.get(0),
    )?)
}
fn cursor_key(connection: &Connection) -> Result<String> {
    Ok(connection.query_row(
        "SELECT value FROM server_state WHERE key='mobile_cursor_key'",
        [],
        |row| row.get(0),
    )?)
}
fn encode_cursor(connection: &Connection, value: &Cursor) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| bad())?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(cursor_key(connection)?.as_bytes()).map_err(|_| bad())?;
    mac.update(&bytes);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&bytes),
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}
fn decode_cursor(connection: &Connection, raw: &str) -> Result<Cursor> {
    let (payload, signature) = raw.split_once('.').ok_or_else(bad)?;
    let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?;
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| bad())?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(cursor_key(connection)?.as_bytes()).map_err(|_| bad())?;
    mac.update(&payload);
    mac.verify_slice(&signature).map_err(|_| bad())?;
    serde_json::from_slice(&payload).map_err(|_| bad())
}
fn label(status: &str) -> String {
    status
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn time_label(timestamp: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Utc)
                .format("%b %-d, %Y %H:%M UTC")
                .to_string()
        })
        .unwrap_or_else(|_| "Date unavailable".into())
}
fn item(kind: &str, id: String, title: String, status: String, updated: String) -> ItemRow {
    let id = format!("{kind}_{id}");
    ItemRow {
        route: format!("services/openmgmt/items/{id}"),
        id,
        title,
        subtitle: format!("Updated {}", time_label(&updated)),
        status_label: label(&status),
    }
}

impl ServerStore {
    pub fn view_page(&self, account: &str, mut query: ViewQuery) -> Result<PageView> {
        query.validate()?;
        let connection = self.connection()?;
        let current_revision = revision(&connection, account)?;
        let mut signature_query = query.clone();
        signature_query.cursor = None;
        let cursor = query
            .cursor
            .as_deref()
            .map(|raw| decode_cursor(&connection, raw))
            .transpose()?;
        if let Some(cursor) = &cursor {
            if cursor.account != account || cursor.query != signature_query {
                return Err(bad());
            }
            if cursor.revision != current_revision {
                return Err(ViewError(
                    StatusCode::CONFLICT,
                    "Data changed. Restart this list.",
                ));
            }
        }
        let (order, key, comparison) = if query.sort_id == "title_asc" {
            (
                "title COLLATE NOCASE ASC,id ASC",
                "title COLLATE NOCASE",
                ">",
            )
        } else {
            ("updated_at DESC,id ASC", "updated_at", "<")
        };
        let sql=format!("SELECT id,title,status,updated_at,{key} FROM mobile_projection WHERE account_id=?1 AND kind=?2 AND valid=1
            AND instr(search_text,?3)>0 AND (?4='all' OR archived=CASE WHEN ?4='archived' THEN 1 ELSE 0 END)
            AND (?5 IS NULL OR {key} {comparison} ?5 OR ({key}=?5 AND id>?6)) ORDER BY {order} LIMIT ?7");
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement
            .query_map(
                params![
                    account,
                    query.kind,
                    query.q.as_deref().unwrap_or(""),
                    query.filter_id,
                    cursor.as_ref().map(|v| v.key.as_str()),
                    cursor.as_ref().map(|v| v.id.as_str()),
                    query.limit + 1
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more = rows.len() > query.limit as usize;
        if has_more {
            rows.pop();
        }
        let next_cursor = if has_more {
            let last = rows.last().unwrap();
            Some(encode_cursor(
                &connection,
                &Cursor {
                    account: account.into(),
                    query: signature_query,
                    revision: current_revision,
                    key: last.4.clone(),
                    id: last.0.clone(),
                },
            )?)
        } else {
            None
        };
        let rows = rows
            .into_iter()
            .map(|(id, title, status, updated, _)| item(&query.kind, id, title, status, updated))
            .collect::<Vec<_>>();
        let empty = rows.is_empty();
        Ok(PageView {
            schema_version: 1,
            screen: Header {
                title: if query.kind == "task" {
                    "Tasks"
                } else {
                    "Projects"
                }
                .into(),
                subtitle: String::new(),
            },
            revision: format!("openmgmt_{current_revision}"),
            rows,
            filters: vec![
                OptionView {
                    id: "active",
                    label: "Active",
                },
                OptionView {
                    id: "archived",
                    label: "Archived",
                },
                OptionView {
                    id: "all",
                    label: "All",
                },
            ],
            sorts: vec![
                OptionView {
                    id: "updated_desc",
                    label: "Recently updated",
                },
                OptionView {
                    id: "title_asc",
                    label: "Title",
                },
            ],
            selected_filter_id: query.filter_id,
            selected_sort_id: query.sort_id,
            next_cursor,
            has_more,
            empty_state: empty.then(|| EmptyState {
                title: "No matching items".into(),
                message: "Only account-owned synced data is shown.".into(),
            }),
        })
    }
    fn view_detail(&self, account: &str, kind: &str, id: &str) -> Result<DetailView> {
        if !matches!(kind, "task" | "project")
            || id.is_empty()
            || id.len() > 128
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(bad());
        }
        let connection = self.connection()?;
        let row=connection.query_row("SELECT title,description,status,updated_at FROM mobile_projection WHERE account_id=?1 AND kind=?2 AND id=?3 AND valid=1",params![account,kind,id],
            |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?))).optional()?
            .ok_or(ViewError(StatusCode::NOT_FOUND,"Item not found."))?;
        Ok(DetailView {
            schema_version: 1,
            screen: Header {
                title: row.0,
                subtitle: label(&row.2),
            },
            revision: format!("openmgmt_{}", revision(&connection, account)?),
            fields: vec![
                Field {
                    label: "Description".into(),
                    value: row.1,
                },
                Field {
                    label: "Status".into(),
                    value: label(&row.2),
                },
                Field {
                    label: "Updated".into(),
                    value: time_label(&row.3),
                },
            ],
            actions: vec![],
        })
    }
}

pub async fn items(
    State(state): State<AppState>,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    Query(query): Query<ViewQuery>,
) -> Result<Response> {
    let account = account(&state, bearer).await?;
    Ok(response(state.store.view_page(&account, query)?))
}
pub async fn detail(
    State(state): State<AppState>,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
    Path((kind, id)): Path<(String, String)>,
) -> Result<Response> {
    let account = account(&state, bearer).await?;
    Ok(response(state.store.view_detail(&account, &kind, &id)?))
}
pub async fn overview(
    State(state): State<AppState>,
    bearer: Option<TypedHeader<Authorization<Bearer>>>,
) -> Result<Response> {
    let account = account(&state, bearer).await?;
    let tasks = state.store.view_page(
        &account,
        ViewQuery {
            limit: 5,
            ..Default::default()
        },
    )?;
    let projects = state.store.view_page(
        &account,
        ViewQuery {
            kind: "project".into(),
            limit: 5,
            ..Default::default()
        },
    )?;
    let connection = state.store.connection()?;
    let invalid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM mobile_projection WHERE account_id=?1 AND valid=0)",
        [&account],
        |row| row.get(0),
    )?;
    // Aggregate the complete account projection, independently of the five-row previews.
    // This uses exactly the valid/non-archived visibility policy of the active list.
    let (task_count, project_count): (i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(kind='task'),0),COALESCE(SUM(kind='project'),0)
         FROM mobile_projection WHERE account_id=?1 AND valid=1 AND archived=0",
        [&account],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(response(Overview {
        schema_version: 1,
        service_id: "openmgmt",
        screen: Header {
            title: "OpenMGMT".into(),
            subtitle: if invalid {
                "Some synced items cannot be displayed by this server version."
            } else {
                "Account-owned synced projects and tasks"
            }
            .into(),
        },
        availability: if invalid { "partial" } else { "available" },
        metrics: vec![
            Metric {
                id: "task_count",
                label: "Active tasks",
                value: task_count.to_string(),
                tone: "muted",
                action: "open_openmgmt",
            },
            Metric {
                id: "project_count",
                label: "Active projects",
                value: project_count.to_string(),
                tone: "muted",
                action: "open_openmgmt",
            },
        ],
        sections: vec![
            Section {
                id: "browse".into(),
                title: "OpenMGMT".into(),
                component: "action_rows",
                items: vec![ItemRow {
                    id: "browse".into(),
                    title: "Browse tasks and projects".into(),
                    subtitle: "View your account-owned synced items.".into(),
                    status_label: String::new(),
                    route: "services/openmgmt/items".into(),
                }],
            },
            Section {
                id: "tasks".into(),
                title: "Recent tasks".into(),
                component: "list_page",
                items: tasks.rows,
            },
            Section {
                id: "projects".into(),
                title: "Recent projects".into(),
                component: "list_page",
                items: projects.rows,
            },
        ],
        capabilities: vec!["list_items", "read_detail"],
        revision: format!("openmgmt_{}", revision(&connection, &account)?),
    }))
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
