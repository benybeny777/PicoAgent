// skills/project_notes/mod.rs - プロジェクトごとの記録（メモ・決定事項・進捗）スキル。
//
// 課題台帳（`project_tasks`）が「これからやること」なのに対し、こちらは
// 「決まったこと・分かったこと」を残す。プロジェクトを切り替えて数日後に戻ったとき、
// 前提をユーザーが毎回説明し直さずに済むようにするのが目的。
//
// 直近の記録は `project_tasks::project_context()` がまとめて会話の system 文脈へ
// 差し込む（`recent_lines()` がその材料を返す）。長期メモリ（`memory.rs`）が会話全体から
// 暗黙に拾うのに対し、こちらはプロジェクトに明示的に紐づく点が違う。

use async_trait::async_trait;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::text;

pub const KEY: &str = "PICOAGENT_PROJECT_NOTES";
const MAX_NOTES: usize = 200;
const MAX_TEXT_CHARS: usize = 300;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
/// system 文脈へ載せる件数（多いと会話の文脈を圧迫する）。
const CONTEXT_LINES: usize = 4;

const KINDS: [&str; 3] = ["note", "decision", "progress"];

#[derive(Serialize, Deserialize, Clone)]
pub struct Note {
    pub id: String,
    pub project: String,
    /// `note`（メモ） / `decision`（決定事項） / `progress`（進捗）。
    pub kind: String,
    pub text: String,
    pub created_at: String,
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "decision" => "決定",
        "progress" => "進捗",
        _ => "メモ",
    }
}

pub fn load(cfg: &Config) -> Result<Vec<Note>, String> {
    crate::store::load_list(cfg, KEY)
}

fn save(cfg: &Config, notes: &[Note]) -> Result<(), String> {
    let encoded = serde_json::to_string(notes)
        .map_err(|error| format!("記録を保存用に変換できません: {error}"))?;
    cfg.set_string(KEY, encoded)
        .map_err(|error| format!("記録を保存できません: {error}"))
}

fn next_id(notes: &[Note]) -> String {
    let max = notes
        .iter()
        .filter_map(|note| note.id.strip_prefix('n'))
        .filter_map(|number| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("n{}", max + 1)
}

fn render(notes: &[Note], limit: usize) -> String {
    if notes.is_empty() {
        return "記録はありません。".to_string();
    }
    let mut out = format!("記録 {} 件（新しい順）", notes.len());
    for note in notes.iter().rev().take(limit) {
        out.push_str(&format!(
            "\n{} [{}] {}: {}",
            note.id,
            kind_label(&note.kind),
            note.created_at,
            note.text
        ));
    }
    if notes.len() > limit {
        out.push_str(&format!("\n…（{limit} 件で表示を打ち切り。limit で増やせます）"));
    }
    out
}

/// system 文脈へ載せる、指定プロジェクトの直近の記録（新しい順）。
pub fn recent_lines(cfg: &Config, project: &str) -> Vec<String> {
    // フォールバック許可: 文脈生成は読み取り専用の付加情報なので、
    // 読めなければ載せずに会話を続ける（保存はしないのでデータは消えない）。
    let notes = load(cfg).unwrap_or_default();
    notes
        .iter()
        .rev()
        .filter(|note| note.project == project)
        .take(CONTEXT_LINES)
        .map(|note| {
            format!(
                "{}（{}・{}）",
                note.text,
                kind_label(&note.kind),
                note.created_at
            )
        })
        .collect()
}

pub struct ProjectNotesTool;

#[async_trait]
impl Tool for ProjectNotesTool {
    fn name(&self) -> &'static str {
        "project_notes"
    }

    fn label(&self) -> &'static str {
        "プロジェクト記録"
    }

    fn description(&self) -> String {
        "プロジェクトごとに、メモ・決定事項・進捗を残して読み返す。次に同じプロジェクトを再開したとき前提を思い出せるようにする。これからやることは project_tasks に登録する。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove"],
                    "description": "add は記録の追加、list は読み返し、remove は削除。"
                },
                "project": {
                    "type": "string",
                    "description": "対象プロジェクト。省略時は選択中のワークスペース。list では \"all\" で全プロジェクト。"
                },
                "kind": {
                    "type": "string",
                    "enum": ["note", "decision", "progress"],
                    "description": "記録の種類（既定 note）。決まったことは decision、進み具合は progress。"
                },
                "text": { "type": "string", "description": "記録する内容（add は必須）。" },
                "id": { "type": "string", "description": "remove の対象ID（n1 など）。" },
                "limit": { "type": "integer", "description": "list の表示件数（既定20、最大100）。" },
                "confirm": { "type": "boolean", "description": "remove の実行確認。" }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "この方針を決定事項として残して",
            "このプロジェクトで前に決めたことを教えて",
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        // 読む→直す→書き戻すの間、同じ一覧を触る並列ツール実行と競合しないようにする。
        let _guard = crate::store::lock_state(KEY);
        // 壊れていても空一覧で始めない（次の保存で記録が丸ごと消えるため）。
        let mut notes = match load(cfg) {
            Ok(notes) => notes,
            Err(error) => return format!("エラー: {error}"),
        };
        let project = args
            .get("project")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|project| !project.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| super::project_tasks::active_project(cfg));
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| (limit as usize).clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);

        match action.as_str() {
            "list" => {
                let scoped: Vec<Note> = notes
                    .into_iter()
                    .filter(|note| project == "all" || note.project == project)
                    .collect();
                format!(
                    "対象: {}\n{}",
                    if project == "all" { "全プロジェクト".to_string() } else { project },
                    render(&scoped, limit)
                )
            }
            "add" => {
                let text_arg = args.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if text_arg.is_empty() {
                    return "エラー: text が指定されていません。".to_string();
                }
                if project == "all" {
                    return "エラー: project に \"all\" は指定できません。".to_string();
                }
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("note")
                    .trim()
                    .to_lowercase();
                if !KINDS.contains(&kind.as_str()) {
                    return format!("エラー: kind は {} のいずれかを指定してください。", KINDS.join(" / "));
                }
                if notes.len() >= MAX_NOTES {
                    // 上限で黙って古いものを消すと決定事項が失われる。消す判断はユーザーに委ねる。
                    return format!(
                        "エラー: 記録は {MAX_NOTES} 件までです。remove で古いものを整理してください。"
                    );
                }
                let note = Note {
                    id: next_id(&notes),
                    project: project.clone(),
                    kind,
                    text: text::preview(text_arg, MAX_TEXT_CHARS),
                    created_at: Local::now().format("%Y-%m-%d").to_string(),
                };
                let line = format!("[{}] {}", kind_label(&note.kind), note.text);
                notes.push(note);
                match save(cfg, &notes) {
                    Ok(()) => format!("OK: {project} に記録しました。\n{line}"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "remove" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
                let Some(index) = notes.iter().position(|note| note.id == id) else {
                    return format!("エラー: 記録 {id} は見つかりません。");
                };
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "プレビュー: 次の記録を削除します。\n[{}] {}\n実行するには confirm=true を指定してください。",
                        kind_label(&notes[index].kind),
                        notes[index].text
                    );
                }
                let removed = notes.remove(index);
                match save(cfg, &notes) {
                    Ok(()) => format!("OK: 記録を削除しました: {} {}", removed.id, removed.text),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            other => format!("エラー: action は add/list/remove のいずれかを指定してください: {other}"),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ProjectNotesTool) } }

#[cfg(test)]
mod tests {
    use super::{kind_label, next_id, render, Note};

    fn note(id: &str, project: &str, kind: &str, text: &str) -> Note {
        Note {
            id: id.to_string(),
            project: project.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
            created_at: "2026-08-02".to_string(),
        }
    }

    #[test]
    fn 種類のラベルを日本語で出す() {
        assert_eq!(kind_label("decision"), "決定");
        assert_eq!(kind_label("progress"), "進捗");
        assert_eq!(kind_label("なにか"), "メモ");
    }

    #[test]
    fn 連番idは既存の最大値の次を使う() {
        assert_eq!(next_id(&[note("n1", "p", "note", "a"), note("n7", "p", "note", "b")]), "n8");
        assert_eq!(next_id(&[]), "n1");
    }

    /// 読み返しは新しい順（直近の決定が先に目に入るように）。
    #[test]
    fn 新しい順に表示する() {
        let notes = vec![
            note("n1", "p", "note", "古い"),
            note("n2", "p", "decision", "新しい"),
        ];
        let text = render(&notes, 20);
        let first = text.find("新しい").unwrap();
        let second = text.find("古い").unwrap();
        assert!(first < second, "text: {text}");
        assert!(text.contains("[決定]"), "text: {text}");
    }

    #[test]
    fn 表示件数を打ち切る() {
        let notes: Vec<Note> = (1..=5)
            .map(|index| note(&format!("n{index}"), "p", "note", "x"))
            .collect();
        assert!(render(&notes, 2).contains("2 件で表示を打ち切り"));
        assert_eq!(render(&[], 2), "記録はありません。");
    }
}
