//! Application-wide locale. Only application-authored text enters this module;
//! commands, paths, names and backend output are passed as opaque arguments.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "en")]
    English,
}

static ENGLISH: AtomicBool = AtomicBool::new(false);
static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();

impl Language {
    pub fn locale(self, system_chinese: bool) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en",
            Self::System if system_chinese => "zh-CN",
            Self::System => "en",
        }
    }
}

pub fn apply(language: Language) {
    #[cfg(target_os = "macos")]
    let chinese = unsafe { macrun_system_chinese() };
    #[cfg(not(target_os = "macos"))]
    let chinese = std::env::var("LANG").is_ok_and(|s| s.to_lowercase().starts_with("zh"));
    ENGLISH.store(language.locale(chinese) == "en", Ordering::SeqCst);
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn macrun_system_chinese() -> bool;
}

pub fn locale() -> &'static str {
    if ENGLISH.load(Ordering::SeqCst) {
        "en"
    } else {
        "zh-CN"
    }
}

pub fn text(source: &'static str) -> &'static str {
    text_in(locale(), source)
}

pub fn text_in(locale: &str, source: &'static str) -> &'static str {
    let fallback = source.split_once("::").map_or(source, |(_, text)| text);
    if locale != "en" {
        return fallback;
    }
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../src/locales/en.json"))
                .expect("valid English catalog")
        })
        .get(source)
        .map_or(fallback, String::as_str)
}

pub fn interpolate(source: &'static str, values: &[String]) -> String {
    interpolate_in(locale(), source, values)
}

fn interpolate_in(locale: &str, source: &'static str, values: &[String]) -> String {
    let template = text_in(locale, source);
    let mut result = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        rest = &rest[open..];
        if let Some(close) = rest.find('}')
            && let Ok(index) = rest[1..close].parse::<usize>()
            && let Some(value) = values.get(index)
        {
            result.push_str(value);
            rest = &rest[close + 1..];
        } else {
            result.push('{');
            rest = &rest[1..];
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_validation_and_system_fallback() {
        assert_eq!(Language::System.locale(true), "zh-CN");
        assert_eq!(Language::System.locale(false), "en");
        assert_eq!(Language::English.locale(true), "en");
        assert_eq!(Language::Chinese.locale(false), "zh-CN");
        for language in [Language::System, Language::Chinese, Language::English] {
            assert_eq!(
                serde_json::from_value::<Language>(serde_json::to_value(language).unwrap())
                    .unwrap(),
                language
            );
        }
        assert!(serde_json::from_str::<Language>("\"invalid\"").is_err());
    }

    #[test]
    fn interpolation_preserves_user_content_and_missing_placeholders() {
        assert_eq!(text_in("en", "设置"), "Settings");
        assert_eq!(text_in("zh-CN", "设置"), "设置");
        assert_eq!(text_in("zh-CN", "pairing.connect::连接"), "连接");
        assert_eq!(text_in("en", "pairing.connect::连接"), "Connect");
        assert_eq!(text_in("en", "unlisted diagnostic"), "unlisted diagnostic");
        assert_eq!(
            interpolate_in("en", "{0} 想运行一条命令", &["中文 {1} /tmp/work".into()]),
            "中文 {1} /tmp/work wants to run a command"
        );
        assert_eq!(
            interpolate_in("zh-CN", "{0} / {1} {}", &["x".into()]),
            "x / {1} {}"
        );
    }
}
