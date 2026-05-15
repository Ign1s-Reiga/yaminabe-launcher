use crate::components::{
    install_sidebar::{InstallJob, InstallSidebar},
};
use crate::pages::{
    home::HomePage,
    library::LibraryPage,
    search::SearchPage,
    settings::SettingsPage,
    instance_detail::InstanceDetailPage,
    play::PlayPage,
};
use crate::ipc;
use bamboo_css_macro::{css, styled};
use leptos::prelude::*;
use leptos::{component, IntoView, view};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;
use phosphor_leptos::{Icon, IconData, IconWeight, BOOKS, GEAR_SIX, HOUSE, MAGNIFYING_GLASS};
use yaminabe_launcher_shared::datatypes::InstanceMeta;

styled!(MainViewWrapper, div, {
    height: 100vh;
    width: 100vw;
    display: grid;
    grid: 1fr 100px / auto-flow;
});
styled!(MainView, main, {
    padding: 64px 128px;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-gutter: stable both-edges;
    scrollbar-color: darkgrey var(--background-color);
});
styled!(MainViewNavbar, nav, {
    padding: 10px 128px;
    background-color: var(--primary-color);
    gap: 16px;
    display: flex;
    flex-direction: row;
    justify-content: center;
});

#[component]
pub fn App() -> impl IntoView {
    let instances: RwSignal<Vec<InstanceMeta>> = RwSignal::new(vec![]);
    let refresh: RwSignal<u32> = RwSignal::new(0);

    Effect::new(move |_| {
        let _ = refresh.get();
        leptos::task::spawn_local(async move {
            if let Ok(list) = ipc::call_noargs::<Vec<InstanceMeta>>("get_instances").await {
                instances.set(list);
            }
        });
    });

    provide_context(instances);
    provide_context(refresh);

    let install_jobs: RwSignal<Vec<InstallJob>> = RwSignal::new(vec![]);
    let sidebar_open: RwSignal<bool> = RwSignal::new(false);

    ipc::on_event::<InstallJob, _>("instance-install-progress", move |job| {
        if !job.done && job.error.is_none() {
            sidebar_open.set(true);
        }
        let job_succeeded = job.done && job.error.is_none();
        install_jobs.update(|list| {
            if let Some(existing) = list.iter_mut().find(|j| j.id == job.id) {
                *existing = job;
            } else {
                list.push(job);
            }
        });
        if job_succeeded {
            refresh.update(|n| *n += 1);
        }
    });

    view! {
        <Router>
            <MainViewWrapper>
                <MainView>
                    <Routes fallback=|| "Page not found.">
                        <Route path=path!("") view=HomePage />
                        <Route path=path!("library") view=move || view! {
                            <LibraryPage />
                            <InstallSidebar jobs=install_jobs open=sidebar_open />
                        }/>
                        <Route path=path!("library/:id") view=move || view! {
                            <InstanceDetailPage />
                            <InstallSidebar jobs=install_jobs open=sidebar_open />
                        }/>
                        <Route path=path!("library/:id/play") view=PlayPage />
                        <Route path=path!("search") view=SearchPage />
                        <Route path=path!("settings") view=SettingsPage />
                    </Routes>
                </MainView>
                // TODO: Add PLAY button in center of the navbar.
                <MainViewNavbar>
                    <NavigationButton href="/library" icon=BOOKS label="Library"/>
                    <NavigationButton href="/" icon=HOUSE label="Home"/>
                    <NavigationButton href="/search" icon=MAGNIFYING_GLASS label="Search"/>
                    <NavigationButton href="/settings" icon=GEAR_SIX label="Settings"/>
                </MainViewNavbar>
            </MainViewWrapper>
        </Router>
    }
}

#[component]
pub fn NavigationButton(
    href: &'static str,
    icon: IconData,
    label: &'static str,
) -> impl IntoView {
    let location = use_location();
    let navigate = use_navigate();
    let is_active = move || location.pathname.get() == href;

    let container_class = css! {
        padding: 8px 6px 12px;
        border-radius: 6px;
        width: 96px;
        cursor: pointer;
        display: flex;
        flex-direction: column;
        justify-content: space-between;
        transition: background-color 0.3s ease;
        &:hover { background-color: var(--secondary-color); }
    };

    view! {
        <div class=container_class on:click=move |_| { navigate(href, Default::default()); }>
            <Show
                when=is_active
                fallback=move || view! { <Icon icon=icon size="32px" weight=IconWeight::Regular /> }
            >
                <Icon icon=icon size="32px" weight=IconWeight::Fill />
            </Show>
            <p class=css! { margin: 0; font-weight: 300; }>{label}</p>
        </div>
    }
}
