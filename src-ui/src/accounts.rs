use std::collections::HashMap;

use leptos::prelude::*;
use serde::Serialize;
use yaminabe_launcher_shared::datamodels::{AccountSession, AccountSummary};

use crate::ipc;

#[derive(Serialize)]
struct SelectArgs {
    uuid: Option<String>,
}

#[derive(Serialize)]
struct AccountArgs {
    uuid: String,
}

/// A player's head, by account UUID.
pub fn avatar_url(uuid: &str) -> String {
    format!("https://mc-heads.net/avatar/{}/64", uuid.replace('-', ""))
}

/// What keeps an account from playing, if anything, as a short label and the
/// reason behind it.
pub fn session_problem(session: &AccountSession) -> Option<(&'static str, String)> {
    match session {
        AccountSession::NeedsSignIn(reason) => Some(("Sign-in expired", reason.clone())),
        AccountSession::NotOwned(reason) => Some(("Game not owned", reason.clone())),
        AccountSession::Valid | AccountSession::Renamed | AccountSession::Unchecked => None,
    }
}

/// The saved Microsoft accounts, the one launches use, and how each one's
/// sign-in stands, shared by the navbar's account switcher and Settings so a
/// change in either shows in both.
#[derive(Clone, Copy)]
pub struct Accounts {
    /// Bumped after any change to the accounts, which refetches them.
    pub revision: RwSignal<u32>,
    /// `None` until the first fetch has landed.
    list: RwSignal<Option<Vec<AccountSummary>>>,
    selected: RwSignal<Option<String>>,
    /// Each account's sign-in by UUID, `None` while its check is out.
    sessions: RwSignal<HashMap<String, Option<AccountSession>>>,
}

impl Accounts {
    /// Load the accounts, reload them on every revision, check each one's
    /// sign-in once, and provide the store as context.
    pub fn provide() -> Self {
        let accounts = Accounts {
            revision: RwSignal::new(0),
            list: RwSignal::new(None),
            selected: RwSignal::new(None),
            sessions: RwSignal::new(HashMap::new()),
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
                    Ok(list) => {
                        accounts.check_new(&list);
                        accounts.list.set(Some(list));
                    }
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

    /// Check the sign-in of each account in `list` not checked yet. One check
    /// per account per run is enough: a launch renews the sign-in anyway.
    fn check_new(self, list: &[AccountSummary]) {
        for account in list {
            let uuid = account.uuid.clone();
            if self.sessions.with_untracked(|known| known.contains_key(&uuid)) {
                continue;
            }
            self.sessions.update(|known| {
                known.insert(uuid.clone(), None);
            });
            leptos::task::spawn_local(async move {
                let args = AccountArgs { uuid: uuid.clone() };
                let session = match ipc::call::<_, AccountSession>("check_account_session", args).await {
                    Ok(session) => session,
                    Err(e) => {
                        log::warn!("check_account_session failed: {e}");
                        AccountSession::Unchecked
                    }
                };
                // A sign-in made while the check was out has already settled
                // the account.
                if self.sessions.with_untracked(|known| matches!(known.get(&uuid), Some(None))) {
                    let renamed = session == AccountSession::Renamed;
                    self.sessions.update(|known| {
                        known.insert(uuid, Some(session));
                    });
                    if renamed {
                        self.changed();
                    }
                }
            });
        }
    }

    pub fn list(self) -> Vec<AccountSummary> {
        self.list.get().unwrap_or_default()
    }

    /// Whether the accounts have been fetched at least once.
    pub fn loaded(self) -> bool {
        self.list.with(Option::is_some)
    }

    /// The account launches use, if one is selected.
    pub fn selected(self) -> Option<AccountSummary> {
        let uuid = self.selected.get()?;
        self.list.with(|list| list.iter().flatten().find(|a| a.uuid == uuid).cloned())
    }

    pub fn is_selected(self, uuid: &str) -> bool {
        self.selected.with(|selected| selected.as_deref() == Some(uuid))
    }

    /// How `uuid`'s sign-in stands, once its check has come back.
    pub fn session(self, uuid: &str) -> Option<AccountSession> {
        self.sessions.with(|known| known.get(uuid).cloned().flatten())
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

    /// Record that `uuid` has just signed in, from either login dialog, which
    /// needs no check, and refetch the accounts.
    pub fn signed_in(self, uuid: String) {
        self.sessions.update(|known| {
            known.insert(uuid, Some(AccountSession::Valid));
        });
        self.changed();
    }
}
