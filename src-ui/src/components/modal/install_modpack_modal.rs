use crate::components::ui::*;
use crate::curseforge::{
    InstallModpackArgs, call_install_modpack, call_list_project_files, usable_instance_name,
};
use crate::ipc;
use bamboo_css_macro::css;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::{IntoView, component, view, web_sys};
use wasm_bindgen::JsCast;
use yaminabe_launcher_shared::datamodels::{
    AppSettings, ModProjectInfo, ProjectFileInfo, ProjectFileTarget,
};

const PAGE_SIZE: usize = 50;

#[derive(Clone)]
pub struct InstallState {
    /// Which opening of the dialog this is, so that an answer asked for by an
    /// earlier one is told apart, even on the same pack.
    pub opening: u32,
    pub pack: ModProjectInfo,
    pub version: String,
    pub versions: Vec<ProjectFileInfo>,
    pub versions_loading: bool,
    pub versions_error: Option<String>,
    pub versions_done: bool,
}

/// The modpack install dialog for a page: the page keeps one, opens it on a
/// pack, and renders it with `ModpackInstallDialog`.
#[derive(Clone, Copy)]
pub struct ModpackInstaller {
    install: RwSignal<Option<InstallState>>,
    /// How many times the dialog has been opened, which numbers each opening.
    openings: StoredValue<u32>,
    install_name: RwSignal<String>,
    default_location: RwSignal<String>,
}

impl ModpackInstaller {
    /// Start closed, with the install location read from the settings.
    pub fn new() -> Self {
        let installer = ModpackInstaller {
            install: RwSignal::new(None),
            openings: StoredValue::new(0),
            install_name: RwSignal::new(String::new()),
            default_location: RwSignal::new(String::new()),
        };
        leptos::task::spawn_local(async move {
            if let Ok(s) = ipc::call_noargs::<AppSettings>("get_settings").await {
                installer.default_location.set(s.instance_install_dir);
            }
        });
        installer
    }

    /// Open the dialog on `pack` and fetch its first page of versions.
    pub fn open(self, pack: ModProjectInfo) {
        self.install_name.set(usable_instance_name(&pack.name));
        let opening = self.openings.get_value() + 1;
        self.openings.set_value(opening);
        self.install.set(Some(InstallState {
            opening,
            pack,
            version: String::new(),
            versions: vec![],
            versions_loading: false,
            versions_error: None,
            versions_done: false,
        }));
        self.load_versions();
    }

    fn close(self) {
        self.install.set(None);
    }

    /// Apply an answer asked for during `opening`, unless the dialog has been
    /// closed or opened again since, on another pack or the same one.
    fn update_if_still_open(self, opening: u32, apply: impl FnOnce(&mut InstallState)) {
        self.install.update(|opt| {
            if let Some(s) = opt.as_mut().filter(|s| s.opening == opening) {
                apply(s);
            }
        });
    }

    /// Fetch the next page of versions. With none loaded yet, on opening or after
    /// that failed, it is the first page, and picks the newest version.
    fn load_versions(self) {
        let install = self.install;
        // Read in place: this runs on every scroll event near the list's end.
        let Some((project_id, opening, index)) = install.with_untracked(|opt| {
            opt.as_ref()
                .filter(|s| !s.versions_loading && !s.versions_done)
                .map(|s| (s.pack.id.clone(), s.opening, s.versions.len() as u32))
        }) else {
            return;
        };
        install.update(|opt| {
            if let Some(s) = opt {
                s.versions_loading = true;
                s.versions_error = None;
            }
        });
        leptos::task::spawn_local(async move {
            match call_list_project_files(project_id, ProjectFileTarget::Modpack, None, None, index)
                .await
            {
                Ok(mut files) => {
                    self.update_if_still_open(opening, |s| {
                        if index == 0 {
                            s.version = files
                                .first()
                                .and_then(|f| f.source.version_key())
                                .unwrap_or_default();
                        }
                        s.versions_done = files.len() < PAGE_SIZE;
                        s.versions.append(&mut files);
                        s.versions_loading = false;
                    });
                }
                Err(e) => {
                    self.update_if_still_open(opening, |s| {
                        s.versions_error = Some(e);
                        s.versions_loading = false;
                    });
                }
            }
        });
    }

    fn submit(self, ev: SubmitEvent) {
        ev.prevent_default();
        let Some(state) = self.install.get_untracked() else {
            return;
        };
        if state.versions_loading {
            return;
        }
        let Some(form) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlFormElement>().ok())
        else {
            return;
        };
        let Ok(data) = web_sys::FormData::new_with_form(&form) else {
            return;
        };

        // The backend reads the install dir from settings now; still gate on a
        // configured location so we don't kick off an install with none set.
        if self.default_location.get_untracked().trim().is_empty() {
            return;
        }

        let version_key = data.get("version").as_string().unwrap_or_default();
        let Some(ver) = state
            .versions
            .into_iter()
            .find(|v| v.source.version_key().as_deref() == Some(version_key.as_str()))
        else {
            return;
        };
        let source = ver.source;

        let Some(args) = InstallModpackArgs::from_form_data(source, state.pack.logo_url, &data) else {
            return;
        };
        self.install.set(None);

        leptos::task::spawn_local(async move {
            // Install progress flows through the install sidebar's event
            // stream; synchronous IPC failures don't, so log them here.
            if let Err(e) = call_install_modpack(args).await {
                log::error!("install_modpack failed: {e}");
            }
        });
    }
}

/// The install dialog, shown while `installer` has a pack open.
#[component]
pub fn ModpackInstallDialog(installer: ModpackInstaller) -> impl IntoView {
    view! {
        <Show when=move || installer.install.with(Option::is_some)>
            <InstallModpackModal
                install=installer.install
                install_name=installer.install_name
                on_submit=Callback::new(move |ev: SubmitEvent| installer.submit(ev))
                on_load_versions=Callback::new(move |_: ()| installer.load_versions())
                on_close=Callback::new(move |_: ()| installer.close())
            />
        </Show>
    }
}

#[component]
fn InstallModpackModal(
    install: RwSignal<Option<InstallState>>,
    install_name: RwSignal<String>,
    on_submit: Callback<SubmitEvent>,
    on_load_versions: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let pack_strip = css! {
        display: flex;
        align-items: center;
        gap: 14px;
        padding: 14px;
        border-radius: 10px;
        background-color: var(--secondary-color);
        margin-bottom: 24px;
    };
    let pack_strip_logo = css! {
        width: 52px;
        height: 52px;
        border-radius: 6px;
        object-fit: cover;
        flex-shrink: 0;
    };
    let pack_strip_logo_ph = css! {
        width: 52px;
        height: 52px;
        border-radius: 6px;
        flex-shrink: 0;
        background-color: var(--background-color);
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 1.5rem;
    };
    let pack_strip_meta = css! {
        flex: 1;
        min-width: 0;
    };
    let pack_strip_name = css! {
        font-weight: 600;
        font-size: 0.95rem;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let pack_strip_summary = css! {
        font-size: 0.8rem;
        opacity: 0.55;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    };
    let version_list = version_list_class();
    let version_note = version_note_class();

    // Split the install state into memos, each reading only its own field, so
    // picking a version only re-runs the row classes. Re-rendering the whole
    // list would reset its scroll position on every click.
    let pack = Memo::new(move |_| install.with(|s| s.as_ref().map(|s| s.pack.clone())));
    let versions = Memo::new(move |_| {
        install.with(|s| s.as_ref().map(|s| s.versions.clone()).unwrap_or_default())
    });
    let selected = Memo::new(move |_| {
        install.with(|s| s.as_ref().map(|s| s.version.clone()).unwrap_or_default())
    });
    let loading = Memo::new(move |_| {
        install.with(|s| s.as_ref().is_none_or(|s| s.versions_loading))
    });
    let versions_error = Memo::new(move |_| {
        install.with(|s| s.as_ref().and_then(|s| s.versions_error.clone()))
    });

    let on_scroll = move |ev: leptos::ev::Event| {
        let Some(list) = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let remaining = list.scroll_height() - list.scroll_top() - list.client_height();
        // After a failed page, wait for "Try again" instead of retrying on every scroll.
        if remaining <= 8 && versions_error.with_untracked(Option::is_none) {
            on_load_versions.run(());
        }
    };

    // Only a failed first page takes the list's place. A later one shows below
    // the versions already loaded, which can still be picked and installed.
    let first_page_error = Memo::new(move |_| {
        versions_error.get().filter(|_| versions.with(Vec::is_empty))
    });

    let rows = move || {
        let list = versions.get();
        if list.is_empty() {
            let note = if loading.get() { "Loading versions…" } else { "No versions available." };
            return view! { <p class=version_note>{note}</p> }.into_any();
        }
        list.into_iter()
            .map(|file| {
                let value = file.source.version_key().unwrap_or_default();
                let picked = value.clone();
                view! {
                    <VersionRow
                        label=file.display_name
                        size=file.size
                        release_type=file.release_type
                        selected=Signal::derive(move || selected.get() == value)
                        on_pick=Callback::new(move |_: ()| {
                            let value = picked.clone();
                            install.update(|state| {
                                if let Some(state) = state { state.version = value; }
                            });
                        })
                    />
                }
            })
            .collect_view()
            .into_any()
    };

    view! {
        <ModalOverlay>
            <ModalBox>
                <form on:submit=move |ev| on_submit.run(ev)>
                    <ModalBody>
                        <h2 style="margin: 0 0 20px 0;">"Configure Instance"</h2>

                        {move || pack.get().map(|pack| {
                            let logo_view = if let Some(url) = pack.logo_url.clone() {
                                view! { <img class=pack_strip_logo src=url alt=""/> }.into_any()
                            } else {
                                view! { <div class=pack_strip_logo_ph>"📦"</div> }.into_any()
                            };
                            view! {
                                <div class=pack_strip>
                                    {logo_view}
                                    <div class=pack_strip_meta>
                                        <div class=pack_strip_name>{pack.name}</div>
                                        <div class=pack_strip_summary>{pack.summary}</div>
                                    </div>
                                </div>
                            }
                        })}

                        <FormFields>
                            <FormField label="Instance Name">
                                <TextInput
                                    default_value=install_name.get_untracked()
                                    name="instance_name"
                                    placeholder="My Modpack"
                                />
                            </FormField>
                            <FormField label="Modpack Version">
                                {move || match first_page_error.get() {
                                    Some(err) => view! {
                                        <VersionPageError error=err on_retry=on_load_versions />
                                    }.into_any(),
                                    None => view! {
                                        // The form reads the pick by name, so the
                                        // selection rides along in a hidden field.
                                        <input type="hidden" name="version" prop:value=move || selected.get() />
                                        <div class=version_list on:scroll=on_scroll>
                                            {rows}
                                            <Show when=move || loading.get() && !versions.with(Vec::is_empty)>
                                                <p class=version_note>"Loading more…"</p>
                                            </Show>
                                            {move || versions_error.get().map(|err| view! {
                                                <VersionPageError error=err on_retry=on_load_versions />
                                            })}
                                        </div>
                                    }.into_any(),
                                }}
                            </FormField>
                            <FormField label="Category">
                                <TextInput
                                    name="category"
                                    placeholder="e.g. Modded, Survival (optional)"
                                />
                            </FormField>
                        </FormFields>
                    </ModalBody>
                    <ModalFooter>
                        <Button
                            variant=ButtonVariant::Secondary
                            on_click=Callback::new(move |_| on_close.run(()))
                        >
                            "Cancel"
                        </Button>
                        <Button
                            variant=ButtonVariant::Primary
                            button_type="submit"
                            // No pick while no version has loaded, so nothing to install.
                            disabled=Signal::derive(move || loading.get() || selected.with(String::is_empty))
                        >
                            "Install →"
                        </Button>
                    </ModalFooter>
                </form>
            </ModalBox>
        </ModalOverlay>
    }
}
