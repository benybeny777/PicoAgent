// message.rs - picoagent-core の正規形をアプリ内の既存パスで公開する互換層。

pub use picoagent_core::message::{
    find_leaked_tool_call, strip_reasoning, Message, TokenUsage, ToolCall,
};
use serde::Serialize;

/// コネクタがPicoAgentの設定画面へ申告する設定項目。
#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    /// `text` / `password` / `textarea` / `checkbox` / `number`。
    pub r#type: String,
    pub placeholder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_auto: Option<bool>,
}

impl Field {
    pub fn new(key: &str, label: &str, ty: &str, placeholder: &str) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            r#type: ty.to_string(),
            placeholder: placeholder.to_string(),
            range: None,
            unit: None,
            allow_auto: None,
        }
    }

    /// 入力・保存・実行時の制約は同じ宣言から生成する。
    pub fn int(setting: &crate::config::IntSetting) -> Self {
        let mut field = Self::new(
            setting.key,
            setting.label,
            if setting.default.is_none() {
                "text"
            } else {
                "number"
            },
            &setting.default_text(),
        );
        field.range = Some([setting.min.to_string(), setting.max.to_string()]);
        field.unit = Some(setting.unit.to_string());
        field.allow_auto = Some(setting.default.is_none());
        field
    }
}

#[cfg(test)]
mod guard {
    use super::Field;

    #[test]
    fn 整数入力の範囲と既定値は共通宣言から生成する() {
        for setting in crate::config::ints::INT_SETTINGS {
            let field = Field::int(setting);
            assert_eq!(field.key, setting.key);
            assert_eq!(field.placeholder, setting.default_text());
            assert_eq!(
                field.range,
                Some([setting.min.to_string(), setting.max.to_string()])
            );
            assert_eq!(field.allow_auto, Some(setting.default.is_none()));
        }
        for field in crate::providers::all()
            .iter()
            .flat_map(|provider| provider.config_schema())
            .chain(
                crate::tools::all()
                    .iter()
                    .flat_map(|tool| tool.config_schema()),
            )
        {
            if let Some(setting) = crate::config::ints::find(&field.key) {
                assert_eq!(
                    serde_json::to_value(&field).unwrap(),
                    serde_json::to_value(Field::int(setting)).unwrap(),
                    "{} の入力制約が共通宣言と違います",
                    field.key
                );
            }
        }
    }

    fn direct_integer_reads(text: &str) -> Vec<String> {
        text.match_indices(".get")
            .filter_map(|(offset, _)| {
                let tail = text[offset + 4..]
                    .trim_start()
                    .strip_prefix('(')?
                    .trim_start()
                    .strip_prefix('"')?;
                let key = tail.split('"').next()?;
                crate::config::ints::find(key).map(|_| key.to_string())
            })
            .collect()
    }

    #[test]
    fn 整数設定の手書き読取を全ソースで禁止する() {
        let violations: Vec<_> = crate::guard_scan::all_sources()
            .into_iter()
            .flat_map(|source| {
                direct_integer_reads(source.body())
                    .into_iter()
                    .map(move |key| format!("{}: {key}", source.name))
            })
            .collect();
        assert!(
            violations.is_empty(),
            "共通整数検証を通さない読取: {violations:?}"
        );
    }

    #[test]
    fn 整数設定ガードは旧既定値への降格を検出する() {
        assert_eq!(
            direct_integer_reads(
                r#"cfg.get("PICOAGENT_HTTP_TIMEOUT_SECS", "20").parse::<u64>().unwrap_or(20)"#
            ),
            vec!["PICOAGENT_HTTP_TIMEOUT_SECS"]
        );
        assert!(
            direct_integer_reads(r#"cfg.get("PICOAGENT_TTS_SPEAKER", "0").parse::<u32>()"#)
                .is_empty()
        );
    }
}
