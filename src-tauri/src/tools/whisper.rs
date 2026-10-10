// whisper.rs - 文字起こし2スキルの実行ファイルと固定モデルの解決。
use crate::config::Config;

pub fn program(cfg: &Config) -> Result<String, String> {
    let path = super::resolve_cli_in(
        &cfg.get("PICOAGENT_TRANSCRIBE_PROGRAM", ""),
        &crate::components::runtime_dirs("whisper"),
        "PICOAGENT_TRANSCRIBE_PROGRAM",
        "whisper-cli",
    )?;
    Ok(path.to_string_lossy().into_owned())
}
pub fn model(cfg: &Config) -> Result<String, String> {
    let configured = cfg.get("PICOAGENT_TRANSCRIBE_MODEL", "");
    model_in(&configured, &crate::components::runtime_dirs("whisper"))
}

fn model_in(configured: &str, dirs: &[std::path::PathBuf]) -> Result<String, String> {
    let path = if !configured.trim().is_empty() {
        std::path::PathBuf::from(configured.trim())
    } else {
        let dir = dirs
            .iter()
            .find(|dir| dir.exists())
            .ok_or("Whisperがありません。設定 → 追加機能・更新 から導入してください")?;
        picoagent_core::paths::whisper_model_at(dir)
    };
    if !path.is_file() {
        return Err(format!("Whisperモデルがありません: {}。修復するかPICOAGENT_TRANSCRIBE_MODELへパスを指定してください", path.display()));
    }
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn 固定モデル欠損と明示モデル欠損で別ファイルを選ばない() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        for root in [&first, &second] {
            std::fs::create_dir_all(root.join("models")).unwrap();
        }
        let other = first.join("models/ggml-large.bin");
        std::fs::write(&other, b"model").unwrap();
        let fixed = picoagent_core::paths::whisper_model_at(&second);
        std::fs::write(&fixed, b"model").unwrap();
        let dirs = [first, second];
        assert!(super::model_in("", &dirs).is_err());
        assert!(super::model_in(&dir.path().join("missing.bin").to_string_lossy(), &dirs).is_err());
        assert_eq!(
            super::model_in(&other.to_string_lossy(), &dirs).unwrap(),
            other.to_string_lossy()
        );
        std::fs::write(picoagent_core::paths::whisper_model_at(&dirs[0]), b"model").unwrap();
        assert_eq!(
            super::model_in("", &dirs).unwrap(),
            picoagent_core::paths::whisper_model_at(&dirs[0]).to_string_lossy()
        );
        assert!(super::model_in("", &[]).is_err());
    }
}
