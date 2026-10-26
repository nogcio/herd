//! First-run and local import presentation; persistence stays in config.
use super::localization::tr;
use super::{Applet, Message};
use cosmic::{Element, prelude::*, widget};
use herd::{
    backend::{Client, protocol::Preferences},
    config::Settings,
};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub enum Field {
    Server,
    Compose,
    Libraries,
    Port,
    Id,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;
    use herd::backend::protocol::{Operation, Report};

    fn management() -> Applet {
        Applet {
            mode: super::super::Mode::Management,
            ..Default::default()
        }
    }

    #[test]
    fn failed_update_connection_needs_explicit_refresh_not_notification_replay() {
        let mut app = management();
        let task = app.update(Message::Connected(Err("Backend update failed".into())));
        assert_eq!(task.units(), 0);
        let task = app.update(Message::ConfigurationChanged);
        assert_eq!(task.units(), 0);
        assert!(!app.connecting);
        assert_eq!(app.error.as_deref(), Some("Backend update failed"));
        let task = app.update(Message::Refresh);
        assert!(task.units() > 0);
        assert!(app.connecting);
        assert!(app.error.is_none());
    }

    fn ready() -> Report {
        Report {
            revision: 1,
            ..Default::default()
        }
    }

    #[test]
    fn window_requests_router_once_without_replaying_after_failure_or_reconnect() {
        let mut app = management();
        assert!(matches!(
            app.startup_action(&ready()),
            Some(Message::ServerUp)
        ));
        app.error = Some("Startup failed".into());
        app.controller = None;
        for report in [Report::default(), ready(), ready()] {
            assert!(app.startup_action(&report).is_none());
        }
        assert!(Applet::default().startup_action(&ready()).is_none());
        assert!(app.snapshots.is_empty());
    }

    #[test]
    fn configured_window_never_reinitializes_deleted_preferences_after_reconnect() {
        let mut app = management();
        let binary = tempfile::NamedTempFile::new().unwrap();
        app.setup.server = binary.path().to_string_lossy().into_owned();
        app.setup.compose = app.setup.server.clone();
        assert!(matches!(
            app.startup_action(&ready()),
            Some(Message::ServerUp)
        ));
        app.controller = None;
        let missing = Report {
            setup_required: true,
            ..ready()
        };
        assert!(app.startup_action(&missing).is_none());
        assert_eq!(app.navigation.page, super::super::navigation::Page::Models);
        assert!(app.startup_action(&ready()).is_none());
    }

    #[test]
    fn invalid_configuration_requires_explicit_retry_even_after_file_repair() {
        let mut app = management();
        let mut report = ready();
        report.error = "Invalid configuration".into();
        assert!(app.startup_action(&report).is_none());
        report.error.clear();
        assert!(app.startup_action(&report).is_none());
        report.setup_required = true;
        assert!(app.startup_action(&report).is_none());
    }

    #[test]
    fn startup_waits_for_no_other_operation_without_changing_it() {
        let mut app = management();
        let mut report = ready();
        report.operations.push(Operation {
            status: "running".into(),
            kind: "download".into(),
            ..Default::default()
        });
        assert!(app.startup_action(&report).is_none());
        report.operations.clear();
        app.busy = true;
        assert!(app.startup_action(&report).is_none());
        app.busy = false;
        assert!(matches!(
            app.startup_action(&report),
            Some(Message::ServerUp)
        ));
    }

    #[test]
    fn missing_preferences_use_detected_binaries_once_then_start_only_after_setup() {
        let mut app = management();
        let binary = tempfile::NamedTempFile::new().unwrap();
        app.setup.server = binary.path().to_string_lossy().into_owned();
        app.setup.compose = app.setup.server.clone();
        let report = Report {
            setup_required: true,
            ..ready()
        };
        assert!(matches!(
            app.startup_action(&report),
            Some(Message::Setup(Event::Save))
        ));
        assert!(!app.router_start_requested);
        assert!(app.startup_action(&report).is_none());
        assert!(matches!(
            app.startup_action(&ready()),
            Some(Message::ServerUp)
        ));
    }

    #[test]
    fn missing_runtime_keeps_manual_setup_available_without_starting() {
        let mut app = management();
        app.setup.server.clear();
        let report = Report {
            setup_required: true,
            ..ready()
        };
        assert!(app.startup_action(&report).is_none());
        assert!(app.setup_requested);
        assert!(!app.router_start_requested);
        assert!(!app.busy);
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Edit(Field, String),
    Choose(Option<Field>),
    Chosen(Option<Field>, Result<Option<(PathBuf, String)>, String>),
    Save,
    Import,
    Done(Result<(), String>),
}

pub(super) struct Form {
    server: String,
    compose: String,
    libraries: String,
    port: String,
    id: String,
    file: Option<PathBuf>,
    revision: Option<String>,
    error: Option<String>,
}

impl Default for Form {
    fn default() -> Self {
        let find = |name| {
            std::env::var_os("PATH")
                .and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|p| p.join(name))
                        .find(|p| p.is_absolute() && p.is_file())
                })
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        Self {
            server: find("llama-server"),
            compose: find("process-compose"),
            libraries: String::new(),
            port: "49173".into(),
            id: String::new(),
            file: None,
            revision: None,
            error: None,
        }
    }
}

impl Applet {
    pub(super) fn startup_action(
        &mut self,
        report: &herd::backend::protocol::Report,
    ) -> Option<Message> {
        if self.mode != super::Mode::Management
            || self.busy
            || report.revision == 0
            || self.router_start_requested
            || report.operations.iter().any(|o| o.status == "running")
        {
            return None;
        }
        if report.setup_required {
            if self.setup_requested {
                return None;
            }
            self.setup_requested = true;
            self.select_page(super::navigation::Page::Setup);
            // Only initialize missing preferences; the service validates and
            // probes both binaries and preserves any existing native presets.
            return [&self.setup.server, &self.setup.compose]
                .iter()
                .all(|p| !p.is_empty() && std::path::Path::new(p).is_file())
                .then_some(Message::Setup(Event::Save));
        }
        if !report.error.is_empty() {
            // Repairing/deleting an invalid file later is not authorization for
            // watcher-driven startup or replacement of existing preferences.
            self.setup_requested = true;
            self.router_start_requested = true;
            return None;
        }
        // Once per window lifetime, not on refresh, watcher events or backend
        // reconnect. No automatic retry after an uncertain/failed mutation.
        self.router_start_requested = true;
        Some(Message::ServerUp)
    }

    pub(super) fn setup_view(&self) -> Element<'_, Message> {
        let mut column = widget::column::with_capacity(12)
            .spacing(12)
            .push(widget::text::body(tr("runtime-install-help")));
        if let Some(error) = &self.setup.error {
            column = column.push(super::view::notice(tr("setup-error"), error));
        }
        for (label, value, field) in [
            ("server-executable", &self.setup.server, Field::Server),
            ("compose-executable", &self.setup.compose, Field::Compose),
            (
                "library-directories",
                &self.setup.libraries,
                Field::Libraries,
            ),
            ("loopback-port", &self.setup.port, Field::Port),
        ] {
            let mut row = widget::row::with_capacity(2).spacing(8).push(
                widget::text_input(tr(label), value)
                    .on_input(move |v| Message::Setup(Event::Edit(field, v))),
            );
            if matches!(field, Field::Server | Field::Compose) {
                row = row.push(
                    widget::button::text(tr("browse"))
                        .on_press(Message::Setup(Event::Choose(Some(field)))),
                );
            }
            column = column.push(
                widget::column::with_capacity(2)
                    .spacing(4)
                    .push(widget::text::body(tr(label)))
                    .push(row),
            );
        }
        column
            .push(
                widget::button::suggested(tr("check-save"))
                    .on_press_maybe((!self.busy).then_some(Message::Setup(Event::Save))),
            )
            .into()
    }

    pub(super) fn import_view(&self) -> Element<'_, Message> {
        let space = cosmic::theme::spacing();
        let choose = widget::button::standard(tr(if self.setup.file.is_some() {
            "change-file"
        } else {
            "import-file"
        }))
        .on_press_maybe((!self.busy).then_some(Message::Setup(Event::Choose(None))));
        let file = widget::settings::item::builder(tr("local-gguf"))
            .description(
                self.setup
                    .file
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| tr("no-file-selected")),
            )
            .control(choose);
        let mut fields = widget::settings::section().add(file);
        let mut column = widget::column::with_capacity(3).spacing(space.space_m);
        if let Some(error) = &self.setup.error {
            column = column.push(super::view::notice(tr("import-error"), error));
        }
        if self.setup.file.is_some() {
            fields = fields.add(widget::settings::item(
                tr("preset-id"),
                widget::text_input("", &self.setup.id)
                    .on_input(|v| Message::Setup(Event::Edit(Field::Id, v))),
            ));
        }
        column
            .push(fields)
            .push(
                widget::row::with_capacity(2)
                    .push(widget::Space::new().width(cosmic::iced::Length::Fill))
                    .push(
                        widget::button::suggested(tr("register-file")).on_press_maybe(
                            (!self.busy
                                && self.setup.file.is_some()
                                && herd::config::validate_id(&self.setup.id).is_ok())
                            .then_some(Message::Setup(Event::Import)),
                        ),
                    ),
            )
            .into()
    }

    pub(super) fn setup_event(&mut self, event: Event) -> Task<cosmic::Action<Message>> {
        match event {
            Event::Edit(field, value) => {
                *match field {
                    Field::Server => &mut self.setup.server,
                    Field::Compose => &mut self.setup.compose,
                    Field::Libraries => &mut self.setup.libraries,
                    Field::Port => &mut self.setup.port,
                    Field::Id => &mut self.setup.id,
                } = value
            }
            Event::Choose(field) if !self.busy => {
                self.setup.error = None;
                return Task::perform(
                    async move {
                        let revision = if field.is_none() {
                            Client::connect()
                                .await
                                .map_err(|e| e.to_string())?
                                .configuration()
                                .await
                                .map_err(|e| e.to_string())?
                                .revision
                        } else {
                            String::new()
                        };
                        use cosmic::dialog::file_chooser::{self, open::Dialog};
                        match Dialog::new()
                            .title(if field.is_some() {
                                tr("browse")
                            } else {
                                tr("import-file")
                            })
                            .open_file()
                            .await
                        {
                            Ok(response) => response
                                .url()
                                .to_file_path()
                                .map(|path| Some((path, revision)))
                                .map_err(|_| tr("local-file-required")),
                            Err(file_chooser::Error::Cancelled) => Ok(None),
                            Err(_) => Err(tr("chooser-unavailable")),
                        }
                    },
                    move |r| cosmic::Action::App(Message::Setup(Event::Chosen(field, r))),
                );
            }
            Event::Chosen(field, result) => match result {
                Ok(Some((path, revision))) => match field {
                    Some(Field::Server) => self.setup.server = path.to_string_lossy().into_owned(),
                    Some(Field::Compose) => {
                        self.setup.compose = path.to_string_lossy().into_owned()
                    }
                    None => {
                        if self.setup.id.is_empty() {
                            let name = path.file_stem().unwrap_or_default().to_string_lossy();
                            let id: String = name
                                .chars()
                                .filter(|c| c.is_ascii_alphanumeric() || "-_ .".contains(*c))
                                .take(160)
                                .collect::<String>()
                                .replace(' ', "-");
                            self.setup.id = if id.is_empty() { "model".into() } else { id };
                        }
                        self.setup.file = Some(path);
                        self.setup.revision = Some(revision);
                    }
                    _ => {}
                },
                Err(e) => self.setup.error = Some(e),
                _ => {}
            },
            Event::Save if !self.busy => {
                self.setup.error = None;
                let result = (|| {
                    let path = Settings::path()?;
                    let port: u16 = self.setup.port.parse()?;
                    anyhow::ensure!(port > 0, tr("nonzero-port"));
                    let settings = Settings {
                        server: self.setup.server.clone().into(),
                        process_compose: self.setup.compose.clone().into(),
                        library_dirs: if self.setup.libraries.is_empty() {
                            vec![]
                        } else {
                            std::env::split_paths(&self.setup.libraries).collect()
                        },
                        endpoint: format!("http://127.0.0.1:{port}"),
                        presets: path.with_file_name("models.ini"),
                        state_dir: dirs::state_dir()
                            .ok_or_else(|| anyhow::anyhow!("Cannot locate XDG state directory"))?
                            .join("herd"),
                        working_dir: dirs::home_dir()
                            .ok_or_else(|| anyhow::anyhow!("Cannot locate home directory"))?,
                    };
                    settings.validate()?;
                    Ok::<_, anyhow::Error>(Preferences {
                        server: settings.server.to_string_lossy().into_owned(),
                        process_compose: settings.process_compose.to_string_lossy().into_owned(),
                        library_dirs: settings
                            .library_dirs
                            .into_iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect(),
                        endpoint: settings.endpoint,
                        presets: settings.presets.to_string_lossy().into_owned(),
                        state_dir: settings.state_dir.to_string_lossy().into_owned(),
                        working_dir: settings.working_dir.to_string_lossy().into_owned(),
                    })
                })();
                let preferences = match result {
                    Ok(v) => v,
                    Err(e) => {
                        self.setup.error = Some(e.to_string());
                        return Task::none();
                    }
                };
                self.busy = true;
                return Task::perform(
                    async move {
                        let c = Client::connect().await.map_err(|e| e.to_string())?;
                        let id = c.setup(preferences).await.map_err(|e| e.to_string())?;
                        c.wait(&id).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Setup(Event::Done(r))),
                );
            }
            Event::Import if !self.busy => {
                self.setup.error = None;
                let (Some(c), Some(file), Some(revision)) = (
                    self.controller.clone(),
                    self.setup.file.clone(),
                    self.setup.revision.clone(),
                ) else {
                    return Task::none();
                };
                let id = self.setup.id.clone();
                self.busy = true;
                return Task::perform(
                    async move {
                        let file = file.to_str().ok_or_else(|| tr("gguf-path-utf8"))?;
                        let job = c
                            .import(&id, file, &revision)
                            .await
                            .map_err(|e| e.to_string())?;
                        c.wait(&job).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Setup(Event::Done(r))),
                );
            }
            Event::Done(result) => {
                self.busy = false;
                self.setup.error = result.err();
                if self.setup.error.is_none() {
                    self.setup.file = None;
                    self.select_page(super::navigation::Page::Models);
                    return cosmic::task::message(cosmic::Action::App(
                        Message::ConfigurationChanged,
                    ));
                }
            }
            _ => {}
        }
        Task::none()
    }
}
