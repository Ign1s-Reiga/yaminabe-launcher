use bamboo_css_macro::{css, cx};
use leptos::prelude::*;
use leptos::{component, view, IntoView};

/// Segmented control with a sliding indicator. One button per `(value, label)`
/// pair; the indicator slides horizontally to the selected segment via a CSS
/// `transform: translateX(...)` transition.
///
/// Use it as a mutually-exclusive picker when every choice fits on one row.
/// For long or dynamic lists, prefer `SelectInput` — the indicator-width
/// calc here divides by the item count and assumes the full row is visible.
///
/// `selected` is `#[prop(into)]` so an `RwSignal<String>` can be passed
/// directly; `on_change` reports the chosen `value` (not the label).
#[component]
pub fn SegmentedControl(
    items: Vec<(&'static str, &'static str)>,
    #[prop(into)] selected: Signal<String>,
    on_change: Callback<String>,
) -> impl IntoView {
    let count = items.len().max(1);
    // `values` lives only in the indicator closure so it can recompute the
    // selected index reactively; the iteration below consumes `items`.
    let values: Vec<&'static str> = items.iter().map(|(v, _)| *v).collect();

    let container = css! {
        position: relative;
        display: grid;
        padding: 4px;
        background-color: var(--secondary-color);
        border-radius: 8px;
        user-select: none;
    };
    let indicator = css! {
        position: absolute;
        top: 4px;
        bottom: 4px;
        left: 4px;
        background-color: #3a9e5f;
        border-radius: 6px;
        transition: transform 0.22s cubic-bezier(0.4, 0, 0.2, 1);
        pointer-events: none;
        z-index: 0;
    };
    let item = css! {
        position: relative;
        z-index: 1;
        padding: 8px 12px;
        text-align: center;
        font-size: 0.88rem;
        font-weight: 500;
        cursor: pointer;
        background: none;
        border: none;
        color: inherit;
        font-family: inherit;
        line-height: 1.2;
        transition: color 0.15s ease;
    };
    let item_selected = css! {
        color: white;
        font-weight: 600;
    };

    let container_style = format!("grid-template-columns: repeat({count}, 1fr);");
    let indicator_style = move || {
        let cur = selected.get();
        let idx = values.iter().position(|v| *v == cur).unwrap_or(0);
        format!(
            "width: calc((100% - 8px) / {count}); transform: translateX({}%);",
            idx * 100
        )
    };

    view! {
        <div class=container style=container_style>
            <div class=indicator style=indicator_style></div>
            {items.into_iter().map(|(value, label)| {
                let value_owned = value.to_string();
                view! {
                    <button
                        type="button"
                        class=move || cx!(item, if selected.get() == value { item_selected } else { "" })
                        on:click=move |_| on_change.run(value_owned.clone())
                    >
                        {label}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}