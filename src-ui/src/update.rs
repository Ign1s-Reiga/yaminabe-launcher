use leptos::prelude::*;
use yaminabe_launcher_shared::ipc::{ReleaseInfo, UpdateCheck, UpdateProgress};

use crate::ipc;

/// Where the launcher stands with its own updates.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    /// No check has run. A development build waits until it is asked.
    Unchecked,
    Checking,
    UpToDate,
    CheckFailed(String),
    Available(ReleaseInfo),
    /// Downloading, with the latest progress reported; the installer then
    /// closes the launcher and starts the new version.
    Installing(ReleaseInfo, Option<UpdateProgress>),
    InstallFailed(ReleaseInfo, String),
}

impl UpdateState {
    /// The release on offer, while there is one to install.
    pub fn offered(&self) -> Option<&ReleaseInfo> {
        match self {
            UpdateState::Available(release) | UpdateState::InstallFailed(release, _) => Some(release),
            _ => None,
        }
    }

    /// Whether a check or an install is in flight.
    pub fn is_busy(&self) -> bool {
        matches!(self, UpdateState::Checking | UpdateState::Installing(..))
    }
}

/// The launcher's update state, shared by Settings and the navbar.
#[derive(Clone, Copy)]
pub struct Updates(pub RwSignal<UpdateState>);

impl Updates {
    /// Provide the shared state, follow download progress, and look for a
    /// newer release once per launch when the backend is a release build.
    pub fn provide() -> Self {
        let updates = Updates(RwSignal::new(UpdateState::Unchecked));
        provide_context(updates);
        ipc::on_event::<UpdateProgress, _>("update-download-progress", move |progress| {
            updates.0.update(|state| {
                if let UpdateState::Installing(_, latest) = state {
                    *latest = Some(progress);
                }
            });
        });
        leptos::task::spawn_local(async move {
            match ipc::call_noargs::<bool>("checks_updates_on_launch").await {
                Ok(true) => updates.check().await,
                Ok(false) => {}
                Err(e) => log::error!("checks_updates_on_launch failed: {e}"),
            }
        });
        updates
    }

    pub async fn check(self) {
        self.0.set(UpdateState::Checking);
        let state = match ipc::call_noargs::<UpdateCheck>("check_for_update").await {
            Ok(UpdateCheck::UpToDate) => UpdateState::UpToDate,
            Ok(UpdateCheck::Available(release)) => UpdateState::Available(release),
            Err(e) => UpdateState::CheckFailed(e),
        };
        self.0.set(state);
    }

    /// Download and install `release`. Success never returns here: the
    /// installer closes the launcher, and starts the new version when done.
    pub async fn install(self, release: ReleaseInfo) {
        self.0.set(UpdateState::Installing(release.clone(), None));
        if let Err(e) = ipc::call_noargs::<()>("install_update").await {
            self.0.set(UpdateState::InstallFailed(release, e));
        }
    }
}
