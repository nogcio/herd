//! Complete per-profile drafts. Inheritance is never flattened by the UI.
use super::localization::{arg, tr};
use super::{Applet, Message};
use cosmic::{
    Element,
    iced::{Alignment, Length, window::Id},
    prelude::*,
    theme, widget,
};
use herd::backend::protocol::PresetDraft;
use std::collections::BTreeMap;

const GROUPS: &[(&str, &[&str])] = &[
    (
        "group-context",
        &[
            "ctx-size",
            "parallel",
            "batch-size",
            "ubatch-size",
            "ctx-checkpoints",
            "no-context-shift",
            "context-shift",
            "cache-ram",
            "dedup-cache-models",
        ],
    ),
    (
        "group-gpu",
        &[
            "n-gpu-layers",
            "gpu-layers",
            "n-cpu-moe",
            "moe-cache-mib",
            "fit",
            "flash-attn",
        ],
    ),
    ("group-kv", &["cache-type-k", "cache-type-v"]),
    ("group-cpu", &["threads", "threads-batch"]),
    ("group-prompt", &["jinja", "reasoning", "reasoning-format"]),
    ("group-sampling", &["temp", "top-p", "top-k", "min-p"]),
    ("group-lifecycle", &["stop-timeout", "load-on-startup"]),
    (
        "group-files",
        &[
            "model",
            "hf-repo",
            "hf-file",
            "alias",
            "tags",
            "mmproj",
            "no-mmproj",
        ],
    ),
];

pub(super) struct Editor {
    pub model: String,
    pub draft: Option<PresetDraft>,
    original: BTreeMap<String, String>,
    ticket: u64,
    loading: bool,
    pub saving: bool,
    pub discard: bool,
    reload_confirmation: bool,
    error: Option<String>,
    expanded: [bool; GROUPS.len()],
    field_ids: BTreeMap<&'static str, widget::Id>,
}
impl Editor {
    fn new(model: String) -> Self {
        Self {
            model,
            draft: None,
            original: BTreeMap::new(),
            ticket: 0,
            loading: true,
            saving: false,
            discard: false,
            reload_confirmation: false,
            error: None,
            expanded: std::array::from_fn(|i| matches!(GROUPS[i].0, "group-context" | "group-gpu")),
            field_ids: herd::config::OPTIONS
                .iter()
                .map(|key| (*key, widget::Id::unique()))
                .collect(),
        }
    }
    pub fn dirty(&self) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|d| d.options != self.original)
    }
    fn loaded(&mut self, ticket: u64, result: Result<PresetDraft, String>) {
        if ticket != self.ticket {
            return;
        }
        self.loading = false;
        match result {
            Ok(draft) => {
                self.original = draft.options.clone();
                self.draft = Some(draft);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn field(&self, id: Id, key: &'static str, enabled: bool) -> Option<Field> {
        let draft = self.draft.as_ref()?;
        let local = draft.options.get(key);
        Some(Field {
            draft: id,
            key,
            value: local
                .or_else(|| draft.defaults.get(key))
                .cloned()
                .unwrap_or_default(),
            overridden: local.is_some(),
            enabled,
            input: self.field_ids.get(key)?.clone(),
            spacing: theme::spacing().space_xs,
        })
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Close(Id),
    Discard(Id),
    ToggleGroup(Id, usize),
    Loaded(Id, u64, Result<PresetDraft, String>),
    Change(Id, String, String),
    Reset(Id, String),
    Revert(Id),
    Save(Id),
    Saved(Id, Result<PresetDraft, String>),
    Reload(Id),
    ConfirmReload(Id),
    KeepDraft(Id),
}

impl Applet {
    pub(super) fn open_editor(&mut self, model: String) -> Task<cosmic::Action<Message>> {
        self.select_page(super::navigation::Page::Models);
        if let Some(id) = self
            .editors
            .iter()
            .find_map(|(id, e)| (e.model == model).then_some(*id))
        {
            self.active_editor = Some(id);
            self.core.set_show_context(true);
            return Task::none();
        }
        let id = Id::unique();
        self.editors.insert(id, Editor::new(model));
        self.active_editor = Some(id);
        self.core.set_show_context(true);
        self.load_editor(id)
    }
    fn load_editor(&mut self, id: Id) -> Task<cosmic::Action<Message>> {
        let Some(editor) = self.editors.get_mut(&id) else {
            return Task::none();
        };
        editor.ticket = editor.ticket.wrapping_add(1);
        editor.loading = true;
        editor.reload_confirmation = false;
        let ticket = editor.ticket;
        let model = editor.model.clone();
        let client = self.controller.clone();
        Task::perform(
            async move {
                let c = client.ok_or_else(|| tr("backend-unavailable"))?;
                c.edit_preset(&model).await.map_err(|e| e.to_string())
            },
            move |r| cosmic::Action::App(Message::Editor(Event::Loaded(id, ticket, r))),
        )
    }
    pub(super) fn editor_event(&mut self, event: Event) -> Task<cosmic::Action<Message>> {
        match event {
            Event::Close(id) => {
                if let Some(e) = self.editors.get_mut(&id)
                    && e.dirty()
                    && !e.saving
                {
                    e.discard = true;
                    self.active_editor = Some(id);
                    self.core.set_show_context(true);
                } else {
                    if !self.editors.get(&id).is_some_and(|e| e.saving) {
                        self.editors.remove(&id);
                    }
                    if self.active_editor == Some(id) {
                        self.active_editor = None;
                        self.core.set_show_context(false);
                    }
                }
            }
            Event::Discard(id) => {
                if !self.editors.get(&id).is_some_and(|e| e.saving) {
                    self.editors.remove(&id);
                }
                if self.active_editor == Some(id) {
                    self.active_editor = None;
                    self.core.set_show_context(false);
                }
                if self.pending_close {
                    return self.close_manager();
                }
            }
            Event::ToggleGroup(id, group) => {
                if group < GROUPS.len()
                    && let Some(e) = self.editors.get_mut(&id)
                {
                    e.expanded[group] = !e.expanded[group];
                }
            }
            Event::Loaded(id, ticket, result) => {
                if let Some(e) = self.editors.get_mut(&id) {
                    e.loaded(ticket, result);
                }
            }
            Event::Change(id, key, value) => {
                if let Some(e) = self.editors.get_mut(&id)
                    && !e.saving
                    && !e.loading
                    && !e.discard
                    && !e.reload_confirmation
                    && let Some(d) = &mut e.draft
                {
                    d.options.insert(key, value);
                }
            }
            Event::Reset(id, key) => {
                if let Some(e) = self.editors.get_mut(&id)
                    && !e.saving
                    && !e.loading
                    && !e.discard
                    && !e.reload_confirmation
                    && let Some(d) = &mut e.draft
                {
                    d.options.remove(&key);
                }
            }
            Event::Revert(id) => {
                if let Some(e) = self.editors.get_mut(&id)
                    && !e.saving
                    && !e.loading
                    && !e.discard
                    && !e.reload_confirmation
                    && let Some(draft) = &mut e.draft
                {
                    draft.options.clone_from(&e.original);
                }
            }
            Event::Reload(id) => {
                if let Some(e) = self.editors.get_mut(&id)
                    && !e.saving
                    && !e.loading
                {
                    if e.dirty() {
                        e.reload_confirmation = true;
                    } else {
                        return self.load_editor(id);
                    }
                }
            }
            Event::ConfirmReload(id) => {
                if self
                    .editors
                    .get(&id)
                    .is_some_and(|e| !e.saving && !e.loading)
                {
                    return self.load_editor(id);
                }
            }
            Event::KeepDraft(id) => {
                self.pending_close = false;
                if let Some(e) = self.editors.get_mut(&id) {
                    e.reload_confirmation = false;
                    e.discard = false;
                }
            }
            Event::Save(id) if !self.busy => {
                let Some(e) = self.editors.get_mut(&id) else {
                    return Task::none();
                };
                if e.saving || e.loading || e.discard || e.reload_confirmation || !e.dirty() {
                    return Task::none();
                }
                let (Some(c), Some(draft)) = (self.controller.clone(), e.draft.clone()) else {
                    return Task::none();
                };
                e.saving = true;
                e.error = None;
                self.busy = true;
                return Task::perform(
                    async move {
                        let job = c.save_preset(&draft).await.map_err(|e| e.to_string())?;
                        c.wait(&job).await.map_err(|e| e.to_string())?;
                        c.edit_preset(&draft.id).await.map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::Editor(Event::Saved(id, r))),
                );
            }
            Event::Saved(id, result) => {
                if !self.editors.get(&id).is_some_and(|e| e.saving) {
                    return Task::none();
                }
                self.busy = false;
                if let Some(e) = self.editors.get_mut(&id) {
                    e.saving = false;
                    e.loaded(e.ticket, result);
                }
                return self.refresh();
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn editor_content(&self, id: Id) -> Element<'_, Message> {
        let Some(e) = self.editors.get(&id) else {
            return widget::Space::new().into();
        };
        let mut content = widget::column::with_capacity(2).spacing(theme::spacing().space_xxs);
        if let Some(snapshot) = self.snapshots.iter().find(|s| s.model.id == e.model) {
            content = content.push(widget::text::caption_heading(format!(
                "{} · {}",
                snapshot.model.name,
                super::view::state_label(&snapshot.state),
            )));
        } else {
            content = content.push(widget::text::heading(&e.model));
        }
        content = content.push(widget::text::caption(tr("settings-pending")));
        if let Some(error) = &e.error {
            content = content.push(super::view::notice(tr("settings-error"), error));
        }
        let enabled = !self.busy && !e.saving && !e.loading && !e.discard && !e.reload_confirmation;
        if e.draft.is_some() {
            for (group, (title, keys)) in GROUPS.iter().enumerate() {
                let expanded = e.expanded[group];
                let mut section = widget::settings::section().add(
                    widget::list::button(widget::settings::item(
                        tr(title),
                        widget::icon::from_name(if expanded {
                            "pan-down-symbolic"
                        } else {
                            "pan-end-symbolic"
                        })
                        .size(16),
                    ))
                    .on_press(Message::Editor(Event::ToggleGroup(id, group))),
                );
                if expanded {
                    for key in *keys {
                        if let Some(field) = e.field(id, key, enabled) {
                            section = section.add(Element::from(widget::lazy(field, field_view)));
                        }
                    }
                }
                content = content.push(section);
            }
        } else if e.error.is_none() {
            content = content.push(widget::text::body(tr("reading-settings")));
        }
        content.into()
    }

    pub(super) fn settings_drawer(
        &self,
    ) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        if !self.core.window.show_context {
            return None;
        }
        let id = self.active_editor?;
        let e = self.editors.get(&id)?;
        let actions = widget::row::with_capacity(3)
            .spacing(theme::spacing().space_xs)
            .push(widget::Space::new().width(Length::Fill))
            .push(
                widget::button::text(tr("discard-draft")).on_press_maybe(
                    (!self.busy && !e.loading && !e.saving && e.dirty())
                        .then_some(Message::Editor(Event::Revert(id))),
                ),
            )
            .push(
                widget::button::suggested(tr(if e.saving { "saving" } else { "save" }))
                    .on_press_maybe(
                        (!self.busy && !e.loading && !e.saving && e.dirty())
                            .then_some(Message::Editor(Event::Save(id))),
                    ),
            );
        let drawer = cosmic::app::context_drawer::context_drawer(
            self.editor_content(id),
            Message::Editor(Event::Close(id)),
        )
        .actions(
            widget::row::with_capacity(2)
                .spacing(theme::spacing().space_xxs)
                .align_y(Alignment::Center)
                .push(widget::text::caption_heading(tr("model-settings")).width(Length::Fill))
                .push(
                    widget::button::icon(widget::icon::from_name("view-refresh-symbolic"))
                        .name(tr("reload-settings"))
                        .on_press_maybe(
                            (!e.loading && !e.saving).then_some(Message::Editor(Event::Reload(id))),
                        ),
                ),
        )
        .footer(actions);
        Some(drawer)
    }

    fn editor_confirmation_id(&self) -> Option<Id> {
        self.editors
            .iter()
            .find_map(|(id, e)| (e.discard || e.reload_confirmation).then_some(*id))
    }

    pub(super) fn cancel_editor_confirmation(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(id) = self.editor_confirmation_id() {
            self.editor_event(Event::KeepDraft(id))
        } else {
            Task::none()
        }
    }

    fn editor_target_identity(&self, model: &str) -> String {
        match self.snapshots.iter().find(|s| s.model.id == model) {
            Some(snapshot) if snapshot.model.name != model => {
                format!("{} ({model})", snapshot.model.name)
            }
            _ => model.to_owned(),
        }
    }

    pub(super) fn editor_confirmation(&self) -> Option<Element<'_, Message>> {
        let id = self.editor_confirmation_id()?;
        let e = self.editors.get(&id)?;
        let action = if e.discard {
            Event::Discard(id)
        } else {
            Event::ConfirmReload(id)
        };
        Some(
            widget::dialog()
                .title(tr(if e.discard {
                    "discard-changes"
                } else {
                    "reload-discard"
                }))
                .body(self.editor_target_identity(&e.model))
                .primary_action(
                    widget::button::destructive(tr("discard")).on_press(Message::Editor(action)),
                )
                .secondary_action(
                    widget::button::text(tr("keep-editing"))
                        .on_press(Message::Editor(Event::KeepDraft(id))),
                )
                .into(),
        )
    }
}

/// Hash dependencies exclude observations and unrelated fields.
#[derive(Clone, Hash, PartialEq, Eq)]
struct Field {
    draft: Id,
    key: &'static str,
    value: String,
    overridden: bool,
    enabled: bool,
    input: widget::Id,
    spacing: u16,
}

fn field_view(field: &Field) -> Element<'static, Message> {
    let Field {
        draft: id,
        key,
        value,
        enabled,
        ..
    } = field.clone();
    let label = tr(&format!("parameter-{key}"));
    let compact =
        key == "load-on-startup" || (is_boolean(key) && matches!(value.as_str(), "true" | "false"));
    let control: Element<'static, Message> = if key == "load-on-startup" {
        widget::text::body(tr("off")).into()
    } else if is_boolean(key) && matches!(value.as_str(), "true" | "false") {
        widget::toggler(value == "true")
            .on_toggle_maybe(enabled.then_some(move |v: bool| {
                Message::Editor(Event::Change(id, key.into(), v.to_string()))
            }))
            .into()
    } else if let Some(choices) = choices(key)
        && let Some(selected) = choices.iter().position(|v| *v == value)
    {
        if enabled {
            widget::dropdown(choices, Some(selected), move |index| {
                Message::Editor(Event::Change(id, key.into(), choices[index].into()))
            })
            .width(Length::Fill)
            .into()
        } else {
            widget::text::body(value).into()
        }
    } else {
        let mut input = widget::text_input(tr("default-value"), value).id(field.input.clone());
        if enabled {
            input =
                input.on_input(move |value| Message::Editor(Event::Change(id, key.into(), value)));
        }
        input.into()
    };
    let control = widget::row::with_capacity(2)
        .spacing(field.spacing)
        .align_y(Alignment::Center)
        .push(widget::container(control).width(if compact {
            Length::Shrink
        } else {
            Length::Fill
        }))
        .push(
            widget::button::icon(widget::icon::from_name("edit-undo-symbolic"))
                .name(arg("reset-parameter", "parameter", &label))
                .on_press_maybe(
                    (enabled && field.overridden && key != "load-on-startup")
                        .then_some(Message::Editor(Event::Reset(id, key.into()))),
                ),
        )
        .width(if compact {
            Length::Shrink
        } else {
            Length::Fill
        });
    let item =
        widget::settings::item::builder(label).description(tr(&format!("description-{key}")));
    if compact {
        item.control(control).into()
    } else {
        item.flex_control(control).into()
    }
}

fn is_boolean(key: &str) -> bool {
    matches!(
        key,
        "jinja" | "no-context-shift" | "context-shift" | "no-mmproj"
    )
}
fn choices(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "fit" => Some(&["on", "off"]),
        "dedup-cache-models" => Some(&["0", "1"]),
        "flash-attn" | "reasoning" => Some(&["on", "off", "auto"]),
        "cache-type-k" | "cache-type-v" => Some(&[
            "f32", "f16", "bf16", "q8_0", "q4_0", "q4_1", "iq4_nl", "q5_0", "q5_1",
        ]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_close_dispatch_confirms_each_draft_and_preserves_accepted_work() {
        use cosmic::app::Action as NativeAction;
        use cosmic::iced::{futures::StreamExt, runtime};
        let mut app = Applet {
            mode: super::super::Mode::Management,
            busy: true,
            operation_label: Some("Accepted operation".into()),
            shared_operation: Some("Accepted transfer".into()),
            ..Default::default()
        };
        let window = Id::unique();
        app.core.set_main_window_id(Some(window));
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
        for model in ["first", "second"] {
            let _ = app.open_editor(model.into());
            let id = app.active_editor.unwrap();
            let _ = app.editor_event(Event::Loaded(id, 1, Ok(PresetDraft::default())));
            let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "8192".into()));
        }
        let mut shell = cosmic::app::cosmic::Cosmic::new(app);
        // Exercise the pinned toolkit's actual header Close dispatch, not only
        // Herd's CloseManager message. No surface-close task may escape it.
        for escape in [true, false] {
            let task = shell.update(cosmic::Action::Cosmic(NativeAction::Close));
            assert_eq!(task.units(), 0);
            assert!(shell.app.pending_close);
            assert!(shell.app.editor_confirmation().is_some());
            let id = shell.app.active_editor.unwrap();
            let task = if escape {
                shell.update(cosmic::Action::Cosmic(NativeAction::KeyboardNav(
                    cosmic::keyboard_nav::Action::Escape,
                )))
            } else {
                shell.update(cosmic::Action::App(Message::Editor(Event::KeepDraft(id))))
            };
            assert_eq!(task.units(), 0);
            assert!(!shell.app.pending_close);
            assert!(shell.app.editor_confirmation().is_none());
            assert_eq!(shell.app.editors.len(), 2);
            assert!(
                shell.app.editors.values().all(|e| {
                    e.dirty() && e.draft.as_ref().unwrap().options["ctx-size"] == "8192"
                })
            );
        }
        // The window::close_requests subscription uses this same wrapper route
        // for compositor/WM requests (requires auto-close disabled at boot).
        let task = shell.update(cosmic::Action::App(Message::CloseRequested(Id::unique())));
        assert_eq!(task.units(), 0);
        assert!(!shell.app.pending_close);
        let task = shell.update(cosmic::Action::App(Message::CloseRequested(window)));
        assert_eq!(task.units(), 0);
        assert!(shell.app.pending_close);
        let first = shell.app.active_editor.unwrap();
        let task = shell.update(cosmic::Action::App(Message::Editor(Event::Discard(first))));
        assert_eq!(task.units(), 0);
        assert!(shell.app.pending_close);
        let second = shell.app.active_editor.unwrap();
        assert_ne!(first, second);
        assert!(shell.app.editor_confirmation().is_some());
        let task = shell.update(cosmic::Action::App(Message::Editor(Event::Discard(second))));
        let mut actions = runtime::task::into_stream(task).unwrap();
        assert!(matches!(actions.next().await, Some(runtime::Action::Exit)));
        assert!(actions.next().await.is_none());
        assert!(shell.app.editors.is_empty());
        assert!(shell.app.busy);
        assert_eq!(
            shell.app.operation_label.as_deref(),
            Some("Accepted operation")
        );
        assert_eq!(
            shell.app.shared_operation.as_deref(),
            Some("Accepted transfer")
        );
        assert_eq!(
            serde_json::to_value(&shell.app.snapshots).unwrap(),
            observations
        );
    }

    #[test]
    fn confirmation_identity_keeps_display_name_and_exact_native_target() {
        use herd::model::{Model, Snapshot, State};
        let app = Applet {
            snapshots: [("atlas", "Atlas 27B"), ("competitor", "competitor")]
                .into_iter()
                .map(|(id, name)| Snapshot {
                    model: Model {
                        id: id.into(),
                        name: name.into(),
                        short_name: String::new(),
                        settings: Default::default(),
                        icon: Default::default(),
                    },
                    state: State::Ready,
                    detail: None,
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(app.editor_target_identity("atlas"), "Atlas 27B (atlas)");
        assert_eq!(app.editor_target_identity("competitor"), "competitor");
        assert_eq!(
            app.editor_target_identity("missing-profile"),
            "missing-profile"
        );
    }

    #[tokio::test]
    async fn short_drawer_exposes_first_input_and_retains_pending_explanation() {
        use cosmic::iced::advanced::{
            Layout, layout,
            renderer::Headless,
            widget::{Operation, Tree, operation},
        };
        use cosmic::iced::{Rectangle, Size};
        struct VisibleInput {
            target: widget::Id,
            input: Option<Rectangle>,
            viewport: Option<Rectangle>,
            pending: bool,
        }
        impl Operation for VisibleInput {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }
            fn scrollable(
                &mut self,
                _: Option<&widget::Id>,
                bounds: Rectangle,
                _: Rectangle,
                _: cosmic::iced::Vector,
                _: &mut dyn operation::Scrollable,
            ) {
                self.viewport = Some(bounds);
            }
            fn text_input(
                &mut self,
                id: Option<&widget::Id>,
                bounds: Rectangle,
                _: &mut dyn operation::TextInput,
            ) {
                if id == Some(&self.target) {
                    self.input = Some(bounds);
                }
            }
            fn text(&mut self, _: Option<&widget::Id>, _: Rectangle, text: &str) {
                self.pending |= text == tr("settings-pending");
            }
        }
        let renderer = <cosmic::Renderer as Headless>::new(
            cosmic::iced::Font::default(),
            14.into(),
            Some("tiny-skia"),
        )
        .await
        .expect("CPU renderer supports headless layout");
        let mut app = Applet::default();
        app.snapshots.push(herd::model::Snapshot {
            model: herd::model::Model {
                id: "atlas".into(),
                name: "Atlas 27B".into(),
                short_name: String::new(),
                settings: Default::default(),
                icon: Default::default(),
            },
            state: herd::model::State::Ready,
            detail: None,
        });
        let _ = app.open_editor("atlas".into());
        let id = app.active_editor.unwrap();
        let _ = app.editor_event(Event::Loaded(id, 1, Ok(PresetDraft::default())));
        let mut operation = VisibleInput {
            target: app.editors[&id].field_ids["ctx-size"].clone(),
            input: None,
            viewport: None,
            pending: false,
        };
        let drawer = app.settings_drawer().unwrap();
        let mut element = cosmic::widget::ContextDrawer::new_inner_overlay(
            drawer.title,
            drawer.actions,
            drawer.header,
            drawer.footer,
            drawer.content,
            drawer.on_close,
            344.,
            false,
        );
        let mut tree = Tree::new(element.as_widget());
        // The drawer's available pane inside the 960x400 native shell.
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(344., 352.)),
        );
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut operation);
        let input = operation.input.expect("context-size input remains present");
        let viewport = operation
            .viewport
            .expect("native drawer has a scroll viewport");
        assert!(input.y >= viewport.y);
        assert!(
            input.y + input.height <= viewport.y + viewport.height,
            "first input {input:?} must fit inside {viewport:?}"
        );
        assert!(
            operation.pending,
            "saving must still explain deferred application"
        );
    }

    #[test]
    fn escape_cancels_editor_confirmations_preserving_values_and_window_lifetime() {
        use cosmic::Application;
        for confirmation in ["drawer-close", "reload", "window-close"] {
            let mut app = Applet::default();
            let _ = app.open_editor("example".into());
            let id = app.active_editor.unwrap();
            let _ = app.editor_event(Event::Loaded(id, 1, Ok(PresetDraft::default())));
            let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "8192".into()));
            let original = app.editors[&id].draft.as_ref().unwrap().options.clone();
            match confirmation {
                "drawer-close" => {
                    let _ = app.editor_event(Event::Close(id));
                }
                "reload" => {
                    let _ = app.editor_event(Event::Reload(id));
                }
                _ => {
                    let _ = app.update(Message::CloseManager);
                }
            }
            assert!(app.editor_confirmation().is_some());
            app.shared_operation = Some("Accepted operation".into());
            let task = app.on_escape();
            assert_eq!(task.units(), 0);
            assert!(app.editor_confirmation().is_none());
            assert!(!app.pending_close);
            assert_eq!(app.editors[&id].draft.as_ref().unwrap().options, original);
            assert!(app.editors[&id].dirty());
            assert_eq!(app.shared_operation.as_deref(), Some("Accepted operation"));
            let _ = app.editor_event(Event::Close(id));
            let task = app.editor_event(Event::Discard(id));
            assert_eq!(task.units(), 0, "a later drawer discard must not exit");
        }
    }

    #[test]
    fn reverting_one_draft_restores_inheritance_without_changing_other_drafts_or_work() {
        let mut app = Applet::default();
        let mut ids = Vec::new();
        for model in ["first", "competitor"] {
            let _ = app.open_editor(model.into());
            let id = app.active_editor.unwrap();
            let _ = app.editor_event(Event::Loaded(
                id,
                1,
                Ok(PresetDraft {
                    id: model.into(),
                    revision: "original".into(),
                    defaults: BTreeMap::from([("ctx-size".into(), "8192".into())]),
                    options: BTreeMap::from([("fit".into(), "off".into())]),
                }),
            ));
            let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "32768".into()));
            ids.push(id);
        }
        let competitor = app.editors[&ids[1]].draft.clone().unwrap();
        app.shared_operation = Some("Accepted operation".into());
        let task = app.editor_event(Event::Revert(ids[0]));
        assert_eq!(task.units(), 0);
        assert!(!app.editors[&ids[0]].dirty());
        let restored = app.editors[&ids[0]].draft.as_ref().unwrap();
        assert!(!restored.options.contains_key("ctx-size"));
        assert_eq!(restored.defaults["ctx-size"], "8192");
        assert_eq!(restored.options["fit"], "off");
        assert_eq!(restored.revision, "original");
        assert_eq!(
            app.editors[&ids[1]].draft.as_ref().unwrap().options,
            competitor.options
        );
        assert_eq!(app.active_editor, Some(ids[1]));
        assert_eq!(app.shared_operation.as_deref(), Some("Accepted operation"));
        // A queued discard cannot alter an in-flight save.
        app.editors.get_mut(&ids[1]).unwrap().saving = true;
        let _ = app.editor_event(Event::Revert(ids[1]));
        assert_eq!(
            app.editors[&ids[1]].draft.as_ref().unwrap().options,
            competitor.options
        );
    }

    #[tokio::test]
    async fn window_close_discards_each_dirty_draft_then_exits_without_cancelling_work() {
        use cosmic::Application;
        use cosmic::iced::{futures::StreamExt, runtime};
        let mut app = Applet::default();
        for model in ["first", "second", "saving"] {
            let _ = app.open_editor(model.into());
            let id = app.active_editor.unwrap();
            let _ = app.editor_event(Event::Loaded(
                id,
                1,
                Ok(PresetDraft {
                    id: model.into(),
                    ..Default::default()
                }),
            ));
            let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "8192".into()));
            if model == "saving" {
                app.editors.get_mut(&id).unwrap().saving = true;
            }
        }
        app.busy = true;
        app.operation_label = Some("Accepted operation".into());
        let _ = app.update(Message::CloseManager);
        let first = app.active_editor.unwrap();
        assert!(app.pending_close);
        assert!(app.editors[&first].discard);
        let task = app.editor_event(Event::Discard(first));
        assert_eq!(task.units(), 0);
        let second = app.active_editor.unwrap();
        assert_ne!(first, second);
        assert!(app.pending_close);
        assert!(app.editors[&second].discard);
        let task = app.editor_event(Event::Discard(second));
        assert!(!app.pending_close);
        assert_eq!(app.editors.len(), 1);
        assert!(app.editors.values().all(|e| e.saving && e.dirty()));
        assert!(app.busy);
        assert_eq!(app.operation_label.as_deref(), Some("Accepted operation"));
        let mut actions = runtime::task::into_stream(task).unwrap();
        assert!(matches!(actions.next().await, Some(runtime::Action::Exit)));
    }

    #[test]
    fn cancelling_window_close_keeps_drafts_and_drawer_discard_does_not_exit() {
        use cosmic::Application;
        let mut app = Applet::default();
        let _ = app.open_editor("example".into());
        let id = app.active_editor.unwrap();
        let _ = app.editor_event(Event::Loaded(id, 1, Ok(PresetDraft::default())));
        let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "8192".into()));
        let _ = app.update(Message::CloseManager);
        let _ = app.editor_event(Event::KeepDraft(id));
        assert!(!app.pending_close);
        assert!(app.editors[&id].dirty());
        assert!(!app.editors[&id].discard);
        let _ = app.editor_event(Event::Close(id));
        let task = app.editor_event(Event::Discard(id));
        assert_eq!(task.units(), 0);
        assert!(app.editors.is_empty());
    }

    #[test]
    fn opening_drawer_resolves_condensed_navigation_conflict() {
        let mut app = Applet::default();
        app.core.window.context_is_overlay = false;
        app.core.nav_bar_set_toggled(true);
        app.core.set_show_context(false);
        assert!(app.core.is_condensed());
        assert!(app.core.nav_bar_active());
        let _ = app.open_editor("example".into());
        assert!(app.core.window.show_context);
        assert!(!app.core.nav_bar_active());
        app.select_page(super::super::navigation::Page::Search);
        assert!(!app.core.window.show_context);
        app.select_page(super::super::navigation::Page::Models);
        assert!(app.core.window.show_context);
        assert!(!app.core.nav_bar_active());
    }

    #[tokio::test]
    async fn expanded_drawer_layout_has_finite_bounds() {
        use cosmic::iced::advanced::{layout, renderer::Headless, widget::Tree};
        let mut renderer = <cosmic::Renderer as Headless>::new(
            cosmic::iced::Font::default(),
            14.into(),
            Some("tiny-skia"),
        )
        .await
        .expect("CPU renderer supports headless layout");
        let mut app = Applet::default();
        let _ = app.open_editor("layout-fixture".into());
        let id = app.active_editor.unwrap();
        let draft = PresetDraft {
            id: "layout-fixture".into(),
            options: BTreeMap::from([
                (
                    "model".into(),
                    "/models/an intentionally long directory/model-file.gguf".into(),
                ),
                ("ctx-size".into(), "131072".into()),
                ("fit".into(), "off".into()),
                ("n-cpu-moe".into(), "20".into()),
                ("jinja".into(), "true".into()),
                ("cache-type-k".into(), "q4_0".into()),
            ]),
            ..Default::default()
        };
        let _ = app.editor_event(Event::Loaded(id, 1, Ok(draft)));
        app.editors.get_mut(&id).unwrap().expanded.fill(true);
        for width in [300., 400., 600.] {
            let mut element = app.editor_content(id);
            let mut tree = Tree::new(element.as_widget());
            let limits = layout::Limits::new(
                cosmic::iced::Size::ZERO,
                cosmic::iced::Size::new(width, f32::INFINITY),
            );
            let start = std::time::Instant::now();
            for _ in 0..30 {
                let layout = element
                    .as_widget_mut()
                    .layout(&mut tree, &renderer, &limits);
                assert!(layout.size().height.is_finite());
                assert!(layout.size().width <= width + 1.);
            }
            eprintln!(
                "All expanded settings: width={width}, 30 layouts={:?}",
                start.elapsed()
            );
        }
        // This test deliberately uses CPU fallback for deterministic, GPU-free CI.
        // The production type must still include the primary GPU renderer.
        assert!(matches!(renderer, cosmic::Renderer::Secondary(_)));
        use cosmic::iced::advanced::{Layout, Renderer as _, mouse, renderer};
        let mut element = app.editor_content(id);
        let mut tree = Tree::new(element.as_widget());
        let size = cosmic::iced::Size::new(400., 700.);
        let limits = layout::Limits::new(
            cosmic::iced::Size::ZERO,
            cosmic::iced::Size::new(400., f32::INFINITY),
        );
        let node = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        let viewport = cosmic::iced::Rectangle::with_size(size);
        let start = std::time::Instant::now();
        for _ in 0..10 {
            renderer.reset(viewport);
            element.as_widget().draw(
                &tree,
                &mut renderer,
                &cosmic::theme::active(),
                &renderer::Style::default(),
                Layout::new(&node),
                mouse::Cursor::Unavailable,
                &viewport,
            );
            let pixels = renderer.screenshot(
                cosmic::iced::Size::new(400, 700),
                1.,
                cosmic::iced::Color::BLACK,
            );
            assert_eq!(pixels.len(), 400 * 700 * 4);
        }
        eprintln!(
            "CPU fallback: 10 clipped settings rasters={:?}",
            start.elapsed()
        );
    }
    #[test]
    fn form_covers_every_supported_option_and_preserves_inherited_values() {
        let keys: std::collections::BTreeSet<_> =
            GROUPS.iter().flat_map(|(_, f)| f.iter().copied()).collect();
        assert_eq!(keys, herd::config::OPTIONS.iter().copied().collect());
        assert_eq!(
            keys.len(),
            GROUPS.iter().map(|(_, fields)| fields.len()).sum::<usize>()
        );
        let mut e = Editor::new("example".into());
        let draft = PresetDraft {
            id: "example".into(),
            revision: "revision".into(),
            options: BTreeMap::from([
                ("n-cpu-moe".into(), "20".into()),
                ("fit".into(), "off".into()),
            ]),
            defaults: BTreeMap::from([("ctx-size".into(), "131072".into())]),
        };
        e.loaded(0, Ok(draft));
        assert!(!e.dirty());
        assert!(!e.draft.as_ref().unwrap().options.contains_key("ctx-size"));
        e.loaded(99, Err("stale".into()));
        assert!(e.error.is_none());
        assert_eq!(e.draft.as_ref().unwrap().options["fit"], "off");
    }
    #[test]
    fn reload_preserves_dirty_draft_until_confirmed_and_ignores_stale_results() {
        let mut app = Applet::default();
        let _ = app.open_editor("example".into());
        let id = *app.editors.keys().next().unwrap();
        let original = PresetDraft {
            id: "example".into(),
            options: BTreeMap::from([("ctx-size".into(), "8192".into())]),
            ..Default::default()
        };
        let _ = app.editor_event(Event::Loaded(id, 1, Ok(original.clone())));
        let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "32768".into()));
        let _ = app.editor_event(Event::Reload(id));
        assert!(app.editors[&id].reload_confirmation);
        assert_eq!(
            app.editors[&id].draft.as_ref().unwrap().options["ctx-size"],
            "32768"
        );
        let _ = app.editor_content(id);
        let _ = app.editor_event(Event::KeepDraft(id));
        assert!(app.editors[&id].dirty());
        let _ = app.editor_event(Event::ConfirmReload(id));
        let _ = app.editor_event(Event::Change(id, "ctx-size".into(), "65536".into()));
        assert_eq!(
            app.editors[&id].draft.as_ref().unwrap().options["ctx-size"],
            "32768"
        );
        let _ = app.editor_event(Event::Loaded(id, 1, Ok(original.clone())));
        assert!(app.editors[&id].loading);
        let _ = app.editor_event(Event::Loaded(id, 2, Ok(original)));
        assert!(!app.editors[&id].dirty());
        let _ = app.editor_content(id);
    }
    #[test]
    fn drawer_preserves_drafts_across_navigation_and_requires_discard_on_close() {
        use super::super::navigation::Page;
        let mut app = Applet::default();
        let _ = app.open_editor("first".into());
        let first = app.active_editor.unwrap();
        let draft = PresetDraft {
            id: "first".into(),
            options: BTreeMap::from([("n-cpu-moe".into(), "20".into())]),
            ..Default::default()
        };
        let _ = app.editor_event(Event::Loaded(first, 1, Ok(draft)));
        let input = app.editors[&first].field_ids["n-cpu-moe"].clone();
        let _ = app.editor_event(Event::Change(first, "fit".into(), "off".into()));
        app.select_page(Page::Search);
        assert!(!app.core.window.show_context);
        let _ = app.open_editor("second".into());
        let second = app.active_editor.unwrap();
        assert_ne!(first, second);
        let _ = app.open_editor("first".into());
        assert_eq!(app.active_editor, Some(first));
        assert!(app.editors[&first].dirty());
        assert_eq!(app.editors[&first].field_ids["n-cpu-moe"], input);
        let _ = app.editor_event(Event::Close(first));
        assert!(app.editor_confirmation().is_some());
        let _ = app.editor_event(Event::KeepDraft(first));
        assert!(app.editors[&first].dirty());
        let _ = app.editor_event(Event::Close(first));
        let _ = app.editor_event(Event::Discard(first));
        assert!(!app.editors.contains_key(&first));
        let _ = app.open_editor("second".into());
        app.busy = true;
        let _ = app.editor_event(Event::Loaded(first, 1, Ok(Default::default())));
        let _ = app.editor_event(Event::Saved(first, Ok(Default::default())));
        assert_eq!(app.active_editor, Some(second));
        assert!(app.busy);
    }
    #[test]
    fn cached_fields_ignore_unrelated_edits_but_track_value_and_inheritance() {
        let mut e = Editor::new("example".into());
        let id = Id::unique();
        e.loaded(
            0,
            Ok(PresetDraft {
                defaults: BTreeMap::from([("ctx-size".into(), "8192".into())]),
                ..Default::default()
            }),
        );
        let field = e.field(id, "ctx-size", true).unwrap();
        e.draft
            .as_mut()
            .unwrap()
            .options
            .insert("fit".into(), "off".into());
        assert!(field == e.field(id, "ctx-size", true).unwrap());
        e.draft
            .as_mut()
            .unwrap()
            .options
            .insert("ctx-size".into(), "8192".into());
        assert!(field != e.field(id, "ctx-size", true).unwrap());
        assert!(GROUPS.iter().all(|(_, fields)| fields.len() <= 9));
        assert_eq!(field.input, e.field_ids["ctx-size"]);
        e.saving = true;
        let mut app = Applet::default();
        app.editors.insert(id, e);
        app.active_editor = Some(id);
        app.busy = true;
        let _ = app.editor_event(Event::Close(id));
        assert!(app.active_editor.is_none());
        assert!(app.editors[&id].saving);
        let _ = app.editor_event(Event::Saved(id, Ok(Default::default())));
        assert!(!app.busy);
    }

    #[test]
    fn parameter_sections_expand_independently_without_changing_values() {
        let mut app = Applet::default();
        let _ = app.open_editor("example".into());
        let id = app.active_editor.unwrap();
        let draft = PresetDraft {
            id: "example".into(),
            options: BTreeMap::from([
                ("n-cpu-moe".into(), "20".into()),
                ("fit".into(), "off".into()),
                ("model".into(), "/models/a long path/model.gguf".into()),
            ]),
            ..Default::default()
        };
        let expected = draft.options.clone();
        let _ = app.editor_event(Event::Loaded(id, 1, Ok(draft)));
        assert!(app.editors[&id].expanded[0]);
        assert!(app.editors[&id].expanded[1]);
        let _ = app.settings_drawer();
        for group in 0..GROUPS.len() {
            if !app.editors[&id].expanded[group] {
                let _ = app.editor_event(Event::ToggleGroup(id, group));
            }
            assert!(app.editors[&id].expanded[group]);
            let _ = app.settings_drawer();
        }
        assert!(app.editors[&id].expanded.iter().all(|expanded| *expanded));
        let _ = app.editor_event(Event::ToggleGroup(id, 0));
        assert!(!app.editors[&id].expanded[0]);
        assert!(app.editors[&id].expanded[1]);
        assert_eq!(app.editors[&id].draft.as_ref().unwrap().options, expected);
        assert!(!app.editors[&id].dirty());
        let expanded = app.editors[&id].expanded;
        let _ = app.editor_event(Event::ToggleGroup(id, usize::MAX));
        assert_eq!(app.editors[&id].expanded, expanded);
    }
}
