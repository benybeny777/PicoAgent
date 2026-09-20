// meeting_recorder/mod.rs - ローカル録音、ローカル文字起こし、議事録生成スキル。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
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

use super::{Tool, ToolReg};
use crate::config::Config;
use crate::message::Field;
use crate::permissions::SafePath;

const MAX_TRANSCRIPT_CHARS: usize = 80_000;
const MAX_COMMAND_OUTPUT_BYTES: usize = 32 * 1024;

struct RecordingSink {
    writer: Option<WavWriter<BufWriter<File>>>,
    error: Option<String>,
}

type SharedWriter = Arc<Mutex<RecordingSink>>;

struct RecordingState {
    path: SafePath,
    started: Instant,
    stop_tx: Sender<()>,
    join: JoinHandle<Result<(), String>>,
}

static RECORDING: OnceLock<Mutex<Option<RecordingState>>> = OnceLock::new();

pub struct MeetingRecorderTool;

fn recording_state() -> &'static Mutex<Option<RecordingState>> {
    RECORDING.get_or_init(|| Mutex::new(None))
}

fn meeting_root(cfg: &Config) -> PathBuf {
    crate::permissions::primary_root(cfg)
}

fn bounded_path(cfg: &Config, input: &str) -> Result<SafePath, String> {
    crate::permissions::resolve(cfg, input)
}

fn default_recording_path() -> String {
    format!(
        "meetings/{}/{}-meeting.wav",
        Local::now().format("%Y-%m-%d"),
        Local::now().format("%Y%m%d-%H%M%S")
    )
}

fn latest_file(cfg: &Config, base: &Path, matcher: fn(&Path) -> bool) -> Option<SafePath> {
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
    // 走査で得たパスも許可判定を通してから返す（型で「検証済み」を担保する）。
    best.and_then(|(path, _)| crate::permissions::resolve(cfg, &path.to_string_lossy()).ok())
}

fn is_wav(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("wav")
}

fn is_transcript(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".transcript.txt"))
}

fn output_stem(cfg: &Config, audio_path: &SafePath) -> Result<SafePath, String> {
    audio_path.sibling(cfg, &audio_path.file_stem_lossy())
}

fn transcript_path(cfg: &Config, audio_path: &SafePath) -> Result<SafePath, String> {
    audio_path.sibling(
        cfg,
        &format!("{}.transcript.txt", audio_path.file_stem_lossy()),
    )
}

fn minutes_path(cfg: &Config, audio_path: &SafePath) -> Result<SafePath, String> {
    audio_path.sibling(cfg, &format!("{}.minutes.md", audio_path.file_stem_lossy()))
}

fn minutes_path_from_transcript(
    cfg: &Config,
    transcript_path: &SafePath,
) -> Result<SafePath, String> {
    let name = transcript_path.file_name_lossy();
    let stem = name
        .strip_suffix(".transcript.txt")
        .map(ToString::to_string)
        .unwrap_or_else(|| transcript_path.file_stem_lossy());
    transcript_path.sibling(cfg, &format!("{stem}.minutes.md"))
}

fn bundled_whisper_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = crate::components::runtime_root() {
        dirs.push(root.join("whisper"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("src-tauri/resources/bundled/whisper"));
        dirs.push(cwd.join("resources/bundled/whisper"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("resources/bundled/whisper"));
            dirs.push(parent.join("../Resources/resources/bundled/whisper"));
            dirs.push(parent.join("../../resources/bundled/whisper"));
        }
    }
    dirs
}

fn bundled_whisper_dir() -> Option<PathBuf> {
    bundled_whisper_dirs()
        .into_iter()
        .find(|path| path.join(whisper_exe_name()).exists())
}

fn whisper_exe_name() -> &'static str {
    if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    }
}

fn first_bundled_model(dir: &Path) -> Option<PathBuf> {
    let model_dir = dir.join("models");
    let entries = std::fs::read_dir(model_dir).ok()?;
    let mut models: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("bin"))
        .collect();
    models.sort();
    models.into_iter().next()
}

fn transcribe_program(cfg: &Config) -> String {
    let configured = cfg.get("PICOAGENT_TRANSCRIBE_PROGRAM", "");
    if !configured.trim().is_empty() {
        return configured;
    }
    bundled_whisper_dir()
        .map(|dir| dir.join(whisper_exe_name()).to_string_lossy().to_string())
        .unwrap_or_else(|| "whisper-cli".to_string())
}

fn transcribe_model(cfg: &Config) -> String {
    let configured = cfg.get("PICOAGENT_TRANSCRIBE_MODEL", "");
    if !configured.trim().is_empty() {
        return configured;
    }
    bundled_whisper_dir()
        .and_then(|dir| first_bundled_model(&dir))
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn wav_spec(config: &cpal::SupportedStreamConfig) -> WavSpec {
    WavSpec {
        channels: config.channels(),
        sample_rate: config.sample_rate(),
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
    let mut guard = match writer.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.error = Some("録音データのロックが破損しました。".to_string());
            return;
        }
    };
    let Some(wav) = guard.writer.as_mut() else {
        return;
    };
    let write_error = data.iter().find_map(|sample| {
        let value: i16 = i16::from_sample(*sample);
        wav.write_sample(value).err()
    });
    if let Some(error) = write_error {
        guard.error = Some(format!("録音データを書き込めません: {error}"));
        guard.writer = None;
    }
}

fn write_input_f32(data: &[f32], writer: &SharedWriter) {
    let mut guard = match writer.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.error = Some("録音データのロックが破損しました。".to_string());
            return;
        }
    };
    let Some(wav) = guard.writer.as_mut() else {
        return;
    };
    let write_error = data
        .iter()
        .find_map(|sample| wav.write_sample(*sample).err());
    if let Some(error) = write_error {
        guard.error = Some(format!("録音データを書き込めません: {error}"));
        guard.writer = None;
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
    let requested = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(default_recording_path);
    let path = match bounded_path(cfg, &requested) {
        Ok(path) => path,
        Err(error) => return format!("エラー: {error}"),
    };
    if path.extension_lossy() != "wav" {
        return "エラー: 録音ファイルは .wav で保存してください。".to_string();
    }
    {
        if let Err(error) = crate::permissions::fs::create_dir_all(&path.parent_or_root(cfg)) {
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
            return Ok(());
        };
        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(error) => {
                let _ = ready_tx.send(Err(format!("マイク設定を取得できません: {error}")));
                return Ok(());
            }
        };
        let spec = wav_spec(&config);
        let writer = match WavWriter::create(&thread_path, spec) {
            Ok(writer) => Arc::new(Mutex::new(RecordingSink {
                writer: Some(writer),
                error: None,
            })),
            Err(error) => {
                let _ = ready_tx.send(Err(format!("WAVファイルを作成できません: {error}")));
                return Ok(());
            }
        };
        let writer_for_stream = writer.clone();
        let error_callback = |error| {
            crate::log_paths::log_detached(&format!(
                "会議録音の取り込みでエラーが発生しました: {error}"
            ))
        };
        let stream_config: cpal::StreamConfig = config.into();
        let stream = match config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                stream_config,
                move |data: &[f32], _| write_input_f32(data, &writer_for_stream),
                error_callback,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                stream_config,
                move |data: &[i16], _| write_input(data, &writer_for_stream),
                error_callback,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                stream_config,
                move |data: &[u16], _| write_input(data, &writer_for_stream),
                error_callback,
                None,
            ),
            other => {
                let _ = ready_tx.send(Err(format!("未対応の録音サンプル形式です: {other:?}")));
                return Ok(());
            }
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                let _ = ready_tx.send(Err(format!("録音ストリームを開始できません: {error}")));
                return Ok(());
            }
        };
        if let Err(error) = stream.play() {
            let _ = ready_tx.send(Err(format!("録音を開始できません: {error}")));
            return Ok(());
        }
        let _ = ready_tx.send(Ok(()));
        if stop_rx.recv().is_err() {
            return Err("録音停止シグナルの受信に失敗しました。".to_string());
        }
        drop(stream);
        let mut sink = writer
            .lock()
            .map_err(|_| "録音データの状態を取得できません。".to_string())?;
        if let Some(error) = sink.error.take() {
            return Err(error);
        }
        if let Some(writer) = sink.writer.take() {
            writer
                .finalize()
                .map_err(|error| format!("WAVファイルを確定できません: {error}"))?;
        }
        Ok(())
    });
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let thread_error = join
                .join()
                .map_err(|_| "録音スレッドが異常終了しました。".to_string())
                .and_then(|result| result)
                .err();
            return format!(
                "エラー: {error}{}",
                thread_error
                    .map(|detail| format!("\n録音スレッドの終了にも失敗しました: {detail}"))
                    .unwrap_or_default()
            );
        }
        Err(_) => {
            let signal_error = stop_tx.send(()).err();
            // OSの音声ドライバ内で準備が止まっている可能性があるため、ここでjoinして
            // UI側まで固めない。停止通知は先に送り、専用の回収スレッドへ所有権を渡す。
            let reaper_error = thread::Builder::new()
                .name("picoagent-recording-reaper".to_string())
                .spawn(move || match join.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => crate::log_paths::log_detached(&format!(
                        "録音スレッドの終了に失敗しました: {error}"
                    )),
                    Err(_) => crate::log_paths::log_detached("録音スレッドが異常終了しました"),
                })
                .err();
            let cleanup = signal_error
                .map(|error| format!("停止通知失敗: {error}"))
                .into_iter()
                .chain(reaper_error.map(|error| format!("回収スレッド起動失敗: {error}")))
                .collect::<Vec<_>>()
                .join("、");
            return if cleanup.is_empty() {
                "エラー: 録音開始の準備がタイムアウトしました。".to_string()
            } else {
                format!("エラー: 録音開始の準備がタイムアウトしました。{cleanup}")
            };
        }
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
    if let Err(error) = active.stop_tx.send(()) {
        return format!("エラー: 録音停止を通知できません: {error}");
    }
    match active.join.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return format!("エラー: {error}"),
        Err(_) => return "エラー: 録音スレッドが異常終了しました。".to_string(),
    }
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
async fn run_transcribe_command(
    program: &str,
    args_list: &[String],
    timeout: u64,
) -> Result<Output, String> {
    let mut command = crate::tools::process::command(program);
    command.args(args_list).stdin(Stdio::null());
    crate::tools::process::output(command, timeout, "ローカル文字起こし")
        .await
        .map_err(|error| format!("エラー: {error}"))
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
        match bounded_path(cfg, audio_text) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        }
    } else {
        match latest_file(cfg, &base, is_wav) {
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
    let program = transcribe_program(cfg);
    let model = transcribe_model(cfg);
    if model.trim().is_empty() {
        return super::config_guide(
            "ローカル文字起こし",
            &["PICOAGENT_TRANSCRIBE_MODEL"],
            "cargo xtask setup whisper で同梱Whisperとモデルを取得するか、ローカルWhisperモデルのパスを指定してください。外部APIは使いません。",
        );
    }
    let output_stem = match output_stem(cfg, &audio) {
        Ok(path) => path,
        Err(error) => return format!("エラー: {error}"),
    };
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
    let output = match run_transcribe_command(&program, &args_list, timeout).await {
        Ok(output) => output,
        Err(error) => return error,
    };
    if !output.status.success() {
        return format!(
            "エラー: ローカル文字起こしが失敗しました。\nstdout:\n{}\nstderr:\n{}",
            trim_output(&output.stdout),
            trim_output(&output.stderr)
        );
    }
    let transcript =
        match output_stem.sibling(cfg, &format!("{}.txt", output_stem.file_stem_lossy())) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        };
    let content = if transcript.exists() {
        match std::fs::read_to_string(&transcript) {
            Ok(content) => content,
            Err(error) => return format!("エラー: 文字起こし結果を読めません: {error}"),
        }
    } else {
        trim_output(&output.stdout)
    };
    let target = match transcript_path(cfg, &audio) {
        Ok(path) => path,
        Err(error) => return format!("エラー: {error}"),
    };
    if let Err(error) = crate::permissions::fs::write(&target, &content) {
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
    let mut transcript_source: Option<SafePath> = None;
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
            match bounded_path(cfg, path_text) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            }
        } else {
            match latest_file(cfg, &base, is_transcript) {
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
        match bounded_path(cfg, path_text) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        }
    } else if let Some(path_text) = args
        .get("audio_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match bounded_path(cfg, path_text) {
            Ok(path) => match minutes_path(cfg, &path) {
                Ok(path) => path,
                Err(error) => return format!("エラー: {error}"),
            },
            Err(error) => return format!("エラー: {error}"),
        }
    } else if let Some(path) = &transcript_source {
        match minutes_path_from_transcript(cfg, path) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        }
    } else {
        // 既定の保存先も許可判定を通す（主ルート配下の相対パスとして再検証される）。
        match crate::permissions::resolve(
            cfg,
            &format!(
                "meetings/{}/{}-minutes.md",
                Local::now().format("%Y-%m-%d"),
                Local::now().format("%Y%m%d-%H%M%S")
            ),
        ) {
            Ok(path) => path,
            Err(error) => return format!("エラー: {error}"),
        }
    };
    {
        if let Err(error) = crate::permissions::fs::create_dir_all(&output.parent_or_root(cfg)) {
            return format!("エラー: 議事録フォルダを作成できません: {error}");
        }
    }
    let minutes = minutes_from_text(title, &transcript);
    match crate::permissions::fs::write(&output, minutes) {
        Ok(()) => format!("議事録を作成しました: {}", output.display()),
        Err(error) => format!("エラー: 議事録を保存できません: {error}"),
    }
}

#[async_trait]
impl Tool for MeetingRecorderTool {
    fn name(&self) -> &'static str {
        "meeting_recorder"
    }

    fn intent_keywords(&self) -> &'static [&'static str] {
        &["録音", "議事録", "会議メモ", "meeting notes"]
    }

    fn intent_priority(&self) -> i32 {
        790
    }

    fn intent_invocation(&self, text: &str) -> Option<crate::tools::IntentInvocation> {
        if !self.matches_intent(text) {
            return None;
        }
        let lower = text.to_lowercase();
        let action = if ["停止", "止め", "終了", "stop"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()))
        {
            "stop"
        } else if ["状態", "ステータス", "status", "録音中"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()))
        {
            "status"
        } else if ["文字起こし", "whisper", "transcribe"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()))
        {
            "transcribe"
        } else if ["議事録", "会議メモ", "meeting notes"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()))
        {
            "minutes"
        } else {
            "start"
        };
        let diarize = ["話者", "speaker", "diarize"]
            .iter()
            .any(|word| lower.contains(&word.to_lowercase()));
        Some(crate::tools::IntentInvocation::direct(json!({
            "action": action,
            "title": text.trim(),
            "diarize": diarize
        })))
    }

    fn label(&self) -> &'static str {
        "会議記録"
    }

    fn description(&self) -> String {
        "会議向けに、マイク録音、既存音声の文字起こし、文字起こしからの議事録Markdown作成までまとめて扱う。外部APIは使わない。".to_string()
    }

    fn examples(&self) -> Vec<&'static str> {
        vec![
            "会議の録音を開始して",
            "録音を止めて",
            "話者分類つきで議事録にして",
        ]
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
                "PICOAGENT_TRANSCRIBE_PROGRAM",
                "ローカル文字起こしコマンド（手動設定が最優先、空なら同梱Whisper）",
                "text",
                "",
            ),
            Field::new(
                "PICOAGENT_TRANSCRIBE_MODEL",
                "ローカル文字起こしモデル（手動設定が最優先、空なら同梱モデル）",
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
    use crate::permissions::SafePath;
    use std::path::Path;

    #[test]
    fn 引数テンプレートを展開する() {
        let args = split_args_template(
            "-m {model} -f {audio} -otxt -of {output_stem}",
            &SafePath::app_owned(Path::new("a.wav").to_path_buf()),
            "model.bin",
            &SafePath::app_owned(Path::new("out").to_path_buf()),
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
        let dir = std::env::temp_dir().join(format!("picoagent-minutes-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("meetings/2026-06-30")).unwrap();
        let cfg = Config::new();
        cfg.set("PICOAGENT_ALLOWED_ROOT", &dir.to_string_lossy());
        let transcript =
            crate::permissions::resolve(&cfg, "meetings/2026-06-30/a.transcript.txt").unwrap();
        let path = minutes_path_from_transcript(&cfg, &transcript).unwrap();
        assert!(
            path.as_path().ends_with("meetings/2026-06-30/a.minutes.md"),
            "path: {}",
            path.display()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
