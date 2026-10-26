//! Panel/popup integration follows pop-os/cosmic-applet-template (libcosmic API).
//! No service-management logic belongs in this module.
#[path = "ui/downloads.rs"]
mod downloads;
#[path = "ui/editor.rs"]
mod editor;
#[path = "ui/icons.rs"]
mod icons;
#[path = "ui/localization.rs"]
mod localization;
#[path = "ui/manager.rs"]
mod manager;
#[path = "ui/navigation.rs"]
mod navigation;
#[path = "ui/setup.rs"]
mod setup;
#[path = "ui/view.rs"]
mod view;
use cosmic::iced::futures::{SinkExt, Stream};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::{
    iced::{Limits, Subscription, window::Id},
    prelude::*,
};
use herd::{
    backend::Client,
    icons::{IconSpec, LoadedIcon},
    model::Snapshot,
};
use localization::tr;
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Panel,
    Management,
}

pub struct Flags {
    pub mode: Mode,
    pub review: Option<String>,
}

impl cosmic::app::CosmicFlags for Flags {
    type SubCommand = &'static str;
    type Args = Vec<String>;
    fn action(&self) -> Option<&Self::SubCommand> {
        self.review.as_ref().map(|_| &"review-download")
    }
    fn args(&self) -> Vec<&str> {
        self.review.as_deref().into_iter().collect()
    }
}

#[derive(Default)]
pub struct Applet {
    core: cosmic::Core,
    mode: Mode,
    popup: Option<Id>,
    controller: Option<Arc<Client>>,
    snapshots: Vec<Snapshot>,
    error: Option<String>,
    discovery_error: Option<String>,
    setup_required: bool,
    setup_requested: bool,
    router_start_requested: bool,
    pending_close: bool,
    pending_stop: Option<String>,
    pending_removal: Option<herd::backend::protocol::RemovalPlan>,
    removal_generation: u64,
    removal_review: Option<u64>,
    busy: bool,
    refreshing: bool,
    connecting: bool,
    operation_label: Option<String>,
    shared_operation: Option<String>,
    icons: icons::ModelIcons,
    editors: std::collections::HashMap<Id, editor::Editor>,
    active_editor: Option<Id>,
    navigation: navigation::Navigation,
    downloads: downloads::Browser,
    setup: setup::Form,
}

#[derive(Clone)]
pub enum Message {
    Editor(editor::Event),
    Setup(setup::Event),
    TogglePopup,
    OpenManager,
    CloseManager,
    CloseRequested(Id),
    WindowClosed(Id),
    LaunchManager(Option<String>, Option<String>),
    ManagerLaunched(bool),
    Connected(Result<Arc<Client>, String>),
    Refresh,
    ConfigurationChanged,
    ServerUp,
    ShowSetup,
    ShowAdd,
    Navigate(navigation::Page),
    LibraryQuery(String),
    LibrarySort,
    DownloadQuery(String),
    SearchDownloads,
    DownloadBack,
    CancelDownload,
    SearchResults(u64, Result<Vec<String>, String>),
    InspectRepository(String),
    RepositoryResult(u64, Result<herd::downloads::Catalogue, String>),
    ChooseDownload(herd::downloads::Plan),
    ReviewDownload(String),
    DownloadPlanReady(u64, Result<herd::downloads::Plan, String>),
    SubmitDownload,
    DownloadSubmitted(Result<(), String>),
    PauseDownload(String),
    Edit(String),
    Refreshed(Result<herd::backend::protocol::Report, String>),
    Start(String),
    AskStop(String),
    ConfirmStop,
    CancelStop,
    AskRemove(String),
    RemovalReady(u64, Result<herd::backend::protocol::RemovalPlan, String>),
    ConfirmRemove,
    CancelRemove,
    Removed(String, Result<(), String>),
    Finished(Result<(), String>),
    IconLoaded(IconSpec, Result<LoadedIcon, String>),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Controller contains an opaque backend, intentionally not Debug.
        f.write_str(match self {
            Self::Editor(_) => "Editor",
            Self::Setup(_) => "Setup",
            Self::TogglePopup => "TogglePopup",
            Self::OpenManager => "OpenManager",
            Self::CloseManager => "CloseManager",
            Self::CloseRequested(_) => "CloseRequested",
            Self::WindowClosed(_) => "WindowClosed",
            Self::LaunchManager(_, _) => "LaunchManager",
            Self::ManagerLaunched(_) => "ManagerLaunched",
            Self::Connected(_) => "Connected",
            Self::Refresh => "Refresh",
            Self::ConfigurationChanged => "ConfigurationChanged",
            Self::ServerUp => "ServerUp",
            Self::ShowSetup => "ShowSetup",
            Self::ShowAdd => "ShowAdd",
            Self::Navigate(_) => "Navigate",
            Self::LibraryQuery(_) => "LibraryQuery",
            Self::LibrarySort => "LibrarySort",
            Self::DownloadQuery(_) => "DownloadQuery",
            Self::SearchDownloads => "SearchDownloads",
            Self::DownloadBack => "DownloadBack",
            Self::CancelDownload => "CancelDownload",
            Self::SearchResults(_, _) => "SearchResults",
            Self::InspectRepository(_) => "InspectRepository",
            Self::RepositoryResult(_, _) => "RepositoryResult",
            Self::ChooseDownload(_) => "ChooseDownload",
            Self::ReviewDownload(_) => "ReviewDownload",
            Self::DownloadPlanReady(_, _) => "DownloadPlanReady",
            Self::SubmitDownload => "SubmitDownload",
            Self::DownloadSubmitted(_) => "DownloadSubmitted",
            Self::PauseDownload(_) => "PauseDownload",
            Self::Edit(_) => "Edit",
            Self::Refreshed(_) => "Refreshed",
            Self::Start(_) => "Start",
            Self::AskStop(_) => "AskStop",
            Self::ConfirmStop => "ConfirmStop",
            Self::CancelStop => "CancelStop",
            Self::AskRemove(_) => "AskRemove",
            Self::RemovalReady(_, _) => "RemovalReady",
            Self::ConfirmRemove => "ConfirmRemove",
            Self::CancelRemove => "CancelRemove",
            Self::Removed(_, _) => "Removed",
            Self::Finished(_) => "Finished",
            Self::IconLoaded(_, _) => "IconLoaded",
        })
    }
}

async fn connect() -> Result<Arc<Client>, String> {
    Client::connect_current()
        .await
        .map(Arc::new)
        .map_err(|error| error.to_string())
}

fn configuration_events() -> impl Stream<Item = Message> {
    cosmic::iced::stream::channel(
        1,
        |mut output: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
            loop {
                if backend_events(&mut output).await.is_err() {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if output.send(Message::ConfigurationChanged).await.is_err() {
                        return;
                    }
                }
            }
        },
    )
}

async fn backend_events(
    output: &mut cosmic::iced::futures::channel::mpsc::Sender<Message>,
) -> anyhow::Result<()> {
    use cosmic::iced::futures::StreamExt;
    let client = Client::connect().await?;
    let mut events = client.events().await?;
    if output.send(Message::ConfigurationChanged).await.is_err() {
        return Ok(());
    }
    while events.next().await.is_some() {
        if output.send(Message::ConfigurationChanged).await.is_err() {
            return Ok(());
        }
    }
    Err(anyhow::anyhow!("Backend disconnected"))
}

impl cosmic::Application for Applet {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = crate::APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, flags: Flags) -> (Self, Task<cosmic::Action<Message>>) {
        let mode = flags.mode;
        let mut app = Self {
            core,
            mode,
            connecting: true,
            ..Default::default()
        };
        let title = if mode == Mode::Management
            && let Some(id) = app.core.main_window_id()
        {
            app.set_window_title(tr("app-title"), id)
        } else {
            Task::none()
        };
        (
            app,
            Task::batch([
                title,
                flags
                    .review
                    .map(|model| {
                        cosmic::task::message(cosmic::Action::App(Message::ReviewDownload(model)))
                    })
                    .unwrap_or_else(Task::none),
                Task::perform(connect(), |result| {
                    cosmic::Action::App(Message::Connected(result))
                }),
            ]),
        )
    }

    fn dbus_activation(
        &mut self,
        msg: cosmic::dbus_activation::Message,
    ) -> Task<cosmic::Action<Message>> {
        if let cosmic::dbus_activation::Details::ActivateAction { action, args } = msg.msg
            && action == "review-download"
            && args.len() == 1
        {
            return self.update(Message::ReviewDownload(args[0].clone()));
        }
        Task::none()
    }

    fn on_app_exit(&mut self) -> Option<Message> {
        // The pinned toolkit invokes this before native header Close; its
        // on_close_requested hook runs only after the surface has closed.
        (self.mode == Mode::Management).then_some(Message::CloseManager)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(
            if self.mode == Mode::Management && self.core.main_window_id() == Some(id) {
                Message::CloseManager
            } else {
                Message::WindowClosed(id)
            },
        )
    }

    fn view(&self) -> Element<'_, Message> {
        match self.mode {
            Mode::Panel => self.panel_view(),
            Mode::Management => self.page_view(),
        }
    }

    fn view_window(&self, id: Id) -> Element<'_, Message> {
        if self.popup == Some(id) {
            self.popup_view()
        } else {
            cosmic::widget::Space::new().into()
        }
    }

    fn nav_model(&self) -> Option<&cosmic::widget::nav_bar::Model> {
        (self.mode == Mode::Management).then_some(&self.navigation.model)
    }

    fn header_end(&self) -> Vec<Element<'_, Message>> {
        if self.mode != Mode::Management {
            return Vec::new();
        }
        vec![
            self.add_menu(),
            cosmic::widget::button::icon(cosmic::widget::icon::from_name("view-refresh-symbolic"))
                .name(tr("refresh"))
                .on_press_maybe(
                    (!self.busy
                        && !self.refreshing
                        && !self.connecting
                        && self.pending_stop.is_none())
                    .then_some(Message::Refresh),
                )
                .into(),
        ]
    }

    fn on_nav_select(&mut self, id: cosmic::widget::nav_bar::Id) -> Task<cosmic::Action<Message>> {
        if let Some(page) = self.navigation.model.data::<navigation::Page>(id).copied() {
            self.select_page(page);
        }
        Task::none()
    }

    fn context_drawer(&self) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        self.settings_drawer()
    }

    fn on_context_drawer(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.core.window.show_context
            && let Some(id) = self.active_editor
        {
            return self.editor_event(editor::Event::Close(id));
        }
        Task::none()
    }

    fn dialog(&self) -> Option<Element<'_, Message>> {
        self.removal_confirmation()
            .or_else(|| self.stop_confirmation())
            .or_else(|| self.download_confirmation())
            .or_else(|| self.editor_confirmation())
    }

    fn on_escape(&mut self) -> Task<cosmic::Action<Message>> {
        self.cancel_removal_review();
        self.downloads.cancel_review();
        self.downloads.selected = None;
        self.pending_stop = None;
        self.cancel_editor_confirmation()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![Subscription::run(configuration_events)];
        if self.mode == Mode::Management {
            subscriptions.push(cosmic::iced::window::close_requests().map(Message::CloseRequested));
        }
        if self.controller.is_some() && self.pending_stop.is_none() {
            subscriptions.push(
                cosmic::iced::time::every(Duration::from_secs(
                    if self
                        .snapshots
                        .iter()
                        .any(|s| matches!(s.state, herd::model::State::Downloading { .. }))
                    {
                        1
                    } else {
                        3
                    },
                ))
                .map(|_| Message::Refresh),
            );
        }
        Subscription::batch(subscriptions)
    }

    fn update(&mut self, message: Message) -> Task<cosmic::Action<Message>> {
        match message {
            Message::Editor(event) => return self.editor_event(event),
            Message::Setup(event) => return self.setup_event(event),
            Message::ShowSetup => self.select_page(navigation::Page::Setup),
            Message::ShowAdd => self.select_page(navigation::Page::Import),
            Message::Navigate(page) => self.select_page(page),
            Message::LibraryQuery(query) => self.navigation.query = query,
            Message::LibrarySort => self.navigation.reverse = !self.navigation.reverse,
            Message::DownloadQuery(query) => self.downloads.change_query(query),
            Message::SearchDownloads => {
                let query = self.downloads.query.trim().to_owned();
                if herd::downloads::validate_repo(&query).is_ok() {
                    return self.update(Message::InspectRepository(query));
                }
                let ticket = self.downloads.request();
                self.downloads.repo = None;
                self.downloads.results.clear();
                self.downloads.searched = true;
                return Task::perform(
                    async move {
                        let hub = Client::connect().await.map_err(|e| e.to_string())?;
                        hub.search(&query).await.map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::SearchResults(ticket, r)),
                );
            }
            Message::SearchResults(ticket, result) => self.downloads.search_result(ticket, result),
            Message::InspectRepository(repo) => {
                let ticket = self.downloads.request();
                self.downloads.repo = Some(repo.clone());
                return Task::perform(
                    async move {
                        let hub = Client::connect().await.map_err(|e| e.to_string())?;
                        hub.files(&repo).await.map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::RepositoryResult(ticket, r)),
                );
            }
            Message::RepositoryResult(ticket, result) => {
                self.downloads.catalogue_result(ticket, result)
            }
            Message::ChooseDownload(plan) => self.downloads.selected = Some(plan),
            Message::CancelDownload => {
                self.downloads.cancel_review();
                self.downloads.selected = None;
            }
            Message::DownloadBack => {
                self.downloads.ticket = self.downloads.ticket.wrapping_add(1);
                self.downloads.busy = false;
                self.downloads.repo = None;
                self.downloads.catalogue = None;
                self.downloads.selected = None;
                self.downloads.error = None;
            }
            Message::ReviewDownload(model) => {
                if self.mode == Mode::Panel {
                    return self.open_manager_review(Some(model));
                }
                self.select_page(navigation::Page::Downloads);
                let ticket = self.downloads.request_review();
                let inspect = Task::perform(
                    async move {
                        let hub = Client::connect().await.map_err(|e| e.to_string())?;
                        hub.plan(&model).await.map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::DownloadPlanReady(ticket, r)),
                );
                return inspect;
            }
            Message::DownloadPlanReady(ticket, result) => {
                self.downloads.plan_result(ticket, result)
            }
            Message::SubmitDownload if !self.busy => {
                let (Some(controller), Some(plan)) =
                    (self.controller.clone(), self.downloads.selected.take())
                else {
                    return Task::none();
                };
                self.busy = true;
                self.operation_label = Some(tr("submitting-download"));
                self.error = None;
                self.downloads.busy = true;
                self.select_page(navigation::Page::Downloads);
                return Task::perform(
                    async move {
                        let id = controller
                            .download(&plan, plan.size_bytes)
                            .await
                            .map_err(|e| e.to_string())?;
                        controller.wait(&id).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::DownloadSubmitted(r)),
                );
            }
            Message::DownloadSubmitted(result) => {
                self.busy = false;
                self.operation_label = None;
                self.downloads.busy = false;
                match result {
                    Ok(()) => {
                        self.downloads.selected = None;
                        return self.refresh();
                    }
                    Err(error) => {
                        self.error = Some(error.clone());
                        self.downloads.error = Some(error);
                    }
                }
            }
            Message::PauseDownload(id) if !self.busy => {
                let Some(controller) = self.controller.clone() else {
                    return Task::none();
                };
                self.busy = true;
                self.operation_label = Some(tr("pausing-download"));
                return Task::perform(
                    async move {
                        let job = controller.pause(&id).await.map_err(|e| e.to_string())?;
                        controller.wait(&job).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Finished(r)),
                );
            }
            Message::Edit(id) => return self.open_editor(id),
            Message::ConfigurationChanged => {
                if self.controller.is_some() {
                    return self.refresh();
                }
                if !self.connecting && self.error.is_none() {
                    self.connecting = true;
                    return Task::perform(connect(), |r| {
                        cosmic::Action::App(Message::Connected(r))
                    });
                }
            }
            Message::ServerUp if !self.busy => {
                let Some(controller) = self.controller.clone() else {
                    return Task::none();
                };
                self.busy = true;
                self.operation_label = Some(tr("starting-router"));
                return Task::perform(
                    async move {
                        let id = controller.server_up().await.map_err(|e| e.to_string())?;
                        controller.wait(&id).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Finished(r)),
                );
            }
            Message::OpenManager => return self.open_manager(),
            Message::CloseRequested(id)
                if self.mode == Mode::Management && self.core.main_window_id() == Some(id) =>
            {
                return self.close_manager();
            }
            Message::CloseManager => return self.close_manager(),
            Message::LaunchManager(token, review) => {
                return Task::perform(manager::launch(token, review), |ok| {
                    cosmic::Action::App(Message::ManagerLaunched(ok))
                });
            }
            Message::ManagerLaunched(false) => {
                self.error = Some(tr("app-launch-error"));
            }
            Message::IconLoaded(source, result) => self.icons.complete(source, result),
            Message::Connected(result) => {
                self.connecting = false;
                match result {
                    Ok(controller) => {
                        self.error = None;
                        self.controller = Some(controller);
                        return self.refresh();
                    }
                    Err(error) => {
                        self.controller = None;
                        self.error = Some(error);
                    }
                }
            }
            Message::Refresh => {
                if self.controller.is_none() {
                    if self.connecting {
                        return Task::none();
                    }
                    self.connecting = true;
                    self.error = None;
                    return Task::perform(connect(), |r| {
                        cosmic::Action::App(Message::Connected(r))
                    });
                }
                if !self.refreshing {
                    return self.refresh();
                }
            }
            Message::Refreshed(snapshots) => {
                let mut artwork = Task::none();
                match snapshots {
                    Ok(report) => {
                        let startup = self.startup_action(&report);
                        self.setup_required = report.setup_required;
                        self.snapshots = report.models.into_iter().map(Into::into).collect();
                        self.discovery_error = (!report.error.is_empty()).then_some(report.error);
                        self.shared_operation = report
                            .operations
                            .iter()
                            .find(|o| o.status == "running")
                            .map(|o| format!("{} {}", localization::operation(&o.kind), o.model));
                        artwork = self.icons.schedule(&self.snapshots);
                        if let Some(startup) = startup {
                            artwork = Task::batch([
                                artwork,
                                cosmic::task::message(cosmic::Action::App(startup)),
                            ]);
                        }
                        if self
                            .pending_stop
                            .as_ref()
                            .is_some_and(|id| !self.snapshots.iter().any(|s| &s.model.id == id))
                        {
                            self.pending_stop = None;
                        }
                    }
                    Err(error) => {
                        self.pending_stop = None;
                        self.discovery_error =
                            Some(format!("{error}\n{}", tr("stale-observations")));
                    }
                }
                self.refreshing = false;
                return artwork;
            }
            Message::Start(id) => return self.operate(id, false),
            Message::AskStop(id) if !self.busy => self.pending_stop = Some(id),
            Message::ConfirmStop => {
                if let Some(id) = self.pending_stop.clone() {
                    return self.operate(id, true);
                }
            }
            Message::CancelStop => self.pending_stop = None,
            Message::AskRemove(model) if !self.busy => {
                if self
                    .editors
                    .values()
                    .any(|e| e.model == model && (e.dirty() || e.saving))
                {
                    self.error = Some(tr("remove-draft-warning"));
                    return Task::none();
                }
                let Some(client) = self.controller.clone() else {
                    return Task::none();
                };
                self.error = None;
                // Inspection is cancellable presentation work, not a mutation.
                // Its late completion must never release an operation's busy flag.
                let ticket = self.begin_removal_review();
                return Task::perform(
                    async move { client.removal_plan(&model).await.map_err(|e| e.to_string()) },
                    move |r| cosmic::Action::App(Message::RemovalReady(ticket, r)),
                );
            }
            Message::RemovalReady(ticket, result) => {
                if self.removal_review != Some(ticket) {
                    return Task::none();
                }
                self.removal_review = None;
                match result {
                    Ok(plan) => self.pending_removal = Some(plan),
                    Err(error) => self.error = Some(error),
                }
            }
            Message::CancelRemove => self.cancel_removal_review(),
            Message::ConfirmRemove if !self.busy => {
                let (Some(client), Some(plan)) =
                    (self.controller.clone(), self.pending_removal.take())
                else {
                    return Task::none();
                };
                let model = plan.model.clone();
                self.busy = true;
                self.operation_label = Some(tr("removing-model"));
                return Task::perform(
                    async move {
                        let job = client
                            .remove(&plan, true)
                            .await
                            .map_err(|e| e.to_string())?;
                        client.wait(&job).await.map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::Removed(model.clone(), r)),
                );
            }
            Message::Removed(model, result) => {
                self.busy = false;
                self.operation_label = None;
                self.error = result.err();
                if self.error.is_none() {
                    self.editors.retain(|_, editor| editor.model != model);
                    if self
                        .active_editor
                        .is_some_and(|id| !self.editors.contains_key(&id))
                    {
                        self.active_editor = None;
                        self.core.set_show_context(false);
                    }
                }
                return self.refresh();
            }
            Message::Finished(result) => {
                self.busy = false;
                self.operation_label = None;
                self.error = result.err();
                return self.refresh();
            }
            Message::TogglePopup if self.mode == Mode::Panel => {
                return if let Some(id) = self.popup.take() {
                    self.pending_stop = None;
                    destroy_popup(id)
                } else {
                    let id = Id::unique();
                    let Some(parent) = self.core.main_window_id() else {
                        return Task::none();
                    };
                    self.popup = Some(id);
                    let mut settings = self
                        .core
                        .applet
                        .get_popup_settings(parent, id, None, None, None);
                    settings.positioner.size_limits = Limits::NONE
                        .min_width(360.)
                        .max_width(440.)
                        .max_height(720.);
                    Task::batch([get_popup(settings), self.refresh()])
                };
            }
            Message::WindowClosed(id) if self.popup == Some(id) => {
                self.popup = None;
                self.pending_stop = None;
            }
            _ => {}
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        (self.mode == Mode::Panel).then(cosmic::applet::style)
    }
}

impl Applet {
    fn close_manager(&mut self) -> Task<cosmic::Action<Message>> {
        self.cancel_removal_review();
        self.downloads.selected = None;
        self.pending_close = true;
        if let Some(id) = self
            .editors
            .iter()
            .find_map(|(id, e)| (e.dirty() && !e.saving).then_some(*id))
        {
            self.active_editor = Some(id);
            self.core.set_show_context(true);
            return self.editor_event(editor::Event::Close(id));
        }
        self.pending_close = false;
        cosmic::iced::exit()
    }

    fn refresh(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(controller) = self.controller.clone() else {
            return Task::none();
        };
        if self.refreshing {
            return Task::none();
        }
        self.refreshing = true;
        Task::perform(
            async move {
                controller
                    .report()
                    .await
                    .map_err(|error| format!("{error:#}"))
            },
            |snapshots| cosmic::Action::App(Message::Refreshed(snapshots)),
        )
    }

    fn operate(&mut self, id: String, stop: bool) -> Task<cosmic::Action<Message>> {
        // Background status reads must not make action buttons flicker or drop
        // clicks. The controller rechecks live state before changing a service.
        if self.busy {
            return Task::none();
        }
        let Some(controller) = self.controller.clone() else {
            return Task::none();
        };
        self.pending_stop = None;
        self.busy = true;
        self.error = None;
        self.operation_label = Some(tr(if stop {
            "stopping-model"
        } else {
            "starting-model"
        }));
        Task::perform(
            async move {
                let result = if stop {
                    controller.stop(&id, true).await
                } else {
                    controller.start(&id).await
                };
                let job = result.map_err(|error| format!("{error:#}"))?;
                controller
                    .wait(&job)
                    .await
                    .map_err(|error| format!("{error:#}"))
            },
            |result| cosmic::Action::App(Message::Finished(result)),
        )
    }
}
