use leptos::prelude::*;
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::AccountSummary;

use crate::ipc;

#[derive(Serialize)]
struct SelectArgs {
    uuid: Option<String>,
}

/// The saved Microsoft accounts and the one launches use, shared by the
/// navbar's account switcher and Settings so a change in either shows in both.
#[derive(Clone, Copy)]
pub struct Accounts {
    /// Bumped after any change to the accounts, which refetches them.
    pub revision: RwSignal<u32>,
    list: RwSignal<Vec<AccountSummary>>,
    selected: RwSignal<Option<String>>,
    signed_in: RwSignal<Option<String>>,
}

impl Accounts {
    /// Load the accounts, reload them on every revision, and provide the store
    /// as context.
    pub fn provide() -> Self {
        let accounts = Accounts {
            revision: RwSignal::new(0),
            list: RwSignal::new(Vec::new()),
            selected: RwSignal::new(None),
            signed_in: RwSignal::new(None),
        };
        Effect::new(move |_| {
            let asked_at = accounts.revision.get();
            leptos::task::spawn_local(async move {
                let list = ipc::call_noargs::<Vec<AccountSummary>>("get_accounts").await;
                let selected = ipc::call_noargs::<Option<String>>("get_selected_account").await;
                // A later revision's answer may already have landed.
                if accounts.revision.get_untracked() != asked_at {
                    return;
                }
                match list {
                    Ok(list) => accounts.list.set(list),
                    Err(e) => log::error!("get_accounts failed: {e}"),
                }
                match selected {
                    Ok(selected) => accounts.selected.set(selected),
                    Err(e) => log::error!("get_selected_account failed: {e}"),
                }
            });
        });
        provide_context(accounts);
        accounts
    }

    pub fn list(self) -> Vec<AccountSummary> {
        self.list.get()
    }

    /// The account launches use, if one is selected.
    pub fn selected(self) -> Option<AccountSummary> {
        let uuid = self.selected.get()?;
        self.list.with(|list| list.iter().find(|a| a.uuid == uuid).cloned())
    }

    pub fn is_selected(self, uuid: &str) -> bool {
        self.selected.with(|selected| selected.as_deref() == Some(uuid))
    }

    /// Make `uuid` the account launches use. The store follows what the
    /// backend saved, so a save that fails changes nothing.
    pub fn select(self, uuid: String) {
        leptos::task::spawn_local(async move {
            match ipc::call::<_, ()>("set_selected_account", SelectArgs { uuid: Some(uuid) }).await {
                Ok(()) => self.changed(),
                Err(e) => log::error!("set_selected_account failed: {e}"),
            }
        });
    }

    /// Refetch the accounts, after a change made elsewhere.
    pub fn changed(self) {
        self.revision.update(|n| *n += 1);
    }

    /// Record that `uuid` has just signed in, from either login dialog, and
    /// refetch the accounts.
    pub fn signed_in(self, uuid: String) {
        self.signed_in.set(Some(uuid));
        self.changed();
    }

    /// The account signed in most recently, tracked.
    pub fn last_signed_in(self) -> Option<String> {
        self.signed_in.get()
    }
}
