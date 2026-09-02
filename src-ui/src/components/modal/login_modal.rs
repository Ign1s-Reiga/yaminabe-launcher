use std::cell::Cell;

use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};
use yaminabe_launcher_shared::datamodels::AccountSummary;
use yaminabe_launcher_shared::ipc::{MsLoginPrompt, MsLoginResult};

use crate::components::ui::*;
use crate::ipc;

#[derive(Clone, PartialEq)]
enum LoginPhase {
    Starting,
    Prompted(MsLoginPrompt),
    Failed(String),
}

// `ipc::on_event` has no unsubscribe handle, so each modal mount claims a
// token and listeners no-op if `ACTIVE_MODAL` no longer matches.
thread_local! {
    static ACTIVE_MODAL: Cell<u32> = const { Cell::new(0) };
}

#[component]
pub fn LoginModal(
    on_close: Callback<()>,
    on_success: Callback<AccountSummary>,
) -> impl IntoView {
    let phase: RwSignal<LoginPhase> = RwSignal::new(LoginPhase::Starting);

    let my_token = ACTIVE_MODAL.with(|c| {
        let next = c.get().wrapping_add(1);
        c.set(next);
        next
    });

    let prompt_token = my_token;
    ipc::on_event::<MsLoginPrompt, _>("ms-login-prompt", move |prompt| {
        if ACTIVE_MODAL.with(|c| c.get()) != prompt_token { return; }
        phase.set(LoginPhase::Prompted(prompt));
    });
    let result_token = my_token;
    ipc::on_event::<MsLoginResult, _>("ms-login-result", move |result| {
        if ACTIVE_MODAL.with(|c| c.get()) != result_token { return; }
        match result.kind.as_str() {
            "success" => {
                if let Some(summary) = result.account {
                    on_success.run(summary);
                }
                on_close.run(());
            }
            "cancelled" => on_close.run(()),
            _ => phase.set(LoginPhase::Failed(result.message)),
        }
    });

    on_cleanup(move || {
        ACTIVE_MODAL.with(|c| c.set(c.get().wrapping_add(1)));
    });

    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            // start_microsoft_login also emits `ms-login-result` on failure,
            // but we still catch a returned Err in case the IPC layer itself
            // fails before the command runs.
            if let Err(e) = ipc::call_noargs::<()>("start_microsoft_login").await {
                phase.set(LoginPhase::Failed(e));
            }
        });
    });

    let cancel_and_close = move || {
        leptos::task::spawn_local(async move {
            if let Err(e) = ipc::call_noargs::<()>("cancel_microsoft_login").await {
                log::warn!("cancel_microsoft_login failed: {e}");
            }
        });
        on_close.run(());
    };

    // QR SVG has intrinsic width/height; we just box it on a white card.
    let qr_box = css! {
        padding: 12px;
        background-color: #ffffff;
        border-radius: 12px;
        margin: 0 auto 20px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
    };
    let user_code = css! {
        font-family: var(--font-mono, monospace);
        font-size: 1.8rem;
        font-weight: 700;
        letter-spacing: 0.3rem;
        text-align: center;
        background-color: var(--secondary-color);
        border-radius: 10px;
        padding: 14px 20px;
        margin: 0 auto 16px;
        max-width: 320px;
    };
    let muted = css! {
        font-size: 0.82rem;
        opacity: 0.6;
        text-align: center;
        margin: 0 0 6px 0;
    };
    let center_col = css! {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 6px;
    };
    let starting = css! {
        text-align: center;
        opacity: 0.65;
        padding: 60px 0;
    };
    let err = css! {
        color: #c0392b;
        font-size: 0.9rem;
        margin: 0;
        text-align: center;
    };

    view! {
        <ModalOverlay>
            <ModalBox>
                <ModalBody>
                    <h2 style="margin: 0 0 6px 0;">"Sign in with Microsoft"</h2>
                    <p style="margin: 0 0 24px 0; font-size: 0.85rem; opacity: 0.6;">
                        "Scan the QR code with your phone, then enter the code below."
                    </p>

                    {move || match phase.get() {
                        LoginPhase::Starting => view! {
                            <div class=starting>"Contacting Microsoft…"</div>
                        }.into_any(),
                        LoginPhase::Prompted(p) => {
                            let uri = p.verification_uri.clone();
                            view! {
                                <div class=center_col>
                                    <div class=qr_box inner_html=p.qr_svg />
                                    <p class=muted>"Code"</p>
                                    <div class=user_code>{p.user_code}</div>
                                    <p class=muted>{format!("or visit {uri} in any browser")}</p>
                                </div>
                            }.into_any()
                        }
                        LoginPhase::Failed(msg) => view! {
                            <div class=starting>
                                <p class=err>{msg}</p>
                            </div>
                        }.into_any(),
                    }}
                </ModalBody>
                <ModalFooter>
                    <span style="font-size: 0.78rem; opacity: 0.55;">
                        {move || match phase.get() {
                            LoginPhase::Prompted(_) => "Waiting for sign-in…",
                            LoginPhase::Failed(_) => "Login failed.",
                            LoginPhase::Starting => "",
                        }}
                    </span>
                    <Button
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| cancel_and_close())
                    >
                        {move || if matches!(phase.get(), LoginPhase::Failed(_)) { "Close" } else { "Cancel" }}
                    </Button>
                </ModalFooter>
            </ModalBox>
        </ModalOverlay>
    }
}