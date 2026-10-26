//! Native COSMIC presentation components. No backend calls or lifecycle logic.
use super::localization::{action_name, arg, tr};
use super::{Applet, Message};
use cosmic::{
    Element,
    iced::{Alignment, ContentFit, Font, Length},
    theme, widget,
};
use herd::model::{Snapshot, State};

impl Applet {
    pub(super) fn panel_view(&self) -> Element<'_, Message> {
        let icon = if self.discovery_error.is_some() || self.error.is_some() {
            "dialog-warning-symbolic"
        } else if self
            .snapshots
            .iter()
            .any(|s| matches!(s.state, State::Downloading { .. }))
        {
            "folder-download-symbolic"
        } else {
            "io.github.nogcio.Herd.Cosmic-symbolic"
        };
        self.core
            .applet
            .icon_button(icon)
            .name(tr("open-controls"))
            .on_press(Message::TogglePopup)
            .into()
    }

    pub(super) fn popup_view(&self) -> Element<'_, Message> {
        let active = theme::active();
        let space = active.cosmic();
        let refresh = widget::button::icon(widget::icon::from_name("view-refresh-symbolic"))
            .name(tr("refresh"))
            .on_press_maybe(
                (!self.busy && !self.refreshing && !self.connecting && self.pending_stop.is_none())
                    .then_some(Message::Refresh),
            );
        let header = widget::row::with_capacity(2)
            .spacing(space.space_xs())
            .align_y(Alignment::Center)
            .push(widget::text::title4(tr("app-title")).width(Length::Fill))
            .push(refresh);

        let mut content = widget::column::with_capacity(8)
            .spacing(space.space_s())
            .padding(space.space_s())
            .width(Length::Fill)
            .push(header);

        if let Some(label) = &self.operation_label {
            content = content.push(
                widget::column::with_capacity(2)
                    .spacing(space.space_xxs())
                    .push(widget::text::caption(label))
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

        let mut models = widget::list::list_column().list_item_padding(space.space_xs());
        if self.snapshots.is_empty() {
            models = models.add(widget::text::caption(tr(if self.connecting {
                "connecting"
            } else {
                "no-models"
            })));
        }
        for snapshot in &self.snapshots {
            models = models.add(self.model_row(snapshot, false));
        }
        content = content.push(widget::scrollable(models).height(Length::Shrink));
        content = content.push(
            widget::button::text(tr("manage-models"))
                .leading_icon(widget::icon::from_name("preferences-system-symbolic"))
                .on_press(Message::OpenManager)
                .width(Length::Fill),
        );
        self.core.applet.popup_container(content).into()
    }

    pub(super) fn model_row<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        management: bool,
    ) -> Element<'a, Message> {
        let active = theme::active();
        let space = active.cosmic();
        let ready = snapshot.state == State::Ready;
        let pending = self.pending_stop.as_deref() == Some(&snapshot.model.id);
        let visual: Element<'_, Message> = if let Some(handle) = self.icons.get(&snapshot.model) {
            widget::icon(handle)
                .size(36)
                .content_fit(ContentFit::Contain)
                .into()
        } else {
            widget::text::heading(snapshot.model.badge())
                .font(Font::MONOSPACE)
                .class(if ready {
                    theme::Text::Accent
                } else {
                    theme::Text::Default
                })
                .into()
        };
        let badge = widget::container(visual)
            .width(40)
            .height(40)
            .center_x(40)
            .center_y(40);
        let mut status = state_label(&snapshot.state);
        if management && let Some(context) = snapshot.model.settings.get("ctx-size") {
            status.push_str(" · ");
            status.push_str(&arg("context-tokens", "value", context));
        }
        let mut state = widget::row::with_capacity(2)
            .spacing(space.space_xxs())
            .align_y(Alignment::Center);
        if matches!(
            snapshot.state,
            State::Failed | State::Unknown(_) | State::Unavailable
        ) {
            state = state.push(widget::icon::from_name("dialog-warning-symbolic").size(14));
        }
        state = state.push(widget::text::caption(status).class(if ready {
            theme::Text::Accent
        } else {
            theme::Text::Default
        }));
        let description = widget::column::with_capacity(2)
            .spacing(space.space_xxxs())
            .width(Length::Fill)
            .push(widget::text::heading(&snapshot.model.name))
            .push(state);
        let identity = widget::row::with_capacity(2)
            .spacing(space.space_xs())
            .align_y(Alignment::Center)
            .push(badge)
            .push(description)
            .width(Length::Fill);
        let identity: Element<'_, Message> = if management {
            let selected = self.core.window.show_context
                && self
                    .active_editor
                    .and_then(|id| self.editors.get(&id))
                    .is_some_and(|e| e.model == snapshot.model.id);
            // Menu overlays must not descend from a COSMIC button: the pinned
            // toolkit adds that ancestor's position to overlay coordinates.
            // Keep only identity clickable, with actions/menu as siblings.
            widget::button::custom(identity)
                .width(Length::Fill)
                .padding(space.space_xs())
                .class(theme::Button::ListItem(space.radius_s()))
                .selected(selected)
                .on_press_maybe(
                    (!self.busy && !snapshot.model.settings.is_empty())
                        .then(|| Message::Edit(snapshot.model.id.clone())),
                )
                .into()
        } else {
            identity.into()
        };
        let mut row = widget::row::with_capacity(3)
            .spacing(space.space_xs())
            .align_y(Alignment::Center)
            .push(identity);
        let mut transfer_action = None;
        if !pending && has_model_action(&snapshot.state) {
            let can_start = snapshot.state.can_start();
            let downloading = matches!(snapshot.state, State::Downloading { .. });
            let incomplete = snapshot.state == State::Incomplete;
            let action = if downloading {
                widget::button::text(tr("pause-download"))
                    .name(action_name("pause-download", &snapshot.model.name))
            } else if incomplete {
                widget::button::standard(tr("resume-download"))
                    .name(action_name("resume-download", &snapshot.model.name))
            } else if can_start {
                widget::button::standard(tr("start"))
                    .name(action_name("start", &snapshot.model.name))
            } else {
                widget::button::text(tr("stop")).name(action_name("stop", &snapshot.model.name))
            };
            let enabled = !self.busy
                && self.controller.is_some()
                && (!can_start || self.discovery_error.is_none())
                && self.pending_stop.is_none()
                && !matches!(
                    snapshot.state,
                    State::Stopping | State::Unavailable | State::Unknown(_)
                );
            let action = action.on_press_maybe(enabled.then(|| {
                if downloading {
                    Message::PauseDownload(snapshot.model.id.clone())
                } else if incomplete {
                    Message::ReviewDownload(snapshot.model.id.clone())
                } else if can_start {
                    Message::Start(snapshot.model.id.clone())
                } else {
                    Message::AskStop(snapshot.model.id.clone())
                }
            }));
            if !management && (downloading || incomplete) {
                transfer_action = Some(action);
            } else {
                row = row.push(action);
            }
        }
        if management {
            row = row.push(self.model_menu(snapshot));
        }
        let mut content = widget::column::with_capacity(4)
            .spacing(space.space_xs())
            .push(row);
        if let State::Downloading { done, total } = snapshot.state {
            if total == 0 {
                content = content.push(widget::progress_bar::indeterminate_linear());
            } else {
                content = content
                    .push(widget::text::caption(format!(
                        "{} / {} {}",
                        done / (1024 * 1024),
                        total / (1024 * 1024),
                        tr("mib")
                    )))
                    .push(widget::progress_bar::determinate_linear(
                        (done as f32 / total as f32).clamp(0., 1.),
                    ));
            }
        }
        if let Some(detail) = &snapshot.detail {
            content = content.push(widget::text::caption(detail));
        }
        if let Some(action) = transfer_action {
            content = content.push(
                widget::row::with_capacity(2)
                    .push(widget::Space::new().width(Length::Fill))
                    .push(action),
            );
        }
        if pending && !management {
            let confirm = widget::button::destructive(tr("stop-model"))
                .on_press_maybe((!self.busy).then_some(Message::ConfirmStop));
            content = content
                .push(widget::text::caption_heading(tr("stop-confirm")))
                .push(widget::text::caption(tr("stop-warning")))
                .push(
                    widget::row::with_capacity(3)
                        .spacing(space.space_xxs())
                        .push(widget::Space::new().width(Length::Fill))
                        .push(widget::button::text(tr("cancel")).on_press(Message::CancelStop))
                        .push(confirm),
                );
        }
        content.into()
    }

    pub(super) fn stop_confirmation(&self) -> Option<Element<'_, Message>> {
        if self.mode != super::Mode::Management {
            return None;
        }
        let id = self.pending_stop.as_ref()?;
        let snapshot = self.snapshots.iter().find(|s| &s.model.id == id)?;
        Some(
            widget::dialog()
                .title(tr("stop-confirm"))
                .body(format!("{}\n\n{}", snapshot.model.name, tr("stop-warning")))
                .primary_action(
                    widget::button::destructive(tr("stop-model"))
                        .on_press_maybe((!self.busy).then_some(Message::ConfirmStop)),
                )
                .secondary_action(widget::button::text(tr("cancel")).on_press(Message::CancelStop))
                .into(),
        )
    }
}

fn has_model_action(state: &State) -> bool {
    !matches!(
        state,
        State::Stopping | State::Unavailable | State::Unknown(_)
    )
}

pub(super) fn state_label(state: &State) -> String {
    tr(match state {
        State::Stopped => "state-stopped",
        State::Starting => "state-starting",
        State::Ready => "state-ready",
        State::Sleeping => "state-sleeping",
        State::Downloading { .. } => "state-downloading",
        State::Incomplete => "state-incomplete",
        State::Stopping => "state-stopping",
        State::Failed => "state-failed",
        State::Unavailable => "state-unavailable",
        State::Unknown(_) => "state-unknown",
    })
}

pub(super) fn notice<'a>(
    title: impl Into<std::borrow::Cow<'a, str>>,
    detail: &'a str,
) -> Element<'a, Message> {
    let title = title.into();
    widget::container(
        widget::row::with_capacity(2)
            .spacing(12)
            .push(
                widget::icon::from_name("dialog-warning-symbolic")
                    .size(20)
                    .icon(),
            )
            .push(
                widget::column::with_capacity(2)
                    .spacing(4)
                    .width(Length::Fill)
                    .push(widget::text::caption_heading(title))
                    .push(widget::text::caption(detail)),
            ),
    )
    .padding(12)
    .width(Length::Fill)
    .class(theme::Container::Card)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use herd::model::Model;

    fn applet() -> Applet {
        let models: Vec<_> = [
            ("example-qwen", "Qwen", "QW"),
            ("example-flint", "Flint", "FL"),
        ]
        .into_iter()
        .map(|(id, name, badge)| Model {
            id: id.into(),
            name: name.into(),
            short_name: badge.into(),
            settings: Default::default(),
            icon: Default::default(),
        })
        .collect();
        Applet {
            snapshots: models
                .into_iter()
                .map(|model| Snapshot {
                    model,
                    state: State::Ready,
                    detail: None,
                })
                .collect(),
            ..Default::default()
        }
    }

    // Widget construction checks do not start a GUI or touch services.
    // Pixel layout and theme contrast still require interactive acceptance.
    #[test]
    fn builds_ready_and_confirmation_views() {
        let mut app = applet();
        let _ = app.panel_view();
        let _ = app.popup_view();
        let _ = app.manager_content();
        app.pending_stop = Some(app.snapshots[0].model.id.clone());
        let _ = app.popup_view();
        let _ = app.manager_content();
    }

    #[tokio::test]
    async fn library_and_transfer_rows_fit_native_narrow_and_wide_layouts() {
        use cosmic::iced::advanced::{layout, renderer::Headless, widget::Tree};
        let renderer =
            <cosmic::Renderer as Headless>::new(Font::default(), 14.into(), Some("tiny-skia"))
                .await
                .expect("CPU renderer supports headless layout");
        let mut app = applet();
        app.snapshots[0].model.name = "Example organization / long model name with context".into();
        app.snapshots[1].state = State::Incomplete;
        for transfers in [false, true] {
            app.select_page(if transfers {
                super::super::navigation::Page::Downloads
            } else {
                super::super::navigation::Page::Models
            });
            for width in [360., 480., 800.] {
                let mut element = app.manager_content();
                let mut tree = Tree::new(element.as_widget());
                let limits = layout::Limits::new(
                    cosmic::iced::Size::ZERO,
                    cosmic::iced::Size::new(width, 700.),
                );
                let node = element
                    .as_widget_mut()
                    .layout(&mut tree, &renderer, &limits);
                assert!(node.size().height.is_finite());
                assert!(node.size().width <= width + 1.);
                fn fits(node: &layout::Node, x: f32, width: f32) {
                    let bounds = node.bounds();
                    let x = x + bounds.x;
                    assert!(
                        x + bounds.width <= width + 1.,
                        "row exceeds viewport: {bounds:?}, x={x}, width={width}"
                    );
                    for child in node.children() {
                        fits(child, x, width);
                    }
                }
                fits(&node, 0., width);
            }
        }
    }

    #[tokio::test]
    async fn model_overflow_opens_without_selecting_or_mutating_a_model() {
        use cosmic::{
            ApplicationExt,
            iced::{
                Event, Point, Size,
                advanced::{Renderer as _, renderer::Headless},
                mouse,
                runtime::{UserInterface, user_interface},
            },
        };
        let mut renderer =
            <cosmic::Renderer as Headless>::new(Font::default(), 14.into(), Some("tiny-skia"))
                .await
                .expect("CPU renderer supports native menu rendering");
        let mut app = applet();
        app.mode = super::super::Mode::Management;
        for snapshot in &mut app.snapshots {
            snapshot
                .model
                .settings
                .insert("ctx-size".into(), "32768".into());
        }
        let original = serde_json::to_value(&app.snapshots).unwrap();
        let window = cosmic::iced::window::Id::unique();
        app.core.set_main_window_id(Some(window));
        let mut app = cosmic::app::cosmic::Cosmic::new(app);
        let _ = app.update(cosmic::Action::Cosmic(cosmic::app::Action::WindowResize(
            window, 1240., 820.,
        )));
        let theme = cosmic::Theme::dark();
        let style = cosmic::iced::advanced::renderer::Style {
            text_color: theme.cosmic().on_bg_color().into(),
            icon_color: theme.cosmic().on_bg_color().into(),
            ..Default::default()
        };
        let mut ui = UserInterface::build(
            app.app.view_main(),
            Size::new(1240., 820.),
            user_interface::Cache::default(),
            &mut renderer,
        );
        let mut clipboard = cosmic::iced::advanced::clipboard::Null;
        let mut messages = Vec::new();
        let _ = ui.update(
            &[],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut clipboard,
            &mut messages,
        );
        renderer.reset(cosmic::iced::Rectangle::with_size(Size::new(1240., 820.)));
        ui.draw(&mut renderer, &theme, &style, mouse::Cursor::Unavailable);
        let before = renderer.screenshot(Size::new(1240, 820), 1., cosmic::iced::Color::BLACK);
        let position = Point::new(1176., 279.);
        let cursor = mouse::Cursor::Available(position);
        let _ = ui.update(
            &[
                Event::Mouse(mouse::Event::CursorMoved { position }),
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
            cursor,
            &mut renderer,
            &mut clipboard,
            &mut messages,
        );
        // Native fallback menus initialize on the redraw following release.
        let _ = ui.update(
            &[Event::Window(cosmic::iced::window::Event::RedrawRequested(
                cosmic::iced::time::Instant::now(),
            ))],
            cursor,
            &mut renderer,
            &mut clipboard,
            &mut messages,
        );
        renderer.reset(cosmic::iced::Rectangle::with_size(Size::new(1240., 820.)));
        ui.draw(&mut renderer, &theme, &style, cursor);
        let after = renderer.screenshot(Size::new(1240, 820), 1., cosmic::iced::Color::BLACK);
        // Check the popup region below the trigger, excluding its hover surface.
        let changed = (300..366)
            .flat_map(|y| (974..1192).map(move |x| (y * 1240 + x) * 4))
            .filter(|i| before[*i..*i + 4] != after[*i..*i + 4])
            .count();
        assert!(
            changed > 100,
            "overflow must paint its settings/removal menu below the trigger ({changed} changed pixels)"
        );
        assert!(!messages.iter().any(|message| matches!(
            message,
            cosmic::Action::App(
                Message::Edit(_) | Message::Start(_) | Message::AskStop(_) | Message::AskRemove(_)
            )
        )));
        assert_eq!(serde_json::to_value(&app.app.snapshots).unwrap(), original);
        assert!(app.app.active_editor.is_none());
        assert!(app.app.pending_stop.is_none());
    }

    #[test]
    fn builds_empty_error_and_busy_views() {
        let mut app = Applet {
            connecting: true,
            ..Default::default()
        };
        let _ = app.popup_view();
        let _ = app.manager_content();
        app.connecting = false;
        app.error = Some("Configuration is unavailable".into());
        let _ = app.popup_view();
        let _ = app.manager_content();
        app.busy = true;
        app.operation_label = Some("Starting…".into());
        let _ = app.popup_view();
        let _ = app.manager_content();
    }

    #[test]
    fn builds_all_model_states() {
        let mut app = applet();
        for state in [
            State::Stopped,
            State::Starting,
            State::Stopping,
            State::Failed,
            State::Unavailable,
            State::Unknown("D-Bus unavailable".into()),
        ] {
            app.snapshots[0].state = state;
            let _ = app.popup_view();
            let _ = app.manager_content();
        }
    }

    #[test]
    fn unavailable_states_never_offer_an_unload_of_an_unobserved_instance() {
        let mut app = applet();
        let competitor = serde_json::to_value(&app.snapshots[1]).unwrap();
        for state in [
            State::Unavailable,
            State::Unknown("unexpected".into()),
            State::Stopping,
        ] {
            app.snapshots[0].state = state;
            assert!(!has_model_action(&app.snapshots[0].state));
            let _ = app.model_row(&app.snapshots[0], true);
            assert!(app.pending_stop.is_none());
            assert_eq!(serde_json::to_value(&app.snapshots[1]).unwrap(), competitor);
        }
        for state in [
            State::Ready,
            State::Starting,
            State::Sleeping,
            State::Stopped,
            State::Failed,
            State::Incomplete,
            State::Downloading { done: 0, total: 0 },
        ] {
            assert!(has_model_action(&state));
        }
    }

    #[test]
    fn configuration_failure_retains_explicitly_stale_observations_instead_of_hiding_models() {
        use cosmic::Application;
        let mut app = applet();
        let ids: Vec<_> = app.snapshots.iter().map(|s| s.model.id.clone()).collect();
        let _ = app.update(Message::Refreshed(Err("Preset file is unreadable".into())));
        assert_eq!(
            app.snapshots
                .iter()
                .map(|s| s.model.id.clone())
                .collect::<Vec<_>>(),
            ids
        );
        assert!(
            app.discovery_error
                .as_ref()
                .unwrap()
                .contains(&tr("stale-observations"))
        );
        assert_eq!(app.snapshots[0].state, State::Ready);
        assert!(
            app.discovery_error
                .as_ref()
                .unwrap()
                .starts_with("Preset file is unreadable\n")
        );
        let _ = app.panel_view();
        let _ = app.manager_content();
    }

    #[tokio::test]
    async fn builds_model_cards_with_loaded_artwork() {
        use herd::icons::{LoadedIcon, Origin};
        let mut app = applet();
        app.snapshots[0].model.icon.url = Some("https://huggingface.co/test/logo.png".into());
        // Register an in-flight request, but deliberately never execute its task.
        let _ = app.icons.schedule(&app.snapshots);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        app.icons.complete(
            app.snapshots[0].model.icon.clone(),
            Ok(LoadedIcon {
                png: bytes.into_inner(),
                origin: Origin::Cached,
            }),
        );
        assert!(app.icons.get(&app.snapshots[0].model).is_some());
        let _ = app.popup_view();
    }

    #[test]
    fn panel_views_follow_both_native_anchor_orientations() {
        use cosmic::applet::cosmic_panel_config::PanelAnchor;
        let mut app = applet();
        for anchor in [
            PanelAnchor::Top,
            PanelAnchor::Bottom,
            PanelAnchor::Left,
            PanelAnchor::Right,
        ] {
            app.core.applet.anchor = anchor;
            let _ = app.panel_view();
        }
    }

    #[test]
    fn application_and_panel_have_distinct_desktop_roles() {
        use cosmic::Application;
        let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
        let desktop =
            std::fs::read_to_string(resources.join(format!("{}.desktop", Applet::APP_ID))).unwrap();
        let is_panel = Applet::APP_ID.ends_with(".Applet");
        assert_eq!(
            desktop.lines().any(|line| line == "X-CosmicApplet=true"),
            is_panel
        );
        assert_eq!(
            desktop.lines().any(|line| line == "NoDisplay=true"),
            is_panel
        );
        let application = "io.github.nogcio.Herd.Cosmic";
        let metadata =
            std::fs::read_to_string(resources.join(format!("{application}.metainfo.xml"))).unwrap();
        assert!(metadata.contains(&format!("<id>{application}</id>")));
        assert!(metadata.contains(&format!(
            "<launchable type=\"desktop-id\">{application}.desktop</launchable>"
        )));
    }
}
