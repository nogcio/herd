//! Public metadata browsing and explicit native-transfer presentation.
use super::localization::{arg, tr};
use super::{Applet, Message};
use cosmic::{
    Element,
    iced::{Alignment, Length},
    theme, widget,
};
use herd::downloads::{Catalogue, Plan};

fn size_label(bytes: u64) -> String {
    let (unit, divisor) = if bytes >= 1024 * 1024 * 1024 {
        ("gib", 1073741824.)
    } else {
        ("mib", 1048576.)
    };
    format!("{:.1} {}", bytes as f64 / divisor, tr(unit))
}

#[derive(Default)]
pub(super) struct Browser {
    pub query: String,
    pub results: Vec<String>,
    pub catalogue: Option<Catalogue>,
    pub selected: Option<Plan>,
    pub ticket: u64,
    pub busy: bool,
    pub error: Option<String>,
    pub repo: Option<String>,
    pub searched: bool,
    review_generation: u64,
    review: Option<ResumeReview>,
}

struct ResumeReview {
    ticket: u64,
    busy: bool,
    error: Option<String>,
}

#[derive(Debug, PartialEq)]
enum BrowserFeedback<'a> {
    SearchError(&'a str),
    RepositoryError(&'a str),
    FindModel,
    NoResults,
}

impl Browser {
    fn feedback(&self) -> Option<BrowserFeedback<'_>> {
        if let Some(error) = self.error.as_deref() {
            return Some(if self.repo.is_some() {
                BrowserFeedback::RepositoryError(error)
            } else {
                BrowserFeedback::SearchError(error)
            });
        }
        if self.repo.is_some() || self.busy || !self.results.is_empty() {
            None
        } else if self.searched {
            Some(BrowserFeedback::NoResults)
        } else {
            Some(BrowserFeedback::FindModel)
        }
    }

    pub fn request_review(&mut self) -> u64 {
        self.review_generation = self.review_generation.wrapping_add(1);
        self.review = Some(ResumeReview {
            ticket: self.review_generation,
            busy: true,
            error: None,
        });
        self.selected = None;
        self.review_generation
    }
    pub fn cancel_review(&mut self) {
        if self.review.take().is_some() {
            self.selected = None;
        }
    }
    pub fn request(&mut self) -> u64 {
        self.ticket = self.ticket.wrapping_add(1);
        self.busy = true;
        self.error = None;
        self.selected = None;
        self.catalogue = None;
        self.ticket
    }
    pub fn change_query(&mut self, query: String) {
        self.query = query;
        self.ticket = self.ticket.wrapping_add(1);
        self.busy = false;
        self.results.clear();
        self.catalogue = None;
        self.selected = None;
        self.error = None;
        self.repo = None;
        self.searched = false;
    }
    pub fn search_result(&mut self, ticket: u64, result: Result<Vec<String>, String>) {
        if ticket != self.ticket {
            return;
        }
        self.busy = false;
        match result {
            Ok(repos) => self.results = repos,
            Err(error) => self.error = Some(error),
        }
    }
    pub fn catalogue_result(&mut self, ticket: u64, result: Result<Catalogue, String>) {
        if ticket != self.ticket {
            return;
        }
        self.busy = false;
        match result {
            Ok(catalogue) => {
                self.catalogue = Some(catalogue);
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn plan_result(&mut self, ticket: u64, result: Result<Plan, String>) {
        let Some(review) = &mut self.review else {
            return;
        };
        if ticket != review.ticket {
            return;
        }
        review.busy = false;
        match result {
            Ok(plan) => self.selected = Some(plan),
            Err(error) => review.error = Some(error),
        }
    }
}

impl Applet {
    pub(super) fn resume_feedback(&self) -> Option<Element<'_, Message>> {
        let review = self.downloads.review.as_ref()?;
        if review.busy {
            Some(widget::progress_bar::indeterminate_linear().into())
        } else {
            review
                .error
                .as_ref()
                .map(|error| super::view::notice(tr("download-error"), error))
        }
    }

    pub(super) fn download_browser(&self) -> Element<'_, Message> {
        let active = theme::active();
        let space = active.cosmic();
        let browser = &self.downloads;
        let enabled = !browser.busy && !self.busy;
        let mut content = widget::column::with_capacity(8).spacing(space.space_m());
        if let Some(repo) = &browser.repo {
            content = content.push(
                widget::row::with_capacity(2)
                    .spacing(space.space_s())
                    .align_y(Alignment::Center)
                    .push(
                        widget::button::icon(widget::icon::from_name("go-previous-symbolic"))
                            .name(tr("back-to-search"))
                            .on_press(Message::DownloadBack),
                    )
                    .push(widget::text::title4(repo).width(Length::Fill)),
            );
        } else {
            content = content.push(
                widget::row::with_capacity(2)
                    .spacing(space.space_s())
                    .align_y(Alignment::Center)
                    .push(
                        widget::search_input(tr("search-placeholder"), &browser.query)
                            .on_input(Message::DownloadQuery)
                            .on_clear(Message::DownloadQuery(String::new()))
                            .on_submit_maybe(
                                (enabled && !browser.query.trim().is_empty())
                                    .then_some(|_| Message::SearchDownloads),
                            ),
                    )
                    .push(
                        widget::button::suggested(tr("search")).on_press_maybe(
                            (enabled && !browser.query.trim().is_empty())
                                .then_some(Message::SearchDownloads),
                        ),
                    ),
            );
        }
        if browser.busy {
            content = content.push(widget::progress_bar::indeterminate_linear());
        }
        match browser.feedback() {
            Some(BrowserFeedback::SearchError(error)) => {
                content = content.push(super::view::notice(tr("search-error"), error));
            }
            Some(BrowserFeedback::RepositoryError(error)) => {
                content = content.push(super::view::notice(tr("repository-error"), error));
            }
            Some(prompt @ (BrowserFeedback::FindModel | BrowserFeedback::NoResults)) => {
                content = content.push(
                    widget::container(
                        widget::column::with_capacity(2)
                            .spacing(space.space_s())
                            .align_x(Alignment::Center)
                            .push(widget::icon::from_name("system-search-symbolic").size(48))
                            .push(widget::text::heading(tr(match prompt {
                                BrowserFeedback::NoResults => "no-search-results",
                                _ => "find-model",
                            }))),
                    )
                    .center_x(Length::Fill)
                    .padding(space.space_l()),
                );
            }
            None => {}
        }
        if browser.repo.is_none() && !browser.results.is_empty() {
            let mut results = widget::settings::section();
            for repo in &browser.results {
                results = results.add(
                    widget::list::button(widget::settings::item(
                        repo,
                        widget::icon::from_name("go-next-symbolic").size(16),
                    ))
                    .on_press_maybe(enabled.then(|| Message::InspectRepository(repo.clone()))),
                );
            }
            content = content.push(results);
        }
        if let Some(catalogue) = &browser.catalogue {
            if let Some(license) = &catalogue.license {
                content = content.push(widget::text::caption(arg("license", "license", license)));
            }
            if catalogue.plans.is_empty() {
                content = content.push(widget::text::caption(tr("no-supported-files")));
            }
            let mut files = widget::settings::section().title(tr("choose-variant"));
            for plan in &catalogue.plans {
                files = files.add(
                    widget::list::button(
                        widget::row::with_capacity(3)
                            .spacing(space.space_s())
                            .align_y(Alignment::Center)
                            .push(
                                widget::column::with_capacity(2)
                                    .width(Length::Fill)
                                    .push(widget::text::body(&plan.quant))
                                    .push(widget::text::caption(&plan.filename)),
                            )
                            .push(widget::text::body(size_label(plan.size_bytes)))
                            .push(widget::icon::from_name("go-next-symbolic").size(16)),
                    )
                    .on_press_maybe(enabled.then(|| Message::ChooseDownload(plan.clone()))),
                );
            }
            content = content.push(files);
        }
        content.into()
    }

    pub(super) fn download_confirmation(&self) -> Option<Element<'_, Message>> {
        let plan = self.downloads.selected.as_ref()?;
        Some(
            widget::dialog()
                .title(tr("download-confirm"))
                .body(format!(
                    "{}\n{}\n\n{}",
                    plan.repo,
                    plan.filename,
                    tr("download-source-warning")
                ))
                .primary_action(
                    widget::button::suggested(arg(
                        "download-size",
                        "size",
                        size_label(plan.size_bytes),
                    ))
                    .on_press_maybe(
                        (!self.busy && self.controller.is_some())
                            .then_some(Message::SubmitDownload),
                    ),
                )
                .secondary_action(
                    widget::button::text(tr("cancel")).on_press(Message::CancelDownload),
                )
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;

    #[test]
    fn browser_failures_replace_empty_prompts_and_retry_restores_feedback() {
        let mut browser = Browser::default();
        assert_eq!(browser.feedback(), Some(BrowserFeedback::FindModel));
        browser.change_query("tiny".into());
        browser.searched = true;
        let ticket = browser.request();
        assert_eq!(browser.feedback(), None);
        browser.search_result(ticket, Err("Metadata timed out".into()));
        assert_eq!(
            browser.feedback(),
            Some(BrowserFeedback::SearchError("Metadata timed out"))
        );
        let ticket = browser.request();
        assert_eq!(browser.feedback(), None);
        browser.search_result(ticket, Ok(vec![]));
        assert_eq!(browser.feedback(), Some(BrowserFeedback::NoResults));
        let ticket = browser.request();
        browser.search_result(ticket, Ok(vec!["example/tiny".into()]));
        assert_eq!(browser.feedback(), None);
        browser.repo = Some("example/tiny".into());
        let ticket = browser.request();
        browser.catalogue_result(ticket, Err("Repository unavailable".into()));
        assert_eq!(
            browser.feedback(),
            Some(BrowserFeedback::RepositoryError("Repository unavailable"))
        );
        assert_eq!(browser.results, ["example/tiny"]);
        assert_eq!(browser.query, "tiny");
        assert!(browser.selected.is_none());
        assert!(browser.review.is_none());
    }

    fn plan() -> Plan {
        Plan {
            model: "example/tiny:Q8_0".into(),
            repo: "example/tiny".into(),
            revision: "a".repeat(40),
            quant: "Q8_0".into(),
            filename: "tiny-Q8_0.gguf".into(),
            size_bytes: 8,
            sha256: "b".repeat(64),
        }
    }

    #[test]
    fn resume_review_exposes_progress_and_failure_on_downloads_without_changing_work() {
        use super::super::{Mode, navigation::Page};
        let mut app = Applet {
            mode: Mode::Management,
            shared_operation: Some("Accepted transfer".into()),
            ..Default::default()
        };
        let _ = app.update(Message::ReviewDownload(plan().model));
        assert_eq!(app.navigation.page, Page::Downloads);
        assert!(app.downloads.review.as_ref().unwrap().busy);
        assert!(app.resume_feedback().is_some());
        let _ = app.manager_content();
        let ticket = app.downloads.review.as_ref().unwrap().ticket;
        let _ = app.update(Message::DownloadPlanReady(
            ticket,
            Err("Inspection failed".into()),
        ));
        let review = app.downloads.review.as_ref().unwrap();
        assert!(!review.busy);
        assert_eq!(review.error.as_deref(), Some("Inspection failed"));
        assert!(app.resume_feedback().is_some());
        let _ = app.manager_content();
        assert!(app.downloads.selected.is_none());
        assert_eq!(app.shared_operation.as_deref(), Some("Accepted transfer"));
        assert!(!app.busy);
        assert!(app.operation_label.is_none());
    }

    #[test]
    fn leaving_resume_review_rejects_late_results_and_preserves_metadata_and_accepted_work() {
        use super::super::{Mode, navigation::Page};
        for destination in [Page::Models, Page::Search, Page::Import] {
            let mut app = Applet {
                mode: Mode::Management,
                busy: true,
                operation_label: Some("Accepted operation".into()),
                ..Default::default()
            };
            app.downloads.query = "tiny".into();
            app.downloads.results = vec!["example/tiny".into()];
            app.downloads.repo = Some("example/tiny".into());
            let metadata_ticket = app.downloads.request();
            app.downloads.catalogue_result(
                metadata_ticket,
                Ok(Catalogue {
                    repo: "example/tiny".into(),
                    revision: "a".repeat(40),
                    license: Some("mit".into()),
                    plans: vec![plan()],
                }),
            );
            let catalogue = serde_json::to_value(&app.downloads.catalogue).unwrap();
            let _ = app.update(Message::ReviewDownload(plan().model));
            let ticket = app.downloads.review.as_ref().unwrap().ticket;
            app.select_page(destination);
            if destination == Page::Models {
                app.select_page(Page::Downloads);
            }
            let _ = app.update(Message::DownloadPlanReady(ticket, Ok(plan())));
            let _ = app.update(Message::DownloadPlanReady(
                ticket,
                Err("Stale failure".into()),
            ));
            assert!(app.downloads.selected.is_none());
            assert!(app.resume_feedback().is_none());
            assert_eq!(app.downloads.query, "tiny");
            assert_eq!(app.downloads.results, ["example/tiny"]);
            assert_eq!(app.downloads.repo.as_deref(), Some("example/tiny"));
            assert_eq!(
                serde_json::to_value(&app.downloads.catalogue).unwrap(),
                catalogue
            );
            assert_eq!(app.downloads.ticket, metadata_ticket);
            assert!(app.busy);
            assert_eq!(app.operation_label.as_deref(), Some("Accepted operation"));
        }
    }

    #[test]
    fn cancelled_or_replaced_resume_review_cannot_reopen_confirmation() {
        let mut app = Applet::default();
        let first = app.downloads.request_review();
        let _ = app.on_escape();
        let next = app.downloads.request_review();
        let _ = app.update(Message::DownloadPlanReady(first, Ok(plan())));
        assert!(app.downloads.selected.is_none());
        assert!(app.downloads.review.as_ref().unwrap().busy);
        let _ = app.update(Message::DownloadPlanReady(next, Ok(plan())));
        assert_eq!(app.downloads.selected, Some(plan()));
        assert!(app.download_confirmation().is_some());
        let _ = app.update(Message::CancelDownload);
        let _ = app.update(Message::DownloadPlanReady(next, Ok(plan())));
        assert!(app.downloads.selected.is_none());
        assert!(app.resume_feedback().is_none());
    }
    #[test]
    fn back_and_cancel_preserve_search_results_and_never_submit_a_download() {
        let mut app = Applet::default();
        app.downloads.query = "tiny".into();
        app.downloads.results = vec!["example/tiny".into()];
        app.downloads.repo = Some("example/tiny".into());
        let ticket = app.downloads.request();
        let _ = app.update(Message::DownloadBack);
        app.downloads
            .catalogue_result(ticket, Err("stale repository failure".into()));
        assert!(app.downloads.repo.is_none());
        assert!(app.downloads.error.is_none());
        assert_eq!(app.downloads.results, ["example/tiny"]);
        assert_eq!(app.downloads.query, "tiny");
        let _ = app.update(Message::CancelDownload);
        assert!(app.downloads.selected.is_none());
        assert!(!app.busy);
        assert!(app.operation_label.is_none());
    }
    #[test]
    fn late_resume_review_does_not_open_a_dialog_over_local_import() {
        let mut app = Applet::default();
        app.select_page(super::super::navigation::Page::Downloads);
        let ticket = app.downloads.request_review();
        app.select_page(super::super::navigation::Page::Import);
        let plan = Plan {
            model: "example/tiny:Q8_0".into(),
            repo: "example/tiny".into(),
            revision: "a".repeat(40),
            quant: "Q8_0".into(),
            filename: "tiny-Q8_0.gguf".into(),
            size_bytes: 8,
            sha256: "b".repeat(64),
        };
        let _ = app.update(Message::DownloadPlanReady(ticket, Ok(plan)));
        assert!(app.downloads.selected.is_none());
        assert!(!app.downloads.busy);
        assert_eq!(app.navigation.page, super::super::navigation::Page::Import);
        assert!(app.operation_label.is_none());
    }
    #[test]
    fn stale_metadata_completions_cannot_replace_a_new_query_or_selection() {
        let mut browser = Browser::default();
        let first = browser.request();
        browser.change_query("new query".into());
        let next = browser.request();
        browser.search_result(first, Ok(vec!["old/repo".into()]));
        assert!(browser.results.is_empty());
        assert!(browser.busy);
        browser.search_result(next, Ok(vec!["new/repo".into()]));
        assert_eq!(browser.results, vec!["new/repo"]);
        assert!(!browser.busy);
        browser.catalogue_result(first, Err("stale error".into()));
        assert!(browser.error.is_none());
    }
    #[test]
    fn browser_constructs_empty_search_and_error_views_without_network() {
        let mut app = Applet::default();
        let _ = app.download_browser();
        app.downloads.results = vec!["example/tiny-GGUF".into()];
        let _ = app.download_browser();
        app.downloads.error = Some("Public repository unavailable".into());
        let _ = app.download_browser();
    }
}
