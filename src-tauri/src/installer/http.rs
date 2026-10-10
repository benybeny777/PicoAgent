// http - 公式配布物のHTTPS取得に限った共通クライアント。
use std::sync::OnceLock;
use std::time::Duration;

static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

pub(crate) fn init() -> Result<(), String> {
    CLIENT
        .get_or_init(build_client)
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .user_agent(concat!("PicoAgent/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| format!("取得用HTTPを初期化できません: {}", error.without_url()))
}

pub(crate) fn client() -> Result<&'static reqwest::Client, String> {
    initialized_client(&CLIENT)
}

fn initialized_client(
    state: &OnceLock<Result<reqwest::Client, String>>,
) -> Result<&reqwest::Client, String> {
    state
        .get()
        .ok_or_else(|| "取得用HTTPが初期化されていません".to_string())?
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    #[test]
    fn 未初期化と初期化失敗を別クライアントへ降格せず理由を保持する() {
        let state = std::sync::OnceLock::new();
        assert!(super::initialized_client(&state)
            .unwrap_err()
            .contains("初期化"));
        state.set(Err("検査用のTLS初期化拒否".to_string())).unwrap();
        assert_eq!(
            super::initialized_client(&state).unwrap_err(),
            "検査用のTLS初期化拒否"
        );
    }

    #[tokio::test]
    async fn 取得用クライアントは平文httpへ送信しない() {
        let client = super::build_client().unwrap();
        let error = client
            .get("http://127.0.0.1:1/?secret=秘密")
            .send()
            .await
            .unwrap_err()
            .without_url()
            .to_string();
        assert!(!error.contains("秘密") && !error.contains("secret"));
    }
}
