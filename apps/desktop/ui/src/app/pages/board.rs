use chrono::{DateTime, Utc};
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::app::board::{ErBoard, board_task_count};
use crate::app::components::*;
use crate::app::state::*;

#[component]
pub fn BoardPage(state: AppState, now: RwSignal<DateTime<Utc>>) -> impl IntoView {
    let board = Signal::derive(move || state.snapshot.get().board);
    let now: Signal<DateTime<Utc>> = now.into();
    view! {
        <PageHeader
            eyebrow="LIVE"
            title="Board"
            description="The operations board, embedded. Open the TV board for a full-screen window."
        >
            <Button variant="ghost" on_click=Callback::new(move |_| state.refresh())>"Refresh"</Button>
            <Button variant="primary" on_click=Callback::new(move |_| {
                spawn_local(async move {
                    match invoke::<()>("open_tv_board_window", json!({})).await {
                        Ok(_) => state.notice.set(Some("TV board opened.".into())),
                        Err(error) => state.fail("Could not open TV board", error),
                    }
                });
            })>"Open TV Board"</Button>
        </PageHeader>

        <p class="board-explainer">
            "Scheduled tasks move to NOW during their time block, Later Today before their block, and Overdue if the block passes unfinished. Tasks auto-start when their scheduled time arrives while the app is open."
        </p>

        <div class="board-surface">
            {move || (board_task_count(&board.get()) == 0).then(|| view! {
                <div class="tv-empty tv-empty-inline">
                    <h2>"No active board tasks"</h2>
                    <p>"Create an organization, project, and task — then start or schedule one — to populate the board."</p>
                </div>
            })}
            <ErBoard board now />
        </div>
    }
}
