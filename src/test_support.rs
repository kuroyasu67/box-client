//! 複数のモジュールのテストで共有する補助関数

use std::time::Duration;

use reqwest::Url;
use secrecy::SecretString;
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use crate::{
    client::{BoxClient, BoxClientBuilder, BoxClientTimeout},
    config::{BoxAppAuth, BoxAppSettings, BoxConfig},
    errors::{BoxClientError, BoxClientResult},
    retry::RetryPolicy,
};

/// トークンエンドポイントのパス
pub(crate) const TOKEN_PATH: &str = "/oauth2/token";

/// アップロードのAPIのベースURLのパス
///
/// REST APIのパス（`/2.0`）と区別できるように、BOXと同じく`/api/2.0`とする。
pub(crate) const UPLOAD_BASE_PATH: &str = "/api/2.0";

/// テスト用のBOX設定を返す。
///
/// 秘密鍵はテスト専用に生成した鍵である。
pub(crate) fn test_config() -> BoxConfig {
    BoxConfig {
        box_app_settings: BoxAppSettings {
            client_id: "client-id".to_owned(),
            client_secret: SecretString::from("client-secret"),
            app_auth: BoxAppAuth {
                public_key_id: "public-key-id".to_owned(),
                private_key: SecretString::from(include_str!(
                    "../resources/tests/private_key_encrypted.pem"
                )),
                passphrase: SecretString::from("test-passphrase"),
            },
        },
        enterprise_id: "12345".to_owned(),
    }
}

/// テストが長時間待機しないように、短い待機時間の再試行設定を返す。
pub(crate) fn short_retry(max_retries: u32) -> anyhow::Result<RetryPolicy> {
    Ok(RetryPolicy::new(
        max_retries,
        Duration::from_millis(10),
        Duration::from_millis(50),
        Duration::from_millis(10),
    )?)
}

/// アクセストークンを含むトークンエンドポイントの成功レスポンスを返す。
pub(crate) fn token_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "access_token": "access-token",
        "expires_in": 3600,
        "token_type": "bearer",
    }))
}

/// トークンエンドポイントが常にアクセストークンを返すモックを登録する。
pub(crate) async fn mount_token(server: &MockServer) {
    Mock::given(path(TOKEN_PATH))
        .respond_with(token_response())
        .mount(server)
        .await;
}

/// モックサーバーが受信したリクエストの数を返す。
pub(crate) async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |r| r.len())
}

/// モックサーバーに接続するBOXクライアントを作成する。
///
/// 呼び出し側がエラーの種類を検証できるように、エラーを`BoxClientError`のまま返す。
pub(crate) async fn create_client(
    server: &MockServer,
    retry: RetryPolicy,
) -> BoxClientResult<BoxClient> {
    create_client_with_timeout(server, retry, Duration::from_secs(2)).await
}

/// モックサーバーに接続し、テストが長時間待機しないタイムアウトを設定したBOXクライアントのビルダーを返す。
///
/// 呼び出し側は、テストする設定をビルダーに追加してからBOXクライアントを作成する。
pub(crate) fn client_builder(
    server: &MockServer,
    retry: RetryPolicy,
) -> BoxClientResult<BoxClientBuilder> {
    let api_base_url = Url::parse(&format!("{}/2.0", server.uri()))
        .map_err(|e| BoxClientError::InvalidBaseUrl(e.to_string()))?;
    let upload_base_url = Url::parse(&format!("{}{UPLOAD_BASE_PATH}", server.uri()))
        .map_err(|e| BoxClientError::InvalidBaseUrl(e.to_string()))?;
    Ok(BoxClient::builder(test_config())
        .timeout(BoxClientTimeout {
            request_timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
        })
        .retry(retry)
        .endpoints(api_base_url, format!("{}{TOKEN_PATH}", server.uri()))
        .upload_base_url(upload_base_url))
}

/// タイムアウトを指定して、モックサーバーに接続するBOXクライアントを作成する。
pub(crate) async fn create_client_with_timeout(
    server: &MockServer,
    retry: RetryPolicy,
    timeout: Duration,
) -> BoxClientResult<BoxClient> {
    client_builder(server, retry)?
        .timeout(BoxClientTimeout {
            request_timeout: timeout,
            connect_timeout: Duration::from_secs(1),
        })
        .build()
        .await
}

/// 分割アップロードの並列数を指定して、モックサーバーに接続するBOXクライアントを作成する。
pub(crate) async fn create_client_with_upload_concurrency(
    server: &MockServer,
    retry: RetryPolicy,
    upload_concurrency: usize,
) -> BoxClientResult<BoxClient> {
    client_builder(server, retry)?
        .upload_concurrency(upload_concurrency)?
        .build()
        .await
}
