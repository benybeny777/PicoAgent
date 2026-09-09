use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

use image::ImageFormat;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    config::MakerConfig,
    sidecar::{SidecarError, SidecarProcess},
    store,
};

pub const STAGES: [&str; 4] = ["isolate", "decompose", "rig2d", "complete"];

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("ファイル操作に失敗しました: {0}")]
    Io(#[from] std::io::Error),
    #[error("状態JSONが不正です: {0}")]
    Json(#[from] serde_json::Error),
    #[error("画像を読み込めません: {0}")]
    Image(#[from] image::ImageError),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sidecar(#[from] SidecarError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageState {
    pub status: String,
    pub message: String,
    pub updated_at_iso: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterManifest {
    pub schema_version: u32,
    pub character_id: String,
    pub display_name: String,
    pub created_at_iso: String,
    pub updated_at_iso: String,
    pub persona_prompt: String,
    pub identity_tags: String,
    pub source_sha256: String,
    pub model: BTreeMap<String, String>,
    pub stages: BTreeMap<String, StageState>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCharacter {
    pub display_name: String,
    pub source_path: String,
    #[serde(default)]
    pub persona_prompt: String,
    #[serde(default)]
    pub identity_tags: String,
}

#[derive(Clone)]
pub struct PipelineContext {
    pub characters_root: PathBuf,
    pub assets_root: PathBuf,
}

impl PipelineContext {
    pub fn character_dir(&self, id: &str) -> Result<PathBuf, PipelineError> {
        validate_id(id)?;
        Ok(self.characters_root.join(id))
    }

    pub fn create_character(&self, input: NewCharacter) -> Result<CharacterManifest, PipelineError> {
        if input.display_name.trim().is_empty() {
            return Err(PipelineError::Invalid("表示名を入力してください".into()));
        }
        let source = PathBuf::from(&input.source_path);
        if !source.is_file() {
            return Err(PipelineError::Invalid(format!(
                "入力画像が見つかりません: {}",
                source.display()
            )));
        }
        let source_bytes = fs::read(&source)?;
        let decoded = image::load_from_memory(&source_bytes)?;
        let mut png = Cursor::new(Vec::new());
        decoded.write_to(&mut png, ImageFormat::Png)?;
        let source_sha256 = format!("{:x}", Sha256::digest(png.get_ref()));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = format!(
            "c_{}",
            &format!(
                "{:x}",
                Sha256::digest(format!("{source_sha256}:{nonce}").as_bytes())
            )[..12]
        );
        let directory = self.character_dir(&id)?;
        if directory.exists() {
            return Err(PipelineError::Invalid("生成IDが既に存在します".into()));
        }
        store::write_bytes_atomic(&directory.join("source/input.png"), png.get_ref())?;
        let now = now_iso();
        let stages = STAGES
            .iter()
            .map(|stage| {
                (
                    (*stage).to_string(),
                    StageState {
                        status: "pending".into(),
                        message: String::new(),
                        updated_at_iso: now.clone(),
                    },
                )
            })
            .collect();
        let manifest = CharacterManifest {
            schema_version: 1,
            character_id: id,
            display_name: input.display_name.trim().to_owned(),
            created_at_iso: now.clone(),
            updated_at_iso: now,
            persona_prompt: input.persona_prompt,
            identity_tags: input.identity_tags,
            source_sha256,
            model: BTreeMap::new(),
            stages,
        };
        self.save_character(&manifest)?;
        Ok(manifest)
    }

    pub fn list_characters(&self) -> Result<Vec<CharacterManifest>, PipelineError> {
        if !self.characters_root.exists() {
            return Ok(Vec::new());
        }
        let mut characters = Vec::new();
        for entry in fs::read_dir(&self.characters_root)? {
            let path = entry?.path().join("character.json");
            if path.is_file() {
                characters.push(self.load_character_file(&path)?);
            }
        }
        characters.sort_by(|left, right| right.updated_at_iso.cmp(&left.updated_at_iso));
        Ok(characters)
    }

    pub fn load_character(&self, id: &str) -> Result<CharacterManifest, PipelineError> {
        self.load_character_file(&self.character_dir(id)?.join("character.json"))
    }

    pub fn save_character(&self, manifest: &CharacterManifest) -> Result<(), PipelineError> {
        validate_id(&manifest.character_id)?;
        store::write_json_atomic(
            &self
                .character_dir(&manifest.character_id)?
                .join("character.json"),
            manifest,
        )?;
        Ok(())
    }

    pub fn run_stage(
        &self,
        config: &MakerConfig,
        id: &str,
        stage: &str,
        cancelled: Arc<AtomicBool>,
        mut relay: impl FnMut(serde_json::Value),
    ) -> Result<CharacterManifest, PipelineError> {
        let stage_index = STAGES
            .iter()
            .position(|candidate| *candidate == stage)
            .ok_or_else(|| PipelineError::Invalid(format!("未知の工程です: {stage}")))?;
        let mut manifest = self.load_character(id)?;
        if stage_index > 0 {
            let previous = STAGES[stage_index - 1];
            if manifest.stages.get(previous).map(|value| value.status.as_str()) != Some("complete")
            {
                return Err(PipelineError::Invalid(format!(
                    "先に{previous}工程を完了してください"
                )));
            }
        }
        self.invalidate_from(&mut manifest, stage_index)?;
        set_stage(&mut manifest, stage, "running", "実行中");
        self.save_character(&manifest)?;
        let result = match stage {
            "isolate" => self.run_isolate(config, &manifest, cancelled, &mut relay),
            "decompose" => self.run_decompose(config, &manifest, cancelled, &mut relay),
            "rig2d" => self.run_rig2d(config, &manifest, cancelled, &mut relay),
            "complete" => self.run_completion(config, &manifest, cancelled, &mut relay),
            _ => unreachable!(),
        };
        match result {
            Ok(message) => {
                set_stage(&mut manifest, stage, "complete", &message);
                if stage == "rig2d" || stage == "complete" {
                    manifest
                        .model
                        .insert("rig2d_base".into(), "rig2d-base/rig.json".into());
                }
                if stage == "complete" {
                    manifest
                        .model
                        .insert("published_rig_reference".into(), "rig-current.json".into());
                }
                self.save_character(&manifest)?;
                Ok(manifest)
            }
            Err(error) => {
                set_stage(&mut manifest, stage, "failed", &error.to_string());
                self.save_character(&manifest)?;
                Err(error)
            }
        }
    }

    fn load_character_file(&self, path: &Path) -> Result<CharacterManifest, PipelineError> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(PipelineError::from)
    }

    fn invalidate_from(
        &self,
        manifest: &mut CharacterManifest,
        start: usize,
    ) -> Result<(), PipelineError> {
        let directory = self.character_dir(&manifest.character_id)?;
        for stage in &STAGES[start..] {
            set_stage(manifest, stage, "pending", "上流工程の再実行により未実行へ戻しました");
        }
        for relative in match STAGES[start] {
            "isolate" => &["source/isolated.png", "analysis", "layers", "rig2d-base"] as &[&str],
            "decompose" => &["analysis", "layers", "rig2d-base"],
            "rig2d" => &["rig2d-base"],
            "complete" => &[],
            _ => unreachable!(),
        } {
            remove_path(&directory.join(relative))?;
        }
        if start <= 2 {
            manifest.model.remove("rig2d_base");
        }
        if start <= 3 {
            manifest.model.remove("published_rig_reference");
        }
        Ok(())
    }

    fn run_isolate(
        &self,
        config: &MakerConfig,
        manifest: &CharacterManifest,
        cancelled: Arc<AtomicBool>,
        relay: &mut impl FnMut(serde_json::Value),
    ) -> Result<String, PipelineError> {
        let directory = self.character_dir(&manifest.character_id)?;
        self.run_sidecar(
            config,
            vec![
                self.assets_root.join("sidecar/isolate/generate.py").into_os_string(),
                "--input".into(),
                directory.join("source/input.png").into_os_string(),
                "--output".into(),
                directory.join("source/isolated.png").into_os_string(),
                "--model".into(),
                config.models_root.join("rembg/isnetis.onnx").into_os_string(),
            ],
            cancelled,
            relay,
        )?;
        Ok("元キャンバスを保った背景除去が完了しました".into())
    }

    fn run_decompose(
        &self,
        config: &MakerConfig,
        manifest: &CharacterManifest,
        cancelled: Arc<AtomicBool>,
        relay: &mut impl FnMut(serde_json::Value),
    ) -> Result<String, PipelineError> {
        let directory = self.character_dir(&manifest.character_id)?;
        self.run_sidecar(
            config,
            vec![
                self.assets_root.join("sidecar/decompose/generate.py").into_os_string(),
                "--grounding-model".into(),
                config.models_root.join("grounding-dino-base").into_os_string(),
                "--grounding-threshold".into(),
                config.grounding_threshold.to_string().into(),
                "--eye-context-margin".into(),
                config.eye_context_margin.to_string().into(),
                "--input".into(),
                directory.join("source/isolated.png").into_os_string(),
                "--output".into(),
                directory.join("layers").into_os_string(),
                "--keep-candidates".into(),
                "--points-per-batch".into(),
                config.sam2_points_per_batch.to_string().into(),
                "--pred-iou-threshold".into(),
                config.sam2_pred_iou_threshold.to_string().into(),
                "--stability-threshold".into(),
                config.sam2_stability_threshold.to_string().into(),
                "--model".into(),
                config.models_root.join("sam2.1-hiera-tiny").into_os_string(),
            ],
            cancelled,
            relay,
        )?;
        Ok("意味解析と原寸素材分離が完了しました".into())
    }

    fn run_rig2d(
        &self,
        config: &MakerConfig,
        manifest: &CharacterManifest,
        cancelled: Arc<AtomicBool>,
        relay: &mut impl FnMut(serde_json::Value),
    ) -> Result<String, PipelineError> {
        let directory = self.character_dir(&manifest.character_id)?;
        self.run_sidecar(
            config,
            vec![
                self.assets_root.join("sidecar/rig2d/generate.py").into_os_string(),
                "--manifest".into(),
                directory.join("layers/manifest.json").into_os_string(),
                "--output".into(),
                directory.join("rig2d-base/rig.json").into_os_string(),
            ],
            cancelled,
            relay,
        )?;
        Ok("lvs-anime25d-v1リグの骨格を生成しました".into())
    }

    fn run_completion(
        &self,
        config: &MakerConfig,
        manifest: &CharacterManifest,
        cancelled: Arc<AtomicBool>,
        relay: &mut impl FnMut(serde_json::Value),
    ) -> Result<String, PipelineError> {
        let directory = self.character_dir(&manifest.character_id)?;
        let mut args: Vec<OsString> = vec![
            self.assets_root.join("sidecar/completion/generate.py").into_os_string(),
            "--character".into(), directory.clone().into_os_string(),
            "--base-rig".into(), directory.join("rig2d-base/rig.json").into_os_string(),
            "--output".into(), directory.join("rig2d").into_os_string(),
            "--comfy".into(), config.comfy().into_os_string(),
            "--models".into(), config.models_root.join("qwen-eval").into_os_string(),
            "--workflow".into(), self.assets_root.join("workflows/qwen-edit-api.json").into_os_string(),
            "--overlay".into(), self.assets_root.join("workflows/qwen-edit-eyes-overlay.json").into_os_string(),
            "--steps".into(), config.completion_steps.to_string().into(),
            "--seed".into(), config.completion_seed.to_string().into(),
            "--resolution".into(), config.completion_resolution.to_string().into(),
            "--mask-margin-ratio".into(), config.completion_mask_margin.to_string().into(),
            "--mask-core-ratio".into(), config.completion_mask_core_ratio.to_string().into(),
            "--port".into(), config.comfy_port.to_string().into(),
            "--startup-timeout".into(), config.comfy_startup_timeout_seconds.to_string().into(),
            "--generation-timeout".into(), config.completion_timeout_seconds.to_string().into(),
        ];
        if config.completion_fast_disk {
            args.push("--fast-disk".into());
        }
        for (flag, value) in [
            ("--hidden-prompt", config.completion_hidden_prompt.clone()),
            ("--side-prompt", config.completion_side_prompt.clone()),
            ("--hidden-band-ratio", config.completion_hidden_band_ratio.to_string()),
            ("--hidden-motion-ratio", config.completion_hidden_motion_ratio.to_string()),
            ("--hair-edge-band-ratio", config.completion_hair_edge_band_ratio.to_string()),
            ("--hair-edge-gain", config.completion_hair_edge_gain.to_string()),
            ("--ear-context", config.completion_ear_context.to_string()),
            ("--grounding-threshold", config.grounding_threshold.to_string()),
        ] {
            args.push(flag.into());
            args.push(value.into());
        }
        args.extend([
            "--grounding-model".into(),
            config.models_root.join("grounding-dino-base").into_os_string(),
            "--sam-model".into(),
            config.models_root.join("sam2.1-hiera-tiny").into_os_string(),
        ]);
        self.run_sidecar(config, args, cancelled, relay)?;
        let reader = crate::generation_reference::Reader::acquire(
            &directory,
            config.snapshot_part_bytes,
        )?;
        let rig: serde_json::Value = serde_json::from_reader(fs::File::open(
            reader.directory.join("rig.json"),
        )?)?;
        reader.finish()?;
        if rig["schema_version"].as_u64() != Some(3)
            || rig["profile"].as_str() != Some("lvs-anime25d-v1")
        {
            return Err(PipelineError::Invalid(
                "公開リグがlvs-anime25d-v1 schema 3ではありません".into(),
            ));
        }
        Ok("局所補完済みの不変世代を公開しました".into())
    }

    fn run_sidecar(
        &self,
        config: &MakerConfig,
        arguments: Vec<OsString>,
        cancelled: Arc<AtomicBool>,
        relay: &mut impl FnMut(serde_json::Value),
    ) -> Result<(), PipelineError> {
        let python = config.python();
        if !python.is_file() {
            return Err(PipelineError::Invalid(
                "キャラ作成ランタイムがありません。追加機能から導入してください".into(),
            ));
        }
        SidecarProcess::spawn(&python, &arguments, &self.assets_root)?
            .relay_json_lines(cancelled, relay)?;
        Ok(())
    }
}

fn set_stage(manifest: &mut CharacterManifest, stage: &str, status: &str, message: &str) {
    manifest.stages.insert(
        stage.to_owned(),
        StageState {
            status: status.to_owned(),
            message: message.to_owned(),
            updated_at_iso: now_iso(),
        },
    );
    manifest.updated_at_iso = now_iso();
}

fn validate_id(id: &str) -> Result<(), PipelineError> {
    if id.len() < 3
        || id.len() > 64
        || !id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-' || value == b'_')
    {
        return Err(PipelineError::Invalid(format!("キャラクターIDが不正です: {id}")));
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), PipelineError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn create_preserves_pixels_as_png_and_initializes_four_stages() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.jpg");
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .save(&source)
            .unwrap();
        let context = PipelineContext {
            characters_root: root.path().join("characters"),
            assets_root: root.path().join("assets"),
        };
        let character = context
            .create_character(NewCharacter {
                display_name: "テスト".into(),
                source_path: source.display().to_string(),
                persona_prompt: "人格".into(),
                identity_tags: "青髪".into(),
            })
            .unwrap();
        assert_eq!(character.stages.len(), 4);
        assert!(character.stages.values().all(|state| state.status == "pending"));
        let input = context.character_dir(&character.character_id).unwrap().join("source/input.png");
        assert_eq!(image::open(input).unwrap().dimensions(), (8, 8));
    }
}
