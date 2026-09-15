use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MakerConfig {
    pub component_root: PathBuf,
    pub python_root: PathBuf,
    pub models_root: PathBuf,
    pub completion_steps: u32,
    pub completion_seed: u32,
    pub completion_resolution: u32,
    pub completion_mask_margin: f32,
    pub completion_mask_core_ratio: f32,
    pub completion_timeout_seconds: u32,
    pub completion_fast_disk: bool,
    pub completion_hidden_prompt: String,
    pub completion_side_prompt: String,
    pub completion_hidden_band_ratio: f32,
    pub completion_hidden_motion_ratio: f32,
    pub completion_hair_edge_band_ratio: f32,
    pub completion_hair_edge_gain: f32,
    pub completion_ear_context: f32,
    pub grounding_threshold: f32,
    pub eye_context_margin: f32,
    pub sam2_points_per_batch: u32,
    pub sam2_pred_iou_threshold: f32,
    pub sam2_stability_threshold: f32,
    /// ComfyUIを`--lowvram`で起動するか。判定はアプリ側のGPU情報が正本。
    pub comfy_lowvram: bool,
    pub comfy_port: u16,
    pub comfy_startup_timeout_seconds: u32,
    pub snapshot_part_bytes: u64,
}

impl MakerConfig {
    pub fn managed(component_root: PathBuf, python_root: PathBuf) -> Self {
        let models_root = component_root.join("models");
        Self {
            component_root,
            python_root,
            models_root,
            completion_steps: 50,
            completion_seed: 777,
            completion_resolution: 1024,
            completion_mask_margin: 0.2,
            completion_mask_core_ratio: 0.5,
            completion_timeout_seconds: 14_400,
            completion_fast_disk: true,
            completion_hidden_prompt: "Complete only the small masked strips of skin hidden behind the hair, continuing the nearby skin color and shading naturally. Keep the original face, visible features, pose and rendering style unchanged. Do not add eyes, eyebrows, mouths, another face, or hair. Preserve everything outside the mask.".into(),
            completion_side_prompt: "Remove only the two side locks of hair that cover the cheeks and ears. Reveal and complete both ears and the cheek contours naturally underneath those side locks. Keep the face exactly the same size and position. Preserve the original eyes, mouth, nose, skin tone, facial expression, bangs, top hair, back hair, hair ornaments, neck, clothing and composition exactly. Maintain the original rendering style and shading. Do not make the character bald. Do not add objects.".into(),
            completion_hidden_band_ratio: 0.08,
            completion_hidden_motion_ratio: 0.35,
            completion_hair_edge_band_ratio: 0.015,
            completion_hair_edge_gain: 40.0,
            completion_ear_context: 0.5,
            grounding_threshold: 0.20,
            eye_context_margin: 0.5,
            sam2_points_per_batch: 8,
            sam2_pred_iou_threshold: 0.7,
            sam2_stability_threshold: 0.85,
            // 専用VRAM量を判定できない呼び出し（テスト・CLI）は安全側のlowvramで始める。
            comfy_lowvram: true,
            // 0はサイドカーがOSに空きポートを選ばせる。固定番号はWindowsの予約ポート範囲と衝突して起動できない。
            comfy_port: 0,
            comfy_startup_timeout_seconds: 600,
            snapshot_part_bytes: 32_000_000,
        }
    }

    pub fn python(&self) -> PathBuf {
        self.python_root.join("Scripts/python.exe")
    }

    pub fn comfy(&self) -> PathBuf {
        self.component_root.join("ComfyUI")
    }
}
