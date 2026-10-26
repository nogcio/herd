//! Embedded Fluent resources with desktop locale negotiation and English fallback.
use i18n_embed::{LanguageLoader, fluent::FluentLanguageLoader};
use rust_embed::RustEmbed;
use std::{collections::HashMap, sync::LazyLock};

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Assets;

fn loader() -> FluentLanguageLoader {
    let loader = FluentLanguageLoader::new("herd", "en".parse().expect("valid fallback locale"));
    loader
        .load_fallback_language(&Assets)
        .expect("embedded English localization is valid");
    loader
}

static LANGUAGE: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader = loader();
    let requested = i18n_embed::DesktopLanguageRequester::requested_languages();
    if i18n_embed::select(&loader, &Assets, &requested).is_err() {
        // The already loaded embedded English resource remains available.
        let _ = loader.load_fallback_language(&Assets);
    }
    loader
});

pub(super) fn tr(key: &str) -> String {
    LANGUAGE.get(key)
}
pub(super) fn arg(key: &str, name: &str, value: impl Into<String>) -> String {
    LANGUAGE.get_args(key, HashMap::from([(name, value.into())]))
}
pub(super) fn count(key: &str, count: usize) -> String {
    LANGUAGE.get_args(key, HashMap::from([("count", count as i64)]))
}
pub(super) fn action_name(key: &str, model: &str) -> String {
    LANGUAGE.get_args(
        "model-action",
        HashMap::from([("action", tr(key)), ("model", model.to_owned())]),
    )
}
pub(super) fn operation(kind: &str) -> String {
    tr(match kind {
        "start" => "starting-model",
        "stop" => "stopping-model",
        "server-up" => "starting-router",
        "download" => "submitting-download",
        "pause" => "pausing-download",
        "remove" => "removing-model",
        _ => "working",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn russian_locale_negotiates_and_unknown_locales_fall_back_without_global_changes() {
        let ru = loader();
        i18n_embed::select(&ru, &Assets, &["ru-RU".parse().unwrap()]).unwrap();
        assert_eq!(ru.get("models"), "Модели");
        assert_eq!(
            ru.get("stale-observations"),
            "Показаны ранее полученные состояния моделей; запуск новых моделей заблокирован."
        );
        let fallback = loader();
        i18n_embed::select(&fallback, &Assets, &["zz".parse().unwrap()]).unwrap();
        assert_eq!(fallback.get("models"), "Models");
        assert_eq!(
            fallback.get("stale-observations"),
            "Displayed model states are previous observations; new starts are blocked."
        );
    }
}
