//! Native management presentation and window ownership; no service operations.
use super::localization::{count, tr};
use super::navigation::Page;
use super::{Applet, Message, view::notice};
use cosmic::{Element, iced::Length, prelude::*, theme, widget};

impl Applet {
    pub(super) fn begin_removal_review(&mut self) -> u64 {
        self.removal_generation = self.removal_generation.wrapping_add(1);
        self.removal_review = Some(self.removal_generation);
        self.pending_removal = None;
        self.removal_generation
    }

    pub(super) fn cancel_removal_review(&mut self) {
        self.removal_review = None;
        self.pending_removal = None;
    }

    pub(super) fn open_manager(&mut self) -> Task<cosmic::Action<Message>> {
        self.open_manager_review(None)
    }

    pub(super) fn open_manager_review(
        &mut self,
        review: Option<String>,
    ) -> Task<cosmic::Action<Message>> {
        cosmic::iced::platform_specific::shell::wayland::commands::activation::request_token(
            Some("io.github.nogcio.Herd.Cosmic".into()),
            self.popup.or(self.core.main_window_id()),
        )
        .map(move |token| cosmic::Action::App(Message::LaunchManager(token, review.clone())))
    }

    pub(super) fn manager_content(&self) -> Element<'_, Message> {
        let active = theme::active();
        let space = active.cosmic();
        let transfers = self.navigation.page == Page::Downloads;
        let snapshots = self.library_models(transfers);
        let header = widget::text::title3(tr(if transfers { "downloads" } else { "models" }));
        let mut content = widget::column::with_capacity(self.snapshots.len() + 6)
            .spacing(space.space_s())
            .padding(space.space_m())
            .width(Length::Fill)
            .push(header)
            .push(widget::text::caption(count(
                if transfers {
                    "transfer-count"
                } else {
                    "model-count"
                },
                snapshots.len(),
            )));
        if !transfers {
            content = content.push(
                widget::row::with_capacity(2)
                    .spacing(space.space_s())
                    .align_y(cosmic::iced::Alignment::Center)
                    .push(
                        widget::search_input(tr("library-search"), &self.navigation.query)
                            .on_input(Message::LibraryQuery)
                            .on_clear(Message::LibraryQuery(String::new())),
                    )
                    .push(
                        widget::button::text(tr("sort-name"))
                            .trailing_icon(widget::icon::from_name(if self.navigation.reverse {
                                "pan-up-symbolic"
                            } else {
                                "pan-down-symbolic"
                            }))
                            .on_press(Message::LibrarySort),
                    ),
            );
        }
        if self
            .snapshots
            .iter()
            .any(|s| s.state == herd::model::State::Unavailable)
        {
            content = content.push(
                widget::button::suggested(tr("start-router"))
                    .on_press_maybe((!self.busy).then_some(Message::ServerUp)),
            );
        }
        let reviewing = self.removal_review.map(|_| tr("working"));
        if let Some(label) = self
            .operation_label
            .as_ref()
            .or(self.shared_operation.as_ref())
            .or(reviewing.as_ref())
        {
            content = content.push(
                widget::column::with_capacity(2)
                    .spacing(space.space_xxs())
                    .push(widget::text::caption(label.clone()))
                    .push(widget::progress_bar::indeterminate_linear().width(Length::Fill)),
            );
        }
        if let Some(error) = &self.error {
            content = content.push(notice(tr("action-unavailable"), error));
        }
        if let Some(error) = &self.discovery_error {
            content = content.push(notice(tr("discovery-error"), error));
        }
        if self.controller.is_none() && !self.snapshots.is_empty() && self.discovery_error.is_none()
        {
            content = content.push(widget::text::caption(tr("stale-observations")));
        }
        if transfers && let Some(feedback) = self.resume_feedback() {
            content = content.push(feedback);
        }
        if self.setup_required {
            content = content
                .push(widget::button::suggested(tr("setup-runtime")).on_press(Message::ShowSetup));
        } else if self.controller.is_some()
            && self.snapshots.is_empty()
            && self.discovery_error.is_none()
        {
            content = content.push(
                widget::button::text(tr("start-router"))
                    .on_press_maybe((!self.busy).then_some(Message::ServerUp)),
            );
        }
        if snapshots.is_empty() {
            content = content.push(widget::text::caption(tr(if self.connecting {
                "connecting"
            } else if transfers {
                "no-downloads"
            } else if !self.navigation.query.is_empty() {
                "no-search-results"
            } else {
                "no-models"
            })));
            if !transfers
                && !self.setup_required
                && self.discovery_error.is_none()
                && !self.connecting
            {
                content = content.push(
                    widget::button::suggested(tr("add-model"))
                        .on_press(Message::Navigate(Page::Search)),
                );
            }
        }
        let mut list = widget::list::list_column().list_item_padding(space.space_s());
        for snapshot in snapshots {
            list = list.add(self.model_row(snapshot, true));
        }
        content = content.push(list);
        let transfer_count = self.snapshots.iter().filter(|s| is_transfer(s)).count();
        if !transfers && transfer_count > 0 {
            content = content.push(
                widget::button::text(count("transfer-count", transfer_count))
                    .leading_icon(widget::icon::from_name("folder-download-symbolic"))
                    .on_press(Message::Navigate(Page::Downloads)),
            );
        }
        widget::container(widget::scrollable(content).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .class(theme::Container::Background)
            .into()
    }

    pub(super) fn library_models(&self, transfers: bool) -> Vec<&herd::model::Snapshot> {
        let query = self.navigation.query.trim().to_lowercase();
        let mut models: Vec<_> = self
            .snapshots
            .iter()
            .filter(|s| {
                is_transfer(s) == transfers
                    && (transfers
                        || query.is_empty()
                        || s.model.name.to_lowercase().contains(&query)
                        || s.model.id.to_lowercase().contains(&query))
            })
            .collect();
        models.sort_by_cached_key(|s| (s.model.name.to_lowercase(), &s.model.id));
        if !transfers && self.navigation.reverse {
            models.reverse();
        }
        models
    }

    pub(super) fn add_menu(&self) -> Element<'static, Message> {
        use widget::{RcElementWrapper, menu};
        menu::bar(vec![menu::Tree::with_children(
            RcElementWrapper::new(
                widget::button::standard(tr("add-model"))
                    .class(theme::Button::MenuRoot)
                    .leading_icon(widget::icon::from_name("list-add-symbolic"))
                    .into(),
            ),
            vec![
                menu::Tree::new(RcElementWrapper::new(
                    menu::menu_button(vec![widget::text::body(tr("find-models")).into()])
                        .on_press(Message::Navigate(Page::Search))
                        .into(),
                )),
                menu::Tree::new(RcElementWrapper::new(
                    menu::menu_button(vec![widget::text::body(tr("import-model")).into()])
                        .on_press(Message::ShowAdd)
                        .into(),
                )),
            ],
        )])
        .item_width(menu::ItemWidth::Uniform(220))
        .into()
    }

    pub(super) fn model_menu(&self, snapshot: &herd::model::Snapshot) -> Element<'static, Message> {
        use widget::{RcElementWrapper, menu};
        let settings = menu::menu_button(vec![widget::text::body(tr("settings")).into()])
            .on_press_maybe(
                (!self.busy && !snapshot.model.settings.is_empty())
                    .then(|| Message::Edit(snapshot.model.id.clone())),
            );
        let can_remove = snapshot.state.can_start()
            || snapshot.state == herd::model::State::Incomplete
            || (snapshot.state == herd::model::State::Unavailable
                && !snapshot.model.settings.is_empty());
        let remove = menu::menu_button(vec![widget::text::body(tr("remove-model-menu")).into()])
            .on_press_maybe(
                (!self.busy && can_remove && self.discovery_error.is_none())
                    .then(|| Message::AskRemove(snapshot.model.id.clone())),
            );
        menu::bar(vec![menu::Tree::with_children(
            RcElementWrapper::new(
                widget::button::icon(widget::icon::from_name("view-more-symbolic"))
                    .class(theme::Button::MenuRoot)
                    .name(tr("model-options"))
                    .into(),
            ),
            vec![
                menu::Tree::new(RcElementWrapper::new(settings.into())),
                menu::Tree::new(RcElementWrapper::new(remove.into())),
            ],
        )])
        .item_width(menu::ItemWidth::Uniform(220))
        .into()
    }

    pub(super) fn removal_confirmation(&self) -> Option<Element<'_, Message>> {
        let plan = self.pending_removal.as_ref()?;
        Some(
            widget::dialog()
                .title(tr(if plan.delete_files {
                    "delete-model-confirm"
                } else {
                    "remove-model-confirm"
                }))
                .body(format!(
                    "{}\n\n{}",
                    self.snapshots
                        .iter()
                        .find(|s| s.model.id == plan.model)
                        .map(|s| if s.model.name == plan.model {
                            plan.model.clone()
                        } else {
                            format!("{}\n{}", s.model.name, plan.model)
                        })
                        .unwrap_or_else(|| plan.model.clone()),
                    tr(if plan.delete_files {
                        "delete-model-warning"
                    } else {
                        "remove-model-warning"
                    })
                ))
                .primary_action(
                    widget::button::destructive(tr(if plan.delete_files {
                        "delete-files"
                    } else {
                        "remove"
                    }))
                    .on_press_maybe((!self.busy).then_some(Message::ConfirmRemove)),
                )
                .secondary_action(
                    widget::button::text(tr("cancel")).on_press(Message::CancelRemove),
                )
                .into(),
        )
    }
}

fn is_transfer(snapshot: &herd::model::Snapshot) -> bool {
    matches!(
        snapshot.state,
        herd::model::State::Downloading { .. } | herd::model::State::Incomplete
    )
}

fn command(
    token: Option<String>,
    review: Option<String>,
) -> std::io::Result<std::process::Command> {
    let executable = std::env::current_exe()?.with_file_name("herd-cosmic");
    let mut command = std::process::Command::new(executable);
    command
        .env_remove("WAYLAND_SOCKET")
        .env_remove("XDG_ACTIVATION_TOKEN");
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token);
    }
    if let Some(model) = review {
        command.arg("--review-download").arg(model);
    }
    Ok(command)
}

pub(super) async fn launch(token: Option<String>, review: Option<String>) -> bool {
    match command(token, review) {
        Ok(command) => cosmic::process::spawn(command).await.is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn removal_plan() -> herd::backend::protocol::RemovalPlan {
        herd::backend::protocol::RemovalPlan {
            model: "example/tiny:Q8_0".into(),
            revision: "a".repeat(64),
            delete_files: true,
        }
    }

    #[test]
    fn abandoned_removal_reviews_ignore_success_and_failure_without_cancelling_work() {
        for cancel in ["escape", "cancel", "navigation", "replacement"] {
            let mut app = Applet {
                shared_operation: Some("Accepted transfer".into()),
                ..Default::default()
            };
            app.snapshots.push(herd::model::Snapshot {
                model: herd::model::Model {
                    id: "competitor".into(),
                    name: "Competitor".into(),
                    short_name: String::new(),
                    settings: Default::default(),
                    icon: Default::default(),
                },
                state: herd::model::State::Ready,
                detail: None,
            });
            let observations = serde_json::to_value(&app.snapshots).unwrap();
            let ticket = app.begin_removal_review();
            assert!(!app.busy, "inspection must not own mutation busy state");
            let replacement = match cancel {
                "escape" => {
                    let _ = app.on_escape();
                    None
                }
                "cancel" => {
                    let _ = app.update(Message::CancelRemove);
                    None
                }
                "navigation" => {
                    app.select_page(Page::Search);
                    None
                }
                _ => Some(app.begin_removal_review()),
            };
            // An independent accepted mutation can begin after cancellation.
            // A stale inspection must never clear that mutation's busy state.
            app.busy = true;
            app.operation_label = Some("Accepted removal".into());
            for result in [Ok(removal_plan()), Err("Stale inspection failure".into())] {
                let _ = app.update(Message::RemovalReady(ticket, result));
                assert!(app.pending_removal.is_none());
                assert!(app.error.is_none());
                assert_eq!(app.removal_review, replacement);
                assert!(app.busy);
                assert_eq!(app.operation_label.as_deref(), Some("Accepted removal"));
                assert_eq!(app.shared_operation.as_deref(), Some("Accepted transfer"));
                assert_eq!(serde_json::to_value(&app.snapshots).unwrap(), observations);
            }
            if let Some(ticket) = replacement {
                let _ = app.update(Message::RemovalReady(ticket, Ok(removal_plan())));
                assert!(app.removal_review.is_none());
                assert!(app.pending_removal.is_some());
                let _ = app.update(Message::CancelRemove);
                assert!(app.pending_removal.is_none());
                assert!(app.busy, "dialog cancel must not cancel accepted removal");
                let _ = app.update(Message::RemovalReady(
                    ticket,
                    Err("Duplicate late result".into()),
                ));
                assert!(app.error.is_none());
                assert!(app.busy);
            }
        }
    }

    #[test]
    fn current_removal_review_failure_is_visible_and_does_not_lock_actions() {
        let mut app = Applet::default();
        let ticket = app.begin_removal_review();
        let _ = app.manager_content();
        let _ = app.update(Message::RemovalReady(
            ticket,
            Err("Inspection failed".into()),
        ));
        assert!(app.removal_review.is_none());
        assert!(!app.busy);
        assert!(app.pending_removal.is_none());
        assert_eq!(app.error.as_deref(), Some("Inspection failed"));
    }

    #[test]
    fn library_filter_sort_and_transfer_navigation_leave_runtime_observations_unchanged() {
        use herd::model::{Model, Snapshot, State};
        let mut app = Applet {
            snapshots: [
                ("z", "Zeta", State::Ready),
                ("a", "Alpha", State::Stopped),
                ("partial", "Partial", State::Incomplete),
                (
                    "transfer",
                    "Transfer",
                    State::Downloading { done: 4, total: 8 },
                ),
            ]
            .into_iter()
            .map(|(id, name, state)| Snapshot {
                model: Model {
                    id: id.into(),
                    name: name.into(),
                    short_name: String::new(),
                    settings: Default::default(),
                    icon: Default::default(),
                },
                state,
                detail: None,
            })
            .collect(),
            shared_operation: Some("Accepted transfer".into()),
            ..Default::default()
        };
        let original = serde_json::to_value(&app.snapshots).unwrap();
        let ids = |app: &Applet, transfers| {
            app.library_models(transfers)
                .iter()
                .map(|s| s.model.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&app, false), ["a", "z"]);
        let _ = app.update(Message::LibrarySort);
        assert_eq!(ids(&app, false), ["z", "a"]);
        let _ = app.update(Message::LibraryQuery(" ALP ".into()));
        assert_eq!(ids(&app, false), ["a"]);
        let _ = app.update(Message::Navigate(Page::Downloads));
        assert_eq!(ids(&app, true), ["partial", "transfer"]);
        let _ = app.page_view();
        // Completion moves the native entry to the library; no history registry.
        app.snapshots[3].state = State::Stopped;
        assert_eq!(ids(&app, true), ["partial"]);
        let _ = app.update(Message::LibraryQuery(String::new()));
        assert!(ids(&app, false).contains(&"transfer".into()));
        app.snapshots[3].state = State::Downloading { done: 4, total: 8 };
        assert_eq!(serde_json::to_value(&app.snapshots).unwrap(), original);
        assert_eq!(app.shared_operation.as_deref(), Some("Accepted transfer"));
        assert!(!app.busy);
        assert!(app.operation_label.is_none());
    }

    #[test]
    fn canceling_unload_dialog_preserves_loaded_competitor_and_operation() {
        use cosmic::Application;
        let mut app = Applet {
            mode: super::super::Mode::Management,
            shared_operation: Some("Accepted work".into()),
            ..Default::default()
        };
        app.snapshots = [
            ("example", herd::model::State::Ready),
            ("competitor", herd::model::State::Stopped),
        ]
        .into_iter()
        .map(|(id, state)| herd::model::Snapshot {
            model: herd::model::Model {
                id: id.into(),
                name: id.into(),
                short_name: String::new(),
                settings: Default::default(),
                icon: Default::default(),
            },
            state,
            detail: None,
        })
        .collect();
        let original = serde_json::to_value(&app.snapshots).unwrap();
        let _ = app.update(Message::AskStop("example".into()));
        assert!(app.stop_confirmation().is_some());
        let _ = app.update(Message::CancelStop);
        assert_eq!(serde_json::to_value(&app.snapshots).unwrap(), original);
        assert!(app.pending_stop.is_none());
        assert!(app.stop_confirmation().is_none());
        assert_eq!(app.shared_operation.as_deref(), Some("Accepted work"));
        assert!(app.operation_label.is_none());
        assert!(!app.busy);
    }

    #[test]
    fn canceling_removal_keeps_accepted_operations_and_other_dialog_state() {
        use cosmic::Application;
        let mut app = Applet {
            shared_operation: Some("accepted transfer".into()),
            pending_removal: Some(herd::backend::protocol::RemovalPlan {
                model: "example/tiny:Q8_0".into(),
                revision: "a".repeat(64),
                delete_files: true,
            }),
            ..Default::default()
        };
        let _ = app.update(Message::CancelRemove);
        assert!(app.pending_removal.is_none());
        assert!(!app.busy);
        assert_eq!(app.shared_operation.as_deref(), Some("accepted transfer"));
        assert!(app.error.is_none());
    }
    use cosmic::Application;
    use cosmic::iced::window::Id;

    #[test]
    fn launch_uses_sibling_application_and_does_not_inherit_panel_socket() {
        let cmd = command(Some("test-token".into()), None).unwrap();
        assert_eq!(
            std::path::Path::new(cmd.get_program()).file_name().unwrap(),
            "herd-cosmic"
        );
        assert_eq!(cmd.get_args().count(), 0);
        let env: std::collections::BTreeMap<_, _> = cmd.get_envs().collect();
        assert_eq!(env[std::ffi::OsStr::new("WAYLAND_SOCKET")], None);
        assert_eq!(
            env[std::ffi::OsStr::new("XDG_ACTIVATION_TOKEN")],
            Some(std::ffi::OsStr::new("test-token"))
        );
    }

    #[test]
    fn window_events_preserve_operations_and_ignore_stale_ids() {
        let popup = Id::unique();
        let mut app = Applet {
            popup: Some(popup),
            busy: true,
            pending_stop: Some("example.service".into()),
            operation_label: Some("Starting…".into()),
            ..Default::default()
        };
        let _ = app.update(Message::WindowClosed(Id::unique()));
        assert!(app.pending_stop.is_some());
        let _ = app.update(Message::WindowClosed(popup));
        assert!(app.busy);
        assert!(app.pending_stop.is_none());
        assert_eq!(app.operation_label.as_deref(), Some("Starting…"));
        let _ = app.update(Message::OpenManager);
        assert!(app.popup.is_none());
        let _ = app.update(Message::OpenManager);
        assert!(app.popup.is_none());
        assert!(app.busy);
    }
}
