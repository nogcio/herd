//! Colibri native catalogue, explicit acquisition review and profile selection.
use super::{
    Applet, Message,
    localization::{arg, tr},
};
use cosmic::{
    Element,
    iced::{Alignment, Length},
    prelude::*,
    theme, widget,
};
use herd::engines::colibri::acquisition::{Choice, Plan};

#[derive(Default)]
pub(super) struct Browser {
    choices: Vec<Choice>,
    query: String,
    directory: String,
    loading: bool,
    generation: u64,
    review: Option<Plan>,
    selected: Option<Plan>,
    error: Option<String>,
    library_revision: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;
    fn plan() -> Plan {
        Plan {
            id: "example".into(),
            family: "glm".into(),
            name: "Example".into(),
            repo: "example/model".into(),
            engine_artifact: "colibri".into(),
            engine_available: true,
            conversion_required: false,
            license: "mit".into(),
            revision: "a".repeat(40),
            directory: "/example/model".into(),
            files: vec![herd::engines::colibri::acquisition::File {
                path: "model.safetensors".into(),
                size: 8,
                sha256: "b".repeat(64),
                git_oid: String::new(),
            }],
            total: 8,
            present: 0,
            fingerprint: "c".repeat(64),
        }
    }
    #[test]
    fn cancelled_metadata_cannot_reopen_review_or_release_accepted_work_and_selection_is_not_automatic()
     {
        let mut app = Applet {
            engine: 1,
            busy: true,
            operation_label: Some("accepted".into()),
            ..Default::default()
        };
        let ticket = app.colibri.generation;
        let _ = app.update(Message::Colibri(Event::Cancel));
        let _ = app.update(Message::Colibri(Event::Planned(ticket, Ok(plan()))));
        assert!(app.colibri.review.is_none());
        assert!(app.busy);
        assert_eq!(app.operation_label.as_deref(), Some("accepted"));
        app.busy = false;
        let task = app.update(Message::Colibri(Event::Download));
        assert_eq!(task.units(), 0);
        app.colibri.review = Some(plan());
        let _ = app.colibri_confirmation();
        let _ = app.update(Message::Navigate(super::super::navigation::Page::Search));
        assert!(app.colibri.review.is_none());
        assert!(app.colibri.selected.is_none());
        let _ = app.colibri_browser();
    }

    #[test]
    fn changed_destination_requires_new_review_and_leaving_search_preserves_accepted_work() {
        let mut app = Applet {
            engine: 1,
            shared_operation: Some("accepted-transfer".into()),
            ..Default::default()
        };
        let reviewed = plan();
        app.colibri.directory.clone_from(&reviewed.directory);
        app.colibri.review = Some(reviewed);
        let _ = app.update(Message::Colibri(Event::Directory(
            "/different/model".into(),
        )));
        assert!(app.colibri.review.is_some());
        assert_eq!(app.update(Message::Colibri(Event::Download)).units(), 0);
        assert!(!app.busy);
        assert!(app.colibri.selected.is_none());
        app.select_page(super::super::navigation::Page::Search);
        app.colibri.review = Some(plan());
        app.select_page(super::super::navigation::Page::Downloads);
        assert!(app.colibri.review.is_none());
        assert_eq!(app.shared_operation.as_deref(), Some("accepted-transfer"));
    }

    #[tokio::test]
    async fn catalogue_actions_stay_on_search_and_native_pages_fit_narrow_and_wide_windows() {
        use super::super::navigation::Page;
        use cosmic::iced::{
            Event as IcedEvent, Font, Point, Size,
            advanced::{layout, renderer::Headless, widget::Tree},
            mouse,
            runtime::{UserInterface, user_interface},
        };
        let mut renderer =
            <cosmic::Renderer as Headless>::new(Font::default(), 14.into(), Some("tiny-skia"))
                .await
                .unwrap();
        let mut app = Applet {
            engine: 1,
            ..Default::default()
        };
        let selected = plan();
        app.colibri.choices.push(Choice {
            id: selected.id.clone(),
            family: selected.family.clone(),
            name: "Example developer / local text model".into(),
            repo: selected.repo.clone(),
            engine_artifact: selected.engine_artifact.clone(),
            engine_available: true,
            conversion_required: false,
            license: "MIT".into(),
            directory: selected.directory.clone(),
            downloaded: true,
        });
        app.runtimes.push(herd::runtime::RuntimeRecord {
            id: "colibri:example".into(),
            engine: "colibri".into(),
            executable_found: true,
            configuration: "valid".into(),
            observation: "not_observed".into(),
            endpoint: String::new(),
            profile: String::new(),
            name: "Example selected model with native settings".into(),
            revision: String::new(),
            state: "stopped".into(),
            managed: false,
            can_start: false,
            can_load_acquired: false,
            can_stop: false,
            pending: false,
            transfer: "downloading".into(),
            transfer_model: selected.id.clone(),
            transfer_fingerprint: selected.fingerprint.clone(),
            done: 4 * 1024 * 1024 * 1024,
            total: 8 * 1024 * 1024 * 1024,
            error: String::new(),
        });
        app.colibri.selected = Some(selected);
        for page in [Page::Models, Page::Search, Page::Downloads, Page::Runtimes] {
            app.select_page(page);
            for width in [360., 800.] {
                let mut element = app.page_view();
                let mut tree = Tree::new(element.as_widget());
                let node = element.as_widget_mut().layout(
                    &mut tree,
                    &renderer,
                    &layout::Limits::new(Size::ZERO, Size::new(width, 700.)),
                );
                fn fits(node: &layout::Node, x: f32, width: f32) {
                    let bounds = node.bounds();
                    let x = x + bounds.x;
                    assert!(bounds.width.is_finite() && bounds.height.is_finite());
                    assert!(
                        x + bounds.width <= width + 1.,
                        "Colibri row overflows its page: {bounds:?}"
                    );
                    for child in node.children() {
                        fits(child, x, width);
                    }
                }
                fits(&node, 0., width);
            }
            let mut ui = UserInterface::build(
                app.page_view(),
                Size::new(800., 700.),
                user_interface::Cache::default(),
                &mut renderer,
            );
            let mut clipboard = cosmic::iced::advanced::clipboard::Null;
            let mut messages = Vec::new();
            for y in (20..650).step_by(16) {
                for x in (20..780).step_by(16) {
                    let position = Point::new(x as f32, y as f32);
                    let _ = ui.update(
                        &[
                            IcedEvent::Mouse(mouse::Event::CursorMoved { position }),
                            IcedEvent::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                            IcedEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                        ],
                        mouse::Cursor::Available(position),
                        &mut renderer,
                        &mut clipboard,
                        &mut messages,
                    );
                }
            }
            let catalogue = messages
                .iter()
                .any(|message| matches!(message, Message::Colibri(Event::Catalogue)));
            let review = messages
                .iter()
                .any(|message| matches!(message, Message::Colibri(Event::Review(_))));
            assert_eq!(catalogue, page == Page::Search);
            assert_eq!(review, page == Page::Search);
            if let Some(directory) = std::env::var_os("HERD_COLIBRI_UI_EVIDENCE") {
                use cosmic::iced::advanced::Renderer as _;
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let theme = cosmic::Theme::dark();
                let style = cosmic::iced::advanced::renderer::Style {
                    text_color: theme.cosmic().on_bg_color().into(),
                    icon_color: theme.cosmic().on_bg_color().into(),
                    ..Default::default()
                };
                renderer.reset(cosmic::iced::Rectangle::with_size(Size::new(800., 700.)));
                ui.draw(&mut renderer, &theme, &style, mouse::Cursor::Unavailable);
                let pixels =
                    renderer.screenshot(Size::new(800, 700), 1., cosmic::iced::Color::BLACK);
                image::save_buffer(
                    directory.join(format!("colibri-{page:?}.png")),
                    &pixels,
                    800,
                    700,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
        }
        assert_eq!(app.runtimes[0].transfer, "downloading");
        assert!(app.colibri.review.is_none());
        assert!(!app.busy);
    }
}
impl Browser {
    pub fn cancel_review(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.review = None;
        self.loading = false;
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Query(String),
    Restored(String, Result<Option<Plan>, String>),
    Catalogue,
    Choices(u64, Result<Vec<Choice>, String>),
    Directory(String),
    Review(String),
    Resume,
    Planned(u64, Result<Plan, String>),
    Download,
    Cancel,
    Pause,
    Select,
    LoadLocal(String, String),
}

impl Applet {
    pub(super) fn refresh_colibri_catalogue(&mut self) -> Task<cosmic::Action<Message>> {
        if self.engine != 1 || self.busy || self.colibri.loading || self.colibri.review.is_some() {
            return Task::none();
        }
        let Some(runtime) = self.runtimes.iter().find(|r| r.engine == "colibri") else {
            return Task::none();
        };
        let key = format!(
            "{}:{}",
            runtime.revision,
            if runtime.transfer == "complete" {
                &runtime.transfer_fingerprint
            } else {
                ""
            }
        );
        if key == self.colibri.library_revision {
            return Task::none();
        }
        self.colibri.library_revision = key;
        self.colibri_event(Event::Catalogue)
    }
    pub(super) fn restore_colibri(&self) -> Task<cosmic::Action<Message>> {
        let Some(runtime) = self
            .runtimes
            .iter()
            .find(|r| r.engine == "colibri" && !r.transfer_fingerprint.is_empty())
        else {
            return Task::none();
        };
        if self
            .colibri
            .selected
            .as_ref()
            .is_some_and(|p| p.fingerprint == runtime.transfer_fingerprint)
        {
            return Task::none();
        }
        let Some(client) = self.controller.clone() else {
            return Task::none();
        };
        let fingerprint = runtime.transfer_fingerprint.clone();
        Task::perform(
            async move {
                client
                    .colibri_transfer_plan()
                    .await
                    .map_err(|e| e.to_string())
            },
            move |r| cosmic::Action::App(Message::Colibri(Event::Restored(fingerprint.clone(), r))),
        )
    }
    pub(super) fn colibri_page(&self) -> Element<'_, Message> {
        let mut content = widget::column::with_capacity(8)
            .spacing(theme::spacing().space_m)
            .padding(theme::spacing().space_m)
            .push(widget::text::title3(tr(match self.navigation.page {
                super::navigation::Page::Search => "find-models",
                super::navigation::Page::Downloads => "downloads",
                _ => "models",
            })));
        if let Some(error) = &self.error {
            content = content.push(super::view::notice(tr("action-unavailable"), error));
        }
        if let Some(label) = self
            .operation_label
            .as_ref()
            .or(self.shared_operation.as_ref())
        {
            content = content.push(widget::text::body(label));
        }
        content = content.push(match self.navigation.page {
            super::navigation::Page::Search => self.colibri_browser(),
            super::navigation::Page::Downloads => self.colibri_transfers(),
            _ => self.colibri_library(),
        });
        widget::scrollable(
            widget::container(
                widget::container(content)
                    .max_width(960)
                    .width(Length::Fill),
            )
            .center_x(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }
    fn colibri_library(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        let mut content = widget::column::with_capacity(5).spacing(space.space_s);
        let runtime = self.runtimes.iter().find(|r| r.engine == "colibri");
        if let Some(runtime) = runtime.filter(|r| r.configuration != "absent" || r.managed) {
            content = content.push(self.colibri_instance_row(runtime));
        }
        let query = self.navigation.query.trim().to_lowercase();
        content = content.push(
            widget::search_input(tr("library-search"), &self.navigation.query)
                .on_input(Message::LibraryQuery)
                .on_clear(Message::LibraryQuery(String::new())),
        );
        let mut list = widget::list::list_column().list_item_padding(space.space_s);
        let mut count = 0;
        for choice in self
            .colibri
            .choices
            .iter()
            .filter(|choice| choice.downloaded)
        {
            if !query.is_empty()
                && !choice.name.to_lowercase().contains(&query)
                && !choice.repo.to_lowercase().contains(&query)
            {
                continue;
            }
            count += 1;
            let enabled = !self.busy
                && self.controller.is_some()
                && choice.engine_available
                && !choice.conversion_required
                && runtime.is_some_and(|r| r.state == "stopped" && r.configuration != "invalid");
            list = list.add(
                widget::row::with_capacity(2)
                    .spacing(space.space_s)
                    .align_y(Alignment::Center)
                    .push(
                        widget::column::with_capacity(2)
                            .width(Length::Fill)
                            .push(widget::text::heading(&choice.name))
                            .push(widget::text::caption(if !choice.engine_available {
                                tr("colibri-engine-missing")
                            } else if choice.conversion_required {
                                tr("colibri-conversion")
                            } else {
                                choice.repo.clone()
                            })),
                    )
                    .push(
                        widget::button::standard(tr("start")).on_press_maybe(enabled.then(|| {
                            Message::Colibri(Event::LoadLocal(
                                choice.id.clone(),
                                choice.directory.clone(),
                            ))
                        })),
                    ),
            );
        }
        content = content.push(list);
        if count == 0
            && runtime.is_none_or(|runtime| runtime.configuration == "absent" && !runtime.managed)
        {
            content = content.push(self.colibri_empty("no-models", "view-grid-symbolic"));
        }
        if let Some(error) = &self.colibri.error {
            content = content.push(super::view::notice(tr("discovery-error"), error));
        }
        content.into()
    }

    fn colibri_instance_row<'a>(
        &'a self,
        runtime: &'a herd::runtime::RuntimeRecord,
    ) -> Element<'a, Message> {
        let space = theme::spacing();
        let target = herd::runtime::Target {
            runtime: runtime.id.clone(),
            model: "configured".into(),
            revision: runtime.revision.clone(),
        };
        let enabled = !self.busy && self.controller.is_some();
        let state = tr(match runtime.state.as_str() {
            "ready" => "runtime-ready",
            "starting" => "runtime-starting",
            "stopped" => "runtime-stopped",
            "external" => "runtime-external",
            "failed" => "runtime-failed",
            _ => "runtime-unknown",
        });
        let action = if runtime.can_stop {
            widget::button::text(tr("stop"))
                .on_press_maybe(enabled.then_some(Message::AskStopRuntime(target)))
        } else {
            widget::button::standard(tr("start")).on_press_maybe(
                (enabled && runtime.can_start).then_some(Message::StartRuntime(target)),
            )
        };
        let mut content = widget::column::with_capacity(3)
            .spacing(space.space_s)
            .push(widget::text::caption(tr("colibri-selected-model")))
            .push(
                widget::row::with_capacity(3)
                    .spacing(space.space_s)
                    .align_y(Alignment::Center)
                    .push(
                        widget::column::with_capacity(2)
                            .width(Length::Fill)
                            .push(widget::text::heading(&runtime.name))
                            .push(widget::text::caption(state)),
                    )
                    .push(action)
                    .push(
                        widget::button::icon(widget::icon::from_name(
                            "preferences-system-symbolic",
                        ))
                        .name(tr("settings"))
                        .on_press_maybe(
                            (enabled && runtime.configuration == "valid").then_some(
                                Message::ColibriEditor(super::colibri_editor::Event::Open),
                            ),
                        ),
                    ),
            );
        if runtime.pending {
            content = content.push(widget::text::caption(tr("runtime-pending")));
        }
        if !runtime.error.is_empty() {
            content = content.push(super::view::notice(
                tr("action-unavailable"),
                &runtime.error,
            ));
        }
        widget::list::list_column()
            .list_item_padding(space.space_s)
            .add(content)
            .into()
    }

    fn colibri_empty(&self, label: &'static str, icon: &'static str) -> Element<'_, Message> {
        let space = theme::spacing();
        widget::container(
            widget::column::with_capacity(3)
                .spacing(space.space_s)
                .align_x(Alignment::Center)
                .push(widget::icon::from_name(icon).size(48))
                .push(widget::text::heading(tr(label)))
                .push(
                    widget::button::suggested(tr("find-models"))
                        .on_press(Message::Navigate(super::navigation::Page::Search)),
                ),
        )
        .center_x(Length::Fill)
        .padding(space.space_l)
        .into()
    }

    fn colibri_transfers(&self) -> Element<'_, Message> {
        let space = theme::spacing();
        let Some(runtime) = self
            .runtimes
            .iter()
            .find(|r| r.engine == "colibri" && !r.transfer.is_empty())
        else {
            return self.colibri_empty("no-downloads", "folder-download-symbolic");
        };
        let active = matches!(
            runtime.transfer.as_str(),
            "starting" | "downloading" | "verifying"
        );
        let mut transfer = widget::column::with_capacity(6).spacing(space.space_s);
        let name = self
            .colibri
            .selected
            .as_ref()
            .map_or_else(|| runtime.transfer_model.clone(), |plan| plan.name.clone());
        transfer = transfer
            .push(widget::text::heading(name))
            .push(widget::text::caption(tr(match runtime.transfer.as_str() {
                "complete" => "colibri-transfer-complete",
                "paused" => "colibri-transfer-paused",
                "failed" => "colibri-transfer-failed",
                "verifying" => "colibri-transfer-verifying",
                "starting" => "submitting-download",
                "downloading" => "state-downloading",
                _ => "runtime-unknown",
            })));
        if runtime.total > 0 {
            transfer = transfer
                .push(
                    widget::progress_bar::determinate_linear(
                        (runtime.done as f32 / runtime.total as f32).clamp(0.0, 1.0),
                    )
                    .width(Length::Fill),
                )
                .push(widget::text::caption(format!(
                    "{} / {}",
                    super::downloads::size_label(runtime.done),
                    super::downloads::size_label(runtime.total)
                )));
        } else if active {
            transfer = transfer.push(widget::progress_bar::indeterminate_linear());
        }
        if let Some(plan) = &self.colibri.selected {
            transfer = transfer.push(widget::text::caption(&plan.directory));
        }
        if !runtime.error.is_empty() {
            transfer = transfer.push(super::view::notice(tr("download-error"), &runtime.error));
        }
        let enabled = !self.busy && self.controller.is_some();
        let action = if active {
            widget::button::text(tr("pause-download"))
                .on_press_maybe(enabled.then_some(Message::Colibri(Event::Pause)))
        } else if runtime.transfer == "complete" {
            widget::button::suggested(tr("start")).on_press_maybe(
                (enabled
                    && runtime.can_load_acquired
                    && self
                        .colibri
                        .selected
                        .as_ref()
                        .is_some_and(|p| !p.conversion_required && p.engine_available))
                .then_some(Message::Colibri(Event::Select)),
            )
        } else {
            widget::button::standard(tr("resume-download")).on_press_maybe(
                (enabled && self.colibri.selected.is_some())
                    .then_some(Message::Colibri(Event::Resume)),
            )
        };
        transfer = transfer.push(action);
        widget::list::list_column()
            .list_item_padding(space.space_s)
            .add(transfer)
            .into()
    }

    pub(super) fn colibri_browser(&self) -> Element<'_, Message> {
        let mut content = widget::column::with_capacity(8)
            .spacing(theme::spacing().space_s)
            .push(
                widget::row::with_capacity(2)
                    .spacing(theme::spacing().space_s)
                    .align_y(Alignment::Center)
                    .push(
                        widget::search_input(tr("library-search"), &self.colibri.query)
                            .on_input(|v| Message::Colibri(Event::Query(v)))
                            .on_clear(Message::Colibri(Event::Query(String::new()))),
                    )
                    .push(
                        widget::button::icon(widget::icon::from_name("view-refresh-symbolic"))
                            .name(tr("colibri-catalogue"))
                            .on_press_maybe(
                                (!self.colibri.loading && !self.busy)
                                    .then_some(Message::Colibri(Event::Catalogue)),
                            ),
                    ),
            );
        if self.colibri.loading {
            content = content.push(widget::progress_bar::indeterminate_linear());
        }
        if let Some(error) = &self.colibri.error {
            content = content.push(super::view::notice(tr("action-unavailable"), error));
        }
        let mut results = widget::list::list_column().list_item_padding(theme::spacing().space_s);
        for choice in &self.colibri.choices {
            let query = self.colibri.query.trim().to_lowercase();
            if !query.is_empty()
                && !choice.name.to_lowercase().contains(&query)
                && !choice.repo.to_lowercase().contains(&query)
                && !choice.id.to_lowercase().contains(&query)
            {
                continue;
            }
            results = results.add(
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(
                        widget::column::with_capacity(3)
                            .push(widget::text::body(choice.name.clone()))
                            .push(widget::text::caption(choice.repo.clone()))
                            .push(widget::text::caption(if !choice.engine_available {
                                tr("colibri-engine-missing")
                            } else if choice.conversion_required {
                                tr("colibri-conversion")
                            } else {
                                choice.license.clone()
                            }))
                            .width(Length::Fill),
                    )
                    .push(
                        widget::button::text(tr("colibri-review")).on_press_maybe(
                            (!self.colibri.loading && !self.busy)
                                .then(|| Message::Colibri(Event::Review(choice.id.clone()))),
                        ),
                    ),
            );
        }
        content = content.push(results);
        content.into()
    }

    pub(super) fn colibri_confirmation(&self) -> Option<Element<'_, Message>> {
        let plan = self.colibri.review.as_ref()?;
        let mut files = widget::column::with_capacity(plan.files.len()).spacing(4);
        for file in &plan.files {
            files = files.push(widget::text::caption(format!(
                "{} · {} B",
                file.path, file.size
            )));
        }
        let mut body = widget::column::with_capacity(10)
            .spacing(8)
            .push(widget::text::body(plan.name.clone()))
            .push(widget::text::caption(plan.repo.clone()))
            .push(widget::text::caption(plan.revision.clone()))
            .push(widget::text::caption(tr("colibri-destination")))
            .push(
                widget::text_input(tr("colibri-destination"), &self.colibri.directory)
                    .on_input(|directory| Message::Colibri(Event::Directory(directory))),
            )
            .push(widget::text::body(arg(
                "colibri-size",
                "size",
                format!("{} B · {} B", plan.total, plan.present),
            )))
            .push(widget::text::body(if plan.license.is_empty() {
                tr("colibri-license-missing")
            } else {
                plan.license.clone()
            }))
            .push(widget::text::body(tr("colibri-acquisition-warning")))
            .push(widget::scrollable(files).height(Length::Fixed(180.0)));
        if plan.conversion_required {
            body = body.push(widget::text::body(tr("colibri-conversion")));
        }
        Some(
            widget::dialog()
                .title(tr("colibri-review"))
                .control(body)
                .primary_action(
                    widget::button::suggested(tr(if self.colibri.directory == plan.directory {
                        "colibri-download"
                    } else {
                        "colibri-review"
                    }))
                    .on_press_maybe((!self.busy).then(|| {
                        Message::Colibri(if self.colibri.directory == plan.directory {
                            Event::Download
                        } else {
                            Event::Review(plan.id.clone())
                        })
                    })),
                )
                .secondary_action(
                    widget::button::text(tr("cancel")).on_press(Message::Colibri(Event::Cancel)),
                )
                .into(),
        )
    }

    pub(super) fn colibri_event(&mut self, event: Event) -> Task<cosmic::Action<Message>> {
        match event {
            Event::LoadLocal(model, directory) if !self.busy => {
                let Some(client) = self.controller.clone() else {
                    return Task::none();
                };
                let revision = self
                    .runtimes
                    .iter()
                    .find(|r| r.engine == "colibri")
                    .map(|r| r.revision.clone())
                    .unwrap_or_default();
                self.busy = true;
                self.operation_label = Some(tr("starting-model"));
                self.error = None;
                return Task::perform(
                    async move {
                        let operation = client
                            .colibri_load_local(&model, &directory, &revision)
                            .await
                            .map_err(|e| e.to_string())?;
                        client.wait(&operation).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Finished(r)),
                );
            }
            Event::Query(query) => self.colibri.query = query,
            Event::Restored(fingerprint, result) => {
                if !self
                    .runtimes
                    .iter()
                    .any(|r| r.transfer_fingerprint == fingerprint)
                {
                    return Task::none();
                }
                match result {
                    Ok(Some(plan)) if plan.fingerprint == fingerprint => {
                        self.colibri.selected = Some(plan)
                    }
                    Err(error) => self.colibri.error = Some(error),
                    _ => {}
                }
            }
            Event::Directory(directory) => {
                self.colibri.directory = directory;
            }
            Event::Cancel => self.colibri.cancel_review(),
            Event::Catalogue if !self.busy => {
                let Some(client) = self.controller.clone() else {
                    return Task::none();
                };
                self.colibri.cancel_review();
                self.colibri.loading = true;
                self.colibri.error = None;
                let ticket = self.colibri.generation;
                return Task::perform(
                    async move { client.colibri_catalogue().await.map_err(|e| e.to_string()) },
                    move |r| cosmic::Action::App(Message::Colibri(Event::Choices(ticket, r))),
                );
            }
            Event::Choices(ticket, result) if ticket == self.colibri.generation => {
                self.colibri.loading = false;
                match result {
                    Ok(choices) => self.colibri.choices = choices,
                    Err(error) => self.colibri.error = Some(error),
                }
            }
            Event::Review(_) | Event::Resume if !self.busy => {
                let Some(client) = self.controller.clone() else {
                    return Task::none();
                };
                let request = match event {
                    Event::Review(model) => Some((model, self.colibri.directory.clone(), None)),
                    _ => self
                        .colibri
                        .selected
                        .as_ref()
                        .map(|p| (p.id.clone(), p.directory.clone(), Some(p.clone()))),
                };
                let Some((model, directory, previous)) = request else {
                    return Task::none();
                };
                self.colibri.cancel_review();
                self.colibri.loading = true;
                self.colibri.error = None;
                let ticket = self.colibri.generation;
                return Task::perform(
                    async move {
                        match previous {
                            Some(plan) => client.colibri_resume_plan(&plan).await,
                            None => client.colibri_plan(&model, &directory).await,
                        }
                        .map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::Colibri(Event::Planned(ticket, r))),
                );
            }
            Event::Planned(ticket, result) if ticket == self.colibri.generation => {
                self.colibri.loading = false;
                match result {
                    Ok(plan) => {
                        self.colibri.directory.clone_from(&plan.directory);
                        self.colibri.review = Some(plan);
                    }
                    Err(error) => self.colibri.error = Some(error),
                }
            }
            Event::Download if !self.busy => {
                if self
                    .colibri
                    .review
                    .as_ref()
                    .is_some_and(|plan| self.colibri.directory != plan.directory)
                {
                    return Task::none();
                }
                let (Some(client), Some(plan)) =
                    (self.controller.clone(), self.colibri.review.take())
                else {
                    return Task::none();
                };
                self.colibri.selected = Some(plan.clone());
                self.busy = true;
                self.error = None;
                self.operation_label = Some(tr("submitting-download"));
                self.select_page(super::navigation::Page::Downloads);
                return Task::perform(
                    async move {
                        let id = client
                            .colibri_download(&plan, plan.total)
                            .await
                            .map_err(|e| e.to_string())?;
                        client.wait(&id).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Finished(r)),
                );
            }
            Event::Pause if !self.busy => {
                let Some(client) = self.controller.clone() else {
                    return Task::none();
                };
                self.busy = true;
                self.operation_label = Some(tr("pausing-download"));
                return Task::perform(
                    async move {
                        let id = client.colibri_pause().await.map_err(|e| e.to_string())?;
                        client.wait(&id).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Finished(r)),
                );
            }
            Event::Select if !self.busy => {
                let (Some(client), Some(plan)) =
                    (self.controller.clone(), self.colibri.selected.clone())
                else {
                    return Task::none();
                };
                let revision = self
                    .runtimes
                    .iter()
                    .find(|r| r.engine == "colibri")
                    .map(|r| r.revision.clone())
                    .unwrap_or_default();
                self.busy = true;
                self.error = None;
                self.operation_label = Some(tr("working"));
                return Task::perform(
                    async move {
                        let id = client
                            .colibri_load(&plan, &revision)
                            .await
                            .map_err(|e| e.to_string())?;
                        client.wait(&id).await.map_err(|e| e.to_string())
                    },
                    |r| cosmic::Action::App(Message::Finished(r)),
                );
            }
            _ => {}
        }
        Task::none()
    }
}
