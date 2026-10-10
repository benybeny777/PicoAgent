// paths.rs - アプリと開発ツールで共有する保存先・ファイル名の正本。

use std::path::{Path, PathBuf};

/// アプリと取得ツールが共有するWhisper既定モデル名。
pub const WHISPER_DEFAULT_MODEL: &str = "ggml-small.bin";
pub fn whisper_model_at(root: &Path) -> PathBuf {
    root.join("models").join(WHISPER_DEFAULT_MODEL)
}

/// OSの利用者データ領域を受け取り、開発専用の根を返す。
pub fn development_root(os_data_dir: &Path) -> PathBuf {
    os_data_dir.join("PicoAgent-Dev")
}

pub fn product_root(os_data_dir: &Path) -> PathBuf {
    os_data_dir.join("PicoAgent")
}

/// 別識別子の検証版も同じ配置定義を使い、データの根だけを分ける。
pub fn managed_dir(root: PathBuf, identifier: &str) -> PathBuf {
    if identifier == "com.picoagent.desktop" {
        root
    } else {
        root.join("instances").join(identifier)
    }
}

pub fn packaged_bundled_at(resource_root: &Path) -> PathBuf {
    resource_root.join("resources/bundled")
}
pub fn packaged_character_maker_at(resource_root: &Path) -> PathBuf {
    resource_root.join("character-maker")
}

/// アプリが所有するファイルと領域。機能側で保存先の名前を重複定義しない。
/// 動的なID・ラベルは呼び出し元で検証済みの値だけを渡す。
#[derive(Clone, Copy)]
pub enum DataPath<'a> {
    Config,
    History(&'a str),
    Relationship(&'a str),
    Memory,
    MemoryVectors,
    Usage,
    ScheduleRuns,
    Jobs,
    Characters,
    Display,
    GeneratedMedia,
    MakerCharacters,
    MakerGpuLock,
    MakerStaging,
    Webview(&'a str),
    WebviewRoot,
    BrowserProfile,
    AivisProfile,
    OfficeProfile,
    GenerationProfile,
    Runtime(&'a str),
    Models,
    Cache,
    Log,
    Bundled,
    UserSkills,
    Gameplay,
}

/// 同じ配置定義を起動・読込・保存・回収の全経路で使う。
pub fn path_at(root: &Path, entry: DataPath<'_>) -> PathBuf {
    match entry {
        DataPath::Config => root.join("config.json"),
        DataPath::History(id) => root.join(format!("history-{id}.json")),
        DataPath::Relationship(id) => root.join(format!("relationship-{id}.json")),
        DataPath::Memory => root.join("memory.json"),
        DataPath::MemoryVectors => root.join("memory-vectors.json"),
        DataPath::Usage => root.join("usage.json"),
        DataPath::ScheduleRuns => root.join("schedule-runs.json"),
        DataPath::Jobs => root.join("durable-jobs.json"),
        DataPath::Characters => root.join("characters"),
        DataPath::Display => root.join("display"),
        DataPath::GeneratedMedia => root.join("generated-media"),
        DataPath::MakerCharacters => root.join("character-maker/characters"),
        DataPath::MakerGpuLock => {
            path_at(root, DataPath::MakerCharacters).join("temp/completion-gpu.lock")
        }
        DataPath::MakerStaging => root.join("character-maker/staging"),
        DataPath::Webview(label) => root.join("webview").join(label),
        DataPath::WebviewRoot => root.join("webview"),
        DataPath::BrowserProfile => root.join("browser-profile"),
        DataPath::AivisProfile => root.join("aivis-userdata/profile"),
        DataPath::OfficeProfile => root.join("office-profile"),
        DataPath::GenerationProfile => root.join("generation-runtime-data"),
        DataPath::Runtime(id) => root.join("bundled").join(id),
        DataPath::Models => root.join("bundled/models"),
        DataPath::Cache => root.join("cache"),
        DataPath::Log => root.join("log"),
        DataPath::Bundled => root.join("bundled"),
        DataPath::UserSkills => root.join("user-skills"),
        DataPath::Gameplay => root.join("gameplay"),
    }
}

/// assetプロトコルが読み込める管理領域。Tauriの静的許可との一致はアプリ側で検査する。
pub const ASSET_SCOPE_DIRS: [&str; 4] = [
    "characters",
    "display",
    "character-maker/characters",
    "generated-media",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// assetプロトコルで読む領域は、保存側も同じ配置定義（`DataPath`）を通す。
    /// 定義が無いと保存側が別の場所（Tauriの識別子別フォルダなど）へ書き、表示が黙って消える。
    #[test]
    fn asset許可の各領域は保存側の配置定義と一致する() {
        let root = Path::new("root");
        let saved = [
            path_at(root, DataPath::Characters),
            path_at(root, DataPath::Display),
            path_at(root, DataPath::MakerCharacters),
            path_at(root, DataPath::GeneratedMedia),
        ];
        let allowed: Vec<PathBuf> = ASSET_SCOPE_DIRS.iter().map(|dir| root.join(dir)).collect();
        assert_eq!(allowed, saved);
    }
}
