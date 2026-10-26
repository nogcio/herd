//! UI-side artwork state: native theme lookup, background fetching, and retry cooldown.
use super::Message;
use cosmic::{Task, widget};
use herd::{
    icons::{IconLoader, IconSpec, LoadedIcon},
    model::{Model, Snapshot},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

enum Entry {
    Pending,
    Ready(widget::icon::Handle),
    Failed(Instant),
}

#[derive(Default)]
pub(super) struct ModelIcons {
    entries: HashMap<IconSpec, Entry>,
    loader: Option<Arc<IconLoader>>,
}

impl ModelIcons {
    pub fn get(&self, model: &Model) -> Option<widget::icon::Handle> {
        match self.entries.get(&model.icon) {
            Some(Entry::Ready(handle)) => Some(handle.clone()),
            _ => None,
        }
    }

    pub fn schedule(&mut self, snapshots: &[Snapshot]) -> Task<cosmic::Action<Message>> {
        let sources: HashSet<_> = snapshots
            .iter()
            .map(|s| s.model.icon.clone())
            .filter(|s| !s.is_empty())
            .collect();
        self.entries.retain(|source, _| sources.contains(source));
        let mut requests = Vec::new();
        for source in sources {
            match self.entries.get(&source) {
                Some(Entry::Failed(retry)) if Instant::now() >= *retry => (),
                Some(_) => continue,
                None => (),
            }
            if let Some(name) = source.theme_name()
                && let Some(path) = widget::icon::from_name(name.to_owned()).size(40).path()
            {
                self.entries
                    .insert(source, Entry::Ready(widget::icon::from_path(path)));
                continue;
            }
            self.entries.insert(source.clone(), Entry::Pending);
            requests.push(source);
        }
        if requests.is_empty() {
            return Task::none();
        }
        if self.loader.is_none() {
            match IconLoader::new() {
                Ok(loader) => self.loader = Some(Arc::new(loader)),
                Err(_) => {
                    for source in requests {
                        self.fail(source);
                    }
                    return Task::none();
                }
            }
        }
        let Some(loader) = &self.loader else {
            return Task::none();
        };
        Task::batch(requests.into_iter().map(|source| {
            let loader = loader.clone();
            Task::perform(
                async move {
                    let result = loader
                        .load(source.clone())
                        .await
                        .map_err(|error| error.to_string());
                    (source, result)
                },
                |(source, result)| cosmic::Action::App(Message::IconLoaded(source, result)),
            )
        }))
    }

    pub fn complete(&mut self, source: IconSpec, result: Result<LoadedIcon, String>) {
        // Ignore completions for removed/changed metadata.
        if !self.entries.contains_key(&source) {
            return;
        }
        match result {
            Ok(icon) => {
                self.entries.insert(
                    source,
                    Entry::Ready(widget::icon::from_raster_bytes(icon.png)),
                );
            }
            Err(_) => self.fail(source),
        }
    }

    fn fail(&mut self, source: IconSpec) {
        self.entries.insert(
            source,
            Entry::Failed(Instant::now() + Duration::from_secs(600)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_completions_are_ignored_and_failures_have_a_cooldown() {
        let mut icons = ModelIcons::default();
        let source = IconSpec {
            url: Some("https://huggingface.co/example.png".into()),
            ..Default::default()
        };
        icons.complete(source.clone(), Err("offline".into()));
        assert!(icons.entries.is_empty());
        icons.entries.insert(source.clone(), Entry::Pending);
        icons.complete(source.clone(), Err("offline".into()));
        assert!(
            matches!(icons.entries.get(&source), Some(Entry::Failed(retry)) if *retry > Instant::now())
        );
    }
}
