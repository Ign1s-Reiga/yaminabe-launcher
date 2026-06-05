# phosphor-leptos

[Phosphor](https://phosphoricons.com) is a flexible icon family for interfaces. `phosphor-leptos` exposes all ~1,500 icons as Leptos components — each icon ships in six weights and renders as an inline `<svg>`.

This project pins it in `src-ui/Cargo.toml`:

```toml
phosphor-leptos = "0.8"
```

## Basic usage

Import the `Icon` component, the `IconWeight` enum, and the constant(s) for the icons you need, then render `<Icon icon=… />`:

```rust
use leptos::prelude::*;
use phosphor_leptos::{Icon, IconWeight, GEAR_SIX, PLAY};

#[component]
fn Toolbar() -> impl IntoView {
    view! {
        <Icon icon=PLAY size="16px" weight=IconWeight::Fill />
        <Icon icon=GEAR_SIX size="16px" weight=IconWeight::Regular />
    }
}
```

Each icon is a `pub const` of type `IconData` (`&'static IconWeightData`) — cheap to pass around, so just name the constant directly as the `icon` prop.

## Finding an icon's constant name

Browse and search icons at [phosphoricons.com](https://phosphoricons.com). The constant is the icon's name in `SCREAMING_SNAKE_CASE`:

| Icon name (site) | Rust constant   |
|------------------|-----------------|
| `caret-up`       | `CARET_UP`      |
| `gear-six`       | `GEAR_SIX`      |
| `magnifying-glass` | `MAGNIFYING_GLASS` |
| `github-logo`    | `GITHUB_LOGO`   |
| `x`              | `X`             |

## The `Icon` component

All props except `icon` are optional.

| Prop       | Type                | Default          | Notes |
|------------|---------------------|------------------|-------|
| `icon`     | `IconData`          | — (required)     | The icon constant, e.g. `PLAY`. |
| `weight`   | `Signal<IconWeight>`| `Regular`        | Style/weight; pass a literal variant or a `Signal`/`Memo` to make it reactive. |
| `size`     | `TextProp`          | `"1em"`          | Width & height. A unitless number or a string with units (`px`, `%`, `em`, `rem`, `pt`, `cm`, `mm`, `in`). |
| `color`    | `TextProp`          | `"currentColor"` | Any CSS color, or `currentColor` to inherit. |
| `mirrored` | `Signal<bool>`      | `false`          | Flip horizontally (useful for RTL). |

### Weights

`IconWeight` has six variants: `Thin`, `Light`, `Regular`, `Bold`, `Fill`, `Duotone`.

### Sizing

`size` defaults to `1em`, so a bare `<Icon icon=HEART />` scales with the surrounding font size. In this codebase we pass an explicit pixel size for predictable layout:

```rust
<Icon icon=CARET_LEFT size="18px" weight=IconWeight::Bold />
```

### Color

`color` defaults to `currentColor`, meaning the icon inherits the text color of its container. The idiomatic way to tint an icon here is therefore to set `color` on a parent element (often via a `styled!`/`css!` class) rather than on the `<Icon>` itself:

```rust
let danger = css! { color: #c0392b; };
view! {
    <button class=danger>
        <Icon icon=TRASH size="16px" weight=IconWeight::Regular />
    </button>
}
```

Pass `color` explicitly only when an icon must differ from its surrounding text:

```rust
<Icon icon=HEART color="#AE2983" weight=IconWeight::Fill size="32px" />
```

## Reactive props

`weight`, `size`, `color`, and `mirrored` accept reactive values. For example, to fill an icon while a row is active, derive the weight from a signal:

```rust
<Icon
    icon=STAR
    weight=Signal::derive(move || {
        if active.get() { IconWeight::Fill } else { IconWeight::Regular }
    })
/>
```

When swapping between two distinct icons (not just weights), prefer a `<Show>` over a reactive prop — this is the pattern used by the navbar's active/inactive states in `app.rs`:

```rust
<Show
    when=is_active
    fallback=move || view! { <Icon icon=icon size="32px" weight=IconWeight::Regular /> }
>
    <Icon icon=icon size="32px" weight=IconWeight::Fill />
</Show>
```

## Project conventions

- Group the icons you use into a single import alongside `Icon`/`IconWeight`, sorted to match the rest of the file's imports:
  ```rust
  use phosphor_leptos::{Icon, IconWeight, CARET_DOWN, CARET_UP, STOP, X};
  ```
- Prefer explicit `px` sizes over the `1em` default so icon sizing doesn't drift with font changes.
- Tint via the parent's `color` (`currentColor` inheritance); reserve the `color` prop for one-off accents.

## Advanced: raw SVG paths

`IconData::get(weight)` returns the raw `<path>` markup for a single weight, for the rare case you need to build the SVG yourself:

```rust
use phosphor_leptos::{ACORN, IconWeight};

let raw = ACORN.get(IconWeight::Regular);
view! { <svg viewBox="0 0 256 256" inner_html=raw /> }
```