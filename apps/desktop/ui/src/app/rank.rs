use super::state::invoke;
use leptos::prelude::*;
use openmgmt_core::scoring::RankExplanation;
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn RankPanel(id: String) -> impl IntoView {
    let explanation = RwSignal::new(None::<RankExplanation>);
    let error = RwSignal::new(None::<String>);
    spawn_local(async move {
        match invoke::<RankExplanation>("explain_task_rank", json!({"id": id})).await {
            Ok(value) => explanation.set(Some(value)),
            Err(e) => error.set(Some(e)),
        }
    });
    view! {
        <section class="agent-section">
            {move || explanation.get().map(|value| view! {
                <h3>{value.rank.map(|n| format!("Why this ranks #{n}")).unwrap_or_else(|| "Ranking factors".into())}</h3>
                <p class="settings-note">"Across active tasks, at drawer open. Board buckets and saved views may use a different order."</p>
                <ul>{value.factors.into_iter().map(|factor| view! { <li>{factor.label}<span class="meta-mono">{format!(" ({:+})", factor.points)}</span></li> }).collect_view()}</ul>
            })}
            {move || error.get().map(|e| view! { <p>{e}</p> })}
        </section>
    }
}
