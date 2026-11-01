//! Native engine selection and capability-driven controls; no runtime policy.
use super::{Applet, Message, localization::tr};
use cosmic::{Element, iced::Length, prelude::*, widget};
use herd::runtime::{RuntimeRecord, Target};

impl Applet {
    pub(super) fn runtimes_view(&self) -> Element<'_, Message> {
        let mut content = widget::column::with_capacity(8)
            .spacing(12)
            .padding(16)
            .push(widget::text::title3(tr("runtimes")))
            .push(widget::text::body(tr("runtimes-help")));
        if let Some(error) = &self.error {
            content = content.push(super::view::notice(tr("action-unavailable"), error));
        }
        if let Some(operation) = self
            .operation_label
            .as_ref()
            .or(self.shared_operation.as_ref())
        {
            content = content
                .push(widget::text::body(operation))
                .push(widget::progress_bar::indeterminate_linear());
        }
        for runtime in &self.runtimes {
            content = content.push(self.runtime_card(runtime));
        }
        widget::scrollable(content).height(Length::Fill).into()
    }

    pub(super) fn runtime_card<'a>(&'a self, runtime: &'a RuntimeRecord) -> Element<'a, Message> {
        let name = if runtime.engine == "llama_cpp" {
            "llama.cpp"
        } else {
            "Colibri"
        };
        let mut content = widget::column::with_capacity(8)
            .spacing(8)
            .push(widget::text::title4(name));
        if runtime.engine == "llama_cpp" {
            content = content
                .push(widget::text::body(tr("llama-library-help")))
                .push(
                    widget::button::standard(tr("models"))
                        .on_press(Message::Navigate(super::navigation::Page::Models)),
                );
        } else {
            content = content
                .push(widget::text::body(if runtime.name.is_empty() {
                    tr("colibri-not-configured")
                } else {
                    runtime.name.clone()
                }))
                .push(widget::text::body(tr(match runtime.state.as_str() {
                    "stopped" => "runtime-stopped",
                    "starting" => "runtime-starting",
                    "ready" => "runtime-ready",
                    "external" => "runtime-external",
                    "failed" => "runtime-failed",
                    _ => "runtime-unknown",
                })));
            if runtime.pending {
                content = content.push(widget::text::body(tr("runtime-pending")));
            }
            if self.mode == super::Mode::Panel && !runtime.transfer.is_empty() {
                content = content.push(widget::text::body(format!(
                    "{} · {} · {} / {} B",
                    runtime.transfer_model, runtime.transfer, runtime.done, runtime.total
                )));
                if matches!(
                    runtime.transfer.as_str(),
                    "starting" | "downloading" | "verifying"
                ) {
                    content = content
                        .push(widget::progress_bar::indeterminate_linear())
                        .push(widget::button::text(tr("pause-download")).on_press_maybe(
                            (!self.busy).then_some(Message::Colibri(super::colibri::Event::Pause)),
                        ));
                }
            }
            let target = Target {
                runtime: runtime.id.clone(),
                model: "configured".into(),
                revision: runtime.revision.clone(),
            };
            let available = !self.busy && self.controller.is_some();
            let confirming = self.mode == super::Mode::Panel
                && self
                    .pending_runtime_stop
                    .as_ref()
                    .is_some_and(|p| p.runtime == target.runtime);
            let actions = if confirming {
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(
                        widget::button::destructive(tr("runtime-stop"))
                            .on_press_maybe(available.then_some(Message::ConfirmStop)),
                    )
                    .push(widget::button::text(tr("cancel")).on_press(Message::CancelStop))
            } else {
                widget::row::with_capacity(2)
                    .spacing(8)
                    .push(
                        widget::button::suggested(tr("runtime-start")).on_press_maybe(
                            (available && runtime.can_start)
                                .then_some(Message::StartRuntime(target.clone())),
                        ),
                    )
                    .push(
                        widget::button::destructive(tr("runtime-stop")).on_press_maybe(
                            (available && runtime.can_stop)
                                .then_some(Message::AskStopRuntime(target)),
                        ),
                    )
            };
            if confirming {
                content = content.push(widget::text::body(tr("stop-colibri-warning")));
            }
            content = content
                .push(actions)
                .push(widget::text::caption(tr("colibri-native-settings")));
            content = content.push(
                widget::button::text(tr("settings")).on_press_maybe(
                    (available && runtime.configuration == "valid")
                        .then_some(Message::ColibriEditor(super::colibri_editor::Event::Open)),
                ),
            );
        }
        if !runtime.error.is_empty() {
            content = content.push(super::view::notice(
                tr("action-unavailable"),
                &runtime.error,
            ));
        }
        widget::container(content)
            .padding(12)
            .width(Length::Fill)
            .into()
    }

    pub(super) fn operate_runtime(
        &mut self,
        target: Target,
        stopping: bool,
    ) -> Task<cosmic::Action<Message>> {
        if self.busy {
            return Task::none();
        }
        let Some(client) = self.controller.clone() else {
            return Task::none();
        };
        self.pending_runtime_stop = None;
        self.pending_stop = None;
        self.busy = true;
        self.error = None;
        self.operation_label = Some(tr(if stopping {
            "stopping-model"
        } else {
            "starting-model"
        }));
        Task::perform(
            async move {
                let operation = if stopping {
                    client.stop_target(&target, true).await
                } else {
                    client.start_target(&target).await
                }
                .map_err(|e| e.to_string())?;
                client.wait(&operation).await.map_err(|e| e.to_string())
            },
            |result| cosmic::Action::App(Message::Finished(result)),
        )
    }
}
