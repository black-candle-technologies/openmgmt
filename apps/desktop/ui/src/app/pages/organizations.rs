use leptos::prelude::*;

use crate::app::components::*;
use crate::app::records::OrganizationCard;
use crate::app::state::*;

#[component]
pub fn OrganizationsPage(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    view! {
        <PageHeader
            eyebrow="PORTFOLIO"
            title="Organizations"
            description="Operating contexts that group your projects and tasks."
        >
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateOrganization))>"New organization"</Button>
        </PageHeader>

        {move || {
            let organizations = state.snapshot.get().organizations;
            if organizations.is_empty() {
                view! {
                    <Panel>
                        <EmptyState title="No organizations yet" hint="Create your first organization to begin.">
                            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateOrganization))>"New organization"</Button>
                        </EmptyState>
                    </Panel>
                }.into_any()
            } else {
                view! {
                    <div class="record-grid">
                        {organizations.into_iter().map(|organization| view! {
                            <OrganizationCard state organization page />
                        }).collect_view()}
                    </div>
                }.into_any()
            }
        }}
    }
}
