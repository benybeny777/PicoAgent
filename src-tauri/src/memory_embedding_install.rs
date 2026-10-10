//! 標準記憶検索の専用環境を共通Python上へ導入する。
use anyhow::Result;
use std::path::PathBuf;
use tauri::AppHandle;

const REVISION: &str = "914f7f89142e33e77833254d9c9b90c3cef7303b";
const MODEL_FILES: &[(&str, u64)] = &[
    ("1_Pooling/config.json", 90),
    ("2_Normalize/config.json", 97),
    ("config.json", 4455),
    ("config_sentence_transformers.json", 1565),
    ("modules.json", 413),
    ("sentence_bert_config.json", 747),
    ("tokenizer_config.json", 1599),
    ("tokenizer.json", 32170510),
    ("tokenizer.model", 4689013),
    ("model.safetensors", 1488915288),
    ("preprocessor_config.json", 511),
    ("processor_config.json", 1788),
    ("chat_template.jinja", 1016),
];

pub async fn ensure(app: &AppHandle, requested: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        matches!(requested, "gpu" | "cpu"),
        "記憶検索の実行先が不正です"
    );
    let _operation = crate::components::COMPONENT_OPERATION_LOCK.lock().await;
    let root = crate::app_paths::bundled_dir(app).join("embedding-memory");
    let python = crate::components::python_executable(&root.join("venv"));
    let backend = if requested == "cpu" {
        "cpu"
    } else if crate::components::nvidia_cuda13_ready().await {
        "cuda"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "mps"
    } else {
        // フォールバック許可: CUDA非対応構成では正式対応するCPU版だけを取得する。（根拠: AGENTS.md 依存と同梱物の取得）
        "cpu"
    };
    let marker_path = root.join("install.json");
    let complete = MODEL_FILES.iter().all(|(name, size)| {
        std::fs::metadata(root.join("model").join(name))
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == *size)
    });
    if marker_path.is_file() && python.is_file() && complete {
        // 破損した導入記録の読取失敗は、後続PR #478で理由付きのエラーへ変更する対象。
        let marker: serde_json::Value = serde_json::from_slice(&std::fs::read(&marker_path)?)
            .unwrap_or(serde_json::Value::Null);
        if marker["revision"] == REVISION
            && (marker["backend"] == backend
                || (requested == "cpu"
                    && matches!(marker["backend"].as_str(), Some("cuda" | "mps"))))
            && marker["torch"]
                == if cfg!(target_os = "macos") {
                    "2.11.0"
                } else {
                    "2.14.1"
                }
            && marker["transformers"] == "5.19.0"
            && marker["sentence_transformers"] == "6.1.0"
            && marker["torchvision"]
                == if cfg!(target_os = "macos") {
                    "0.26.0"
                } else {
                    "0.29.1"
                }
            && marker["pillow"] == "12.3.0"
            && marker["requirements_revision"] == "2026-10-08.1"
        {
            return Ok(python);
        }
    }
    let uv = crate::components::acquire_embedding_uv(app).await?;
    let shared = crate::components::ensure_shared_python(app, &uv).await?;
    std::fs::create_dir_all(&root)?;
    let script = root.join("install.py");
    crate::store::write_atomic(
        &script,
        include_str!("../resources/embedding-memory/install.py").as_bytes(),
    )?;
    for (name, content) in [
        (
            "requirements-cuda.lock",
            include_str!("../resources/embedding-memory/requirements-cuda.lock"),
        ),
        (
            "requirements-cpu.lock",
            include_str!("../resources/embedding-memory/requirements-cpu.lock"),
        ),
    ] {
        crate::store::write_atomic(&root.join(name), content.as_bytes())?;
    }
    crate::components::run_labeled_command(
        &shared,
        &[
            &script.to_string_lossy(),
            "--root",
            &root.to_string_lossy(),
            "--uv",
            &uv.to_string_lossy(),
            "--backend",
            backend,
        ],
        "記憶検索環境の導入",
    )
    .await?;
    anyhow::ensure!(
        python.is_file() && marker_path.is_file(),
        "記憶検索環境の導入完了を確認できません"
    );
    // 成功時だけ、本処理専用の展開物と取得キャッシュを回収する。
    let tools = crate::app_paths::bundled_dir(app).join("embedding-install-tools");
    if tools.exists() {
        std::fs::remove_dir_all(&tools)?;
    }
    Ok(python)
}
