// meeting_recorder/mod.rs - ローカル録音、ローカル文字起こし、議事録生成スキル。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat};
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};
use serde_json::{json, Value};
use tokio::process::Command;

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;

const MAX_TRANSCRIPT_CHARS: usize = 80_000;
const MAX_COMMAND_OUTPUT_BYTES: usize = 32 * 1024;

type SharedWriter = Arc<Mutex<Option<WavWriter<BufWriter<File>>>>>;

struct RecordingState {
    path: PathBuf,
    started: Instant,
    stop_tx: Sender<()>,
    join: JoinHandle<()>,
}

static RECORDING: OnceLock<Mutex<Option<RecordingState>>> = OnceLock::new();

pub struct MeetingRecorderTool;

fn recording_state() -> &'static Mutex<Option<RecordingState>> {
    RECORDING.get_or_init(|| Mutex::new(None))
}

fn home() -> String {
    dirs::home_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

fn workspace_root(cfg: &Config) -> PathBuf {
    PathBuf::from(cfg.get("PICOAGENT_WORKSPACE_ROOT", &home()))
}

fn meeting_root(cfg: &Config) -> PathBuf {
    let meeting_root = cfg.get("PICOAGENT_MEETING_ROOT", "");
    if meeting_root.trim().is_empty() {
        workspace_root(cfg)
    } else {
        PathBuf::from(meeting_root)
    }
}

fn normalize(base: &Path, input: &str) -> PathBuf {
    let joined = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        base.join(input)
    };
    let mut out = PathBuf::new();
    for comp in joined.components() {
        use std::path::Component::*;
        match comp {
            ParentDir => {
                out.pop();
            }
            CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn bounded_path(base: &Path, input: &str) -> Result<PathBuf, String> {
    let target = normalize(base, input);
    if target == base || target.starts_with(base) {
        Ok(target)
    } else {
        Err(format!(
            "許可ルート（{}）の外は使えません: {}",
            base.display(),
            target.display()
        ))
    }
}

fn default_recording_path() -> String {
    format!(
        "meetings/{}/{}-meeting.wav",
        Local::now().format("%Y-%m-%d"),
        Local::now().format("%Y%m%d-%H%M%S")
    )
}

fn latest_file(base: &Path, matcher: fn(&Path) -> bool) -> Option<PathBuf> {
    let root = base.join("meetings");
    let mut stack = vec![root];
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() && matcher(&path) {
                let modified = metadata
                    .modified()
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                if best
                    .as_ref()
                    .is_none_or(|(_, best_modified)| modified > *best_modified)
                {
                    best = Some((path, modified));
                }
            }
        }
    }
    best.map(|(path, _)| path)
}

fn is_wav(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("wav")
}

fn is_transcript(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".transcript.txt"))
}

fn output_stem(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("")
}

fn transcript_path(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("transcript.txt")
}

fn minutes_path(audio_path: &Path) -> PathBuf {
    audio_path.with_extension("minutes.md")
}

fn minutes_path_from_transcript(transcript_path: &Path) -> PathBuf {
    if let Some(name) = transcript_path.file_name().and_then(|name| name.to_str()) {
        if let Some(stem) = name.strip_suffix(".transcript.txt") {
            return transcript_path.with_file_name(format!("{stem}.minutes.md"));
        }
    }
    transcript_path.with_extension("minutes.md")
}

fn wav_spec(config: &cpal::SupportedStreamConfig) -> WavSpec {
    WavSpec {
        channels: config.channels(),
        sample_rate: config.sample_rate().0,
        bits_per_sample: match config.sample_format() {
            SampleFormat::F32 => 32,
            _ => 16,
        },
        sample_format: match config.sample_format() {
            SampleFormat::F32 => WavSampleFormat::Float,
            _ => WavSampleFormat::Int,
        },
    }
}

fn write_input<T>(data: &[T], writer: &SharedWriter)
where
    T: cpal::Sample,
    i16: cpal::FromSample<T>,
{
    let Ok(mut guard) = writer.lock() else {
        return;
    };
    let Some(wav) = guard.as_mut() else {
        return;
    };
    for sample in data {
        let value: i16 = i16::from_sample(*sample);
        let _ = wav.write_sample(value);
    }
}

fn write_input_f32(data: &[f32], writer: &SharedWriter) {
    let Ok(mut guard) = writer.lock() else {
        return;
    };
    let Some(wav) = guard.as_mut() else {
        return;
    };
    for sample in data {
        let _ = wav.write_sample(*sample);
    }
}

fn start_recording(cfg: &Config, args: &Value) -> String {
    let mut state = match recording_state().lock() {
        Ok(state) => state,
        Err(_) => return "エラー: 録音状態を取得できません。".to_string(),
    };
    if let Some(active) = state.as_ref() {
        return format!("エラー: すでに録音中です: {}", active.path.display());
    }
    let base = meeting_root(cfg);
    let requested = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(default_recording_path);
    let path = match bounded_path(&base, &requested) {
        Ok(path) => path,
        Err(error) => return format!("エラー: {error}"),
    };
    if path.extension().and_then(|ext| ext.to_str()) != Some("wav") {
        return "エラー: 録音ファイルは .wav で保存してください。".to_string();
    }
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return format!("エラー: 録音フォルダを作成できません: {error}");
        }
    }
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let thread_path = path.clone();
    let join = thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            let _ = ready_tx.send(Err("既定のマイク入力デバイスが見つかりません。".to_string()));
            return;
        };
        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(error) => {
                let _ = ready_tx.send(Err(format!("マイク設定を取得できません: {error}")));
                return;
            }
        };
        let spec = wav_spec(&config);
        let writer = match WavWriter::create(&thread_path, spec) {
            Ok(writer) => Arc::new(Mutex::new(Some(writer))),
            Err(error) => {
                let _ = ready_tx.send(Err(format!("WAVファイルを作成できません: {error}")));
                return;
            }
        };
        let writer_for_stream = writer.clone();
        let error_callback = |error| eprintln!("meeting_recorder stream error: {error}");
        let stream_config = config.clone().into();
        let stream = match config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| write_input_f32(data, &writer_for_stream),
                error_callback,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _| write_input(data, &writer_for_stream),
                error_callback,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                move |data: &[u16], _| write_input(data, &writer_for_stream),
                error_callback,
                None,
            ),
            other => {
                let _ = ready_tx.send(Err(format!("未対応の録音サンプル形式です: {other:?}")));
                return;
            }
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                let _ = ready_tx.send(Err(format!("録音ストリームを開始できません: {error}")));
                return;
            }
        };
        if let Err(error) = stream.play() {
            let _ = ready_tx.send(Err(format!("録音を開始できません: {error}")));
            return;
        }
        let _ = ready_tx.send(Ok(()));
        let _ = stop_rx.recv();
        drop(stream);
        let mut writer_guard = match writer.lock() {
            Ok(writer_guard) => writer_guard,
            Err(_) => return,
        };
        if let Some(writer) = writer_guard.take() {
            let _ = writer.finalize();
        }
        drop(writer_guard);
    });
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return format!("エラー: {error}"),
        Err(_) => return "エラー: 録音開始の準備がタイムアウトしました。".to_string(),
    }
    *state = Some(RecordingState {
        path: path.clone(),
        started: Instant::now(),
        stop_tx,
        join,
    });
    format!(
        "録音を開始しました: {}\n停止するには meeting_recorder action=stop を実行してください。",
        path.display()
    )
}

fn stop_recording() -> String {
    let mut state = match recording_state().lock() {
        Ok(state) => state,
        Err(_) => return "エラー: 録音状態を取得できません。".to_string(),
    };
    let Some(active) = state.take() else {
        return "エラー: 録音中ではありません。".to_string();
    };
    let elapsed = active.started.elapsed();
    let _ = active.stop_tx.send(());
    let _ = active.join.join();
    format!(
        "録音を停止しました: {}\n録音時間: {} 秒",
        active.path.display(),
        elapsed.as_secs()
    )
}

fn status() -> String {
    let state = match recording_state().lock() {
        Ok(state) => state,
        Err(_) => return "エラー: 録音状態を取得できません。".to_string(),
    };
    if let Some(active) = state.as_ref() {
        format!(
            "録音中: {}\n経過: {} 秒",
            active.path.display(),
            active.started.elapsed().as_secs()
        )
    } else {
        "録音中ではありません。".to_string()
    }
}

fn split_args_template(
    template: &str,
    audio: &Path,
    model: &str,
    output_stem: &Path,
) -> Vec<String> {
    template
        .split_whitespace()
        .map(|token| {
            token
                .replace("{audio}", &audio.to_string_lossy())
                .replace("{model}", model)
                .replace("{output_stem}", &output_stem.to_string_lossy())
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

async fn transcribe(cfg: &Config, args: &Value) -> String {
    let base = meeting_root(cfg);
    let audio = if let Some(audio_text) = args
        .get("audio_path")
        .or_else(|| args.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match bounded_path(&base, audio_text) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        }
    } else {
        match latest_file(&base, is_wav) {
            Some(path) => path,
            None => {
                return format!(
                    "エラー: 文字起こし対象のWAVが見つかりません。先に録音するか、{} 配下の音声パスを指定してください。",
                    base.display()
                )
            }
        }
    };
    if !audio.exists() {
        return format!("エラー: 音声ファイルが見つかりません: {}", audio.display());
    }
    let program = cfg.get("PICOAGENT_TRANSCRIBE_PROGRAM", "whisper-cli");
    let model = cfg.get("PICOAGENT_TRANSCRIBE_MODEL", "");
    if model.trim().is_empty() {
        return super::config_guide(
            "ローカル文字起こし",
            &["PICOAGENT_TRANSCRIBE_MODEL"],
            "whisper.cpp などのローカル音声認識モデル（例: ggml-large-v3-turbo.bin）を指定してください。外部APIは使いません。",
        );
    }
    let output_stem = output_stem(&audio);
    let template = cfg.get(
        "PICOAGENT_TRANSCRIBE_ARGS",
        "-m {model} -f {audio} -otxt -of {output_stem}",
    );
    let mut args_list = split_args_template(&template, &audio, &model, &output_stem);
    append_diarize_args(
        cfg,
        &mut args_list,
        args.get("diarize")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
    let timeout = cfg
        .get("PICOAGENT_TRANSCRIBE_TIMEOUT_SECS", "1800")
        .parse::<u64>()
        .ok()
        .filter(|value| (10..=7200).contains(value))
        .unwrap_or(1800);
    let output = match tokio::time::timeout(
        Duration::from_secs(timeout),
        Command::new(&program)
            .args(&args_list)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return format!("エラー: ローカル文字起こしを起動できません: {error}"),
        Err(_) => {
            return format!("エラー: ローカル文字起こしが {timeout} 秒でタイムアウトしました。")
        }
    };
    if !output.status.success() {
        return format!(
            "エラー: ローカル文字起こしが失敗しました。\nstdout:\n{}\nstderr:\n{}",
            trim_output(&output.stdout),
            trim_output(&output.stderr)
        );
    }
    let transcript = output_stem.with_extension("txt");
    let content = if transcript.exists() {
        match std::fs::read_to_string(&transcript) {
            Ok(content) => content,
            Err(error) => return format!("エラー: 文字起こし結果を読めません: {error}"),
        }
    } else {
        trim_output(&output.stdout)
    };
    let target = transcript_path(&audio);
    if let Err(error) = std::fs::write(&target, &content) {
        return format!("エラー: 文字起こし結果を保存できません: {error}");
    }
    format!(
        "ローカル文字起こしが完了しました:\n- 音声: {}\n- 文字起こし: {}",
        audio.display(),
        target.display()
    )
}

fn minutes_from_text(title: &str, transcript: &str) -> String {
    let trimmed = transcript.trim();
    let mut out = format!("# 議事録: {title}\n\n");
    out.push_str(&format!(
        "- 作成日: {}\n- 生成元: ローカル文字起こし\n\n",
        Local::now().format("%Y-%m-%d")
    ));
    out.push_str("## 要約\n\n");
    for line in trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(8)
    {
        out.push_str(&format!("- {line}\n"));
    }
    let speaker_turns = trimmed.matches("[SPEAKER_TURN]").count()
        + trimmed.matches("[speaker_turn]").count()
        + trimmed.matches("<speaker_turn>").count();
    if speaker_turns > 0 {
        out.push_str("\n## 話者交代メモ\n\n");
        out.push_str(&format!(
            "- 話者交代マーカーを {} 件検出しました。話者名は録音後に手動で Speaker 1 / Speaker 2 へ割り当ててください。\n",
            speaker_turns
        ));
    }
    out.push_str("\n## 決定事項\n\n- 文字起こし本文から確認してください。\n");
    out.push_str("\n## TODO\n\n- [ ] 担当者と期限を追記する\n- [ ] 決定事項を確認する\n");
    out.push_str("\n## 共有メール下書き\n\n本日の議事録を共有します。要点、決定事項、TODOを確認してください。\n");
    out.push_str("\n## 文字起こし本文\n\n");
    if trimmed.chars().count() > MAX_TRANSCRIPT_CHARS {
        let body: String = trimmed.chars().take(MAX_TRANSCRIPT_CHARS).collect();
        out.push_str(&body);
        out.push_str("\n\n…（本文が長いため打ち切りました）\n");
    } else {
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

fn make_minutes(cfg: &Config, args: &Value) -> String {
    let base = meeting_root(cfg);
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("会議");
    let mut transcript_source: Option<PathBuf> = None;
    let transcript = if let Some(text) = args
        .get("transcript")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        text.to_string()
    } else {
        let path = if let Some(path_text) = args
            .get("transcript_path")
            .or_else(|| args.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match bounded_path(&base, path_text) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            }
        } else {
            match latest_file(&base, is_transcript) {
                Some(path) => path,
                None => {
                    return format!(
                        "エラー: 議事録化する文字起こしファイルが見つかりません。先に文字起こしするか、{} 配下の transcript パスを指定してください。",
                        base.display()
                    )
                }
            }
        };
        transcript_source = Some(path.clone());
        match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => return format!("エラー: 文字起こしファイルを読めません: {error}"),
        }
    };
    let output = if let Some(path_text) = args
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match bounded_path(&base, path_text) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        }
    } else if let Some(path_text) = args
        .get("audio_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match bounded_path(&base, path_text) {
            Ok(path) => minutes_path(&path),
            Err(error) => return format!("エラー: {error}"),
        }
    } else if let Some(path) = &transcript_source {
        minutes_path_from_transcript(path)
    } else {
        normalize(
            &base,
            &format!(
                "meetings/{}/{}-minutes.md",
                Local::now().format("%Y-%m-%d"),
                Local::now().format("%Y%m%d-%H%M%S")
            ),
        )
    };
    if let Some(parent) = output.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return format!("エラー: 議事録フォルダを作成できません: {error}");
        }
    }
    let minutes = minutes_from_text(title, &transcript);
    match std::fs::write(&output, minutes) {
        Ok(()) => format!("議事録を作成しました: {}", output.display()),
        Err(error) => format!("エラー: 議事録を保存できません: {error}"),
    }
}

#[async_trait]
impl Tool for MeetingRecorderTool {
    fn name(&self) -> &'static str {
        "meeting_recorder"
    }

    fn label(&self) -> &'static str {
        "ローカル録音・文字起こし議事録"
    }

    fn description(&self) -> String {
        "マイク録音をローカルWAVへ保存し、ローカル音声認識コマンドで文字起こしして議事録を作る。外部APIは使わない。".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["start", "stop", "status", "transcribe", "minutes"], "description": "録音開始/停止/状態/文字起こし/議事録生成。" },
                "path": { "type": "string", "description": "録音WAV、文字起こし、または議事録の対象パス。" },
                "audio_path": { "type": "string", "description": "文字起こし対象のWAVファイル。" },
                "transcript_path": { "type": "string", "description": "議事録生成に使う文字起こしテキスト。" },
                "output_path": { "type": "string", "description": "議事録の保存先。" },
                "title": { "type": "string", "description": "議事録タイトル。" },
                "transcript": { "type": "string", "description": "既存の文字起こし本文。" },
                "diarize": { "type": "boolean", "description": "whisper.cpp の tinydiarize 等、ローカル話者分類引数を追加する。" }
            },
            "required": ["action"]
        })
    }

    fn config_schema(&self) -> Vec<Field> {
        vec![
            Field::new(
                "PICOAGENT_MEETING_ROOT",
                "録音・議事録の保存許可ルート（空なら共通ワークスペース）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_PROGRAM",
                "ローカル文字起こしコマンド",
                "text",
                "whisper-cli",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_MODEL",
                "ローカル文字起こしモデル",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_ARGS",
                "ローカル文字起こし引数",
                "text",
                "-m {model} -f {audio} -otxt -of {output_stem}",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_DIARIZE_ARGS",
                "ローカル話者分類引数",
                "text",
                "-tdrz",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_TIMEOUT_SECS",
                "ローカル文字起こしタイムアウト秒",
                "text",
                "1800",
            ),
        ]
    }

    async fn run(&self, cfg: &Config, args: Value) -> String {
        match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "start" => start_recording(cfg, &args),
            "stop" => stop_recording(),
            "status" => status(),
            "transcribe" => transcribe(cfg, &args).await,
            "minutes" => make_minutes(cfg, &args),
            _ => "エラー: action は start/stop/status/transcribe/minutes のいずれかを指定してください。".to_string(),
        }
    }
}

inventory::submit! { ToolReg { make: || Box::new(MeetingRecorderTool) } }

#[cfg(test)]
mod tests {
    use super::{
        append_diarize_args, default_recording_path, minutes_from_text,
        minutes_path_from_transcript, split_args_template,
    };
    use crate::config::Config;
    use std::path::Path;

    #[test]
    fn 引数テンプレートを展開する() {
        let args = split_args_template(
            "-m {model} -f {audio} -otxt -of {output_stem}",
            Path::new("a.wav"),
            "model.bin",
            Path::new("out"),
        );
        assert_eq!(
            args,
            ["-m", "model.bin", "-f", "a.wav", "-otxt", "-of", "out"]
        );
    }

    #[test]
    fn 議事録を生成する() {
        let minutes = minutes_from_text("定例", "Aを確認\nBを決定");
        assert!(minutes.contains("# 議事録: 定例"));
        assert!(minutes.contains("Aを確認"));
        assert!(minutes.contains("TODO"));
    }

    #[test]
    fn 話者分類引数を追加する() {
        let cfg = Config::default();
        let mut args = vec!["-m".to_string(), "model.bin".to_string()];
        append_diarize_args(&cfg, &mut args, true);
        assert!(args.contains(&"-tdrz".to_string()));
    }

    #[test]
    fn 話者交代マーカーを議事録に反映する() {
        let minutes = minutes_from_text("定例", "A\n[SPEAKER_TURN]\nB");
        assert!(minutes.contains("話者交代マーカー"));
    }

    #[test]
    fn 既定録音パスは日付フォルダに置く() {
        let path = default_recording_path();
        assert!(path.starts_with("meetings/"));
        assert!(path.ends_with("-meeting.wav"));
        assert!(path.matches('/').count() >= 2);
    }

    #[test]
    fn transcript名から議事録名を揃える() {
        let path = minutes_path_from_transcript(Path::new("meetings/2026-06-30/a.transcript.txt"));
        assert_eq!(
            path,
            Path::new("meetings/2026-06-30/a.minutes.md").to_path_buf()
        );
    }
}
