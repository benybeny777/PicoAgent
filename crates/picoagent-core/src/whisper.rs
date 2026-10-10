//! Whisperの固定配布情報とMacソースビルドの引数を、製品とxtaskで共有する。
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 固定ソースのggmlはCMake 3.14以降とMetalのコンパイラ・リンクツールを使う。
pub const XCRUN_PROBES: &[&[&str]] = &[
    &["--find", "clang"],
    &["--find", "clang++"],
    &["--show-sdk-path"],
    &["--find", "metal"],
    &["--find", "metallib"],
];
pub const COMPILER_PROBE: &str =
    "#include <vector>\nint main() { std::vector<int> values; return values.size(); }\n";
pub fn validate_cmake(report: &str) -> Result<(), String> {
    let version = report
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("cmake version "))
        .and_then(|version| {
            let mut parts = version.split('.');
            Some((
                parts.next()?.parse::<u32>().ok()?,
                parts.next()?.parse::<u32>().ok()?,
            ))
        });
    if version.is_none_or(|version| version < (3, 14)) {
        return Err("Whisperの固定ソースにはCMake 3.14以降が必要です".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePin {
    pub target_version: String,
    pub binary_release: String,
    pub source_commit: String,
    pub source_sha256: String,
    pub source_size: u64,
}

impl ReleasePin {
    pub fn from_manifest(text: &str) -> Result<Self, String> {
        let manifest: serde_json::Value =
            serde_json::from_str(text).map_err(|_| "Whisper配布情報のJSONが不正です")?;
        let pin: Self = serde_json::from_value(
            manifest
                .pointer("/components/whisper")
                .ok_or("Whisper配布情報がありません")?
                .clone(),
        )
        .map_err(|_| "Whisper固定版の項目が不足または不正です")?;
        if !pin.target_version.starts_with('v')
            || pin.target_version.contains(['/', '\\'])
            || pin.binary_release.len() < 2
            || !pin.binary_release.starts_with('b')
            || !pin.binary_release.strip_prefix('b').is_some_and(|digits| {
                !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
            })
            || pin.source_commit.len() != 40
            || !pin.source_commit.chars().all(|c| c.is_ascii_hexdigit())
            || pin.source_sha256.len() != 64
            || !pin.source_sha256.chars().all(|c| c.is_ascii_hexdigit())
            || pin.source_size == 0
        {
            return Err("Whisper固定版の値が不正です".into());
        }
        Ok(pin)
    }
    pub fn release_api(&self) -> String {
        format!(
            "https://api.github.com/repos/ggml-org/whisper.cpp/releases/tags/{}",
            self.binary_release
        )
    }
    pub fn source_url(&self) -> String {
        format!(
            "https://github.com/ggml-org/whisper.cpp/archive/{}.tar.gz",
            self.source_commit
        )
    }
    pub fn validate_release(&self, value: &serde_json::Value) -> Result<(), String> {
        if value.get("tag_name").and_then(serde_json::Value::as_str)
            != Some(self.binary_release.as_str())
            || value
                .get("target_commitish")
                .and_then(serde_json::Value::as_str)
                != Some(self.source_commit.as_str())
        {
            return Err("Whisper公式配布の版またはコミットが固定値と一致しません".into());
        }
        Ok(())
    }
    pub fn validate_override(&self, value: Option<&str>) -> Result<(), String> {
        if value.is_some_and(|tag| tag != self.binary_release) {
            return Err("WHISPER_RELEASEは固定binaryReleaseと一致する値だけ指定できます".into());
        }
        Ok(())
    }
}

pub fn binary_name(os: &str, arch: &str) -> Result<Option<&'static str>, String> {
    match (os, arch) {
        ("windows", "aarch64") => Ok(Some("whisper-bin-win-cpu-arm64.zip")),
        ("windows", "x86_64") => Ok(Some("whisper-bin-x64.zip")),
        ("linux", "aarch64") => Ok(Some("whisper-bin-ubuntu-arm64.tar.gz")),
        ("linux", "x86_64") => Ok(Some("whisper-bin-ubuntu-x64.tar.gz")),
        ("macos", "aarch64" | "x86_64") => Ok(None),
        _ => Err("このOS・CPU向けWhisperの公式導入方式がありません".into()),
    }
}

pub fn configure_args(
    source: &Path,
    build: &Path,
    compiler: &Path,
    cpp_compiler: &Path,
    sdk: &Path,
) -> Vec<String> {
    vec![
        "-S".into(),
        source.to_string_lossy().into_owned(),
        "-B".into(),
        build.to_string_lossy().into_owned(),
        "-DCMAKE_BUILD_TYPE=Release".into(),
        "-DBUILD_SHARED_LIBS=OFF".into(),
        "-DWHISPER_BUILD_TESTS=OFF".into(),
        "-DWHISPER_BUILD_EXAMPLES=ON".into(),
        "-DWHISPER_BUILD_SERVER=OFF".into(),
        "-DGGML_METAL=ON".into(),
        "-DGGML_METAL_EMBED_LIBRARY=ON".into(),
        format!("-DCMAKE_C_COMPILER={}", compiler.display()),
        format!("-DCMAKE_CXX_COMPILER={}", cpp_compiler.display()),
        format!("-DCMAKE_OSX_SYSROOT={}", sdk.display()),
    ]
}
pub fn build_args(build: &Path) -> Vec<String> {
    vec![
        "--build".into(),
        build.to_string_lossy().into_owned(),
        "--config".into(),
        "Release".into(),
        "--target".into(),
        "whisper-cli".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn 公式配布のosとcpuを一つだけ選びmacはソースビルドにする() {
        assert_eq!(
            binary_name("windows", "aarch64").unwrap(),
            Some("whisper-bin-win-cpu-arm64.zip")
        );
        for arch in ["aarch64", "x86_64"] {
            assert_eq!(binary_name("macos", arch).unwrap(), None);
        }
        assert!(binary_name("windows", "未知").is_err());
    }
    #[test]
    fn ビルドはshellを介さず対象cliとmetalと静的依存を指定する() {
        let args = configure_args(
            Path::new("空 白/source"),
            Path::new("空 白/build"),
            Path::new("clang"),
            Path::new("clang++"),
            Path::new("sdk"),
        );
        assert!(args.contains(&"空 白/source".to_string()));
        assert!(args.contains(&"-DGGML_METAL=ON".to_string()));
        assert!(args.contains(&"-DBUILD_SHARED_LIBS=OFF".to_string()));
        assert_eq!(
            build_args(Path::new("build")).last().unwrap(),
            "whisper-cli"
        );
    }
    #[test]
    fn 固定manifestだけから版と取得urlを決め別タグと別commitを拒否する() {
        let text = include_str!("../../../src-tauri/resources/component-manifest.json");
        let pin = ReleasePin::from_manifest(text).unwrap();
        assert_eq!(pin.target_version, "v1.9.5");
        assert_eq!(pin.binary_release, "b5454");
        assert!(pin.release_api().ends_with("/tags/b5454"));
        assert!(pin.source_url().contains(&pin.source_commit));
        let valid =
            serde_json::json!({"tag_name":pin.binary_release,"target_commitish":pin.source_commit});
        assert!(pin.validate_release(&valid).is_ok());
        for (field, wrong) in [("tag_name", "latest"), ("target_commitish", "master")] {
            let mut value = valid.clone();
            value[field] = wrong.into();
            assert!(pin.validate_release(&value).is_err());
        }
        assert!(pin.validate_override(None).is_ok());
        assert!(pin.validate_override(Some("b5454")).is_ok());
        for value in ["latest", "v1.9.5", "b5455", ""] {
            assert!(pin.validate_override(Some(value)).is_err());
        }
        for invalid in ["ば5454", "bあ", "b", "b５４５４"] {
            let mut manifest: serde_json::Value = serde_json::from_str(text).unwrap();
            manifest["components"]["whisper"]["binaryRelease"] = invalid.into();
            assert!(ReleasePin::from_manifest(&manifest.to_string()).is_err());
        }
        let mut broken: serde_json::Value = serde_json::from_str(text).unwrap();
        broken["components"]["whisper"]["sourceSha256"] = "".into();
        assert!(ReleasePin::from_manifest(&broken.to_string()).is_err());
    }
    #[test]
    fn 古いcmakeと不明応答を取得前に拒否しmetalツールを省略しない() {
        assert!(validate_cmake("cmake version 3.14.0").is_ok());
        assert!(validate_cmake("cmake version 4.0.0").is_ok());
        for text in ["cmake version 3.13.9", "cmake version 不正", ""] {
            assert!(validate_cmake(text).is_err());
        }
        assert!(XCRUN_PROBES.contains(&["--find", "metal"].as_slice()));
        assert!(XCRUN_PROBES.contains(&["--find", "metallib"].as_slice()));
    }
}
