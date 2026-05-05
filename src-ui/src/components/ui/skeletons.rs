use bamboo_css_macro::css;
use leptos::prelude::*;
use leptos::{component, view, IntoView};

fn shimmer_class() -> &'static str {
    css! {
        background-color: var(--tertiary-color);
        animation: pulse 1.6s ease-in-out infinite;
    }
}

/// Animated shimmer placeholder block.
/// Size and corner radius are set via props; defaults to full-width, 14 px tall.
#[component]
pub fn Skeleton(
    #[prop(default = "100%")] width: &'static str,
    #[prop(default = "14px")] height: &'static str,
    #[prop(default = "6px")] border_radius: &'static str,
) -> impl IntoView {
    let style = format!("width:{width};height:{height};border-radius:{border_radius}");
    view! { <div class=shimmer_class() style=style /> }
}

// ── Private helpers ───────────────────────────────────────────────────────────

#[component]
fn SkeletonPropRow(
    #[prop(default = "40px")] input_height: &'static str,
) -> impl IntoView {
    let row = css! {
        display: grid;
        grid-template-columns: 200px 1fr;
        align-items: start;
        gap: 10px 24px;
        margin-bottom: 14px;
    };
    let right = css! {
        display: flex;
        flex-direction: column;
        gap: 6px;
    };
    view! {
        <div class=row>
            <Skeleton width="90px" height="13px" />
            <div class=right>
                <Skeleton height=input_height border_radius="8px" />
                <Skeleton width="55%" height="10px" />
            </div>
        </div>
    }
}

#[component]
fn SkeletonSection(children: Children) -> impl IntoView {
    let section = css! {
        margin-bottom: 56px;
    };
    let heading_wrap = css! {
        margin-bottom: 20px;
        padding-bottom: 12px;
        border-bottom: 1px solid var(--secondary-color);
    };
    view! {
        <div class=section>
            <div class=heading_wrap>
                <Skeleton width="110px" height="18px" border_radius="4px" />
            </div>
            {children()}
        </div>
    }
}

#[component]
fn SkeletonFooter() -> impl IntoView {
    let footer = css! {
        display: flex;
        justify-content: flex-end;
        margin-top: 20px;
        padding-top: 14px;
        border-top: 1px solid var(--secondary-color);
    };
    view! {
        <div class=footer>
            <Skeleton width="80px" height="36px" border_radius="8px" />
        </div>
    }
}

// ── Public page skeletons ─────────────────────────────────────────────────────

/// Full-page skeleton that mirrors the Settings page layout.
#[component]
pub fn SkeletonSettingsPage() -> impl IntoView {
    view! {
        <SkeletonSection>
            <SkeletonPropRow />
            <SkeletonPropRow />
            <SkeletonPropRow />
        </SkeletonSection>
        <SkeletonSection>
            <SkeletonPropRow />
            <SkeletonPropRow input_height="24px" />
            <SkeletonPropRow input_height="80px" />
            <SkeletonFooter />
        </SkeletonSection>
        <SkeletonSection>
            <SkeletonPropRow />
            <SkeletonFooter />
        </SkeletonSection>
    }
}