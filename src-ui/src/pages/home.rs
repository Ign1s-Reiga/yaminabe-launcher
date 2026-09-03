use bamboo_css_macro::css;
use leptos::control_flow::Show;
use leptos::prelude::*;
use leptos::{IntoView, component, view};
use leptos_router::components::A;
use phosphor_leptos::{Icon, IconWeight, PLAY};
use yaminabe_launcher_shared::datamodels::{InstanceMeta, LaunchMode};

use crate::changelog;
use crate::components::activity_dock::{RunningRegistry, start_launch};

/// How many instances the recently-played strip shows before deferring to the
/// library.
const RECENT_LIMIT: usize = 5;

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
            .enumerate()
            .map(|(index, release)| {
                view! {
                    <div>
                        <div class=release_head>
                            <span class=release_version>{release.version}</span>
                            <span class=release_date>{release.date}</span>
                            {(index == 0).then(move || view! {
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
                    <h2 class=section_title>"What is new"</h2>
                </div>
                <Show
                    when=move || !releases.get_value().is_empty()
                    fallback=move || view! { <p class=empty>"No release notes yet."</p> }
                >
                    <div class=release_list>{release_entries}</div>
                </Show>
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
    let on_play = move |_| {
        if running() {
            return;
        }
        start_launch(registry, &launchable.get_value(), LaunchMode::Online);
    };

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
                title=move || if running() { "Already running" } else { "Play" }
                aria-label="Play"
                on:click=on_play
            >
                <Icon icon=PLAY size="16px" weight=IconWeight::Fill />
            </span>
        </div>
    }
}
