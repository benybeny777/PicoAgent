// whisper_transcribe/mod.rs - 既存音声をローカルで文字起こしするスキル。

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions::SafePath;

const MAX_COMMAND_OUTPUT_BYTES: usize = 32 * 1024;

pub struct WhisperTranscribeTool;

fn meeting_root(cfg: &Config) -> PathBuf {
    crate::permissions::primary_root(cfg)
}

fn bounded_path(cfg: &Config, requested: &str) -> Result<SafePath, String> {
    crate::permissions::resolve(cfg, requested)
}

fn is_audio(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "wav" | "mp3" | "m4a" | "flac" | "ogg")
    )
}

fn latest_file(cfg: &Config, root: &Path) -> Option<SafePath> {
    let mut best: Option<(PathBuf, SystemTime)> = None;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !is_audio(&path) {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if best
                .as_ref()
                .map(|(_, current)| modified > *current)
                .unwrap_or(true)
            {
                best = Some((path, modified));
            }
        }
    }
    // 走査で得たパスも許可判定を通してから返す。
    best.and_then(|(path, _)| crate::permissions::resolve(cfg, &path.to_string_lossy()).ok())
}

fn output_stem(cfg: &Config, audio: &SafePath) -> Result<SafePath, String> {
    audio.sibling(cfg, &audio.file_stem_lossy())
}

fn transcript_path(cfg: &Config, audio: &SafePath) -> Result<SafePath, String> {
    audio.sibling(cfg, &format!("{}.transcript.txt", audio.file_stem_lossy()))
}

fn split_args_template(
    template: &str,
    audio: &SafePath,
    model: &str,
    output_stem: &SafePath,
) -> Vec<String> {
    template
        .split_whitespace()
        .map(|token| {
            token
                .replace("{audio}", &audio.as_path().to_string_lossy())
                .replace("{model}", model)
                .replace("{output_stem}", &output_stem.as_path().to_string_lossy())
        })
        .collect()
}

fn append_diarize_args(cfg: &Config, args_list: &mut Vec<String>, enabled: bool) {
    if !enabled {
        return;
    }
    let extra = cfg.get("PICOAGENT_TRANSCRIBE_DIARIZE_ARGS", "-tdrz");
    args_list.extend(
        extra
            .split_whitespace()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    );
}

fn trim_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= MAX_COMMAND_OUTPUT_BYTES {
        return text.to_string();
    }
    let mut end = MAX_COMMAND_OUTPUT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n…（{}バイトで打ち切り）",
        &text[..end],
        MAX_COMMAND_OUTPUT_BYTES
    )
}

/// 共通層でタイムアウト付き実行する。起動失敗・タイムアウト・出力回収の
/// 打ち切りはすべて共通層の判断に任せ、ここでは案内文の先頭だけ揃える。
async fn run_command(program: &str, args_list: &[String], timeout: u64) -> Result<Output, String> {
    let mut command = crate::tools::process::command(program);
    command.args(args_list).stdin(Stdio::null());
    crate::tools::process::output(command, timeout, "Whisper文字起こし")
        .await
        .map_err(|error| format!("エラー: {error}"))
}

/// ffmpeg の実行パスを解決する。優先順: 明示設定 → 同梱（setup tools）。
fn ffmpeg_path(cfg: &Config) -> Result<std::path::PathBuf, String> {
    crate::tools::resolve_cli_path(cfg, "PICOAGENT_FFMPEG_PATH", "ffmpeg", "ffmpeg")
}

/// 非 WAV 音声を ffmpeg で 16kHz モノラル WAV（whisper.cpp の推奨入力）へ前処理する。
/// WAVは変換不要としてNoneを返す。必要な変換が失敗したら理由を返して停止する。
async fn convert_to_wav(cfg: &Config, audio: &SafePath) -> Result<Option<SafePath>, String> {
    let is_wav = audio.extension_lossy() == "wav";
    if is_wav {
        return Ok(None);
    }
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out = std::env::temp_dir().join(format!(
        "picoagent-transcribe-{}-{}.wav",
        std::process::id(),
        unique
    ));
    let args_list: Vec<String> = [
        "-y",
        "-i",
        &audio.as_path().to_string_lossy(),
        "-ar",
        "16000",
        "-ac",
        "1",
        "-c:a",
        "pcm_s16le",
        &out.to_string_lossy(),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    match run_command(&ffmpeg_path(cfg)?.to_string_lossy(), &args_list, 300).await {
        Ok(output) if output.status.success() && out.exists() => Ok(Some(SafePath::app_owned(out))),
        result => {
            let error = match result {
                Err(error) => error,
                Ok(output) => format!(
                    "音声をWAVへ変換できません: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
            };
            match remove_internal_file(&out) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!(
                    "{error}。一時ファイルの後始末にも失敗しました: {cleanup}"
                )),
            }
        }
    }
}

fn remove_internal_file(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "内部一時ファイルを削除できません: {} ({error})",
            path.display()
        )),
    }
}

/// 音声会話は120秒の応答期限を保つ。合法な設定待機時間の範囲内で短い方を使う。
fn voice_input_timeout(cfg: &Config) -> Result<u64, String> {
    Ok((cfg.int(&crate::config::ints::TRANSCRIBE_TIMEOUT_SECS)? as u64).min(120))
}

/// 音声ファイルを文字起こしして本文を返す（マイク入力コマンドなど内部利用向け）。
/// 同じ設定を検査し、会話用の応答期限内で実行する。
pub async fn transcribe_file(cfg: &Config, audio: &SafePath) -> Result<String, String> {
    let timeout = voice_input_timeout(cfg)?;
    let model = crate::tools::whisper::model(cfg)?;
    let program = crate::tools::whisper::program(cfg)?;
    let converted = convert_to_wav(cfg, audio).await?;
    let source: &SafePath = converted.as_ref().unwrap_or(audio);
    let output_stem = output_stem(cfg, audio)?;
    let template = cfg.get(
        "PICOAGENT_TRANSCRIBE_ARGS",
        "-m {model} -f {audio} -otxt -of {output_stem}",
    );
    let args_list = split_args_template(&template, source, &model, &output_stem);
    let output = run_command(&program, &args_list, timeout).await;
    if let Some(temp) = &converted {
        remove_internal_file(temp.as_path())?;
    }
    let output = output?;
    if !output.status.success() {
        return Err(format!(
            "Whisper文字起こしが失敗しました: {}",
            trim_output(&output.stderr)
        ));
    }
    let generated = output_stem.sibling(cfg, &format!("{}.txt", output_stem.file_stem_lossy()))?;
    let text = if generated.exists() {
        let content = std::fs::read_to_string(&generated)
            .map_err(|error| format!("文字起こし結果を読めません: {error}"))?;
        remove_internal_file(generated.as_path())?;
        content
    } else {
        trim_output(&output.stdout)
    };
    Ok(text.trim().to_string())
}

#[async_trait]
impl Tool for WhisperTranscribeTool {
    fn name(&self) -> &'static str {
        "whisper_transcribe"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["文字起こし", "whisper", "transcribe", "話者", "speaker"]
    }

    fn intent_priority(&self) -> i32 {
        800
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        let lower = text.to_lowercase();
        let recording_workflow = ["録音", "議事録", "会議メモ", "meeting notes"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()));
        if !self.matches_intent(text) || recording_workflow {
            return None;
        }
        let diarize = ["話者", "speaker", "diarize"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()));
        Some(crate::tools::IntentInvocation::direct(json!({
            "diarize": diarize
        })))
    }

    fn label(&self) -> &'static str {
        "音声文字起こし"
    }

    fn description(&self) -> String {
        "PicoAgent管理下の同梱Whisper CLI、または明示設定したローカルWhisperで音声ファイルを文字起こしする。mp3/m4a 等は ffmpeg（明示設定または同梱）で16kHz WAVへ自動前処理。常駐サーバやポートは使わない。".to_string()
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "最新の録音を文字起こしして",
            "この音声ファイルを文字起こしして",
            "話者分類つきで文字起こしして",
        ]
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "audio_path": { "type": "string", "description": "文字起こし対象の音声ファイル。省略時は録音・議事録ルート配下の最新音声。" },
                "output_path": { "type": "string", "description": "文字起こしテキストの保存先。省略時は音声名に .transcript.txt を付ける。" },
                "diarize": { "type": "boolean", "description": "話者分類用の追加引数を渡す。" }
            },
            "required": []
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_TRANSCRIBE_PROGRAM",
                "Whisper実行ファイル（手動設定が最優先、空なら同梱）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_MODEL",
                "Whisperモデル（手動設定が最優先、空なら同梱）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_ARGS",
                "Whisper文字起こし引数",
                "text",
                "-m {model} -f {audio} -otxt -of {output_stem}",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_DIARIZE_ARGS",
                "話者分類時に追加するWhisper引数",
                "text",
                "-tdrz",
            ),
            Field::new(
                "PICOAGENT_FFMPEG_PATH",
                "ffmpeg 実行ファイルパス（mp3/m4a等の前処理用。空ならPicoAgent管理下）",
                "text",
                "",
            ),
            Field::int(&crate::config::ints::TRANSCRIBE_TIMEOUT_SECS),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        let timeout =
            crate::tool_try!(cfg.int(&crate::config::ints::TRANSCRIBE_TIMEOUT_SECS)) as u64;
        let base = meeting_root(cfg);
        let audio = if let Some(path_text) = args
            .get("audio_path")
            .or_else(|| args.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match bounded_path(cfg, path_text) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            }
        } else {
            match latest_file(cfg, &base) {
                Some(path) => path,
                None => {
                    return format!(
                        "エラー: 文字起こし対象の音声が見つかりません。先に録音するか、{} 配下の音声パスを指定してください。",
                        base.display()
                    )
                }
            }
        };
        if !audio.exists() {
            return format!("エラー: 音声ファイルが見つかりません: {}", audio.display());
        }
        let model = crate::tool_try!(crate::tools::whisper::model(cfg));
        let program = crate::tool_try!(crate::tools::whisper::program(cfg));
        // mp3/m4a 等は ffmpeg で 16kHz WAV へ前処理してから渡す（精度・互換性の向上）。
        let converted = match convert_to_wav(cfg, &audio).await {
            Ok(converted) => converted,
            Err(error) => return format!("エラー: {error}"),
        };
        let source = converted.clone().unwrap_or_else(|| audio.clone());
        let output_stem = match output_stem(cfg, &audio) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        };
        let template = cfg.get(
            "PICOAGENT_TRANSCRIBE_ARGS",
            "-m {model} -f {audio} -otxt -of {output_stem}",
        );
        let mut args_list = split_args_template(&template, &source, &model, &output_stem);
        append_diarize_args(
            cfg,
            &mut args_list,
            args.get("diarize")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        let output = run_command(&program, &args_list, timeout).await;
        if let Some(temp) = &converted {
            if let Err(error) = remove_internal_file(temp.as_path()) {
                return format!("エラー: {error}");
            }
        }
        let output = match output {
            Ok(output) => output,
            Err(error) => return error,
        };
        if !output.status.success() {
            return format!(
                "エラー: Whisper文字起こしが失敗しました。\nstdout:\n{}\nstderr:\n{}",
                trim_output(&output.stdout),
                trim_output(&output.stderr)
            );
        }
        let generated =
            match output_stem.sibling(cfg, &format!("{}.txt", output_stem.file_stem_lossy())) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            };
        let content = if generated.exists() {
            let content = match std::fs::read_to_string(&generated) {
                Ok(content) => content,
                Err(error) => return format!("エラー: 文字起こし結果を読めません: {error}"),
            };
            if let Err(error) = remove_internal_file(generated.as_path()) {
                return format!("エラー: {error}");
            }
            content
        } else {
            trim_output(&output.stdout)
        };
        let target = if let Some(path_text) = args
            .get("output_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match bounded_path(cfg, path_text) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            }
        } else {
            match transcript_path(cfg, &audio) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            }
        };
        if let Err(error) = crate::permissions::fs::create_dir_all(&target.parent_or_root(cfg)) {
            return format!("エラー: 文字起こしフォルダを作成できません: {error}");
        }
        if let Err(error) = crate::permissions::fs::write(&target, &content) {
            return format!("エラー: 文字起こし結果を保存できません: {error}");
        }
        format!(
            "音声文字起こしが完了しました:\n- 音声: {}\n- 文字起こし: {}\n- 作成日: {}",
            audio.display(),
            target.display(),
            Local::now().format("%Y-%m-%d")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_to_wav, WhisperTranscribeTool};
    use crate::permissions::SafePath;
    use crate::tools::Tool;

    #[tokio::test]
    async fn 必要な前処理が欠損したら元音声で続行しない() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("input.mp3");
        std::fs::write(&audio, b"audio").unwrap();
        let cfg = crate::config::Config::new();
        cfg.set(
            "PICOAGENT_FFMPEG_PATH",
            &dir.path().join("missing-ffmpeg").to_string_lossy(),
        );
        let result = convert_to_wav(&cfg, &SafePath::app_owned(audio)).await;
        assert!(result.unwrap_err().contains("PICOAGENT_FFMPEG_PATH"));
    }

    // ffmpeg がある環境だけで走る E2E（無ければ即成功で抜ける）。
    // sine 波の mp3 を生成し、16kHz WAV へ前処理できることを確認する。
    #[test]
    fn 非wav音声をffmpegでwavへ前処理する() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            eprintln!("ffmpeg が無いためスキップ");
            return;
        }
        let dir = std::env::temp_dir().join(format!("picoagent-ffmpeg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mp3 = dir.join("tone.mp3");
        let status = std::process::Command::new("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .arg(&mp3)
            .output()
            .unwrap();
        assert!(status.status.success());

        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_FFMPEG_PATH", "ffmpeg");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let converted = runtime.block_on(convert_to_wav(&cfg, &SafePath::app_owned(mp3.clone())));
        let wav = converted
            .expect("変換処理は成功する")
            .expect("mp3 は WAV へ変換されるはず");
        assert!(wav.exists());
        // RIFF ヘッダを持つ本物の WAV になっている。
        let head = std::fs::read(&wav).unwrap();
        assert_eq!(&head[..4], b"RIFF");
        let _ = std::fs::remove_file(&wav);

        // WAV 入力は前処理せず None（元ファイルのまま使う）。
        let plain = dir.join("plain.wav");
        std::fs::write(&plain, b"dummy").unwrap();
        assert!(runtime
            .block_on(convert_to_wav(&cfg, &SafePath::app_owned(plain.clone())))
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `PICOAGENT_QA_LONG_AUDIO` に10分以上のWAVを指定して明示実行する実機検査。
    /// 通常の単体テストでは大容量モデルを起動しない。
    #[test]
    #[ignore = "10分以上の音声と同梱Whisperモデルを使う実機検査"]
    fn 長時間音声を共通プロセス回収経由で文字起こしできる() {
        let audio = std::env::var("PICOAGENT_QA_LONG_AUDIO")
            .expect("PICOAGENT_QA_LONG_AUDIO に10分以上のWAVを指定してください");
        let audio = std::path::PathBuf::from(audio);
        assert!(audio.exists(), "音声がありません: {}", audio.display());
        let root = audio.parent().expect("音声の親フォルダが必要です");
        let target = root.join("long-audio.transcript.txt");
        let generated = audio.with_extension("txt");
        let cfg = crate::config::Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &root.to_string_lossy());
        cfg.set("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS", "7200");
        let started = std::time::Instant::now();
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(WhisperTranscribeTool.run(
                &cfg,
                serde_json::json!({
                    "audio_path": audio,
                    "output_path": target,
                }),
            ));
        assert!(result.contains("音声文字起こしが完了しました"), "{result}");
        let transcript = std::fs::read_to_string(&target).expect("文字起こし結果を読める");
        assert!(!transcript.trim().is_empty(), "文字起こし結果が空です");
        assert!(!generated.exists(), "Whisperの中間テキストが残っています");
        eprintln!(
            "長時間文字起こし: elapsed={:?} transcript_bytes={}",
            started.elapsed(),
            transcript.len()
        );
    }
    #[tokio::test]
    async fn 不正な文字起こし待ち時間は前処理や出力先作成より前に返す() {
        use crate::tools::meeting_recorder::MeetingRecorderTool;
        let cases: [(&dyn Tool, serde_json::Value); 2] = [
            (&WhisperTranscribeTool, serde_json::json!({})),
            (
                &MeetingRecorderTool,
                serde_json::json!({"action":"transcribe"}),
            ),
        ];
        for (tool, args) in cases {
            let cfg = crate::config::Config::new();
            cfg.set("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS", "abc");
            let result = tool.run(&cfg, args).await;
            assert!(
                result.contains("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS"),
                "{}: {result}",
                tool.name()
            );
        }
    }
    #[tokio::test]
    async fn 音声入力は有効設定と応答期限の短い方を使い不正なら前処理しない() {
        let cfg = crate::config::Config::new();
        for (configured, expected) in [("1800", 120), ("7200", 120), ("10", 10)] {
            cfg.set("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS", configured);
            assert_eq!(super::voice_input_timeout(&cfg).unwrap(), expected);
        }
        cfg.set("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS", "abc");
        let result = super::transcribe_file(
            &cfg,
            &SafePath::app_owned(std::path::PathBuf::from("未使用の入力.mp3")),
        )
        .await;
        assert!(result
            .unwrap_err()
            .contains("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS"));
    }
}

inventory::submit! { ToolReg { make: || Box::new(WhisperTranscribeTool) } }
