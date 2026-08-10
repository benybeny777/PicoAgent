// http.rs - 外向きHTTPの共有クライアントと設定値解決。

use std::sync::OnceLock;
use std::time::Duration;

use crate::config::Config;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// 接続確立の上限。応答本文には掛からないので、モデルDLや生成のストリーミングは妨げない。
/// 到達しないホストでTCP接続がOS既定（分単位）まで滞留するのを防ぐ。
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// 接続プールを共有する標準クライアント。個別の待ち時間はRequestBuilder::timeoutで指定する。
pub fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .build()
            // フォールバック許可: TLSバックエンド初期化の失敗でHTTPを一切使えなくしないため、
            // 接続上限なしの既定クライアントで続行する。
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// モデル一覧のような短い問い合わせの待ち時間。設定画面のプロバイダ選択が固まらないようにする。
pub fn metadata_timeout(cfg: &Config) -> Duration {
    timeout(cfg, "PICOAGENT_HTTP_METADATA_TIMEOUT_SECS", 15, 3, 60)
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

/// 外部サービスの応答本文を、読取失敗を空本文へ落とさず取得する。
pub async fn response_text(response: reqwest::Response) -> Result<String, String> {
    response
        .text()
        .await
        .map_err(|error| format!("外部サービスの応答本文を読み取れません: {error}"))
}

/// 外部サービスがエラー応答を返したときの、利用者へ出す共通メッセージ。
///
/// この文字列は**キャラクターのフキダシにそのまま出る**。以前は各プロバイダが
/// `"Ollama /api/chat failed: {status} {body}"` のように英語の定型文と応答本文を
/// 無制限に連結しており、日本語の利用者に英語＋JSONの塊が見えていた。
/// 本文は原因の手がかりとして残すが、長さを抑える（全文は診断ログへ残る）。
pub fn service_error(
    service: &str,
    endpoint: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> String {
    let detail = crate::text::preview(body.trim(), 200);
    if detail.is_empty() {
        format!("{service} がエラーを返しました（{endpoint} / HTTP {status}）")
    } else {
        format!("{service} がエラーを返しました（{endpoint} / HTTP {status}）: {detail}")
    }
}

/// 外部サービスのJSONを、解析失敗を空オブジェクトへ落とさず取得する。
pub fn response_json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text)
        .map_err(|error| format!("外部サービスの応答JSONを解析できません: {error}"))
}

/// `Tool::run()` の `String` 戻り値で、本文読取失敗をその場でエラー文字列へ変換する。
#[macro_export]
macro_rules! tool_response_text {
    ($response:expr) => {
        match $crate::tools::http::response_text($response).await {
            Ok(body) => body,
            Err(error) => return format!("エラー: {error}"),
        }
    };
}

/// `Tool::run()` の `String` 戻り値で、不正JSONをその場でエラー文字列へ変換する。
#[macro_export]
macro_rules! tool_response_json {
    ($text:expr) => {
        match $crate::tools::http::response_json($text) {
            Ok(value) => value,
            Err(error) => return format!("エラー: {error}"),
        }
    };
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
    fn 不正な応答jsonを空オブジェクトへ落とさない() {
        let error = super::response_json("not-json").expect_err("不正JSONはエラーにする");
        assert!(error.contains("応答JSONを解析できません"));
    }
}

/// 設計ガードは `guard` モジュールへ置く（`::guard::` で一覧できないと、
/// ガードを数えるときに見落とす。実際にこのガードだけ `tests` に紛れていた）。
#[cfg(test)]
mod guard {
    #[test]
    fn 共有外の既定クライアントを増やさない() {
        // `reqwest::get()` は既定クライアントを毎回作り直すうえ待ち時間も付かないため、
        // `Client::new()` と同じ違反として検出する（この穴を通って ollama の
        // list_models が待ち時間なしのまま残っていた）。
        const BANNED: [&str; 2] = ["reqwest::Client::new()", "reqwest::get("];
        let violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| !source.path.ends_with("tools/http.rs"))
            .flat_map(|source| {
                BANNED
                    .iter()
                    .filter(|banned| source.text.contains(**banned))
                    .map(|banned| format!("{} ({banned})", source.path.display()))
                    .collect::<Vec<String>>()
            })
            .collect();
        assert!(
            violations.is_empty(),
            "標準HTTPはtools::http::client()を共有してください:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn 応答本文とjsonの失敗を空へ落とさない() {
        let banned = [
            ".text().await.unwrap_or_default()",
            ".json().await.unwrap_or(",
            "serde_json::from_str(&body).unwrap_or_else(|_| json!({}))",
            "serde_json::from_str(&body).unwrap_or_else(|_| json!([]))",
            "serde_json::from_str(&text).unwrap_or_else(|_| json!({}))",
        ];
        let violations: Vec<String> = crate::guard_scan::all_sources()
            .into_iter()
            .filter(|source| !source.path.ends_with("tools/http.rs"))
            .flat_map(|source| {
                banned
                    .iter()
                    .filter(|pattern| source.text.contains(**pattern))
                    .map(|pattern| format!("{} ({pattern})", source.path.display()))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            violations.is_empty(),
            "外部応答の失敗を空本文・空JSONへ落とさないでください:\n{}",
            violations.join("\n")
        );
    }
}
