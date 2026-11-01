//! Concrete Colibri native settings drawer with independent drafts and close guards.
use super::{Applet, Message, localization::tr};
use cosmic::{Element, iced::Length, prelude::*, widget};
use herd::engines::colibri::settings::{self, Draft};

pub(super) struct Editor {
    pub original: Draft,
    draft: Draft,
    pub stale: bool,
    pub saving: bool,
    pub confirm_close: bool,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;
    fn editor() -> Editor {
        let draft = Draft {
            revision: "a".repeat(64),
            arguments: std::collections::BTreeMap::from([("--ctx".into(), "4096".into())]),
            flags: Vec::new(),
            environment: std::collections::BTreeMap::new(),
        };
        Editor {
            original: draft.clone(),
            draft,
            stale: false,
            saving: false,
            confirm_close: false,
            error: None,
        }
    }
    #[test]
    fn edits_survive_engine_switches_dirty_close_cancel_and_escape_without_changing_accepted_work()
    {
        let mut app = Applet {
            mode: super::super::Mode::Management,
            engine: 1,
            colibri_editor: Some(editor()),
            shared_operation: Some("accepted-operation".into()),
            ..Default::default()
        };
        let _ = app.update(Message::ColibriEditor(Event::Argument(
            "--ctx".into(),
            "8192".into(),
        )));
        assert!(app.colibri_editor.as_ref().unwrap().dirty());
        let _ = app.update(Message::EngineSelected(0));
        let _ = app.update(Message::EngineSelected(1));
        assert_eq!(
            app.colibri_editor.as_ref().unwrap().draft.arguments["--ctx"],
            "8192"
        );
        let _ = app.update(Message::CloseManager);
        assert!(app.pending_close);
        assert!(app.colibri_editor.as_ref().unwrap().confirm_close);
        let _ = app.update(Message::ColibriEditor(Event::Cancel));
        assert!(!app.pending_close);
        assert!(app.colibri_editor.as_ref().unwrap().dirty());
        let _ = app.update(Message::ColibriEditor(Event::Close));
        let _ = app.on_escape();
        assert!(!app.colibri_editor.as_ref().unwrap().confirm_close);
        assert!(app.colibri_editor.as_ref().unwrap().dirty());
        assert_eq!(app.shared_operation.as_deref(), Some("accepted-operation"));
        let _ = app.colibri_drawer();
    }
    #[test]
    fn late_native_reads_do_not_overwrite_new_edits_or_reopen_after_engine_selection() {
        let mut app = Applet {
            engine: 1,
            colibri_editor: Some(editor()),
            colibri_editor_generation: 4,
            ..Default::default()
        };
        let original = app.colibri_editor.as_ref().unwrap().original.clone();
        let _ = app.update(Message::ColibriEditor(Event::Argument(
            "--ctx".into(),
            "8192".into(),
        )));
        let _ = app.update(Message::ColibriEditor(Event::Loaded(
            4,
            Ok(original.clone()),
        )));
        assert_eq!(
            app.colibri_editor.as_ref().unwrap().draft.arguments["--ctx"],
            "8192"
        );
        let ticket = app.colibri_editor_generation;
        let _ = app.update(Message::EngineSelected(0));
        let _ = app.update(Message::ColibriEditor(Event::Loaded(ticket, Ok(original))));
        assert_eq!(app.engine, 0);
    }
}
impl Editor {
    pub fn dirty(&self) -> bool {
        self.original != self.draft
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Open,
    Loaded(u64, Result<Draft, String>),
    Argument(String, String),
    Environment(String, String),
    Flag(String, bool),
    Save,
    Saved(u64, Result<Draft, String>),
    Close,
    Discard,
    Cancel,
}

impl Applet {
    pub(super) fn colibri_drawer(
        &self,
    ) -> Option<cosmic::app::context_drawer::ContextDrawer<'_, Message>> {
        let editor = self.colibri_editor.as_ref()?;
        let mut content = widget::column::with_capacity(16)
            .spacing(12)
            .push(widget::text::body(tr("colibri-settings-help")));
        if let Some(error) = &editor.error {
            content = content.push(super::view::notice(tr("action-unavailable"), error));
        }
        if editor.stale {
            content = content.push(widget::text::body(tr("colibri-settings-stale")));
        }
        for (title, keys) in [
            ("group-context", &["--ctx", "--kv-slots", "--ngen"][..]),
            (
                "group-gpu",
                &["--gpu", "--vram", "--ram", "--cap", "--repin", "--policy"][..],
            ),
            (
                "colibri-sampling",
                &["--temp", "--topp", "--topk", "--effort"][..],
            ),
            ("colibri-queue", &["--max-queue", "--queue-timeout"][..]),
        ] {
            let mut section = widget::column::with_capacity(keys.len() + 1)
                .spacing(8)
                .push(widget::text::title4(tr(title)));
            for key in keys {
                let value = editor
                    .draft
                    .arguments
                    .get(*key)
                    .map(String::as_str)
                    .unwrap_or("");
                let field = key.to_string();
                section =
                    section.push(
                        widget::column::with_capacity(2)
                            .spacing(4)
                            .push(widget::text::body(*key))
                            .push(widget::text_input(tr("native-default"), value).on_input(
                                move |v| Message::ColibriEditor(Event::Argument(field.clone(), v)),
                            )),
                    );
            }
            content = content.push(section);
        }
        content = content.push(widget::text::title4(tr("colibri-native-switches")));
        for flag in settings::FLAGS {
            let key = flag.to_string();
            content = content.push(
                widget::checkbox(editor.draft.flags.iter().any(|f| f == flag))
                    .label(*flag)
                    .on_toggle(move |enabled| {
                        Message::ColibriEditor(Event::Flag(key.clone(), enabled))
                    }),
            );
        }
        content = content.push(widget::text::title4(tr("colibri-native-environment")));
        for key in settings::ENVIRONMENT {
            let field = key.to_string();
            let value = editor
                .draft
                .environment
                .get(*key)
                .map(String::as_str)
                .unwrap_or("");
            content = content.push(
                widget::column::with_capacity(2)
                    .spacing(4)
                    .push(widget::text::body(*key))
                    .push(
                        widget::text_input(tr("native-default"), value).on_input(move |v| {
                            Message::ColibriEditor(Event::Environment(field.clone(), v))
                        }),
                    ),
            );
        }
        content = content.push(
            widget::row::with_capacity(2)
                .spacing(8)
                .push(
                    widget::button::suggested(tr("save")).on_press_maybe(
                        (editor.dirty() && !editor.saving && !self.busy && !editor.stale)
                            .then_some(Message::ColibriEditor(Event::Save)),
                    ),
                )
                .push(
                    widget::button::text(tr("reload-settings")).on_press_maybe(
                        (!editor.dirty() && !editor.saving)
                            .then_some(Message::ColibriEditor(Event::Open)),
                    ),
                ),
        );
        Some(
            cosmic::app::context_drawer::context_drawer(
                widget::scrollable(content).height(Length::Fill),
                Message::ColibriEditor(Event::Close),
            )
            .title(tr("settings")),
        )
    }

    pub(super) fn colibri_editor_confirmation(&self) -> Option<Element<'_, Message>> {
        self.colibri_editor.as_ref().filter(|e| e.confirm_close)?;
        Some(
            widget::dialog()
                .title(tr("colibri-discard"))
                .primary_action(
                    widget::button::destructive(tr("discard"))
                        .on_press(Message::ColibriEditor(Event::Discard)),
                )
                .secondary_action(
                    widget::button::text(tr("cancel"))
                        .on_press(Message::ColibriEditor(Event::Cancel)),
                )
                .into(),
        )
    }

    pub(super) fn colibri_editor_event(&mut self, event: Event) -> Task<cosmic::Action<Message>> {
        match event {
            Event::Open => {
                if self
                    .colibri_editor
                    .as_ref()
                    .is_some_and(|e| e.dirty() || e.saving)
                {
                    self.core.set_show_context(true);
                    return Task::none();
                }
                let Some(client) = self.controller.clone() else {
                    return Task::none();
                };
                self.colibri_editor_generation = self.colibri_editor_generation.wrapping_add(1);
                let ticket = self.colibri_editor_generation;
                return Task::perform(
                    async move { client.colibri_settings().await.map_err(|e| e.to_string()) },
                    move |r| cosmic::Action::App(Message::ColibriEditor(Event::Loaded(ticket, r))),
                );
            }
            Event::Loaded(ticket, result) if ticket == self.colibri_editor_generation => {
                match result {
                    Ok(draft) => {
                        self.colibri_editor = Some(Editor {
                            original: draft.clone(),
                            draft,
                            stale: false,
                            saving: false,
                            confirm_close: false,
                            error: None,
                        });
                        self.engine = 1;
                        self.core.set_show_context(true);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Event::Argument(key, value) | Event::Environment(key, value) => {
                let Some(editor) = &mut self.colibri_editor else {
                    return Task::none();
                };
                if editor.saving {
                    return Task::none();
                }
                self.colibri_editor_generation = self.colibri_editor_generation.wrapping_add(1);
                let map = if key.starts_with("--") {
                    &mut editor.draft.arguments
                } else {
                    &mut editor.draft.environment
                };
                if value.is_empty() {
                    map.remove(&key);
                } else {
                    map.insert(key, value);
                }
            }
            Event::Flag(key, enabled) => {
                if self.colibri_editor.as_ref().is_some_and(|e| !e.saving) {
                    self.colibri_editor_generation = self.colibri_editor_generation.wrapping_add(1);
                }
                if let Some(editor) = &mut self.colibri_editor
                    && !editor.saving
                {
                    editor.draft.flags.retain(|f| {
                        f != &key
                            && !(enabled
                                && matches!(key.as_str(), "--think" | "--no-think")
                                && matches!(f.as_str(), "--think" | "--no-think"))
                    });
                    if enabled {
                        editor.draft.flags.push(key);
                        editor.draft.flags.sort();
                    }
                }
            }
            Event::Save if !self.busy => {
                let (Some(editor), Some(client)) =
                    (&mut self.colibri_editor, self.controller.clone())
                else {
                    return Task::none();
                };
                if editor.saving || editor.stale {
                    return Task::none();
                }
                editor.saving = true;
                editor.error = None;
                let draft = editor.draft.clone();
                let ticket = self.colibri_editor_generation;
                self.busy = true;
                self.operation_label = Some(tr("working"));
                return Task::perform(
                    async move {
                        let id = client
                            .save_colibri_settings(&draft)
                            .await
                            .map_err(|e| e.to_string())?;
                        client.wait(&id).await.map_err(|e| e.to_string())?;
                        client.colibri_settings().await.map_err(|e| e.to_string())
                    },
                    move |r| cosmic::Action::App(Message::ColibriEditor(Event::Saved(ticket, r))),
                );
            }
            Event::Saved(ticket, result) if ticket == self.colibri_editor_generation => {
                self.busy = false;
                self.operation_label = None;
                if let Some(editor) = &mut self.colibri_editor {
                    editor.saving = false;
                    match result {
                        Ok(draft) => {
                            editor.original = draft.clone();
                            editor.draft = draft;
                            editor.stale = false;
                        }
                        Err(error) => editor.error = Some(error),
                    }
                }
                return self.refresh();
            }
            Event::Close => {
                if let Some(editor) = &mut self.colibri_editor
                    && editor.dirty()
                    && !editor.saving
                {
                    editor.confirm_close = true;
                    self.core.set_show_context(true);
                } else {
                    self.core.set_show_context(false);
                }
            }
            Event::Discard => {
                self.colibri_editor = None;
                self.colibri_editor_generation = self.colibri_editor_generation.wrapping_add(1);
                self.core.set_show_context(false);
                if self.pending_close {
                    return self.close_manager();
                }
            }
            Event::Cancel => {
                if let Some(editor) = &mut self.colibri_editor {
                    editor.confirm_close = false;
                }
                self.pending_close = false;
            }
            _ => {}
        }
        Task::none()
    }
}
