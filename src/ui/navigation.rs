//! Standard COSMIC navigation; settings are a native context drawer, not windows.
use super::localization::tr;
use super::{Applet, Message};
use cosmic::{Element, iced::Length, theme, widget};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Models,
    Search,
    Downloads,
    Import,
    Setup,
}

pub(super) struct Navigation {
    pub model: widget::nav_bar::Model,
    pub page: Page,
    pub query: String,
    pub reverse: bool,
}
impl Default for Navigation {
    fn default() -> Self {
        let mut model = widget::nav_bar::Model::default();
        for (page, label, icon) in [
            (Page::Models, "models", "view-grid-symbolic"),
            (Page::Search, "find-models", "system-search-symbolic"),
            (Page::Downloads, "downloads", "folder-download-symbolic"),
        ] {
            let id = model
                .insert()
                .text(tr(label))
                .icon(widget::icon::from_name(icon))
                .data(page)
                .id();
            if page == Page::Models {
                model.activate(id);
            }
        }
        Self {
            model,
            page: Page::Models,
            query: String::new(),
            reverse: false,
        }
    }
}
impl Applet {
    pub(super) fn select_page(&mut self, page: Page) {
        if self.navigation.page != page {
            self.downloads.cancel_review();
            self.cancel_removal_review();
        }
        self.navigation.page = page;
        self.core
            .set_show_context(page == Page::Models && self.active_editor.is_some());
        let id = self
            .navigation
            .model
            .iter()
            .find(|id| self.navigation.model.data::<Page>(*id) == Some(&page));
        if let Some(id) = id {
            self.navigation.model.activate(id);
        } else {
            self.navigation.model.deactivate();
        }
    }
    pub(super) fn page_view(&self) -> Element<'_, Message> {
        if matches!(self.navigation.page, Page::Models | Page::Downloads) {
            return self.manager_content();
        }
        let space = theme::spacing();
        let mut content = widget::column::with_capacity(5)
            .spacing(space.space_m)
            .push(widget::text::title3(tr(match self.navigation.page {
                Page::Search => "find-models",
                Page::Import => "import-model",
                _ => "setup-title",
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
        if self.setup_required || self.navigation.page == Page::Setup {
            content = content.push(self.setup_view());
        } else {
            if let Some(error) = &self.discovery_error {
                content = content.push(super::view::notice(tr("discovery-error"), error));
            }
            content = content.push(match self.navigation.page {
                Page::Search => self.download_browser(),
                _ => self.import_view(),
            });
        }
        widget::scrollable(
            widget::container(
                widget::container(content)
                    .max_width(800)
                    .width(Length::Fill),
            )
            .center_x(Length::Fill)
            .padding(space.space_m),
        )
        .height(Length::Fill)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;
    #[test]
    fn only_backend_setup_status_selects_first_run_form() {
        use herd::backend::protocol::Report;
        let mut app = Applet::default();
        let _ = app.update(Message::Refreshed(Ok(Report {
            setup_required: true,
            error: "Hosting preferences are absent".into(),
            ..Default::default()
        })));
        assert!(app.setup_required);
        let _ = app.update(Message::ShowSetup);
        assert_eq!(app.navigation.page, Page::Setup);
        let _ = app.page_view();
        let _ = app.manager_content();
        let diagnostic = "Native presets are unreadable";
        let _ = app.update(Message::Refreshed(Ok(Report {
            setup_required: false,
            error: diagnostic.into(),
            ..Default::default()
        })));
        assert!(!app.setup_required);
        assert!(app.snapshots.is_empty());
        assert_eq!(app.discovery_error.as_deref(), Some(diagnostic));
        let _ = app.page_view();
        let _ = app.manager_content();
        let _ = app.update(Message::Refreshed(Ok(Report::default())));
        assert!(!app.setup_required);
        assert!(app.discovery_error.is_none());
        let _ = app.page_view();
    }

    #[test]
    fn navigation_preserves_pending_operations_and_drafts() {
        let mut app = Applet {
            busy: true,
            ..Default::default()
        };
        app.select_page(Page::Search);
        assert_eq!(app.navigation.page, Page::Search);
        let _ = app.page_view();
        app.select_page(Page::Import);
        let _ = app.page_view();
        assert!(app.busy);
        let _ = app.update(Message::WindowClosed(cosmic::iced::window::Id::unique()));
        assert!(app.busy);
    }
}
