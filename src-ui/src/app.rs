use crate::components::activity_dock::{
    ActivityDock, ActivityDockOpen, InstallJob, RunStatus, RunningInstance, RunningRegistry,
};
use crate::pages::search::PendingInstall;
use crate::pages::{
    home::HomePage,
    library::LibraryPage,
    search::SearchPage,
    settings::SettingsPage,
    instance_detail::InstanceDetailPage,
    play::PlayPage,
};
use crate::components::ui::{ToastHost, Toasts};
use crate::ipc;
use crate::signal_ext::VecSignalExt;
use crate::update::Updates;
use bamboo_css_macro::{css, cx, styled};
use leptos::prelude::*;
use leptos::{component, IntoView, view, web_sys};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;
use phosphor_leptos::{
    Icon, IconData, IconWeight, BOOKS, GEAR_SIX, HOUSE, MAGNIFYING_GLASS, PLAY, WARNING,
};
use yaminabe_launcher_shared::datamodels::{
    AppSettings, InstanceList, InstanceMeta, LaunchMode, ModProjectInfo, SkippedInstance,
};
use yaminabe_launcher_shared::ipc::LogLine;

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
    let toasts = Toasts::provide();
    // Only the load at launch reports skipped instances; reloading after an
    // install would otherwise raise the same toast again.
    let skipped_reported = StoredValue::new(false);

    Effect::new(move |_| {
        // Track only — re-fetch instances whenever `refresh` bumps without
        // depending on its actual value.
        refresh.track();
        leptos::task::spawn_local(async move {
            match ipc::call_noargs::<InstanceList>("get_instances").await {
                Ok(list) => {
                    if !skipped_reported.get_value() {
                        skipped_reported.set_value(true);
                        report_skipped(toasts, &list.skipped);
                    }
                    instances.set(list.instances);
                }
                Err(e) => log::error!("get_instances failed: {e}"),
            }
        });
    });

    provide_context(instances);
    provide_context(refresh);

    // Id of the most recently launched instance, seeded from persisted settings
    // and kept fresh by `PlayPage` on each launch. Drives the navbar's
    // Instant-Play button.
    let last_played: RwSignal<Option<String>> = RwSignal::new(None);
    provide_context(last_played);

    // Lets the Home page's popular strip hand a pack to the Search page's
    // install dialog instead of duplicating that flow.
    let pending_install: RwSignal<Option<ModProjectInfo>> = RwSignal::new(None);
    provide_context(PendingInstall(pending_install));
    leptos::task::spawn_local(async move {
        if let Ok(s) = ipc::call_noargs::<AppSettings>("get_settings").await {
            if !s.last_played_instance_id.is_empty() {
                last_played.set(Some(s.last_played_instance_id));
            }
        }
    });

    let install_jobs: RwSignal<Vec<InstallJob>> = RwSignal::new(vec![]);
    // Also read by Settings, which holds an update back while a pack installs.
    provide_context(install_jobs);

    let updates = Updates::provide();
    let update_offered = Signal::derive(move || updates.0.with(|state| state.offered().is_some()));

    // Global registry of launched instances so several can run at once and a
    // launch survives navigating away from its play page. App-level listeners
    // route per-instance log/lifecycle events into the matching entry.
    let running_registry: RwSignal<Vec<RunningInstance>> = RwSignal::new(vec![]);
    provide_context(running_registry);

    // Shared expanded/collapsed state for the bottom-right activity dock. Auto-
    // expanded when a new install begins (below) or a launch starts (PlayPage).
    let dock_expanded = RwSignal::new(false);
    provide_context(ActivityDockOpen(dock_expanded));

    ipc::on_event::<InstallJob, _>("instance-install-progress", move |job| {
        let job_succeeded = job.done && job.error.is_none();
        let is_new = install_jobs.with_untracked(|list| !list.iter().any(|j| j.id == job.id));
        // A job that arrives already failed still has something to say. Without
        // this the dock stays shut and the install looks like it did nothing.
        if is_new && (!job.done || job.error.is_some()) {
            dock_expanded.set(true);
        }
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

    ipc::on_event::<LogLine, _>("instance-log", move |msg| {
        running_registry.update(|list| {
            if let Some(r) = list.iter_mut().find(|r| r.id == msg.instance_id) {
                r.log_lines.push(msg.line);
                if msg.done {
                    r.status = match msg.error {
                        Some(e) => RunStatus::Errored(e),
                        None => RunStatus::Stopped,
                    };
                }
            }
        });
    });
    ipc::on_event::<String, _>("instance-process-started", move |id| {
        running_registry.update(|list| {
            if let Some(r) = list.iter_mut().find(|r| r.id == id) {
                // Promote only from Preparing so a race can't resurrect a
                // finished/errored entry.
                if matches!(r.status, RunStatus::Preparing) {
                    r.status = RunStatus::Running;
                }
            }
        });
    });

    view! {
        <Router>
            <MainViewWrapper>
                <MainView>
                    <Routes fallback=|| "Page not found.">
                        <Route path=path!("") view=HomePage />
                        <Route path=path!("library") view=LibraryPage />
                        <Route path=path!("library/:id") view=InstanceDetailPage />
                        <Route path=path!("library/:id/play") view=PlayPage />
                        <Route path=path!("search") view=SearchPage />
                        <Route path=path!("settings") view=SettingsPage />
                    </Routes>
                </MainView>
                <MainViewNavbar>
                    <NavigationButton href="/library" icon=BOOKS label="Library"/>
                    <NavigationButton href="/" icon=HOUSE label="Home"/>
                    <InstantPlayButton last_played=last_played instances=instances />
                    <NavigationButton href="/search" icon=MAGNIFYING_GLASS label="Search"/>
                    <NavigationButton href="/settings" icon=GEAR_SIX label="Settings" badge=update_offered/>
                </MainViewNavbar>
                <ActivityDock jobs=install_jobs registry=running_registry expanded=dock_expanded />
                <ToastHost />
            </MainViewWrapper>
        </Router>
    }
}

/// Tell the user which instances were left out of the library at launch. The
/// backend's log has each one's reason; the toast names the folders.
fn report_skipped(toasts: Toasts, skipped: &[SkippedInstance]) {
    const SHOWN: usize = 3;
    let (title, cause) = match skipped.len() {
        0 => return,
        1 => (
            "1 instance could not be loaded".to_string(),
            "Its instance.json is missing or damaged.",
        ),
        n => (
            format!("{n} instances could not be loaded"),
            "Their instance.json is missing or damaged.",
        ),
    };
    let mut lines: Vec<String> = skipped.iter().take(SHOWN).map(|s| s.folder.clone()).collect();
    if skipped.len() > SHOWN {
        lines.push(format!("and {} more", skipped.len() - SHOWN));
    }
    lines.push(format!("{cause} The launcher's log has the details."));
    toasts.push(WARNING, title, lines);
}

#[component]
pub fn NavigationButton(
    href: &'static str,
    icon: IconData,
    label: &'static str,
    /// Marks the button with a dot, for something waiting behind it.
    #[prop(optional, into)] badge: Signal<bool>,
) -> impl IntoView {
    let location = use_location();
    let navigate = use_navigate();
    let is_active = move || location.pathname.get() == href;

    let container_class = css! {
        position: relative;
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

    let badge_class = css! {
        position: absolute;
        top: 8px;
        right: 26px;
        width: 10px;
        height: 10px;
        border-radius: 50%;
        background-color: #3a9e5f;
        box-shadow: 0 0 0 2px var(--primary-color);
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
            <Show when=move || badge.get() fallback=|| ()>
                <span class=badge_class></span>
            </Show>
        </div>
    }
}

/// Centre-of-navbar shortcut that relaunches the most recently played instance.
/// Sized to match a `NavigationButton` (96px wide, full navbar height) and
/// filled with the Primary green. A slide button is fixed to the right edge;
/// the Online/Offline section to its left slides vertically — Online shows by
/// default, and pressing the slide button slides it down to Offline. Greyed and
/// inert when no instance has been played, or it no longer exists.
#[component]
pub fn InstantPlayButton(
    last_played: RwSignal<Option<String>>,
    instances: RwSignal<Vec<InstanceMeta>>,
) -> impl IntoView {
    let navigate = use_navigate();
    let mode: RwSignal<LaunchMode> = RwSignal::new(LaunchMode::Online);
    let registry = use_context::<RunningRegistry>().expect("running registry");

    let target = Signal::derive(move || {
        let id = last_played.get()?;
        instances.get().into_iter().find(|i| i.id == id)
    });
    let is_running = Signal::derive(move || {
        target.get()
            .and_then(|i| registry.map_by_id(&i.id, |r| r.status.is_active()))
            .unwrap_or(false)
    });
    // Inert when there's nothing to launch, or the target is already running —
    // we never start a second copy of the same instance.
    let disabled = Signal::derive(move || target.get().is_none() || is_running.get());
    let title = move || {
        match target.get() {
            Some(i) if is_running.get() => format!("{} is already running", i.name),
            Some(i) => format!("Instant Play — {}", i.name),
            None => "No recently played instance".to_string(),
        }
    };

    // The wrapper has no fixed height — it stretches to the navbar like a
    // NavigationButton. The sliding column is twice the viewport height (two
    // stacked panels), so a translateY of -50% / 0% shows Online / Offline.
    let wrapper_base = css! {
        position: relative;
        align-self: stretch;
        display: flex;
        overflow: hidden;
        border-radius: 6px;
        color: white;
        transition: background-color 0.3s ease;
    };
    let fill_enabled = css! {
        background-color: #3a9e5f;
        cursor: pointer;
        &:hover { background-color: #2e7d4f; }
    };
    let fill_disabled = css! {
        background-color: var(--secondary-color);
        opacity: 0.5;
        cursor: not-allowed;
    };
    let viewport = css! {
        flex: 1;
        width: 96px;
        height: 100%;
        overflow: hidden;
    };
    let column = css! {
        display: flex;
        flex-direction: column;
        height: 200%;
        transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
    };
    let panel = css! {
        height: 50%;
        flex-shrink: 0;
        box-sizing: border-box;
        padding: 8px 6px 12px;
        display: flex;
        flex-direction: column;
        justify-content: space-between;
    };
    let toggle = css! {
        flex-shrink: 0;
        width: 22px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.85rem;
        border-left: 1px solid rgb(255 255 255 / 0.22);
        transition: background-color 0.15s ease;
        user-select: none;
        &:hover { background-color: rgb(255 255 255 / 0.14); }
    };
    let label_class = css! { margin: 0; font-weight: 300; };

    let content_style = move || {
        let y = if mode.get() == LaunchMode::Online { "0" } else { "-50%" };
        format!("transform: translateY({y});")
    };

    let launch = move || {
        if disabled.get_untracked() {
            return;
        }
        if let Some(inst) = target.get_untracked() {
            // A per-click nonce forces PlayPage to relaunch even when we are
            // already sitting on this instance's (stopped) play page.
            let nonce = js_sys::Date::now() as u64;
            navigate(
                &format!("/library/{}/play?mode={}&t={nonce}", inst.id, mode.get_untracked().as_str()),
                Default::default(),
            );
        }
    };

    view! {
        <div
            class=move || if disabled.get() { cx!(wrapper_base, fill_disabled) } else { cx!(wrapper_base, fill_enabled) }
            title=title
            on:click=move |_| launch()
        >
            <div class=viewport>
                <div class=column style=content_style>
                    <div class=panel>
                        <Icon icon=PLAY size="32px" weight=IconWeight::Fill />
                        <p class=label_class>
                            {move || if is_running.get() { "Running" } else { "Online" }}
                        </p>
                    </div>
                    <div class=panel>
                        <Icon icon=PLAY size="32px" weight=IconWeight::Fill />
                        <p class=label_class>
                            {move || if is_running.get() { "Running" } else { "Offline" }}
                        </p>
                    </div>
                </div>
            </div>
            <div
                class=toggle
                title="Toggle online / offline"
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    if !disabled.get_untracked() {
                        mode.update(|m| {
                            *m = if *m == LaunchMode::Online { LaunchMode::Offline } else { LaunchMode::Online };
                        });
                    }
                }
            >
                "⇅"
            </div>
        </div>
    }
}
