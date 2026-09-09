use std::{
    fs,
    path::PathBuf,
    sync::{Arc, atomic::{AtomicBool, Ordering}},
};

use anyhow::{Context, Result, bail};
use picoagent_character_maker::{
    config::MakerConfig,
    generation_reference::Reader,
    pipeline::{CharacterManifest, NewCharacter, PipelineContext, STAGES},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct CharacterMakerState {
    execution: tokio::sync::Mutex<()>,
    cancellation: std::sync::Mutex<Option<Arc<AtomicBool>>>,
}

impl CharacterMakerState {
    pub fn new() -> Self {
        Self {
            execution: tokio::sync::Mutex::new(()),
            cancellation: std::sync::Mutex::new(None),
        }
    }
}

fn paths(app: &AppHandle) -> Result<(PipelineContext, MakerConfig)> {
    let data = crate::log_paths::data_dir(app);
    let managed = crate::log_paths::bundled_dir(app).join("character-maker");
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
    Ok((
        PipelineContext {
            characters_root: data.join("character-maker/characters"),
            assets_root,
        },
        MakerConfig::managed(managed),
    ))
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
                    let _ = event_app.emit("character-maker:progress", progress);
                },
            )?;
            let _ = task_app.emit("character-maker:state", &manifest);
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
    if manifest.stages.get("complete").map(|stage| stage.status.as_str()) != Some("complete") {
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
    fs::copy(reader.directory.join("rig.json"), pack.join("rig2d/rig.json"))?;
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
    let version = format!("lvs-schema3-{}", &hex::encode(Sha256::digest(&rig_bytes))[..16]);
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
    let (id, name, updated) = crate::user_characters::install_or_update_generated(
        app,
        pack,
        &manifest.character_id,
    )?;
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
        let source = include_str!("character_maker.rs");
        for retired in ["mesh/generate.py", "rigging/generate.py", "facepatch", "obs"] {
            assert!(!source.contains(retired), "旧経路が残っています: {retired}");
        }
    }
}
