use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use leptos_router::components::A;
use leptos_router::hooks::use_navigate;
use phosphor_leptos::{Icon, IconWeight, PLAY};
use yaminabe_launcher_shared::datamodels::{InstanceMeta, LaunchMode, ModProjectInfo};

use crate::changelog;
use crate::trivia;
use crate::components::activity_dock::{ActivityDockOpen, RunningRegistry, note_played, start_launch};
use crate::curseforge::{call_get_popular_modpacks, fmt_downloads};
use crate::pages::search::PendingInstall;

/// How many instances the recently-played strip shows before deferring to the
/// library.
const RECENT_LIMIT: usize = 5;
/// How many popular modpacks to tease before deferring to the search page.
const POPULAR_LIMIT: usize = 6;
/// How many facts to show beside the release notes — enough to fill the column
/// to about the height of the notes without crowding it.
const TRIVIA_COUNT: usize = 2;

#[component]
pub fn HomePage() -> impl IntoView {
    let instances = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let registry = use_context::<RunningRegistry>().expect("running registry");

    // Most recently launched first. An instance that has never been played has
    // no stamp and is left out, so a fresh install shows an empty strip rather
    // than an arbitrary order.
    let recent = Memo::new(move |_| {
        let mut played: Vec<InstanceMeta> = instances
            .get()
            .into_iter()
            .filter(|instance| instance.last_played.is_some())
            .collect();
        played.sort_by(|a, b| b.last_played.cmp(&a.last_played));
        played.truncate(RECENT_LIMIT);
        played
    });
    let releases = StoredValue::new(changelog::releases());
    // Drawn once per visit: a StoredValue so a reactive update elsewhere on the
    // page cannot reshuffle the facts under the reader.
    let facts = StoredValue::new(trivia::pick(TRIVIA_COUNT));

    // The backend serves these from a disk cache, so revisiting Home costs no
    // request and the strip still fills in offline.
    let popular = LocalResource::new(move || async move {
        match call_get_popular_modpacks().await {
            Ok(mut results) => {
                results.items.truncate(POPULAR_LIMIT);
                Ok(results.items)
            }
            Err(e) => {
                log::error!("popular modpacks failed: {e}");
                Err(e)
            }
        }
    });

    let section = css! {
        margin-bottom: 40px;
    };
    let section_head = css! {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 16px;
        margin-bottom: 16px;
    };
    let section_title = css! {
        margin: 0;
        font-size: 1.1rem;
        font-weight: 700;
    };
    let section_link = css! {
        font-size: 0.82rem;
        color: inherit;
        opacity: 0.6;
        text-decoration: none;
        &:hover { opacity: 1; }
    };
    let empty = css! {
        margin: 0;
        font-size: 0.875rem;
        opacity: 0.45;
    };
    let recent_list = css! {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
        gap: 12px;
    };
    let release_list = css! {
        display: flex;
        flex-direction: column;
        gap: 20px;
        max-width: 640px;
    };
    let release_head = css! {
        display: flex;
        align-items: baseline;
        gap: 10px;
        margin-bottom: 8px;
    };
    let release_version = css! {
        font-weight: 700;
        font-size: 0.95rem;
    };
    let release_date = css! {
        font-size: 0.78rem;
        opacity: 0.5;
    };
    let release_current = css! {
        font-size: 0.68rem;
        font-weight: 700;
        letter-spacing: 0.3px;
        text-transform: uppercase;
        color: #3a9e5f;
        background-color: rgba(58, 158, 95, 0.14);
        border-radius: 999px;
        padding: 2px 8px;
    };
    let change_list = css! {
        margin: 0;
        padding-left: 18px;
        display: flex;
        flex-direction: column;
        gap: 4px;
        font-size: 0.85rem;
        opacity: 0.75;
        line-height: 1.55;
    };
    // Release notes beside the trivia. The trivia column stretches to the row's
    // height rather than sitting at its natural size, so the two read as a pair.
    let news_row = css! {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(220px, 300px);
        gap: 24px;
    };
    let trivia_card = css! {
        display: flex;
        flex-direction: column;
        gap: 12px;
        padding: 16px 18px;
        border: 1px solid var(--secondary-color);
        border-radius: 10px;
    };
    let trivia_title = css! {
        margin: 0;
        font-size: 0.72rem;
        font-weight: 700;
        letter-spacing: 0.4px;
        text-transform: uppercase;
        opacity: 0.5;
    };
    let trivia_fact = css! {
        margin: 0;
        font-size: 0.85rem;
        line-height: 1.6;
        opacity: 0.8;
    };

    let popular_list = css! {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
        gap: 12px;
    };

    // A failed browse says so rather than leaving a heading over blank space,
    // which is what an offline launcher or a missing API key would otherwise show.
    let popular_cards = move || match popular.get().map(|result| result.clone()) {
        Some(Ok(packs)) if packs.is_empty() => {
            view! { <p class=empty>"No modpacks to show."</p> }.into_any()
        }
        Some(Ok(packs)) => view! {
            <div class=popular_list>
                {packs.into_iter().map(|pack| view! { <PopularCard pack=pack /> }).collect_view()}
            </div>
        }
        .into_any(),
        Some(Err(_)) => view! {
            <p class=empty>"Could not reach CurseForge. Check your connection and API key."</p>
        }
        .into_any(),
        None => ().into_any(),
    };
    let recent_cards = move || {
        recent
            .get()
            .into_iter()
            .map(|instance| view! { <RecentCard instance=instance registry=registry /> })
            .collect_view()
    };
    let release_entries = move || {
        releases
            .get_value()
            .into_iter()
            .map(|release| {
                // Marked by version rather than by position, so an Unreleased
                // section or notes staged for the next release cannot claim to
                // be what is running.
                let current = release.version == env!("CARGO_PKG_VERSION");
                view! {
                    <div>
                        <div class=release_head>
                            <span class=release_version>{release.version}</span>
                            <span class=release_date>{release.date}</span>
                            {current.then(move || view! {
                                <span class=release_current>"Current"</span>
                            })}
                        </div>
                        <ul class=change_list>
                            {release.changes.into_iter()
                                .map(|change| view! { <li>{change}</li> })
                                .collect_view()}
                        </ul>
                    </div>
                }
            })
            .collect_view()
    };

    view! {
        <div>
            <h1>"# Home"</h1>

            <div class=section>
                <div class=section_head>
                    <h2 class=section_title>"Recently played"</h2>
                    <A href="/library" attr:class=section_link>"View all"</A>
                </div>
                <Show
                    when=move || !recent.get().is_empty()
                    fallback=move || view! {
                        <p class=empty>
                            "Nothing played yet. Launch an instance and it will show up here."
                        </p>
                    }
                >
                    <div class=recent_list>{recent_cards}</div>
                </Show>
            </div>

            <div class=section>
                <div class=section_head>
                    <h2 class=section_title>"Popular modpacks"</h2>
                    <A href="/search" attr:class=section_link>"Browse all"</A>
                </div>
                <Transition fallback=move || view! { <p class=empty>"Loading modpacks…"</p> }>
                    {popular_cards}
                </Transition>
            </div>

            <div class=section>
                <div class=news_row>
                    <div>
                        <div class=section_head>
                            <h2 class=section_title>"What is new"</h2>
                        </div>
                        <Show
                            when=move || !releases.get_value().is_empty()
                            fallback=move || view! { <p class=empty>"No release notes yet."</p> }
                        >
                            <div class=release_list>{release_entries}</div>
                        </Show>
                    </div>
                    <aside class=trivia_card>
                        <p class=trivia_title>"Did you know?"</p>
                        {facts.get_value().into_iter()
                            .map(|fact| view! { <p class=trivia_fact>{fact}</p> })
                            .collect_view()}
                    </aside>
                </div>
            </div>
        </div>
    }
}

/// One popular modpack. Picking it hands the pack to the search page rather
/// than duplicating the install dialog and its version fetching here.
#[component]
fn PopularCard(pack: ModProjectInfo) -> impl IntoView {
    let pending = use_context::<PendingInstall>().expect("pending install context");
    let navigate = StoredValue::new(use_navigate());

    let card = css! {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 12px;
        border: 1.5px solid var(--secondary-color);
        border-radius: 10px;
        cursor: pointer;
        user-select: none;
        transition: border-color 0.12s ease, background-color 0.12s ease;
        &:hover {
            border-color: rgba(58, 158, 95, 0.45);
            background-color: rgba(58, 158, 95, 0.04);
        }
    };
    let logo = css! {
        width: 100%;
        aspect-ratio: 1 / 1;
        border-radius: 8px;
        object-fit: cover;
        background-color: var(--secondary-color);
    };
    let logo_placeholder = css! {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 100%;
        aspect-ratio: 1 / 1;
        border-radius: 8px;
        background-color: var(--secondary-color);
        font-size: 1.6rem;
    };
    let name = css! {
        font-weight: 600;
        font-size: 0.85rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let downloads = css! {
        font-size: 0.72rem;
        opacity: 0.5;
    };

    let logo_view = match pack.logo_url.clone() {
        Some(url) => view! { <img class=logo src=url alt="" /> }.into_any(),
        None => view! { <div class=logo_placeholder>"📦"</div> }.into_any(),
    };
    let count = format!("{} downloads", fmt_downloads(pack.download_count));
    let title = pack.name.clone();
    let hover = title.clone();
    let chosen = StoredValue::new(pack);
    let pick = move || {
        pending.0.set(Some(chosen.get_value()));
        navigate.with_value(|nav| nav("/search", Default::default()));
    };

    view! {
        <div
            class=card
            role="button"
            tabindex="0"
            title=hover
            on:click=move |_| pick()
            // A div has no activation behaviour of its own.
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if ev.key() == "Enter" || ev.key() == " " {
                    ev.prevent_default();
                    pick();
                }
            }
        >
            {logo_view}
            <div>
                <div class=name>{title}</div>
                <div class=downloads>{count}</div>
            </div>
        </div>
    }
}

/// One entry in the recently-played strip: enough to recognise the instance,
/// plus a Play that launches it without a trip through the library.
#[component]
fn RecentCard(instance: InstanceMeta, registry: RunningRegistry) -> impl IntoView {
    let card = css! {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px 14px;
        border: 1.5px solid var(--secondary-color);
        border-radius: 10px;
        transition: border-color 0.12s ease;
        &:hover { border-color: rgba(58, 158, 95, 0.4); }
    };
    let body = css! {
        flex: 1;
        min-width: 0;
        text-decoration: none;
        color: inherit;
    };
    let name = css! {
        font-weight: 600;
        font-size: 0.9rem;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let meta = css! {
        font-size: 0.76rem;
        opacity: 0.5;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    };
    let play = css! {
        flex-shrink: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 34px;
        height: 34px;
        border-radius: 50%;
        background-color: #3a9e5f;
        color: white;
        cursor: pointer;
        transition: background-color 0.15s ease;
        &:hover { background-color: #2e7d4f; }
    };
    let play_running = css! {
        flex-shrink: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 34px;
        height: 34px;
        border-radius: 50%;
        background-color: var(--secondary-color);
        opacity: 0.5;
        cursor: default;
    };

    // The same side effects every launch entry point performs: point the navbar's
    // Instant-Play at what was just started, open the dock so there is more than
    // a pill for feedback, and reorder the strip immediately.
    let instances = use_context::<RwSignal<Vec<InstanceMeta>>>().expect("instances context");
    let last_played_ctx = use_context::<RwSignal<Option<String>>>();
    let dock_open = use_context::<ActivityDockOpen>();

    let id = StoredValue::new(instance.id.clone());
    let launchable = StoredValue::new(instance.clone());
    let detail = format!("/library/{}", instance.id);
    let category = if instance.category.is_empty() {
        String::new()
    } else {
        format!(" - {}", instance.category)
    };
    let summary = format!(
        "MC {} - {}{category}",
        instance.game_version, instance.mod_loader
    );
    // A running instance must not be launched twice; the dock owns that truth.
    let running = move || {
        registry.with(|list| {
            list.iter()
                .any(|r| r.id == id.get_value() && r.status.is_active())
        })
    };
    let launch = move || {
        if running() {
            return;
        }
        let instance = launchable.get_value();
        start_launch(registry, &instance, LaunchMode::Online);
        note_played(instances, &instance.id);
        if let Some(last_played) = last_played_ctx {
            last_played.set(Some(instance.id));
        }
        if let Some(dock) = dock_open {
            dock.0.set(true);
        }
    };
    let label = move || if running() { "Already running" } else { "Play" };

    view! {
        <div class=card>
            <A href=detail attr:class=body>
                <div class=name>{instance.name}</div>
                <div class=meta>{summary}</div>
            </A>
            <span
                class=move || if running() { play_running } else { play }
                role="button"
                tabindex="0"
                title=label
                aria-label=label
                on:click=move |_| launch()
                // A span has no activation behaviour of its own.
                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                    if ev.key() == "Enter" || ev.key() == " " {
                        ev.prevent_default();
                        launch();
                    }
                }
            >
                <Icon icon=PLAY size="16px" weight=IconWeight::Fill />
            </span>
        </div>
    }
}
