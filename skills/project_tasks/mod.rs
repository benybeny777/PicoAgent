// skills/project_tasks/mod.rs - プロジェクト単位の課題台帳スキル。
//
// `todo_plan` が「いま取りかかっている一連の手順」を1本だけ持つのに対し、こちらは
// プロジェクトをまたいで**残り続ける課題**を持つ。期限・優先度・タグで絞り込み、
// 「動画制作の残りは何だっけ」「今週締切のものは」に答えられるようにする。
//
// 対象プロジェクトを省略したときは `project_workspace` で選択中のワークスペースを使う。
// プロジェクトの切り替えと課題台帳が同じ名前で結びつくようにするため。
//
// 現在の課題と記録は `project_context()` が短くまとめ、`commands.rs` が会話の
// system 文脈へ差し込む（切り替えたプロジェクトの状況を毎回説明しなくて済むように）。

use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::text;

pub const KEY: &str = "PICOAGENT_PROJECT_TASKS";
const ACTIVE_WORKSPACE_KEY: &str = "PICOAGENT_ACTIVE_WORKSPACE";
const DEFAULT_PROJECT: &str = "default";
const MAX_TASKS: usize = 200;
const MAX_TITLE_CHARS: usize = 120;
const MAX_NOTE_CHARS: usize = 400;
const MAX_TAGS: usize = 5;
const DEFAULT_LIMIT: usize = 30;
const MAX_LIMIT: usize = 100;
/// system 文脈へ差し込むまとめの長さ上限（会話の文脈を食い潰さないため）。
const MAX_CONTEXT_CHARS: usize = 1200;

const STATUSES: [&str; 3] = ["todo", "doing", "done"];
const PRIORITIES: [&str; 3] = ["high", "normal", "low"];

#[derive(Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: String,
    pub project: String,
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

/// 選択中のプロジェクト（`project_workspace` のアクティブID）。未設定なら `default`。
pub fn active_project(cfg: &Config) -> String {
    let active = cfg.get(ACTIVE_WORKSPACE_KEY, "");
    let active = active.trim();
    if active.is_empty() {
        DEFAULT_PROJECT.to_string()
    } else {
        active.to_string()
    }
}

pub fn load(cfg: &Config) -> Result<Vec<Task>, String> {
    crate::store::load_list(cfg, KEY)
}

fn save(cfg: &Config, tasks: &[Task]) -> Result<(), String> {
    let encoded = serde_json::to_string(tasks)
        .map_err(|error| format!("課題を保存用に変換できません: {error}"))?;
    cfg.set_string(KEY, encoded)
        .map_err(|error| format!("課題を保存できません: {error}"))
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn now_stamp() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// `due` は `YYYY-MM-DD`。`due_in_days` が来たら今日からの相対日で組み立てる。
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

fn next_id(tasks: &[Task]) -> String {
    let max = tasks
        .iter()
        .filter_map(|task| task.id.strip_prefix('t'))
        .filter_map(|number| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("t{}", max + 1)
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

fn render_list(tasks: &[Task], limit: usize) -> String {
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

fn render_summary(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "登録されている課題はありません。".to_string();
    }
    let today = today();
    let mut projects: Vec<&str> = tasks.iter().map(|task| task.project.as_str()).collect();
    projects.sort_unstable();
    projects.dedup();
    let mut out = String::from("プロジェクト別の状況");
    for project in projects {
        let scoped: Vec<&Task> = tasks.iter().filter(|task| task.project == project).collect();
        let open = scoped.iter().filter(|task| task.status != "done").count();
        let overdue = scoped
            .iter()
            .filter(|task| is_overdue(task, today))
            .count();
        out.push_str(&format!(
            "\n{project}: 未完了 {open} / 全 {} 件{}",
            scoped.len(),
            if overdue > 0 {
                format!("（期限切れ {overdue} 件）")
            } else {
                String::new()
            }
        ));
    }
    out
}

/// system 文脈へ差し込む「いまのプロジェクト」まとめ。空文字なら差し込まない。
pub fn project_context(cfg: &Config) -> String {
    let project = active_project(cfg);
    // フォールバック許可: 文脈生成は読み取り専用の付加情報なので、
    // 読めなければ差し込まずに会話を続ける（保存は一切しないのでデータは消えない）。
    let tasks = load(cfg).unwrap_or_default();
    let today = today();
    let mut open: Vec<Task> = tasks
        .into_iter()
        .filter(|task| task.project == project && task.status != "done")
        .collect();
    sort_tasks(&mut open);
    let notes = super::project_notes::recent_lines(cfg, &project);
    if open.is_empty() && notes.is_empty() {
        return String::new();
    }

    let mut out = format!("現在のプロジェクト: {project}");
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
        for task in open.iter().take(5) {
            out.push_str(&format!("\n- {}", render_task(task, today)));
        }
    }
    if !notes.is_empty() {
        out.push_str("\n最近の記録:");
        for line in notes {
            out.push_str(&format!("\n- {line}"));
        }
    }
    out.push_str("\n（保存済みの台帳から自動で添えた情報。変更するときは project_tasks / project_notes を呼ぶこと）");
    text::preview(&out, MAX_CONTEXT_CHARS)
}

pub struct ProjectTasksTool;

#[async_trait]
impl Tool for ProjectTasksTool {
    fn name(&self) -> &'static str {
        "project_tasks"
    }

    fn label(&self) -> &'static str {
        "プロジェクト課題管理"
    }

    fn description(&self) -> String {
        "プロジェクトごとの課題を登録・一覧・更新・完了・削除する。期限、優先度、タグで絞り込める。長く残る「やること」の台帳で、いま進めている一連の手順は todo_plan を使う。"
            .to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "update", "done", "remove", "summary"],
                    "description": "summary はプロジェクト別の件数と期限切れの集計。"
                },
                "project": {
                    "type": "string",
                    "description": "対象プロジェクト。省略時は選択中のワークスペース。list では \"all\" で全プロジェクト。"
                },
                "id": { "type": "string", "description": "update / done / remove の対象ID（t1 など）。" },
                "title": { "type": "string", "description": "課題の内容（add は必須、update で変更可）。" },
                "status": {
                    "type": "string",
                    "enum": ["todo", "doing", "done", "all"],
                    "description": "update での状態変更、または list での絞り込み。list の省略時は未完了のみ。"
                },
                "priority": { "type": "string", "enum": ["high", "normal", "low"], "description": "優先度（既定 normal）。" },
                "due": { "type": "string", "description": "期限 YYYY-MM-DD。空文字で期限を外す。" },
                "due_in_days": { "type": "integer", "description": "今日から何日後を期限にするか（due より優先）。" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "タグ（最大5個）。" },
                "note": { "type": "string", "description": "補足メモ。" },
                "tag": { "type": "string", "description": "list でこのタグを含むものだけに絞る。" },
                "overdue": { "type": "boolean", "description": "list で期限切れのものだけに絞る。" },
                "limit": { "type": "integer", "description": "list の表示件数（既定30、最大100）。" },
                "confirm": { "type": "boolean", "description": "remove の実行確認。" }
            },
            "required": ["action"]
        })
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "このプロジェクトの残りの課題を教えて",
            "サムネ差し替えを来週までの課題に追加して",
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
        // 壊れていても空一覧で始めない（次の保存で台帳が丸ごと消えるため）。
        let mut tasks = match load(cfg) {
            Ok(tasks) => tasks,
            Err(error) => return format!("エラー: {error}"),
        };

        match action.as_str() {
            "summary" => render_summary(&tasks),
            "list" => {
                let project = args
                    .get("project")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|project| !project.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| active_project(cfg));
                let status = args
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_lowercase();
                let tag = args.get("tag").and_then(Value::as_str).unwrap_or("").trim();
                let overdue_only = args.get("overdue").and_then(Value::as_bool).unwrap_or(false);
                let today = today();
                let mut filtered: Vec<Task> = tasks
                    .into_iter()
                    .filter(|task| project == "all" || task.project == project)
                    .filter(|task| match status.as_str() {
                        "" => task.status != "done",
                        "all" => true,
                        wanted => task.status == wanted,
                    })
                    .filter(|task| tag.is_empty() || task.tags.iter().any(|item| item == tag))
                    .filter(|task| !overdue_only || is_overdue(task, today))
                    .collect();
                sort_tasks(&mut filtered);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|limit| (limit as usize).clamp(1, MAX_LIMIT))
                    .unwrap_or(DEFAULT_LIMIT);
                format!(
                    "対象: {}\n{}",
                    if project == "all" { "全プロジェクト".to_string() } else { project },
                    render_list(&filtered, limit)
                )
            }
            "add" => {
                if tasks.len() >= MAX_TASKS {
                    return format!(
                        "エラー: 課題は {MAX_TASKS} 件までです。完了したものを remove で片付けてください。"
                    );
                }
                let title = args.get("title").and_then(Value::as_str).unwrap_or("").trim();
                if title.is_empty() {
                    return "エラー: title が指定されていません。".to_string();
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
                let project = args
                    .get("project")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|project| !project.is_empty() && *project != "all")
                    .map(str::to_string)
                    .unwrap_or_else(|| active_project(cfg));
                let task = Task {
                    id: next_id(&tasks),
                    project,
                    title: text::preview(title, MAX_TITLE_CHARS),
                    status: "todo".to_string(),
                    priority,
                    due,
                    tags,
                    note: text::preview(
                        args.get("note").and_then(Value::as_str).unwrap_or("").trim(),
                        MAX_NOTE_CHARS,
                    ),
                    created_at: now_stamp(),
                    updated_at: now_stamp(),
                };
                let line = render_task(&task, today());
                let project = task.project.clone();
                tasks.push(task);
                match save(cfg, &tasks) {
                    Ok(()) => format!("OK: {project} に課題を追加しました。\n{line}"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "update" | "done" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
                if id.is_empty() {
                    return format!("エラー: id が指定されていません。\n{}", render_list(&tasks, 10));
                }
                let status = if action == "done" {
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
                let Some(task) = tasks.iter_mut().find(|task| task.id == id) else {
                    return format!("エラー: 課題 {id} は見つかりません。");
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
                if let Some(project) = args.get("project").and_then(Value::as_str) {
                    let project = project.trim();
                    if !project.is_empty() && project != "all" {
                        task.project = project.to_string();
                    }
                }
                task.updated_at = now_stamp();
                let line = render_task(task, today());
                match save(cfg, &tasks) {
                    Ok(()) => format!("OK: 課題を更新しました。\n{line}"),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            "remove" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
                let Some(index) = tasks.iter().position(|task| task.id == id) else {
                    return format!("エラー: 課題 {id} は見つかりません。");
                };
                if !args.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
                    return format!(
                        "プレビュー: 次の課題を削除します。\n{}\n実行するには confirm=true を指定してください。",
                        render_task(&tasks[index], today())
                    );
                }
                let removed = tasks.remove(index);
                match save(cfg, &tasks) {
                    Ok(()) => format!("OK: 課題を削除しました: {} {}", removed.id, removed.title),
                    Err(error) => format!("エラー: {error}"),
                }
            }
            other => format!(
                "エラー: action は add/list/update/done/remove/summary のいずれかを指定してください: {other}"
            ),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(ProjectTasksTool) } }

#[cfg(test)]
mod tests {
    use super::{
        due_arg, is_overdue, next_id, render_summary, sort_tasks, tags_arg, today, Task,
    };
    use serde_json::json;

    fn task(id: &str, project: &str, due: &str, priority: &str, status: &str) -> Task {
        Task {
            id: id.to_string(),
            project: project.to_string(),
            title: format!("課題{id}"),
            status: status.to_string(),
            priority: priority.to_string(),
            due: due.to_string(),
            tags: Vec::new(),
            note: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn 期限は日付形式か相対日で指定する() {
        assert_eq!(
            due_arg(&json!({ "due": "2026-08-10" })).unwrap().as_deref(),
            Some("2026-08-10")
        );
        // 空文字は「期限を外す」意味。
        assert_eq!(due_arg(&json!({ "due": "" })).unwrap().as_deref(), Some(""));
        assert!(due_arg(&json!({ "due": "8/10" })).is_err());
        assert_eq!(due_arg(&json!({})).unwrap(), None);
        let relative = due_arg(&json!({ "due_in_days": 0 })).unwrap().unwrap();
        assert_eq!(relative, today().format("%Y-%m-%d").to_string());
    }

    #[test]
    fn タグは配列でも区切り文字列でも受け取る() {
        assert_eq!(tags_arg(&json!({ "tags": ["a", "b"] })).unwrap().unwrap().len(), 2);
        assert_eq!(tags_arg(&json!({ "tags": "a, b、c" })).unwrap().unwrap().len(), 3);
        assert!(tags_arg(&json!({ "tags": ["1", "2", "3", "4", "5", "6"] })).is_err());
        assert_eq!(tags_arg(&json!({})).unwrap(), None);
    }

    /// 完了済みと期限なしを期限切れに数えない（数えると「期限切れ」が常に膨らむ）。
    #[test]
    fn 期限切れの判定() {
        let past = "2000-01-01";
        assert!(is_overdue(&task("t1", "p", past, "normal", "todo"), today()));
        assert!(!is_overdue(&task("t2", "p", past, "normal", "done"), today()));
        assert!(!is_overdue(&task("t3", "p", "", "normal", "todo"), today()));
        assert!(!is_overdue(&task("t4", "p", "2999-01-01", "normal", "todo"), today()));
    }

    #[test]
    fn 期限の近い順から優先度順に並べる() {
        let mut tasks = vec![
            task("t1", "p", "", "high", "todo"),
            task("t2", "p", "2026-09-01", "low", "todo"),
            task("t3", "p", "2026-08-01", "normal", "todo"),
            task("t4", "p", "2026-08-01", "high", "todo"),
        ];
        sort_tasks(&mut tasks);
        let ids: Vec<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
        assert_eq!(ids, vec!["t4", "t3", "t2", "t1"], "並び: {ids:?}");
    }

    #[test]
    fn 連番idは既存の最大値の次を使う() {
        let tasks = vec![task("t1", "p", "", "normal", "todo"), task("t9", "p", "", "normal", "done")];
        assert_eq!(next_id(&tasks), "t10");
        assert_eq!(next_id(&[]), "t1");
    }

    #[test]
    fn プロジェクト別に未完了と期限切れを集計する() {
        let tasks = vec![
            task("t1", "a", "2000-01-01", "normal", "todo"),
            task("t2", "a", "", "normal", "done"),
            task("t3", "b", "", "normal", "doing"),
        ];
        let summary = render_summary(&tasks);
        assert!(summary.contains("a: 未完了 1 / 全 2 件（期限切れ 1 件）"), "{summary}");
        assert!(summary.contains("b: 未完了 1 / 全 1 件"), "{summary}");
    }
}
