// キャラクターパックと保存済み上書きの声を、同じ検証と解決規則で扱う。
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterVoice {
    pub provider: String,
    #[serde(alias = "modelUUID")]
    pub model_uuid: Option<String>,
    pub model_name: Option<String>,
    #[serde(alias = "speakerUUID")]
    pub speaker_uuid: Option<String>,
    pub style_name: Option<String>,
    #[serde(alias = "speaker")]
    pub style_id: Option<String>,
    #[serde(alias = "sourceURL")]
    pub source_url: Option<String>,
}

impl CharacterVoice {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider != "aivis" {
            return Err(format!(
                "声のproviderはaivisを指定してください: {}",
                self.provider
            ));
        }
        for (label, value) in [
            ("modelUuid", &self.model_uuid),
            ("speakerUuid", &self.speaker_uuid),
        ] {
            if let Some(value) = value {
                let valid = value.len() == 36
                    && value.bytes().enumerate().all(|(index, byte)| {
                        if [8, 13, 18, 23].contains(&index) {
                            byte == b'-'
                        } else {
                            byte.is_ascii_hexdigit()
                        }
                    });
                if !valid {
                    return Err(format!("声の{label}がUUID形式ではありません: {value}"));
                }
            }
        }
        if let Some(id) = &self.style_id {
            if id.is_empty()
                || !id.bytes().all(|byte| byte.is_ascii_digit())
                || id.parse::<u32>().is_err()
            {
                return Err(format!("声のstyleIdは数値IDで指定してください: {id}"));
            }
        }
        if self.speaker_uuid.is_none() && self.style_id.is_none() {
            return Err("声にはspeakerUuidまたはstyleIdを指定してください。".into());
        }
        if self.model_uuid.is_some() && self.speaker_uuid.is_none() {
            return Err("モデルを導入する声にはspeakerUuidも指定してください。".into());
        }
        Ok(())
    }

    pub fn failure(&self, character_id: &str, reason: impl std::fmt::Display) -> String {
        let model = self
            .model_name
            .as_deref()
            .or(self.model_uuid.as_deref())
            .unwrap_or("指定話者");
        let style = self
            .style_name
            .as_deref()
            .or(self.style_id.as_deref())
            .unwrap_or("未指定スタイル");
        format!("{character_id}の声（{model}/{style}）を準備できません: {reason}")
    }
}

#[cfg(test)]
mod tests {
    use super::CharacterVoice;
    #[test]
    fn パックのuuidとurlの大文字表記も同じ型で受け付ける() {
        let voice: CharacterVoice = serde_json::from_value(serde_json::json!({
            "provider": "aivis", "modelName": "モデル",
            "modelUUID": "4f281e78-eba6-495a-8e50-5c322d02b5b1",
            "speakerUUID": "aa63d66e-92fe-499e-9e0d-15dad4a61d0d",
            "styleName": "ノーマル", "sourceURL": "https://example.com/model"
        }))
        .unwrap();
        assert!(voice.validate().is_ok());
        assert_eq!(
            voice.model_uuid.as_deref(),
            Some("4f281e78-eba6-495a-8e50-5c322d02b5b1")
        );
        assert_eq!(
            voice.source_url.as_deref(),
            Some("https://example.com/model")
        );
    }

    #[test]
    fn 声の指定は不正値を拒否して明示した数値文字列を保持する() {
        for (value, valid) in [
            (
                serde_json::json!({"provider":"aivis","styleId":"00123"}),
                true,
            ),
            (
                serde_json::json!({"provider":"aivis","styleId":"abc"}),
                false,
            ),
            (
                serde_json::json!({"provider":"other","styleId":"123"}),
                false,
            ),
            (
                serde_json::json!({"provider":"aivis","speakerUuid":"broken"}),
                false,
            ),
            (serde_json::json!({"provider":"aivis"}), false),
        ] {
            let voice: CharacterVoice = serde_json::from_value(value).unwrap();
            assert_eq!(voice.validate().is_ok(), valid);
            if valid {
                assert_eq!(voice.style_id.as_deref(), Some("00123"));
            }
        }
    }
}
