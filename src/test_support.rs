//! 複数のモジュールのテストで共有する補助関数

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use reqwest::Url;
use secrecy::SecretString;
use serde_json::json;
use tokio::{
    io::AsyncReadExt as _,
    net::{TcpListener, TcpStream},
};
use wiremock::{
    Match, Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    client::{BoxClient, BoxClientBuilder, TimeoutConfig},
    config::{BoxAppAuth, BoxAppSettings, BoxConfig},
    errors::{ApiError, BoxClientError, BoxClientResult},
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

/// すべてのフィールドを指定した再試行設定を返す。
pub(crate) fn retry_policy(
    max_retries: u32,
    initial_delay: Duration,
    max_delay: Duration,
    max_jitter: Duration,
) -> RetryPolicy {
    RetryPolicy::default()
        .with_max_retries(max_retries)
        .with_initial_delay(initial_delay)
        .with_max_delay(max_delay)
        .with_max_jitter(max_jitter)
}

/// テストが長時間待機しないように、短い待機時間の再試行設定を返す。
pub(crate) fn short_retry(max_retries: u32) -> RetryPolicy {
    retry_policy(
        max_retries,
        Duration::from_millis(10),
        Duration::from_millis(50),
        Duration::from_millis(10),
    )
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

/// モックサーバーが受信したリクエストのうち、指定したメソッドとパスのリクエストを返す。
pub(crate) async fn requests(server: &MockServer, method: &str, path: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == method && r.url.path() == path)
        .collect()
}

/// モックサーバーが受信した、指定したパスのリクエストのクエリ文字列を、受信した順に返す。
///
/// クエリ文字列がないリクエストは、空文字列として返す。
pub(crate) async fn request_queries(server: &MockServer, path: &str) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == path)
        .map(|r| r.url.query().unwrap_or_default().to_owned())
        .collect()
}

/// モックサーバーが受信した、指定したメソッドとパスのリクエストの数を返す。
pub(crate) async fn count_requests(server: &MockServer, method: &str, path: &str) -> usize {
    requests(server, method, path).await.len()
}

/// モックサーバーが受信した、指定したパスのリクエストの数をメソッドによらず返す。
pub(crate) async fn count_requests_to_path(server: &MockServer, path: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == path)
        .count()
}

/// リクエストの`If-Match`ヘッダーを返す。
pub(crate) fn if_match(request: &Request) -> Option<String> {
    request
        .headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// モックサーバーが受信した、指定したメソッドとパスのリクエストの`If-Match`ヘッダーを返す。
pub(crate) async fn if_match_headers(
    server: &MockServer,
    method: &str,
    path: &str,
) -> Vec<Option<String>> {
    requests(server, method, path)
        .await
        .iter()
        .map(if_match)
        .collect()
}

/// `matcher`に一致するリクエストに、最初の`n`回だけ`response`を返すモックを登録する。
pub(crate) async fn mount_first_n(
    server: &MockServer,
    matcher: impl Match + 'static,
    response: ResponseTemplate,
    n: u64,
) {
    Mock::given(matcher)
        .respond_with(response)
        .up_to_n_times(n)
        .mount(server)
        .await;
}

/// `matcher`に一致するリクエストに、常に`response`を返すモックを登録する。
pub(crate) async fn mount_always(
    server: &MockServer,
    matcher: impl Match + 'static,
    response: ResponseTemplate,
) {
    Mock::given(matcher)
        .respond_with(response)
        .mount(server)
        .await;
}

/// 項目が存在しないことを示す、BOXの404のエラーレスポンスを返す。
pub(crate) fn not_found_response() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(json!({
        "type": "error",
        "status": 404,
        "code": "not_found",
        "message": "Not Found",
    }))
}

/// 他の操作によるロックを示す、エラーコードが`operation_blocked_temporary`の409のエラーレスポンスを返す。
pub(crate) fn operation_blocked_response() -> ResponseTemplate {
    ResponseTemplate::new(409).set_body_json(json!({
        "type": "error",
        "status": 409,
        "code": "operation_blocked_temporary",
        "message": "The operation is blocked by another on-going operation",
    }))
}

/// 接続からリクエストヘッダーの終わり（空行）まで読み込む。
///
/// 接続が閉じられた場合または読み込みに失敗した場合は、その時点で読み込みを終える。
pub(crate) async fn read_request_head(stream: &mut TcpStream) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
}

/// ローカルホストの空いているポートで待ち受け、接続ごとにレスポンスを返すHTTPサーバーを起動する。
///
/// サーバーは、接続を受け付けた順に、リクエストヘッダーを読み終えてから`handler`を呼び出す。
/// `handler`は、0から始まる接続の番号と接続を受け取り、レスポンスを書き込む。
/// 戻り値は、サーバーを指すREST APIのベースURLと、サーバーが受け付けた接続の数である。
pub(crate) async fn start_raw_server<F, Fut>(handler: F) -> anyhow::Result<(Url, Arc<AtomicUsize>)>
where
    F: Fn(usize, TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let api_base_url = Url::parse(&format!("http://{}/2.0", listener.local_addr()?))?;
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&connections);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let connection = counter.fetch_add(1, Ordering::SeqCst);
            read_request_head(&mut stream).await;
            handler(connection, stream).await;
        }
    });

    Ok((api_base_url, connections))
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
pub(crate) fn client_builder(server: &MockServer, retry: RetryPolicy) -> BoxClientBuilder {
    BoxClient::builder(test_config())
        .timeout(TimeoutConfig {
            request_timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(1),
            download_read_timeout: Duration::from_secs(2),
        })
        .retry(retry)
        .api_base_url(&format!("{}/2.0", server.uri()))
        .upload_base_url(&format!("{}{UPLOAD_BASE_PATH}", server.uri()))
        .token_url(&format!("{}{TOKEN_PATH}", server.uri()))
}

/// `api_base_url`のサーバーにREST APIのリクエストを送信し、`token_server`からアクセストークンを取得するBOXクライアントのビルダーを返す。
///
/// `start_raw_server`で起動したサーバーなど、モックサーバー以外のサーバーに接続するテストで使用する。
pub(crate) fn raw_server_client_builder(
    api_base_url: &Url,
    token_server: &MockServer,
    retry: RetryPolicy,
) -> BoxClientBuilder {
    BoxClient::builder(test_config())
        .retry(retry)
        .api_base_url(api_base_url.as_str())
        .token_url(&format!("{}{TOKEN_PATH}", token_server.uri()))
}

/// アップロードのAPIのパスを返す。
pub(crate) fn upload_path(suffix: &str) -> String {
    format!("{UPLOAD_BASE_PATH}{suffix}")
}

/// タイムアウトを指定して、モックサーバーに接続するBOXクライアントを作成する。
pub(crate) async fn create_client_with_timeout(
    server: &MockServer,
    retry: RetryPolicy,
    timeout: Duration,
) -> BoxClientResult<BoxClient> {
    client_builder(server, retry)
        .timeout(TimeoutConfig {
            request_timeout: timeout,
            connect_timeout: Duration::from_secs(1),
            download_read_timeout: Duration::from_secs(2),
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
    client_builder(server, retry)
        .upload_concurrency(upload_concurrency)
        .build()
        .await
}

/// BOXがエラーレスポンスを返した場合に、BOXクライアントが再試行せずにエラーを返すことを確認する。
///
/// この関数は、モックサーバーの`http_method`と`request_path`に`status`のエラーレスポンスを登録し、
/// 再試行を有効にしたBOXクライアントで`call`を呼び出す。
/// そして、`call`がステータスが`status`の[`BoxClientError::Api`]を返すこと、および
/// BOXクライアントがリクエストを1回だけ送信したことを確認する。
pub(crate) async fn assert_not_retried<T: std::fmt::Debug>(
    http_method: &str,
    request_path: &str,
    status: u16,
    call: impl AsyncFnOnce(&BoxClient) -> BoxClientResult<T>,
) -> anyhow::Result<()> {
    let error = assert_not_retried_with_response(
        http_method,
        request_path,
        ResponseTemplate::new(status),
        call,
    )
    .await?;
    assert_eq!(error.status, status, "error: {error:?}");

    Ok(())
}

/// BOXが`response`を返した場合に、BOXクライアントが再試行せずにエラーを返すことを確認する。
///
/// この関数は、[`assert_not_retried`]と同様に、`call`が[`BoxClientError::Api`]を返すこと、および
/// BOXクライアントがリクエストを1回だけ送信したことを確認する。
/// 呼び出し側がステータスやエラーコードを検証できるように、この関数はBOXのエラーを返す。
pub(crate) async fn assert_not_retried_with_response<T: std::fmt::Debug>(
    http_method: &str,
    request_path: &str,
    response: ResponseTemplate,
    call: impl AsyncFnOnce(&BoxClient) -> BoxClientResult<T>,
) -> anyhow::Result<Box<ApiError>> {
    let server = MockServer::start().await;
    mount_token(&server).await;
    Mock::given(method(http_method))
        .and(path(request_path))
        .respond_with(response)
        .mount(&server)
        .await;
    let client = create_client(&server, short_retry(2)).await?;

    let result = call(&client).await;

    let Err(BoxClientError::Api(error)) = result else {
        panic!("result: {result:?}");
    };
    assert_eq!(count_requests(&server, http_method, request_path).await, 1);

    Ok(error)
}

/// BOXが最初に`first`を返した場合に、BOXクライアントが再試行して成功することを確認する。
///
/// この関数は、モックサーバーの`http_method`と`request_path`に、最初の1回だけ`first`を返し、
/// その後は`success`を返すモックを登録する。そして、再試行を有効にしたBOXクライアントで`call`を呼び出し、
/// `call`が成功すること、およびBOXクライアントがリクエストを2回送信したことを確認する。
/// 呼び出し側が再送したリクエストを検証できるように、この関数は受信したリクエストを返す。
pub(crate) async fn assert_retried_after<T>(
    http_method: &str,
    request_path: &str,
    first: ResponseTemplate,
    success: ResponseTemplate,
    call: impl AsyncFnOnce(&BoxClient) -> BoxClientResult<T>,
) -> anyhow::Result<Vec<Request>> {
    let server = MockServer::start().await;
    mount_token(&server).await;
    Mock::given(method(http_method))
        .and(path(request_path))
        .respond_with(first)
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method(http_method))
        .and(path(request_path))
        .respond_with(success)
        .mount(&server)
        .await;
    let client = create_client(&server, short_retry(1)).await?;

    call(&client).await?;

    let requests = requests(&server, http_method, request_path).await;
    assert_eq!(requests.len(), 2);

    Ok(requests)
}

/// エラーが[`BoxClientError::InvalidArgument`]であるか確認する。
///
/// テストケースごとに期待するエラーの種類を関数として渡すときに使用する。
pub(crate) fn is_invalid_argument(error: &BoxClientError) -> bool {
    matches!(error, BoxClientError::InvalidArgument(_))
}

/// エラーが[`BoxClientError::InvalidRequestBody`]であるか確認する。
///
/// テストケースごとに期待するエラーの種類を関数として渡すときに使用する。
pub(crate) fn is_invalid_request_body(error: &BoxClientError) -> bool {
    matches!(error, BoxClientError::InvalidRequestBody(_))
}
