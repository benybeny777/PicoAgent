use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::{bail, Context, Result};
use picoagent_character_maker::{
    config::MakerConfig,
    generation_reference::Reader,
    pipeline::{CharacterManifest, NewCharacter, PipelineContext, STAGES},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

struct CharacterPreviewSession {
    reader: Reader,
    source_path: PathBuf,
    display_name: String,
}

pub struct CharacterMakerState {
    execution: tokio::sync::Mutex<()>,
    cancellation: std::sync::Mutex<Option<Arc<AtomicBool>>>,
    previews: std::sync::Mutex<BTreeMap<String, CharacterPreviewSession>>,
}

impl CharacterMakerState {
    pub fn new() -> Self {
        Self {
            execution: tokio::sync::Mutex::new(()),
            cancellation: std::sync::Mutex::new(None),
            previews: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.cancellation
            .lock()
            .map(|active| active.is_some())
            .unwrap_or(true)
    }
}

#[tauri::command(async)]
pub fn settings_character_maker_preview(
    app: AppHandle,
    character_id: String,
    state: State<'_, CharacterMakerState>,
) -> Result<Value, String> {
    let label = format!("character-preview-{character_id}");
    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(error_text)?;
        window.set_focus().map_err(error_text)?;
        return Ok(json!({ "ok": true, "existing": true }));
    }

    let (pipeline, config) = paths(&app).map_err(error_text)?;
    let manifest = pipeline.load_character(&character_id).map_err(error_text)?;
    if manifest
        .stages
        .get("complete")
        .map(|stage| stage.status.as_str())
        != Some("complete")
    {
        return Err("complete工程が完了していないため確認できません".into());
    }
    let source_path = pipeline
        .character_dir(&character_id)
        .map_err(error_text)?
        .join("source/input.png");
    let reader = Reader::acquire(
        source_path
            .parent()
            .and_then(|path| path.parent())
            .ok_or_else(|| "キャラクターの保存先が不正です".to_string())?,
        config.snapshot_part_bytes,
    )
    .map_err(error_text)?;
    let session = CharacterPreviewSession {
        reader,
        source_path,
        display_name: manifest.display_name.clone(),
    };
    {
        let mut previews = state
            .previews
            .lock()
            .map_err(|_| "キャラ確認画面の状態が壊れました".to_string())?;
        if let Some(stale) = previews.insert(character_id.clone(), session) {
            let _ = stale.reader.finish();
        }
    }

    let title = format!("{} - PicoAgent キャラ確認", manifest.display_name);
    let url = format!("character-preview.html?characterId={character_id}");
    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()));
    // メインと同じWebView2保存領域を使うため、起動オプションも揃える。
    if let Some(args) = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .and_then(|window| window.additional_browser_args.as_deref())
    {
        builder = builder.additional_browser_args(args);
    }
    let window = match builder
        .title(title)
        .inner_size(1180.0, 820.0)
        .min_inner_size(720.0, 560.0)
        .build()
    {
        Ok(window) => window,
        Err(error) => {
            if let Ok(mut previews) = state.previews.lock() {
                if let Some(session) = previews.remove(&character_id) {
                    let _ = session.reader.finish();
                }
            }
            return Err(error_text(error));
        }
    };
    let event_app = app.clone();
    window.on_window_event(move |event| {
        if !matches!(event, WindowEvent::Destroyed) {
            return;
        }
        let preview_state = event_app.state::<CharacterMakerState>();
        let session = preview_state
            .previews
            .lock()
            .ok()
            .and_then(|mut previews| previews.remove(&character_id));
        if let Some(session) = session {
            if let Err(error) = session.reader.finish() {
                crate::log_paths::append_app_log(
                    &event_app,
                    &format!("キャラ確認画面の世代保持を終了できませんでした: {error}"),
                );
            }
        }
    });
    Ok(json!({ "ok": true, "existing": false }))
}

#[tauri::command]
pub fn settings_character_maker_preview_data(
    character_id: String,
    state: State<'_, CharacterMakerState>,
) -> Result<Value, String> {
    let previews = state
        .previews
        .lock()
        .map_err(|_| "キャラ確認画面の状態が壊れました".to_string())?;
    let session = previews
        .get(&character_id)
        .ok_or_else(|| "キャラ確認用の公開世代がありません".to_string())?;
    Ok(json!({
        "characterId": character_id,
        "displayName": session.display_name,
        "sourcePath": session.source_path.to_string_lossy(),
        "rigPath": session.reader.directory.join("rig.json").to_string_lossy(),
        "rigRootPath": session.reader.directory.to_string_lossy(),
        "assetVersion": session.reader.generation,
    }))
}

fn paths(app: &AppHandle) -> Result<(PipelineContext, MakerConfig)> {
    let data = crate::log_paths::data_dir(app);
    let bundled = crate::log_paths::bundled_dir(app);
    let managed = bundled.join("character-maker");
    let installed_assets = app
        .path()
        .resource_dir()
        .context("キャラ作成リソース領域を取得できません")?
        .join("character-maker");
    let development_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("PicoAgentリポジトリルートを取得できません")?
        .join("character-maker");
    let assets_root = if installed_assets.join("sidecar").is_dir() {
        installed_assets
    } else {
        development_assets
    };
    let mut config = MakerConfig::managed(managed, crate::components::shared_python_dir(app));
    // 専用VRAM16GB未満・共有メモリ・VRAM不明では、ComfyUIを省メモリ設定で起動する。
    config.comfy_lowvram = crate::generation_backend::lowvram_decision(
        crate::generation_backend::generation_device(
            &crate::gpu::devices(),
            crate::components::installed_generation_runtime(app).as_ref(),
        )
        .as_ref(),
    )
    .lowvram;
    Ok((
        PipelineContext {
            characters_root: data.join("character-maker/characters"),
            assets_root,
        },
        config,
    ))
}

/// キャラ作成の再生成可能な中間物と旧公開世代を、起動時に必ず回収する。
/// 原画、manifest、現行の公開世代、導入済みキャラは削除しない。
pub fn cleanup_startup(app: &AppHandle) {
    let (pipeline, _) = match paths(app) {
        Ok(value) => value,
        Err(error) => {
            crate::log_paths::append_app_log(
                app,
                &format!("キャラ作成の起動時清掃を準備できませんでした: {error}"),
            );
            return;
        }
    };
    for warning in
        picoagent_character_maker::recovery::recover_final_rigs(&pipeline.characters_root)
    {
        crate::log_paths::append_app_log(app, &warning);
    }
    let _cleanup_guard =
        match picoagent_character_maker::recovery::startup_cleanup_guard(&pipeline.characters_root)
        {
            Ok(guard) => guard,
            Err(error) => {
                crate::log_paths::append_app_log(
                    app,
                    &format!("キャラ作成処理が使用中のため起動時清掃を見送りました: {error}"),
                );
                return;
            }
        };
    if let Err(error) = remove_regenerable_path(&pipeline.characters_root.join("staging")) {
        crate::log_paths::append_app_log(
            app,
            &format!("キャラ導入の一時領域を清掃できませんでした: {error}"),
        );
    }
    let mut characters = match pipeline.list_characters() {
        Ok(characters) => characters,
        Err(error) => {
            crate::log_paths::append_app_log(
                app,
                &format!("キャラ作成の一覧を清掃できませんでした: {error}"),
            );
            return;
        }
    };
    for manifest in &mut characters {
        let directory = match pipeline.character_dir(&manifest.character_id) {
            Ok(directory) => directory,
            Err(error) => {
                crate::log_paths::append_app_log(app, &error.to_string());
                continue;
            }
        };
        let has_publication =
            picoagent_character_maker::generation_reference::present(&directory).unwrap_or(false);
        let published = if has_publication {
            if let Err(error) =
                picoagent_character_maker::generation_reference::collect_startup(&directory)
            {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "{}の旧公開世代を清掃できませんでした: {error}",
                        manifest.display_name
                    ),
                );
                false
            } else {
                true
            }
        } else {
            false
        };
        for relative in [
            "source/isolated.png",
            "analysis",
            "layers",
            "rig2d-base",
            "rig2d",
            "completion-source",
            "completion-hidden-source",
            "completion-side-source",
            "completion-original-ears",
            "completion-generated-ears",
            "temp",
        ] {
            if let Err(error) = remove_regenerable_path(&directory.join(relative)) {
                crate::log_paths::append_app_log(
                    app,
                    &format!(
                        "{}の再生成キャッシュを清掃できませんでした: {error}",
                        manifest.display_name
                    ),
                );
            }
        }
        manifest.model.remove("rig2d_base");
        let complete = manifest
            .stages
            .get("complete")
            .map(|stage| stage.status.as_str())
            == Some("complete");
        if !published || !complete {
            let now = chrono::Utc::now().to_rfc3339();
            for stage in STAGES {
                manifest.stages.insert(
                    stage.to_string(),
                    picoagent_character_maker::pipeline::StageState {
                        status: "pending".into(),
                        message: "起動時に中間キャッシュを清掃したため最初から再生成します".into(),
                        updated_at_iso: now.clone(),
                    },
                );
            }
            manifest.model.remove("published_rig_reference");
            manifest.updated_at_iso = now;
        }
        if let Err(error) = pipeline.save_character(manifest) {
            crate::log_paths::append_app_log(
                app,
                &format!(
                    "{}の清掃後状態を保存できませんでした: {error}",
                    manifest.display_name
                ),
            );
        }
    }
}

fn remove_regenerable_path(path: &std::path::Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(path),
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn settings_character_maker_pick_source(app: AppHandle) -> Result<Value, String> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .clone()
        .file()
        .add_filter("キャラクター原画", &["png", "jpg", "jpeg"])
        .pick_file(move |picked| {
            let _ = sender.send(picked.map(|path| path.to_string()));
        });
    let picked = receiver
        .await
        .map_err(|_| "原画選択の結果を受け取れませんでした".to_string())?;
    Ok(json!({ "path": picked }))
}

#[tauri::command]
pub fn settings_character_maker_list(app: AppHandle) -> Result<Value, String> {
    let (pipeline, config) = paths(&app).map_err(error_text)?;
    let characters = pipeline.list_characters().map_err(error_text)?;
    Ok(json!({
        "supported": cfg!(all(target_os = "windows", target_arch = "x86_64")),
        "runtimeInstalled": config.python().is_file() && config.comfy().is_dir(),
        "characters": characters,
    }))
}

#[tauri::command]
pub fn settings_character_maker_create(
    app: AppHandle,
    input: NewCharacter,
) -> Result<CharacterManifest, String> {
    let (pipeline, _) = paths(&app).map_err(error_text)?;
    pipeline.create_character(input).map_err(error_text)
}

#[tauri::command]
pub async fn settings_character_maker_run(
    app: AppHandle,
    character_id: String,
    from_stage: Option<String>,
    state: State<'_, CharacterMakerState>,
) -> Result<CharacterManifest, String> {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Err("キャラ作成は現在Windows 10/11 x64専用です".into());
    }
    let _execution = state.execution.lock().await;
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = state
            .cancellation
            .lock()
            .map_err(|_| "キャラ作成の停止状態が壊れました".to_string())?;
        *active = Some(cancelled.clone());
    }
    let start = from_stage.unwrap_or_else(|| "isolate".into());
    let start_index = STAGES
        .iter()
        .position(|stage| *stage == start)
        .ok_or_else(|| format!("開始工程が不正です: {start}"))?;
    let (pipeline, config) = paths(&app).map_err(error_text)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut manifest = pipeline.load_character(&character_id)?;
        for stage in &STAGES[start_index..] {
            let event_app = task_app.clone();
            manifest = pipeline.run_stage(
                &config,
                &character_id,
                stage,
                cancelled.clone(),
                move |progress| {
                    if let Err(error) = event_app.emit("character-maker:progress", progress) {
                        crate::log_paths::append_app_log(
                            &event_app,
                            &format!("キャラ作成の進捗通知に失敗しました: {error}"),
                        );
                    }
                },
            )?;
            if let Err(error) = task_app.emit("character-maker:state", &manifest) {
                crate::log_paths::append_app_log(
                    &task_app,
                    &format!("キャラ作成の状態通知に失敗しました: {error}"),
                );
            }
        }
        Ok::<_, picoagent_character_maker::pipeline::PipelineError>(manifest)
    })
    .await
    .map_err(error_text)?
    .map_err(error_text);
    if let Ok(mut active) = state.cancellation.lock() {
        *active = None;
    }
    result
}

#[tauri::command]
pub fn settings_character_maker_cancel(
    state: State<'_, CharacterMakerState>,
) -> Result<Value, String> {
    let active = state
        .cancellation
        .lock()
        .map_err(|_| "キャラ作成の停止状態が壊れました".to_string())?;
    let Some(cancelled) = active.as_ref() else {
        return Ok(json!({ "ok": true, "running": false }));
    };
    cancelled.store(true, Ordering::Relaxed);
    Ok(json!({ "ok": true, "running": true }))
}

#[tauri::command(async)]
pub fn settings_character_maker_install(
    app: AppHandle,
    character_id: String,
) -> Result<Value, String> {
    install_completed(&app, &character_id).map_err(error_text)
}

fn install_completed(app: &AppHandle, character_id: &str) -> Result<Value> {
    let (pipeline, config) = paths(app)?;
    let manifest = pipeline.load_character(character_id)?;
    if manifest
        .stages
        .get("complete")
        .map(|stage| stage.status.as_str())
        != Some("complete")
    {
        bail!("complete工程が完了していません");
    }
    let source = pipeline.character_dir(character_id)?;
    let reader = Reader::acquire(&source, config.snapshot_part_bytes)?;
    let staging_parent = crate::log_paths::data_dir(app).join("character-maker/staging");
    fs::create_dir_all(&staging_parent)?;
    let staging = tempfile::Builder::new()
        .prefix("install-")
        .tempdir_in(staging_parent)?;
    let pack = staging.path();
    fs::create_dir_all(pack.join("rig2d/parts"))?;
    fs::copy(
        reader.directory.join("rig.json"),
        pack.join("rig2d/rig.json"),
    )?;
    for entry in fs::read_dir(reader.directory.join("parts"))? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if !name_text.ends_with(".png") || name_text.contains('/') || name_text.contains('\\') {
            bail!("公開世代の素材名が不正です: {name_text}");
        }
        fs::copy(entry.path(), pack.join("rig2d/parts").join(name))?;
    }
    fs::copy(source.join("source/input.png"), pack.join("preview.png"))?;
    let rig_bytes = fs::read(pack.join("rig2d/rig.json"))?;
    let rig_digest = hex::encode(Sha256::digest(&rig_bytes));
    let short_digest = rig_digest.chars().take(16).collect::<String>();
    let version = format!("lvs-schema3-{short_digest}");
    crate::store::write_json_atomic(
        &pack.join("character.json"),
        &json!({
            "id": manifest.character_id,
            "name": manifest.display_name,
            "personality": manifest.persona_prompt,
            "sourceFraming": "native-full",
            "assetVersion": version,
            "characterMaker": {
                "sourceCharacterId": manifest.character_id,
                "sourceSha256": manifest.source_sha256,
                "publishedGeneration": reader.generation,
            },
            "meshRig": { "rig": "rig2d/rig.json" }
        }),
    )?;
    reader.finish()?;
    let (id, name, updated) =
        crate::user_characters::install_or_update_generated(app, pack, &manifest.character_id)?;
    Ok(json!({ "ok": true, "id": id, "name": name, "updated": updated }))
}

fn error_text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_are_the_current_four_stage_pipeline() {
        assert_eq!(STAGES, ["isolate", "decompose", "rig2d", "complete"]);
    }

    #[test]
    fn source_does_not_reference_the_retired_product_paths() {
        let source = include_str!("../../character-maker/src/pipeline.rs");
        for retired in [
            "mesh/generate.py",
            "rigging/generate.py",
            "facepatch",
            "obs",
        ] {
            assert!(!source.contains(retired), "旧経路が残っています: {retired}");
        }
    }
}
