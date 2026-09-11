// 導入済みランタイムで4工程を検証する。製品の登録先と入力原画は変更しない。
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use picoagent_character_maker::{
    config::MakerConfig,
    pipeline::{NewCharacter, PipelineContext, STAGES},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "引数: <管理component_root> <管理python_root> <入力原画> <新規出力フォルダ>".into(),
        );
    }
    let config = MakerConfig::managed(PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    if !config.python().is_file() || !config.comfy().is_dir() {
        return Err("管理ランタイムが未導入です。自動取得は行いません".into());
    }
    let source = PathBuf::from(&args[2]).canonicalize()?;
    let output = PathBuf::from(&args[3]);
    // 既存成果物を再利用・上書きせず、試行ごとに独立したルートを使う。
    std::fs::create_dir(&output)?;
    let output = output.canonicalize()?;
    let context = PipelineContext {
        characters_root: output.join("characters"),
        assets_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    };
    let manifest = context.create_character(NewCharacter {
        display_name: "実機検証用".into(),
        source_path: source.to_string_lossy().into_owned(),
        persona_prompt: String::new(),
        identity_tags: String::new(),
    })?;
    let started = Instant::now();
    for stage in STAGES {
        println!(
            "{}",
            serde_json::json!({"stage": stage, "status": "running"})
        );
        if let Err(error) = context.run_stage(
            &config,
            &manifest.character_id,
            stage,
            Arc::new(AtomicBool::new(false)),
            |event| println!("{event}"),
        ) {
            let report = serde_json::json!({"characterId":manifest.character_id,
                "status":"failed", "stage":stage, "error":error.to_string(),
                "elapsedSeconds":started.elapsed().as_secs_f64()});
            std::fs::write(
                output.join("report.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            return Err(error.into());
        }
    }
    let report = serde_json::json!({"characterId":manifest.character_id,
        "status":"complete", "elapsedSeconds":started.elapsed().as_secs_f64(),
        "manifest":context.load_character(&manifest.character_id)?});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
