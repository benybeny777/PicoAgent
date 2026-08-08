// http.rs - 外向きHTTPの共有クライアントと設定値解決。

use std::sync::OnceLock;
use std::time::Duration;

use crate::config::Config;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 接続プールを共有する標準クライアント。個別の待ち時間はRequestBuilder::timeoutで指定する。
pub fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(reqwest::Client::new)
}

/// 秒単位の設定を共通の範囲へ丸めてDurationへ変換する。
pub fn timeout(
    cfg: &Config,
    key: &str,
    default_secs: u64,
    min_secs: u64,
    max_secs: u64,
) -> Duration {
    let default_value = default_secs.to_string();
    let seconds = cfg
        .get(key, &default_value)
        .parse::<u64>()
        .unwrap_or(default_secs)
        .clamp(min_secs, max_secs);
    Duration::from_secs(seconds)
}

#[cfg(test)]
mod tests {
    #[test]
    fn 待ち時間設定を範囲内へ正規化する() {
        let cfg = crate::config::Config::new();
        cfg.set("TEST_TIMEOUT", "999");
        assert_eq!(
            super::timeout(&cfg, "TEST_TIMEOUT", 20, 3, 60).as_secs(),
            60
        );
        cfg.set("TEST_TIMEOUT", "broken");
        assert_eq!(
            super::timeout(&cfg, "TEST_TIMEOUT", 20, 3, 60).as_secs(),
            20
        );
    }

    #[test]
    fn 共有外の既定クライアントを増やさない() {
        let violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| !source.path.ends_with("tools/http.rs"))
            .filter_map(|source| {
                source
                    .text
                    .contains("reqwest::Client::new()")
                    .then(|| source.path.display().to_string())
            })
            .collect();
        assert!(
            violations.is_empty(),
            "標準HTTPはtools::http::client()を共有してください:\n{}",
            violations.join("\n")
        );
    }
}
