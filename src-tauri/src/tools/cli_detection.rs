// 利用者が押した検出操作だけが使うCLI候補表・名乗り確認。実行時の探索とは分離する。
use serde::Serialize;
use std::future::Future;
use std::path::{Path, PathBuf};

pub const MAGICK_SETTING_KEY: &str = "PICOAGENT_MAGICK_PATH";
const CONVERT_COMMAND: &str = "convert";
pub(crate) const IDENTIFY_COMMAND: &str = "identify";
struct CandidateSpec {
    name: &'static str,
    major: &'static str,
}
struct CliSpec {
    id: &'static str,
    setting_key: &'static str,
    identity: &'static str,
    candidates: &'static [CandidateSpec],
}
const IMAGE_MAGICK: CliSpec = CliSpec {
    id: "imagemagick",
    setting_key: MAGICK_SETTING_KEY,
    identity: "Version: ImageMagick ",
    candidates: &[
        CandidateSpec {
            name: "magick",
            major: "7",
        },
        CandidateSpec {
            name: CONVERT_COMMAND,
            major: "6",
        },
    ],
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub path: String,
    pub version: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub setting_key: &'static str,
    pub candidates: Vec<Candidate>,
    pub warnings: Vec<String>,
}

pub(crate) fn is_named(program: &Path, name: &str) -> bool {
    program
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(name))
}
/// 明示保存されたv6のconvertだけ、同じ配布のidentifyを名指しする。PATHや候補探索はしない。
pub(crate) fn info_program(program: &Path) -> Result<PathBuf, String> {
    if !is_named(program, CONVERT_COMMAND) {
        return Ok(program.to_path_buf());
    }
    let name = if cfg!(windows) {
        format!("{IDENTIFY_COMMAND}.exe")
    } else {
        IDENTIFY_COMMAND.to_owned()
    };
    let parent = program
        .parent()
        .ok_or_else(|| "ImageMagickの保存パスに親フォルダがありません".to_string())?;
    let identify = parent.join(name);
    if !super::process::is_executable_file(&identify) {
        return Err(format!("指定したImageMagick 6と同じフォルダにidentifyがありません: {}。検出ボタンで配置を確認してください", identify.display()));
    }
    Ok(identify)
}

fn identity(spec: &CliSpec, candidate: &CandidateSpec, output: &str) -> Result<String, String> {
    let version = output
        .lines()
        .find_map(|line| line.trim().strip_prefix(spec.identity))
        .and_then(|line| line.split_whitespace().next())
        .filter(|version| version.split('.').next() == Some(candidate.major))
        .ok_or_else(|| {
            format!(
                "ImageMagick {} の名乗りを確認できません（別製品や対応外の版です）",
                candidate.major
            )
        })?;
    Ok(version.to_owned())
}

pub async fn detect(id: &str) -> Result<Detection, String> {
    detect_with(id, super::process::resolve_executable, |path| async move {
        let mut command = super::process::command(path);
        command.arg("--version");
        let output = super::process::output(command, 3, "ImageMagick検出").await?;
        if !output.status.success() {
            return Err(format!(
                "名乗り確認が失敗しました（終了コード {:?}）: {}",
                output.status.code(),
                crate::text::preview(&String::from_utf8_lossy(&output.stderr), 200)
            ));
        }
        String::from_utf8(output.stdout)
            .map_err(|error| format!("名乗りをUTF-8として読めません: {error}"))
    })
    .await
}

async fn detect_with<R, P, F>(id: &str, mut resolve: R, mut probe: P) -> Result<Detection, String>
where
    R: FnMut(&str) -> Option<PathBuf>,
    P: FnMut(PathBuf) -> F,
    F: Future<Output = Result<String, String>>,
{
    // IPCから任意のコマンド名・引数・shellを受け取らず、型付き表の対象だけを検出する。
    let spec = if id == IMAGE_MAGICK.id {
        &IMAGE_MAGICK
    } else {
        return Err(format!("対応していないCLI検出対象です: {id}"));
    };
    let mut result = Detection {
        setting_key: spec.setting_key,
        candidates: Vec::new(),
        warnings: Vec::new(),
    };
    for candidate in spec.candidates {
        let Some(path) = resolve(candidate.name) else {
            continue;
        };
        let checked = async {
            if !path.is_absolute() {
                return Err("検出した実行パスが絶対パスではありません".into());
            }
            let version = identity(spec, candidate, &probe(path.clone()).await?)?;
            if candidate.name == CONVERT_COMMAND {
                let identify = info_program(&path)?;
                identity(spec, candidate, &probe(identify).await?)?;
            }
            Ok::<_, String>(Candidate {
                path: path
                    .to_str()
                    .ok_or_else(|| "実行パスをUTF-8として扱えません".to_string())?
                    .to_owned(),
                version,
            })
        }
        .await;
        match checked {
            Ok(candidate) => result.candidates.push(candidate),
            Err(reason) => result
                .warnings
                .push(format!("{}: {reason}", path.display())),
        }
    }
    if result.candidates.is_empty() && !result.warnings.is_empty() {
        return Err(result.warnings.join("\n"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn 旧版は同じフォルダのidentifyも名乗り確認する() {
        let dir = tempfile::tempdir().unwrap();
        let convert = dir.path().join(if cfg!(windows) {
            "convert.exe"
        } else {
            "convert"
        });
        let identify = dir.path().join(if cfg!(windows) {
            "identify.exe"
        } else {
            "identify"
        });
        std::fs::write(&identify, "検査用実行ファイル").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&identify, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let requests = std::sync::Mutex::new(Vec::new());
        let report = detect_with(
            "imagemagick",
            |name| (name == CONVERT_COMMAND).then(|| convert.clone()),
            |path| {
                requests.lock().unwrap().push(path);
                async { Ok("Version: ImageMagick 6.9.13-38 Q16 x64".into()) }
            },
        )
        .await
        .unwrap();
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(
            *requests.lock().unwrap(),
            vec![convert.clone(), identify.clone()]
        );
        let rejected = detect_with(
            "imagemagick",
            |name| (name == CONVERT_COMMAND).then(|| convert.clone()),
            |path| {
                let wrong = path == identify;
                async move {
                    Ok(if wrong {
                        "別製品"
                    } else {
                        "Version: ImageMagick 6.9.13-38 Q16 x64"
                    }
                    .into())
                }
            },
        )
        .await
        .unwrap_err();
        assert!(rejected.contains("名乗り"));
    }
    #[tokio::test]
    async fn 任意コマンドを検出せず未発見と名乗り失敗を区別する() {
        assert!(detect_with(
            "cmd /c anything",
            |_| panic!("任意コマンドを探索してはいけません"),
            |_| async { Ok(String::new()) }
        )
        .await
        .is_err());
        let absent = detect_with(
            "imagemagick",
            |_| None,
            |_| async { panic!("未発見時は実行しません") },
        )
        .await
        .unwrap();
        assert!(absent.candidates.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let false_convert = dir.path().join(if cfg!(windows) {
            "convert.exe"
        } else {
            "convert"
        });
        let rejected = detect_with(
            "imagemagick",
            |name| (name == "convert").then(|| false_convert.clone()),
            |_| async { Ok("Microsoft Windows Convert\nVersion: other".into()) },
        )
        .await
        .unwrap_err();
        assert!(rejected.contains("名乗り"));
        let failed = detect_with(
            "imagemagick",
            |name| (name == "magick").then(|| dir.path().join("magick")),
            |_| async { Err("起動失敗の理由".into()) },
        )
        .await
        .unwrap_err();
        assert!(failed.contains("起動失敗の理由"));
    }
    #[tokio::test]
    async fn 正式な名乗りの候補だけ表示し検出では保存しない() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("magick");
        let report = detect_with(
            "imagemagick",
            |name| (name == "magick").then(|| path.clone()),
            |_| async { Ok("Version: ImageMagick 7.1.2-8 Q16-HDRI x64".into()) },
        )
        .await
        .unwrap();
        assert_eq!(report.setting_key, MAGICK_SETTING_KEY);
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].path, path.to_string_lossy());
        assert_eq!(report.candidates[0].version, "7.1.2-8");
        assert!(dir.path().read_dir().unwrap().next().is_none());
    }
}
