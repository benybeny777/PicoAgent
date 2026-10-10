// installer - 取得・展開・配置の共通部品。管理領域を呼出元が確定してから渡す。
pub(crate) mod archive;
pub(crate) mod disk;
pub(crate) mod download;
pub(crate) mod fs;
pub(crate) mod http;
pub(crate) mod swap;

/// 用途が所有する上限。形式別上限があるときだけfile_limitを渡す。
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub file_limit: Option<fn(&std::path::Path) -> u64>,
}
