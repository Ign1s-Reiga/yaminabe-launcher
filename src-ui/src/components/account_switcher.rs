use bamboo_css_macro::css;
use leptos::html;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use leptos_router::hooks::use_navigate;
use phosphor_leptos::{Icon, IconWeight, CARET_UP, CHECK, PLUS, USER_CIRCLE, USER_GEAR};
use yaminabe_launcher_shared::datamodels::AccountSummary;

use crate::accounts::Accounts;
use crate::components::modal::login_modal::LoginModal;
use crate::components::ui::DropdownItem;

/// A player's head, as Settings shows it.
fn avatar_url(account: &AccountSummary) -> String {
    format!("https://mc-heads.net/avatar/{}/64", account.uuid.replace('-', ""))
}

/// The navbar's account switcher, at its right end: shaped like a
/// `NavigationButton`, with the selected player's head and name. It opens a
/// menu upward, over the page, to pick the account launches use, add one, or
/// manage them in Settings. A click outside or Escape closes it.
#[component]
pub fn AccountSwitcher() -> impl IntoView {
    let accounts = use_context::<Accounts>().expect("accounts");
    let navigate = StoredValue::new(use_navigate());
    let open = RwSignal::new(false);
    let show_login = RwSignal::new(false);
    let trigger_ref = NodeRef::<html::Button>::new();

    let close = move || open.set(false);

    let anchor = css! {
        position: absolute;
        top: 10px;
        right: 24px;
        bottom: 10px;
        display: flex;
    };
    // `NavigationButton`'s box, as a real button so the keyboard reaches it.
    let trigger = css! {
        position: relative;
        padding: 8px 6px 12px;
        border: none;
        border-radius: 6px;
        min-width: 96px;
        max-width: 168px;
        background-color: transparent;
        color: inherit;
        font: inherit;
        text-align: left;
        cursor: pointer;
        display: flex;
        flex-direction: column;
        justify-content: space-between;
        transition: background-color 0.3s ease;
        &:hover { background-color: var(--secondary-color); }
        &[aria-expanded="true"] { background-color: var(--secondary-color); }
    };
    let head = css! {
        width: 32px;
        height: 32px;
        border-radius: 4px;
        background-color: var(--secondary-color);
        image-rendering: pixelated;
        object-fit: cover;
    };
    let label = css! {
        display: flex;
        align-items: center;
        gap: 4px;
        min-width: 0;
        margin: 0;
        font-weight: 300;
    };
    let label_text = css! {
        min-width: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let caret = css! {
        display: flex;
        flex-shrink: 0;
        opacity: 0.6;
    };
    // Clicks that land outside the menu close it.
    let backdrop = css! {
        position: fixed;
        inset: 0;
        z-index: 149;
    };
    // `Dropdown`'s panel, opening upward to sit 4px above the navbar.
    let menu = css! {
        position: absolute;
        right: 0;
        bottom: calc(100% + 14px);
        z-index: 150;
        min-width: 240px;
        max-width: 320px;
        box-sizing: border-box;
        background-color: var(--background-color);
        border: 1px solid var(--secondary-color);
        border-radius: 8px;
        padding: 4px;
        box-shadow: 0 8px 24px rgb(0 0 0 / 0.2);
    };
    let row = css! {
        display: flex;
        align-items: center;
        gap: 10px;
        min-width: 0;
    };
    let row_head = css! {
        width: 24px;
        height: 24px;
        flex-shrink: 0;
        border-radius: 4px;
        background-color: var(--secondary-color);
        image-rendering: pixelated;
        object-fit: cover;
    };
    let row_name = css! {
        flex: 1;
        min-width: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let row_icon = css! {
        display: flex;
        flex-shrink: 0;
        width: 24px;
        justify-content: center;
        opacity: 0.7;
    };
    let tick = css! {
        display: flex;
        flex-shrink: 0;
        color: #3a9e5f;
    };
    let divider = css! {
        height: 1px;
        margin: 4px 0;
        background-color: var(--secondary-color);
    };

    let title = move || match accounts.selected() {
        Some(account) => format!("Playing as {}", account.username),
        None => "No account selected".to_string(),
    };

    view! {
        <div
            class=anchor
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if ev.key() == "Escape" && open.get_untracked() {
                    close();
                    if let Some(button) = trigger_ref.get_untracked() {
                        if let Err(e) = button.focus() {
                            log::warn!("cannot focus the account switcher: {e:?}");
                        }
                    }
                }
            }
        >
            <button
                node_ref=trigger_ref
                class=trigger
                title=title
                aria-haspopup="menu"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {move || match accounts.selected() {
                    Some(account) => view! { <img class=head src=avatar_url(&account) alt="" /> }.into_any(),
                    None => view! { <Icon icon=USER_CIRCLE size="32px" weight=IconWeight::Regular /> }.into_any(),
                }}
                <p class=label>
                    <span class=label_text>
                        {move || accounts.selected().map(|a| a.username).unwrap_or_else(|| "Sign in".to_string())}
                    </span>
                    <span class=caret><Icon icon=CARET_UP size="14px" weight=IconWeight::Bold /></span>
                </p>
            </button>

            <Show when=move || open.get() fallback=|| ()>
                <div class=backdrop on:click=move |_| close()></div>
                <div class=menu role="menu" on:click=move |_| close()>
                    <For
                        each=move || accounts.list()
                        key=|account| account.uuid.clone()
                        children=move |account: AccountSummary| {
                            let uuid = account.uuid.clone();
                            let picked = uuid.clone();
                            view! {
                                <DropdownItem on_select=Callback::new(move |_| {
                                    if !accounts.is_selected(&picked) {
                                        accounts.select(picked.clone());
                                    }
                                })>
                                    <span class=row>
                                        <img class=row_head src=avatar_url(&account) alt="" />
                                        <span class=row_name>{account.username}</span>
                                        <Show when=move || accounts.is_selected(&uuid) fallback=|| ()>
                                            <span class=tick>
                                                <Icon icon=CHECK size="16px" weight=IconWeight::Bold />
                                            </span>
                                        </Show>
                                    </span>
                                </DropdownItem>
                            }
                        }
                    />
                    <Show when=move || !accounts.list().is_empty() fallback=|| ()>
                        <div class=divider role="separator"></div>
                    </Show>
                    <DropdownItem on_select=Callback::new(move |_| show_login.set(true))>
                        <span class=row>
                            <span class=row_icon><Icon icon=PLUS size="16px" weight=IconWeight::Bold /></span>
                            "Add Microsoft Account"
                        </span>
                    </DropdownItem>
                    <DropdownItem on_select=Callback::new(move |_| {
                        navigate.with_value(|nav| nav("/settings", Default::default()));
                    })>
                        <span class=row>
                            <span class=row_icon><Icon icon=USER_GEAR size="16px" weight=IconWeight::Regular /></span>
                            "Manage accounts"
                        </span>
                    </DropdownItem>
                </div>
            </Show>
        </div>

        <Show when=move || show_login.get() fallback=|| ()>
            <LoginModal
                on_close=Callback::new(move |_| show_login.set(false))
                on_success=Callback::new(move |summary: AccountSummary| accounts.signed_in(summary.uuid))
            />
        </Show>
    }
}
