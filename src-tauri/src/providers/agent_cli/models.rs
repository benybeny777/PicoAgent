//! CLIのモデル設定と候補一覧。外部設定は読み取り専用で、モデル以外は実行へ持ち込まない。
use super::Kind;
use std::path::{Path, PathBuf};

fn config_dir(selected: Kind) -> Option<PathBuf> {
    let (variable, directory) = match selected {
        Kind::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
        Kind::Codex => ("CODEX_HOME", ".codex"),
        Kind::Custom => return None,
    };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(directory)))
}

fn read_optional(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => anyhow::bail!("CLIのモデル設定を読み取れません: {}", path.display()),
    }
}

fn parse_default(selected: Kind, text: &str) -> anyhow::Result<Option<String>> {
    // 解析エラーへ設定の原文を含めない。秘密情報を持つ他のキーもここでは使わない。
    let model = match selected {
        Kind::Codex => {
            let config: toml::Value = toml::from_str(text)
                .map_err(|_| anyhow::anyhow!("Codexのconfig.tomlを解析できません"))?;
            let profile = config.get("profile").and_then(toml::Value::as_str);
            profile
                .and_then(|name| config.get("profiles")?.get(name)?.get("model"))
                .or_else(|| config.get("model"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        }
        Kind::Claude => {
            let config: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| anyhow::anyhow!("Claudeのsettings.jsonを解析できません"))?;
            config
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        }
        Kind::Custom => None,
    };
    Ok(model
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty()))
}

pub(super) fn default_model(selected: Kind) -> anyhow::Result<Option<String>> {
    let Some(directory) = config_dir(selected) else {
        return Ok(None);
    };
    let filename = if selected == Kind::Codex {
        "config.toml"
    } else {
        "settings.json"
    };
    read_optional(&directory.join(filename))?
        .map(|text| parse_default(selected, &text))
        .transpose()
        .map(Option::flatten)
}

fn cached_models(text: &str) -> anyhow::Result<Vec<String>> {
    let cache: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| anyhow::anyhow!("Codexのモデル一覧キャッシュを解析できません"))?;
    Ok(cache
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| model.get("visibility").and_then(serde_json::Value::as_str) == Some("list"))
        .filter_map(|model| model.get("slug").and_then(serde_json::Value::as_str))
        .filter(|model| !model.trim().is_empty())
        .map(str::to_owned)
        .collect())
}

pub(super) fn list(selected: Kind) -> anyhow::Result<Vec<String>> {
    let mut models = match selected {
        // バージョンIDを固定せず、CLIが解決するモデル系列の別名を使う。
        Kind::Claude => vec!["opus".into(), "sonnet".into(), "haiku".into()],
        Kind::Codex => match config_dir(selected) {
            Some(directory) => read_optional(&directory.join("models_cache.json"))?
                .map(|text| cached_models(&text))
                .transpose()?
                .unwrap_or_default(),
            None => Vec::new(),
        },
        Kind::Custom => Vec::new(),
    };
    if let Some(model) = default_model(selected)? {
        if !models.contains(&model) {
            models.insert(0, model);
        }
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 外部設定の既定値と選択プロファイルだけを読む() {
        assert_eq!(
            parse_default(
                Kind::Codex,
                "model = 'future-model'\n[profiles.other]\nmodel = 'wrong'"
            )
            .unwrap()
            .as_deref(),
            Some("future-model")
        );
        assert_eq!(
            parse_default(
                Kind::Codex,
                "model = 'base'\nprofile = 'active'\n[profiles.active]\nmodel = 'next-model'"
            )
            .unwrap()
            .as_deref(),
            Some("next-model")
        );
        assert_eq!(
            parse_default(Kind::Claude, r#"{"model":"opus","env":{"OTHER":"secret"}}"#)
                .unwrap()
                .as_deref(),
            Some("opus")
        );
        assert!(parse_default(Kind::Claude, "{}").unwrap().is_none());
        assert!(!parse_default(Kind::Codex, "invalid secret")
            .unwrap_err()
            .to_string()
            .contains("secret"));
    }

    #[test]
    fn 新モデルを動的に取り込み非表示モデルは除外する() {
        assert_eq!(cached_models(r#"{"models":[{"slug":"future-model","visibility":"list"},{"slug":"hidden","visibility":"hide"}]}"#).unwrap(), vec!["future-model"]);
        assert!(cached_models("{}").unwrap().is_empty());
    }

    #[test]
    fn 未作成の設定ファイルは未設定として扱う() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_optional(&dir.path().join("missing.json"))
            .unwrap()
            .is_none());
    }
}
