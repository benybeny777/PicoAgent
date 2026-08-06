// skills/project/mod.rs - プロジェクト管理（環境の切り替え・課題台帳・記録）。
//
// 以前は「ワークスペース（許可ルート・AI・モデル・キャラクターの名前付きセット）」と
// 「プロジェクトの課題・記録」が別スキルだったが、ユーザーから見ればどちらも同じ
// 「いま取り組んでいる案件」であり、名前も概念も二重だった。1つの `Project` に
// 環境・課題・記録をまとめ、切り替えれば全部が一緒に切り替わるようにする。
// 旧ワークスペースのデータは引き継がない（`PICOAGENT_WORKSPACES` は読まない）。
//
// - 環境: `switch` が許可ルート・AI・モデル・キャラクターを設定へ書き戻す
// - 課題: 長く残る「やること」。いま取りかかっている一連の手順は `todo_plan` の役割
// - 記録: 決定事項・進捗。次に再開したとき前提を思い出すためのもの
//
// 現在のプロジェクトの課題と記録は `project_context()` が短くまとめ、`commands.rs` が
// 会話の system 文脈へ差し込む（切り替えた案件の状況を毎回説明せずに済むように）。

use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::text;

pub const KEY: &str = "PICOAGENT_PROJECTS";
pub const ACTIVE_KEY: &str = "PICOAGENT_ACTIVE_PROJECT";

const DEFAULT_PROJECT: &str = "default";
const MAX_PROJECTS: usize = 50;
const MAX_TASKS: usize = 200;
const MAX_NOTES: usize = 200;
const MAX_TITLE_CHARS: usize = 120;
const MAX_NOTE_CHARS: usize = 400;
const MAX_TEXT_CHARS: usize = 300;
const MAX_TAGS: usize = 5;
const DEFAULT_LIMIT: usize = 30;
const MAX_LIMIT: usize = 100;
/// system 文脈へ差し込むまとめの長さ上限（会話の文脈を食い潰さないため）。
const MAX_CONTEXT_CHARS: usize = 1200;
/// system 文脈へ載せる件数。
const CONTEXT_TASKS: usize = 5;
const CONTEXT_NOTES: usize = 4;

const STATUSES: [&str; 3] = ["todo", "doing", "done"];
const PRIORITIES: [&str; 3] = ["high", "normal", "low"];
const NOTE_KINDS: [&str; 3] = ["note", "decision", "progress"];

/// `switch` で設定へ書き戻す対応表（プロジェクトの項目 → 設定キー）。
const ENVIRONMENT: [(&str, &str); 5] = [
    ("allowed_root", "PICOAGENT_ALLOWED_ROOT"),
    ("allowed_paths", "PICOAGENT_ALLOWED_PATHS"),
    ("provider", "PICOAGENT_PROVIDER"),
    ("model", "PICOAGENT_MODEL"),
    ("character", "PICOAGENT_CHARACTER"),
];

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Task {
    pub id: String,
    pub title: String,
    /// `todo` / `doing` / `done`。
    pub status: String,
    /// `high` / `normal` / `low`。
    pub priority: String,
    #[serde(default)]
    pub due: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Note {
    pub id: String,
    /// `note`（メモ） / `decision`（決定事項） / `progress`（進捗）。
    pub kind: String,
    pub text: String,
    pub created_at: String,
}

/// 案件ひとつ。環境設定と課題・記録を同じレコードに持つ。
///
/// 追加した項目を欠く古い保存データでも読めるよう、`id` 以外は `#[serde(default)]` にする
/// （項目を1つ足すたびに保存済みの一覧が丸ごと読めなくなるのを防ぐ）。
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Project {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub allowed_root: String,
    #[serde(default)]
    pub allowed_paths: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub character: String,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub notes: Vec<Note>,
}

impl Project {
    fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// 選択中のプロジェクトID。未設定なら `default`。
pub fn active_project(cfg: &Config) -> String {
    let active = cfg.get(ACTIVE_KEY, "");
    let active = active.trim();
    if active.is_empty() {
        DEFAULT_PROJECT.to_string()
    } else {
        active.to_string()
    }
}

pub fn load(cfg: &Config) -> Result<Vec<Project>, String> {
    crate::store::load_list(cfg, KEY)
}

fn save(cfg: &Config, projects: &[Project]) -> Result<(), String> {
    let encoded = serde_json::to_string(projects)
        .map_err(|error| format!("プロジェクトを保存用に変換できません: {error}"))?;
    cfg.set_string(KEY, encoded)
        .map_err(|error| format!("プロジェクトを保存できません: {error}"))
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn stamp() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// 対象プロジェクトの位置。無ければ作る（課題を足す前に登録を強いないため）。
fn ensure_project(projects: &mut Vec<Project>, id: &str) -> Result<usize, String> {
    if let Some(index) = projects.iter().position(|project| project.id == id) {
        return Ok(index);
    }
    if projects.len() >= MAX_PROJECTS {
        return Err(format!("プロジェクトは {MAX_PROJECTS} 件までです。"));
    }
    projects.push(Project {
        id: id.to_string(),
        name: id.to_string(),
        ..Project::default()
    });
    Ok(projects.len() - 1)
}

fn target_id(cfg: &Config, args: &Value) -> String {
    args.get("project")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|project| !project.is_empty())
        .map(slug)
        .filter(|project| !project.is_empty())
        .unwrap_or_else(|| active_project(cfg))
}

// ---- 課題 -------------------------------------------------------------

fn due_arg(args: &Value) -> Result<Option<String>, String> {
    if let Some(days) = args.get("due_in_days").and_then(Value::as_i64) {
        let date = today()
            .checked_add_signed(chrono::Duration::days(days))
            .ok_or_else(|| "due_in_days が範囲外です。".to_string())?;
        return Ok(Some(date.format("%Y-%m-%d").to_string()));
    }
    let Some(raw) = args.get("due").and_then(Value::as_str) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        // 明示的な空文字は「期限を外す」意味にする。
        return Ok(Some(String::new()));
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map(|date| Some(date.format("%Y-%m-%d").to_string()))
        .map_err(|_| format!("due は YYYY-MM-DD で指定してください: {raw}"))
}

fn enum_arg(args: &Value, key: &str, allowed: &[&str]) -> Result<Option<String>, String> {
    let Some(raw) = args.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    let value = raw.trim().to_lowercase();
    if value.is_empty() {
        return Ok(None);
    }
    if allowed.contains(&value.as_str()) {
        Ok(Some(value))
    } else {
        Err(format!("{key} は {} のいずれかを指定してください。", allowed.join(" / ")))
    }
}

fn tags_arg(args: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(value) = args.get("tags") else {
        return Ok(None);
    };
    let raw: Vec<String> = match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|tag| tag.trim().to_string())
            .collect(),
        Value::String(text) => text
            .split([',', '、', ' '])
            .map(|tag| tag.trim().to_string())
            .collect(),
        _ => return Err("tags は文字列または文字列配列で指定してください。".to_string()),
    };
    let tags: Vec<String> = raw.into_iter().filter(|tag| !tag.is_empty()).collect();
    if tags.len() > MAX_TAGS {
        return Err(format!("tags は {MAX_TAGS} 個以内で指定してください。"));
    }
    Ok(Some(tags))
}

fn next_task_id(tasks: &[Task]) -> String {
    let max = tasks
        .iter()
        .filter_map(|task| task.id.strip_prefix('t'))
        .filter_map(|number| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("t{}", max + 1)
}

fn next_note_id(notes: &[Note]) -> String {
    let max = notes
        .iter()
        .filter_map(|note| note.id.strip_prefix('n'))
        .filter_map(|number| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("n{}", max + 1)
}

fn is_overdue(task: &Task, today: NaiveDate) -> bool {
    if task.status == "done" || task.due.is_empty() {
        return false;
    }
    NaiveDate::parse_from_str(&task.due, "%Y-%m-%d")
        .map(|due| due < today)
        .unwrap_or(false)
}

fn priority_rank(priority: &str) -> u8 {
    match priority {
        "high" => 0,
        "normal" => 1,
        _ => 2,
    }
}

/// 期限の近い順 → 優先度 → ID 順。期限なしは期限ありのあとに置く。
fn sort_tasks(tasks: &mut [Task]) {
    tasks.sort_by(|a, b| {
        let due = (a.due.is_empty(), a.due.as_str()).cmp(&(b.due.is_empty(), b.due.as_str()));
        due.then(priority_rank(&a.priority).cmp(&priority_rank(&b.priority)))
            .then(a.id.cmp(&b.id))
    });
}

fn status_mark(status: &str) -> &'static str {
    match status {
        "done" => "[x]",
        "doing" => "[>]",
        _ => "[ ]",
    }
}

fn priority_label(priority: &str) -> &'static str {
    match priority {
        "high" => "高",
        "low" => "低",
        _ => "中",
    }
}

fn note_kind_label(kind: &str) -> &'static str {
    match kind {
        "decision" => "決定",
        "progress" => "進捗",
        _ => "メモ",
    }
}

fn render_task(task: &Task, today: NaiveDate) -> String {
    let mut line = format!(
        "{} {} [{}] {}",
        status_mark(&task.status),
        task.id,
        priority_label(&task.priority),
        task.title
    );
    if !task.due.is_empty() {
        line.push_str(&format!(
            "（期限 {}{}）",
            task.due,
            if is_overdue(task, today) { "・期限切れ" } else { "" }
        ));
    }
    if !task.tags.is_empty() {
        line.push_str(&format!(" #{}", task.tags.join(" #")));
    }
    line
}

fn render_tasks(tasks: &[Task], limit: usize) -> String {
    if tasks.is_empty() {
        return "該当する課題はありません。".to_string();
    }
    let today = today();
    let mut out = format!("課題 {} 件", tasks.len());
    for task in tasks.iter().take(limit) {
        out.push('\n');
        out.push_str(&render_task(task, today));
    }
    if tasks.len() > limit {
        out.push_str(&format!("\n…（{limit} 件で表示を打ち切り。limit で増やせます）"));
    }
    out
}

fn render_notes(notes: &[Note], limit: usize) -> String {
    if notes.is_empty() {
        return "記録はありません。".to_string();
    }
    let mut out = format!("記録 {} 件（新しい順）", notes.len());
    for note in notes.iter().rev().take(limit) {
        out.push_str(&format!(
            "\n{} [{}] {}: {}",
            note.id,
            note_kind_label(&note.kind),
            note.created_at,
            note.text
        ));
    }
    if notes.len() > limit {
        out.push_str(&format!("\n…（{limit} 件で表示を打ち切り。limit で増やせます）"));
    }
    out
}

fn open_counts(project: &Project, today: NaiveDate) -> (usize, usize) {
    let open = project
        .tasks
        .iter()
        .filter(|task| task.status != "done")
        .count();
    let overdue = project
        .tasks
        .iter()
        .filter(|task| is_overdue(task, today))
        .count();
    (open, overdue)
}

fn render_projects(projects: &[Project], active: &str) -> String {
    if projects.is_empty() {
        return "登録済みのプロジェクトはありません。".to_string();
    }
    let today = today();
    let mut out = format!("プロジェクト {} 件（現在: {active}）", projects.len());
    for project in projects {
        let (open, overdue) = open_counts(project, today);
        let mut line = format!(
            "\n{} {} ({}): 未完了 {open} 件",
            if project.id == active { "▶" } else { "  " },
            project.display_name(),
            project.id
        );
        if overdue > 0 {
            line.push_str(&format!("（期限切れ {overdue} 件）"));
        }
        if !project.notes.is_empty() {
            line.push_str(&format!(" / 記録 {} 件", project.notes.len()));
        }
        if !project.allowed_root.is_empty() {
            line.push_str(&format!(" / ルート {}", project.allowed_root));
        }
        out.push_str(&line);
    }
    out
}

/// system 文脈へ差し込む「いまのプロジェクト」まとめ。空文字なら差し込まない。
pub fn project_context(cfg: &Config) -> String {
    let active = active_project(cfg);
    // フォールバック許可: 文脈生成は読み取り専用の付加情報なので、
    // 読めなければ差し込まずに会話を続ける（保存は一切しないのでデータは消えない）。
    let projects = load(cfg).unwrap_or_default();
    let Some(project) = projects.into_iter().find(|project| project.id == active) else {
        return String::new();
    };
    let today = today();
    let mut open: Vec<Task> = project
        .tasks
        .iter()
        .filter(|task| task.status != "done")
        .cloned()
        .collect();
    sort_tasks(&mut open);
    if open.is_empty() && project.notes.is_empty() {
        return String::new();
    }

    let mut out = format!("現在のプロジェクト: {} ({})", project.display_name(), project.id);
    if !open.is_empty() {
        let overdue = open.iter().filter(|task| is_overdue(task, today)).count();
        out.push_str(&format!(
            "\n未完了の課題 {} 件{}:",
            open.len(),
            if overdue > 0 {
                format!("（期限切れ {overdue} 件）")
            } else {
                String::new()
            }
        ));
        for task in open.iter().take(CONTEXT_TASKS) {
            out.push_str(&format!("\n- {}", render_task(task, today)));
        }
    }
    if !project.notes.is_empty() {
        out.push_str("\n最近の記録:");
        for note in project.notes.iter().rev().take(CONTEXT_NOTES) {
            out.push_str(&format!(
                "\n- {}（{}・{}）",
                note.text,
                note_kind_label(&note.kind),
                note.created_at
            ));
        }
    }
    out.push_str("\n（保存済みの台帳から自動で添えた情報。変更するときは project スキルを呼ぶこと）");
    text::preview(&out, MAX_CONTEXT_CHARS)
}

pub struct ProjectTool;

#[async_trait]
impl Tool for ProjectTool {
    fn name(&self) -> &'static str {
        "project"
    }

    fn label(&self) -> &'static str {
        "プロジェクト管理"
    }

    fn description(&self) -> String {
        "案件ごとに、作業環境（許可ルート・AI・モデル・キャラクター）の切り替えと、課題台帳・記録（決定事項・進捗）をまとめて扱う。切り替えると課題も記録も一緒に切り替わる。いま取りかかっている一連の手順は todo_plan を使う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list", "switch", "save", "forget", "summary",
                        "task_add", "task_list", "task_update", "task_done", "task_remove",
                        "note_add", "note_list", "note_remove"
                    ],
                    "description": "save はプロジェクトの登録・更新、switch は切り替え、forget は登録解除（課題と記録も消える）。"
                },
                "project": {
                    "type": "string",
                    "description": "対象プロジェクトID。省略時は現在のプロジェクト。task_list / note_list では \"all\" で全プロジェクト。"
                },
                "name": { "type": "string", "description": "save で付ける表示名。" },
                "allowed_root": { "type": "string", "description": "save: このプロジェクトの許可ルート。" },
                "allowed_paths": { "type": "string", "description": "save: 追加許可パス（改行または ; 区切り）。" },
                "provider": { "type": "string", "description": "save: 使う AI プロバイダ。" },
                "model": { "type": "string", "description": "save: 使うモデル。" },
                "character": { "type": "string", "description": "save: 使うキャラクター。" },
                "id": { "type": "string", "description": "task_* / note_* の対象ID（t1 / n1 など）。" },
                "title": { "type": "string", "description": "課題の内容（task_add は必須）。" },
                "status": {
                    "type": "string",
                    "enum": ["todo", "doing", "done", "all"],
                    "description": "task_update での状態変更、または task_list の絞り込み。task_list の省略時は未完了のみ。"
                },
                "priority": { "type": "string", "enum": ["high", "normal", "low"], "description": "優先度（既定 normal）。" },
                "due": { "type": "string", "description": "期限 YYYY-MM-DD。空文字で期限を外す。" },
                "due_in_days": { "type": "integer", "description": "今日から何日後を期限にするか（due より優先）。" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "課題のタグ（最大5個）。" },
                "note": { "type": "string", "description": "課題の補足メモ。" },
                "tag": { "type": "string", "description": "task_list でこのタグを含むものだけに絞る。" },
                "overdue": { "type": "boolean", "description": "task_list で期限切れのものだけに絞る。" },
                "kind": {
                    "type": "string",
                    "enum": ["note", "decision", "progress"],
                    "description": "note_add の種類（既定 note）。決まったことは decision、進み具合は progress。"
                },
                "text": { "type": "string", "description": "note_add で記録する内容。" },
                "limit": { "type": "integer", "description": "一覧の表示件数（既定30、最大100）。" },
                "confirm": { "type": "boolean", "description": "forget / task_remove / note_remove の実行確認。" }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "動画制作プロジェクトへ切り替えて",
            "サムネ差し替えを来週までの課題に追加して",
            "この方針を決定事項として残して",
        ]
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![Field::new(ACTIVE_KEY, "現在のプロジェクトID", "text", "")]
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
        // 壊れていても空一覧で始めない（次の保存でプロジェクトが丸ごと消えるため）。
        let mut projects = match load(cfg) {
            Ok(projects) => projects,
            Err(error) => return format!("エラー: {error}"),
        };
        let active = active_project(cfg);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| (limit as usize).clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);
        let scope = args
            .get("project")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let all_projects = scope == "all";
        let id = target_id(cfg, &args);

        match action.as_str() {
            "list" => render_projects(&projects, &active),
            "summary" => {
                if projects.is_empty() {
                    return "登録済みのプロジェクトはありません。".to_string();
                }
                let today = today();
                let mut out = String::from("プロジェクト別の状況");
                for project in &projects {
                    let (open, overdue) = open_counts(project, today);
                    out.push_str(&format!(
                        "\n{}: 未完了 {open} / 全 {} 件{} / 記録 {} 件",
                        project.display_name(),
                        project.tasks.len(),
                        if overdue > 0 {
                            format!("（期限切れ {overdue} 件）")
                        } else {
                            String::new()
                        },
                        project.notes.len()
                    ));
                }
                out
            }
            "save" => {
                let index = match ensure_project(&mut projects, &id) {
                    Ok(index) => index,
                    Err(error) => return format!("エラー: {error}"),
                };
                let project = &mut projects[index];
                for (field, value) in [
                    ("name", args.get("name")),
                    ("allowed_root", args.get("allowed_root")),
                    ("allowed_paths", args.get("allowed_paths")),
                    ("provider", args.get("provider")),
                    ("model", args.get("model")),
                    ("character", args.get("character")),
                ] {
                    let Some(value) = value.and_then(Value::as_str) else {
                        continue;
                    };
                    let value = value.trim().to_string();
                    match field {
                        "name" => project.name = value,
                        "allowed_root" => project.allowed_root = value,
                        "allowed_paths" => project.allowed_paths = value,
                        "provider" => project.provider = value,
                        "model" => project.model = value,
                        _ => project.character = value,
                    }
                }
                let label = project.display_name().to_string();
                match save(cfg, &projects) {
                    Ok(()) => format!("OK: プロジェクトを保存しました: {label} ({id})"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "switch" => {
                let Some(project) = projects.iter().find(|project| project.id == id) else {
                    return format!(
                        "エラー: プロジェクトが見つかりません: {id}\n{}",
                        render_projects(&projects, &active)
                    );
                };
                let mut patch = Map::new();
                patch.insert(ACTIVE_KEY.to_string(), Value::String(id.clone()));
                for (field, config_key) in ENVIRONMENT {
                    let value = match field {
                        "allowed_root" => &project.allowed_root,
                        "allowed_paths" => &project.allowed_paths,
                        "provider" => &project.provider,
                        "model" => &project.model,
                        _ => &project.character,
                    };
                    if !value.is_empty() {
                        patch.insert(config_key.to_string(), Value::String(value.clone()));
                    }
                }
                let label = project.display_name().to_string();
                match cfg.set_all(patch) {
                    Ok(()) => format!(
                        "OK: {label} ({id}) へ切り替えました。\n許可ルートは次のスキル実行から反映されます。AI・モデル・キャラクターの変更は安全に読み直すため、PicoAgent を再起動すると反映されます。"
                    ),
                    Err(error) => format!("エラー: 切り替えを保存できません: {error}"),
                }
            }
            "forget" => {
                let Some(index) = projects.iter().position(|project| project.id == id) else {
                    return format!("エラー: プロジェクトが見つかりません: {id}");
                };
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    let project = &projects[index];
                    return format!(
                        "プレビュー: プロジェクト {} ({id}) の登録を解除します。課題 {} 件と記録 {} 件も一緒に消えます（ファイルは削除しません）。\n実行するには confirm=true を指定してください。",
                        project.display_name(),
                        project.tasks.len(),
                        project.notes.len()
                    );
                }
                let removed = projects.remove(index);
                let encoded = match serde_json::to_string(&projects) {
                    Ok(encoded) => encoded,
                    Err(error) => return format!("エラー: プロジェクトを保存できません: {error}"),
                };
                let mut patch = Map::new();
                patch.insert(KEY.to_string(), Value::String(encoded));
                if active == id {
                    // 使用中を消したら選択状態も外す（存在しないIDが残ると文脈が空振りする）。
                    patch.insert(ACTIVE_KEY.to_string(), Value::String(String::new()));
                }
                match cfg.set_all(patch) {
                    Ok(()) => format!(
                        "OK: プロジェクトの登録を解除しました: {} ({id})",
                        removed.display_name()
                    ),
                    Err(error) => format!("エラー: プロジェクトを保存できません: {error}"),
                }
            }
            "task_list" | "note_list" => {
                let scoped: Vec<&Project> = if all_projects {
                    projects.iter().collect()
                } else {
                    projects.iter().filter(|project| project.id == id).collect()
                };
                if scoped.is_empty() {
                    return format!("プロジェクト {id} には何も登録されていません。");
                }
                let header = if all_projects {
                    "全プロジェクト".to_string()
                } else {
                    id.clone()
                };
                if action == "note_list" {
                    let notes: Vec<Note> = scoped
                        .iter()
                        .flat_map(|project| project.notes.iter().cloned())
                        .collect();
                    return format!("対象: {header}\n{}", render_notes(&notes, limit));
                }
                let status = args
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let tag = args.get("tag").and_then(Value::as_str).unwrap_or("").trim();
                let overdue_only = args.get("overdue").and_then(Value::as_bool).unwrap_or(false);
                let today = today();
                let mut tasks: Vec<Task> = scoped
                    .iter()
                    .flat_map(|project| project.tasks.iter().cloned())
                    .filter(|task| match status.as_str() {
                        "" => task.status != "done",
                        "all" => true,
                        wanted => task.status == wanted,
                    })
                    .filter(|task| tag.is_empty() || task.tags.iter().any(|item| item == tag))
                    .filter(|task| !overdue_only || is_overdue(task, today))
                    .collect();
                sort_tasks(&mut tasks);
                format!("対象: {header}\n{}", render_tasks(&tasks, limit))
            }
            "task_add" => {
                let title = args.get("title").and_then(Value::as_str).unwrap_or("").trim();
                if title.is_empty() {
                    return "エラー: title が指定されていません。".to_string();
                }
                if all_projects {
                    return "エラー: project に \"all\" は指定できません。".to_string();
                }
                let priority = match enum_arg(&args, "priority", &PRIORITIES) {
                    Ok(priority) => priority.unwrap_or_else(|| "normal".to_string()),
                    Err(error) => return format!("エラー: {error}"),
                };
                let due = match due_arg(&args) {
                    Ok(due) => due.unwrap_or_default(),
                    Err(error) => return format!("エラー: {error}"),
                };
                let tags = match tags_arg(&args) {
                    Ok(tags) => tags.unwrap_or_default(),
                    Err(error) => return format!("エラー: {error}"),
                };
                let index = match ensure_project(&mut projects, &id) {
                    Ok(index) => index,
                    Err(error) => return format!("エラー: {error}"),
                };
                let project = &mut projects[index];
                if project.tasks.len() >= MAX_TASKS {
                    return format!(
                        "エラー: 1プロジェクトの課題は {MAX_TASKS} 件までです。完了したものを task_remove で片付けてください。"
                    );
                }
                let task = Task {
                    id: next_task_id(&project.tasks),
                    title: text::preview(title, MAX_TITLE_CHARS),
                    status: "todo".to_string(),
                    priority,
                    due,
                    tags,
                    note: text::preview(
                        args.get("note").and_then(Value::as_str).unwrap_or("").trim(),
                        MAX_NOTE_CHARS,
                    ),
                    created_at: stamp(),
                    updated_at: stamp(),
                };
                let line = render_task(&task, today());
                project.tasks.push(task);
                match save(cfg, &projects) {
                    Ok(()) => format!("OK: {id} に課題を追加しました。\n{line}"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "task_update" | "task_done" => {
                let task_id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
                if task_id.is_empty() {
                    return "エラー: id が指定されていません。".to_string();
                }
                let status = if action == "task_done" {
                    Some("done".to_string())
                } else {
                    match enum_arg(&args, "status", &STATUSES) {
                        Ok(status) => status,
                        Err(error) => return format!("エラー: {error}"),
                    }
                };
                let priority = match enum_arg(&args, "priority", &PRIORITIES) {
                    Ok(priority) => priority,
                    Err(error) => return format!("エラー: {error}"),
                };
                let due = match due_arg(&args) {
                    Ok(due) => due,
                    Err(error) => return format!("エラー: {error}"),
                };
                let tags = match tags_arg(&args) {
                    Ok(tags) => tags,
                    Err(error) => return format!("エラー: {error}"),
                };
                let Some(project) = projects.iter_mut().find(|project| project.id == id) else {
                    return format!("エラー: プロジェクトが見つかりません: {id}");
                };
                let Some(task) = project.tasks.iter_mut().find(|task| task.id == task_id) else {
                    return format!("エラー: 課題 {task_id} は {id} にありません。");
                };
                if let Some(status) = status {
                    task.status = status;
                }
                if let Some(priority) = priority {
                    task.priority = priority;
                }
                if let Some(due) = due {
                    task.due = due;
                }
                if let Some(tags) = tags {
                    task.tags = tags;
                }
                if let Some(title) = args.get("title").and_then(Value::as_str) {
                    let title = title.trim();
                    if !title.is_empty() {
                        task.title = text::preview(title, MAX_TITLE_CHARS);
                    }
                }
                if let Some(note) = args.get("note").and_then(Value::as_str) {
                    task.note = text::preview(note.trim(), MAX_NOTE_CHARS);
                }
                task.updated_at = stamp();
                let line = render_task(task, today());
                match save(cfg, &projects) {
                    Ok(()) => format!("OK: 課題を更新しました。\n{line}"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "task_remove" => {
                let task_id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
                let Some(project) = projects.iter_mut().find(|project| project.id == id) else {
                    return format!("エラー: プロジェクトが見つかりません: {id}");
                };
                let Some(index) = project.tasks.iter().position(|task| task.id == task_id) else {
                    return format!("エラー: 課題 {task_id} は {id} にありません。");
                };
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "プレビュー: 次の課題を削除します。\n{}\n実行するには confirm=true を指定してください。",
                        render_task(&project.tasks[index], today())
                    );
                }
                let removed = project.tasks.remove(index);
                match save(cfg, &projects) {
                    Ok(()) => format!("OK: 課題を削除しました: {} {}", removed.id, removed.title),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "note_add" => {
                let text_arg = args.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if text_arg.is_empty() {
                    return "エラー: text が指定されていません。".to_string();
                }
                if all_projects {
                    return "エラー: project に \"all\" は指定できません。".to_string();
                }
                let kind = match enum_arg(&args, "kind", &NOTE_KINDS) {
                    Ok(kind) => kind.unwrap_or_else(|| "note".to_string()),
                    Err(error) => return format!("エラー: {error}"),
                };
                let index = match ensure_project(&mut projects, &id) {
                    Ok(index) => index,
                    Err(error) => return format!("エラー: {error}"),
                };
                let project = &mut projects[index];
                if project.notes.len() >= MAX_NOTES {
                    // 上限で黙って古いものを消すと決定事項が失われる。消す判断はユーザーに委ねる。
                    return format!(
                        "エラー: 1プロジェクトの記録は {MAX_NOTES} 件までです。note_remove で古いものを整理してください。"
                    );
                }
                let note = Note {
                    id: next_note_id(&project.notes),
                    kind,
                    text: text::preview(text_arg, MAX_TEXT_CHARS),
                    created_at: stamp(),
                };
                let line = format!("[{}] {}", note_kind_label(&note.kind), note.text);
                project.notes.push(note);
                match save(cfg, &projects) {
                    Ok(()) => format!("OK: {id} に記録しました。\n{line}"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "note_remove" => {
                let note_id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
                let Some(project) = projects.iter_mut().find(|project| project.id == id) else {
                    return format!("エラー: プロジェクトが見つかりません: {id}");
                };
                let Some(index) = project.notes.iter().position(|note| note.id == note_id) else {
                    return format!("エラー: 記録 {note_id} は {id} にありません。");
                };
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "プレビュー: 次の記録を削除します。\n[{}] {}\n実行するには confirm=true を指定してください。",
                        note_kind_label(&project.notes[index].kind),
                        project.notes[index].text
                    );
                }
                let removed = project.notes.remove(index);
                match save(cfg, &projects) {
                    Ok(()) => format!("OK: 記録を削除しました: {} {}", removed.id, removed.text),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            other => format!("エラー: 未知の action です: {other}"),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ProjectTool) } }

#[cfg(test)]
mod tests {
    use super::{
        due_arg, is_overdue, next_note_id, next_task_id, slug, sort_tasks, tags_arg, today,
        Project, Task,
    };
    use crate::config::Config;
    use serde_json::json;

    fn task(id: &str, due: &str, priority: &str, status: &str) -> Task {
        Task {
            id: id.to_string(),
            title: format!("課題{id}"),
            status: status.to_string(),
            priority: priority.to_string(),
            due: due.to_string(),
            ..Task::default()
        }
    }

    /// 記号・空白はハイフンへ畳み、日本語はそのまま使える（英数字扱いのため）。
    #[test]
    fn 識別子は記号をハイフンに畳む() {
        assert_eq!(slug("Video Project"), "video-project");
        assert_eq!(slug("a__b"), "a-b");
        assert_eq!(slug("動画 制作"), "動画-制作");
        assert_eq!(slug("///"), "");
    }

    #[test]
    fn 期限は日付形式か相対日で指定する() {
        assert_eq!(due_arg(&json!({ "due": "2026-08-10" })).unwrap().as_deref(), Some("2026-08-10"));
        assert_eq!(due_arg(&json!({ "due": "" })).unwrap().as_deref(), Some(""));
        assert!(due_arg(&json!({ "due": "8/10" })).is_err());
        assert_eq!(due_arg(&json!({})).unwrap(), None);
        assert_eq!(
            due_arg(&json!({ "due_in_days": 0 })).unwrap().unwrap(),
            today().format("%Y-%m-%d").to_string()
        );
    }

    #[test]
    fn タグは配列でも区切り文字列でも受け取る() {
        assert_eq!(tags_arg(&json!({ "tags": ["a", "b"] })).unwrap().unwrap().len(), 2);
        assert_eq!(tags_arg(&json!({ "tags": "a, b、c" })).unwrap().unwrap().len(), 3);
        assert!(tags_arg(&json!({ "tags": ["1", "2", "3", "4", "5", "6"] })).is_err());
    }

    /// 完了済みと期限なしを期限切れに数えない（数えると「期限切れ」が常に膨らむ）。
    #[test]
    fn 期限切れの判定() {
        assert!(is_overdue(&task("t1", "2000-01-01", "normal", "todo"), today()));
        assert!(!is_overdue(&task("t2", "2000-01-01", "normal", "done"), today()));
        assert!(!is_overdue(&task("t3", "", "normal", "todo"), today()));
    }

    #[test]
    fn 期限の近い順から優先度順に並べる() {
        let mut tasks = vec![
            task("t1", "", "high", "todo"),
            task("t2", "2026-09-01", "low", "todo"),
            task("t3", "2026-08-01", "normal", "todo"),
            task("t4", "2026-08-01", "high", "todo"),
        ];
        sort_tasks(&mut tasks);
        let ids: Vec<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
        assert_eq!(ids, vec!["t4", "t3", "t2", "t1"], "並び: {ids:?}");
    }

    #[test]
    fn 連番idは既存の最大値の次を使う() {
        assert_eq!(next_task_id(&[task("t1", "", "normal", "todo"), task("t9", "", "normal", "done")]), "t10");
        assert_eq!(next_task_id(&[]), "t1");
        assert_eq!(next_note_id(&[]), "n1");
    }

    /// 旧ワークスペースのデータは引き継がない。
    ///
    /// 読んでしまうと、統合前の環境設定だけのレコードが課題ゼロのプロジェクトとして
    /// 復活し、消したはずのものが戻ってくる。
    #[test]
    fn 旧ワークスペースのデータは読まない() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_PROJECTS", "");
        cfg.set(
            "PICOAGENT_WORKSPACES",
            r#"[{"id":"video","name":"動画制作","allowed_root":"C:/work"}]"#,
        );
        assert!(super::load(&cfg).unwrap().is_empty(), "旧キーを読んでいる");

        cfg.set("PICOAGENT_ACTIVE_PROJECT", "");
        cfg.set("PICOAGENT_ACTIVE_WORKSPACE", "video");
        assert_eq!(super::active_project(&cfg), "default", "旧キーを読んでいる");
    }

    /// 項目を欠く保存データでも読めること（項目追加のたびに一覧が消えないように）。
    #[test]
    fn 項目が欠けたデータも読める() {
        let cfg = Config::new();
        cfg.set("PICOAGENT_PROJECTS", r#"[{"id":"video"}]"#);
        let projects = super::load(&cfg).expect("読めない");
        assert_eq!(projects[0].id, "video");
        assert!(projects[0].tasks.is_empty());
        assert_eq!(projects[0].display_name(), "video", "名前が空なら id を出す");
    }

    #[test]
    fn 未登録のプロジェクトは課題追加時に作られる() {
        let mut projects: Vec<Project> = Vec::new();
        let index = super::ensure_project(&mut projects, "video").unwrap();
        assert_eq!(index, 0);
        assert_eq!(projects[0].id, "video");
        // 2回目は同じものを指す（重複登録しない）。
        assert_eq!(super::ensure_project(&mut projects, "video").unwrap(), 0);
        assert_eq!(projects.len(), 1);
    }
}
