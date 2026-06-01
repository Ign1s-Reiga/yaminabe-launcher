use leptos::prelude::*;
use yaminabe_launcher_shared::datatypes::InstanceMeta;

/// Types carrying a stable string id, so a `RwSignal<Vec<_>>` of them can be
/// looked up by id through [`VecSignalExt`].
pub trait HasId {
    fn id(&self) -> &str;
}

impl HasId for InstanceMeta {
    fn id(&self) -> &str {
        &self.id
    }
}

/// Lookup helper for a reactive `Vec` keyed by id, so the `iter().find(id)`
/// dance lives in one place instead of being copy-pasted at every read site.
/// Used for both the running-instances registry and the instance library.
pub trait VecSignalExt<T> {
    /// Map over the first item whose id matches, if present. Reads (and so
    /// tracks) the signal, and only clones what `f` extracts.
    fn map_by_id<R>(&self, id: &str, f: impl FnOnce(&T) -> R) -> Option<R>;
}

impl<T: HasId + Send + Sync + 'static> VecSignalExt<T> for RwSignal<Vec<T>> {
    fn map_by_id<R>(&self, id: &str, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.with(|list| list.iter().find(|item| item.id() == id).map(f))
    }
}