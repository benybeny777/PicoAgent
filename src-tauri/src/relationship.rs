// relationship.rs - キャラクターごとの関係性と共有した出来事を永続化する。

use std::path::PathBuf;

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const MAX_MOMENTS: usize = 36;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedMoment {
    pub at: String,
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipState {
    pub character_id: String,
    pub mode: String,
    pub familiarity: u32,
    pub trust: u32,
    pub conversations: u64,
    pub completed_tasks: u64,
    pub interactions: u64,
    pub first_seen: String,
    pub last_seen: String,
    pub recent_moments: Vec<SharedMoment>,
}

impl RelationshipState {
    fn new(character_id: &str, mode: &str) -> Self {
        let now = Local::now().to_rfc3339();
        Self {
            character_id: character_id.to_string(),
            mode: normalize_mode(mode).to_string(),
            familiarity: 1,
            trust: 0,
            conversations: 0,
            completed_tasks: 0,
            interactions: 0,
            first_seen: now.clone(),
            last_seen: now,
            recent_moments: Vec::new(),
        }
    }

    pub fn bond_label(&self) -> &'static str {
        match self.familiarity.saturating_add(self.trust) {
            0..=14 => "出会ったばかり",
            15..=44 => "顔なじみ",
            45..=89 => "気の合う相棒",
            90..=149 => "頼れる相棒",
            _ => "長い付き合い",
        }
    }
}

fn normalize_mode(mode: &str) -> &'static str {
    match mode {
        "life" => "life",
        "work" => "work",
        _ => "both",
    }
}

fn path(app: &AppHandle, character_id: &str) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("関係性データの保存先を取得できません: {error}"))?;
    Ok(root.join(format!("relationship-{character_id}.json")))
}

fn read(
    path: &std::path::Path,
    character_id: &str,
    mode: &str,
) -> Result<RelationshipState, String> {
    if !path.exists() {
        return Ok(RelationshipState::new(character_id, mode));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("関係性データを読み取れません: {error}"))?;
    match serde_json::from_str::<RelationshipState>(&text) {
        Ok(mut state) => {
            state.character_id = character_id.to_string();
            state.mode = normalize_mode(&state.mode).to_string();
            Ok(state)
        }
        Err(error) => {
            let backup = crate::store::quarantine_corrupt(path).ok_or_else(|| {
                format!("壊れた関係性データを退避できません。上書きを中止しました: {error}")
            })?;
            // フォールバック許可: 壊れた元データは backup に残したため、新規状態で継続できる。
            let _ = backup;
            Ok(RelationshipState::new(character_id, mode))
        }
    }
}

pub fn load(app: &AppHandle, character_id: &str, mode: &str) -> Result<RelationshipState, String> {
    let path = path(app, character_id)?;
    let _guard = crate::store::lock_state(&format!("relationship:{character_id}"));
    read(&path, character_id, mode)
}

pub fn set_mode(
    app: &AppHandle,
    character_id: &str,
    mode: &str,
) -> Result<RelationshipState, String> {
    let path = path(app, character_id)?;
    let _guard = crate::store::lock_state(&format!("relationship:{character_id}"));
    let mut state = read(&path, character_id, mode)?;
    state.mode = normalize_mode(mode).to_string();
    state.last_seen = Local::now().to_rfc3339();
    crate::store::write_json_atomic(&path, &state).map_err(|error| error.to_string())?;
    Ok(state)
}

pub fn record(
    app: &AppHandle,
    character_id: &str,
    mode: &str,
    kind: &str,
    detail: &str,
) -> Result<RelationshipState, String> {
    let path = path(app, character_id)?;
    let _guard = crate::store::lock_state(&format!("relationship:{character_id}"));
    let mut state = read(&path, character_id, mode)?;
    let now = Local::now();
    state.last_seen = now.to_rfc3339();
    match kind {
        "conversation" => {
            state.conversations += 1;
            state.familiarity = state.familiarity.saturating_add(2).min(100);
        }
        "task_success" => {
            state.completed_tasks += 1;
            state.familiarity = state.familiarity.saturating_add(2).min(100);
            state.trust = state.trust.saturating_add(3).min(100);
        }
        "task_failure" => {
            state.trust = state.trust.saturating_add(1).min(100);
        }
        "touch_head" | "touch_cheek" | "touch_hand" | "petting" => {
            state.interactions += 1;
            state.familiarity = state.familiarity.saturating_add(1).min(100);
        }
        "return" => {
            state.familiarity = state.familiarity.saturating_add(1).min(100);
        }
        _ => return Err("記録できない関係性イベントです。".to_string()),
    }
    let clean = detail.trim().chars().take(120).collect::<String>();
    if !clean.is_empty() && matches!(kind, "task_success" | "task_failure" | "conversation") {
        state.recent_moments.push(SharedMoment {
            at: now.to_rfc3339(),
            kind: kind.to_string(),
            text: clean,
        });
        if state.recent_moments.len() > MAX_MOMENTS {
            state
                .recent_moments
                .drain(0..state.recent_moments.len() - MAX_MOMENTS);
        }
    }
    crate::store::write_json_atomic(&path, &state).map_err(|error| error.to_string())?;
    Ok(state)
}

pub fn context(state: &RelationshipState) -> String {
    let mode = match state.mode.as_str() {
        "life" => "暮らしを一緒に楽しむ",
        "work" => "作業を一緒に進める",
        _ => "暮らしと作業の両方を支える",
    };
    let mut text = format!(
        "このユーザーとの関係: {}。距離感は「{}」。{}相棒として接してください。数値や内部ランクは口にせず、共有した経験を自然な反応にだけ反映してください。",
        state.bond_label(), state.bond_label(), mode
    );
    if !state.recent_moments.is_empty() {
        text.push_str("\n最近いっしょにしたこと:");
        for moment in state.recent_moments.iter().rev().take(4) {
            text.push_str("\n- ");
            text.push_str(&moment.text);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 関係ラベルは数値を見せず段階的に変わる() {
        let mut state = RelationshipState::new("mugi", "both");
        assert_eq!(state.bond_label(), "出会ったばかり");
        state.familiarity = 50;
        state.trust = 50;
        assert_eq!(state.bond_label(), "頼れる相棒");
    }

    #[test]
    fn 不明なモードは両方向けに正規化する() {
        assert_eq!(normalize_mode("unknown"), "both");
    }
}
