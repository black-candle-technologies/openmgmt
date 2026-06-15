//! Reusable, presentation-only UI primitives.
//!
//! These components carry no business logic. They define the shared visual
//! language (buttons, badges, panels, empty/error/loading states) so feature
//! pages stay small and consistent and future views are quick to assemble.

use leptos::prelude::*;

/// Primary action element. `variant` maps to a `.btn-{variant}` modifier:
/// `primary`, `ghost`, `subtle`, `danger`, or `danger-soft`.
#[component]
pub fn Button(
    #[prop(into)] variant: String,
    #[prop(into)] on_click: Callback<()>,
    #[prop(optional)] disabled: bool,
    children: Children,
) -> impl IntoView {
    view! {
        <button
            class=format!("btn btn-{variant}")
            disabled=disabled
            on:click=move |_| on_click.run(())
        >
            {children()}
        </button>
    }
}

/// Compact icon/short-label button used in dense toolbars.
#[component]
pub fn IconButton(
    #[prop(into)] label: String,
    #[prop(into)] title: String,
    #[prop(into)] on_click: Callback<()>,
    children: Children,
) -> impl IntoView {
    let _ = label;
    view! {
        <button class="icon-btn" title=title on:click=move |_| on_click.run(())>
            {children()}
        </button>
    }
}

/// Neutral status pill. `tone` maps to `.badge-{tone}`.
#[component]
pub fn Badge(#[prop(into)] label: String, #[prop(into, optional)] tone: String) -> impl IntoView {
    let tone = if tone.is_empty() {
        "neutral".to_string()
    } else {
        tone
    };
    view! { <span class=format!("badge badge-{tone}")>{label}</span> }
}

fn status_tone(status: &str) -> &'static str {
    match status {
        "in_progress" => "active",
        "done" => "done",
        "blocked" | "waiting" => "blocked",
        "ready" | "scheduled" => "ready",
        "canceled" => "muted",
        _ => "neutral",
    }
}

/// Badge that colour-codes a task or project status string.
#[component]
pub fn StatusBadge(#[prop(into)] status: String) -> impl IntoView {
    let tone = status_tone(&status).to_string();
    let label = super::state::humanize(&status);
    view! { <span class=format!("badge badge-{tone}")>{label}</span> }
}

/// Priority chip (`P1`..`P5`), coloured by severity.
#[component]
pub fn PriorityBadge(value: i32) -> impl IntoView {
    view! { <span class=format!("priority priority-p{value}") title=format!("Priority {value}")>{format!("P{value}")}</span> }
}

/// Standard page header with eyebrow, title, optional description, and an
/// optional right-aligned action area (passed as children).
#[component]
pub fn PageHeader(
    #[prop(into)] title: String,
    #[prop(into, optional)] eyebrow: String,
    #[prop(into, optional)] description: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let has_eyebrow = !eyebrow.is_empty();
    let has_description = !description.is_empty();
    view! {
        <header class="page-header">
            <div class="page-header-text">
                {has_eyebrow.then(|| view! { <p class="eyebrow">{eyebrow}</p> })}
                <h1>{title}</h1>
                {has_description.then(|| view! { <p class="page-header-desc">{description}</p> })}
            </div>
            {children.map(|children| view! { <div class="page-header-actions">{children()}</div> })}
        </header>
    }
}

/// Generic surface container.
#[component]
pub fn Panel(#[prop(into, optional)] class: String, children: Children) -> impl IntoView {
    view! { <section class=format!("panel {class}")>{children()}</section> }
}

/// Titled panel with an optional count chip and optional header action.
#[component]
pub fn Section(
    #[prop(into)] title: String,
    #[prop(optional)] count: Option<usize>,
    #[prop(optional)] action: Option<Children>,
    children: Children,
) -> impl IntoView {
    view! {
        <section class="panel">
            <div class="section-head">
                <div class="section-head-title">
                    <h2>{title}</h2>
                    {count.map(|count| view! { <span class="count-chip">{count}</span> })}
                </div>
                {action.map(|action| view! { <div class="section-head-actions">{action()}</div> })}
            </div>
            {children()}
        </section>
    }
}

/// Calm empty placeholder with an optional hint and action.
#[component]
pub fn EmptyState(
    #[prop(into)] title: String,
    #[prop(into, optional)] hint: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let has_hint = !hint.is_empty();
    view! {
        <div class="empty-state">
            <div class="empty-state-mark"></div>
            <strong>{title}</strong>
            {has_hint.then(|| view! { <p>{hint}</p> })}
            {children.map(|children| view! { <div class="empty-state-action">{children()}</div> })}
        </div>
    }
}

/// Inline loading row (non-blocking; keeps surrounding content stable).
#[component]
pub fn LoadingState(#[prop(into, optional)] label: String) -> impl IntoView {
    let label = if label.is_empty() {
        "Loading…".to_string()
    } else {
        label
    };
    view! {
        <div class="loading-state"><span class="spinner"></span><span>{label}</span></div>
    }
}

/// A single metric tile used on the dashboard.
#[component]
pub fn Metric(
    #[prop(into)] label: String,
    value: Signal<usize>,
    #[prop(into, optional)] tone: String,
) -> impl IntoView {
    let tone = if tone.is_empty() {
        "neutral".to_string()
    } else {
        tone
    };
    view! {
        <div class=format!("metric metric-{tone}")>
            <span class="metric-label">{label}</span>
            <strong class="metric-value">{move || value.get()}</strong>
        </div>
    }
}

/// Labelled form control wrapper.
#[component]
pub fn FormField(
    #[prop(into)] label: String,
    #[prop(into, optional)] hint: String,
    children: Children,
) -> impl IntoView {
    let has_hint = !hint.is_empty();
    view! {
        <label class="form-field">
            <span class="form-field-label">{label}</span>
            {children()}
            {has_hint.then(|| view! { <span class="form-field-hint">{hint}</span> })}
        </label>
    }
}
