use leptos::{component, IntoView};
use serde::{Deserialize, Serialize};

#[component]
pub fn InstanceCard() -> impl IntoView {}

#[derive(Serialize, Deserialize)]
struct Instance<'a> {
    id: &'a str,
    name: String,
    mods: Vec<String>,
}

impl Instance<'_> {
    
}
