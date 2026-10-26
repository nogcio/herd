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
    pub(super) fn setup_view(&self) -> Element<'_, Message> {
        let mut column = widget::column::with_capacity(12)
            .spacing(12)
            .push(widget::text::body(tr("runtime-versions")));
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
