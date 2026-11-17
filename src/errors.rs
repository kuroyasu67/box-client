use std::{fmt, time::Duration};

use reqwest::StatusCode;
use serde::Deserialize;

/// BOXクライアントエラー型
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BoxClientError {
    /// BOX設定ファイルのオープンエラー
    #[error("Failed to open config file: {0}")]
    OpenConfigFile(std::io::Error),

    /// BOX設定ファイルの解析エラー
    #[error("Failed to parse config file: {0}")]
    ParseConfig(serde_json::Error),

    /// 再試行設定の不正エラー
    #[error("Invalid retry policy: {0}")]
    InvalidRetryPolicy(&'static str),

    /// APIメソッドの引数の不正エラー
    #[error("Invalid argument: {0}")]
    InvalidArgument(&'static str),

    /// BOXが想定外のレスポンスを返したエラー
    #[error("Unexpected response: {0}")]
    UnexpectedResponse(&'static str),

    /// ベースURLの不正エラー
    #[error("Invalid base URL: {0}")]
    InvalidBaseUrl(String),

    /// パス要素の不正エラー
    ///
    /// 空文字列、`.`または`..`をパス要素に指定すると、意図しないエンドポイントを呼び出すため拒否する。
    #[error("Invalid path segment: {0:?}")]
    InvalidPathSegment(String),

    /// リクエストヘッダーの値の不正エラー
    #[error("Invalid header value: {0}")]
    InvalidHeaderValue(reqwest::header::InvalidHeaderValue),

    /// リクエストボディのシリアライズエラー
    #[error("Failed to serialize request body: {0}")]
    SerializeBody(serde_json::Error),

    /// 秘密鍵の復号エラー
    ///
    /// 復号した秘密鍵をPEM形式に変換できない場合も、このエラーを返す。
    #[error("Failed to decrypt private key: {0}")]
    DecryptPrivateKey(#[from] pkcs8::Error),

    /// 復号した秘密鍵から署名鍵を作成できないエラー
    #[error("Failed to create signing key: {0}")]
    CreateSigningKey(jsonwebtoken::errors::Error),

    /// JWTアサーションの署名エラー
    #[error("Failed to sign JWT assertion: {0}")]
    SignAssertion(#[from] jsonwebtoken::errors::Error),

    /// システム時刻の取得エラー
    #[error("Failed to get system time: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),

    /// HTTP通信エラー
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// BOXのエラーレスポンス
    ///
    /// エラー型のサイズを小さく保つため、詳細をヒープに確保する。
    #[error("BOX API request failed: {0}")]
    Api(Box<ApiError>),

    /// `Retry-After`ヘッダーが示す待機時間が上限を超えたエラー
    #[error("Retry-After {retry_after:?} exceeds max delay {max_delay:?}")]
    RetryAfterTooLong {
        /// `Retry-After`ヘッダーが示す待機時間
        retry_after: Duration,

        /// 待機時間の上限
        max_delay: Duration,
    },
}

/// BOXクライアントの結果型
pub type BoxClientResult<T> = Result<T, BoxClientError>;

/// BOXが返したエラーレスポンス
///
/// REST APIは`code`、`message`および`request_id`を返し、トークンエンドポイントは`error`および
/// `error_description`を返す。どちらの形式でも`code`と`message`で参照できるように正規化する。
#[derive(Debug)]
#[non_exhaustive]
pub struct ApiError {
    /// HTTPステータス
    pub status: StatusCode,

    /// エラーコード
    pub code: Option<String>,

    /// エラーメッセージ
    pub message: Option<String>,

    /// BOXのサポートに問い合わせるときに使用するリクエストID
    pub request_id: Option<String>,

    /// レスポンスボディ
    pub body: String,
}

/// BOXのエラーレスポンスのボディ
#[derive(Deserialize)]
struct ErrorBody {
    code: Option<String>,
    message: Option<String>,
    request_id: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

impl ApiError {
    /// HTTPステータスとレスポンスボディからエラーを作成する。
    ///
    /// ボディをBOXのエラー形式として解釈できない場合は、`code`、`message`および`request_id`を`None`とする。
    pub(crate) fn new(status: StatusCode, body: String) -> Self {
        let (code, message, request_id) = match serde_json::from_str::<ErrorBody>(&body) {
            Ok(parsed) => (
                parsed.code.or(parsed.error),
                parsed.message.or(parsed.error_description),
                parsed.request_id,
            ),
            Err(_) => (None, None, None),
        };

        Self {
            status,
            code,
            message,
            request_id,
            body,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "status={}", self.status)?;
        if let Some(code) = &self.code {
            write!(f, ", code={code}")?;
        }
        if let Some(message) = &self.message {
            write!(f, ", message={message}")?;
        }
        if let Some(request_id) = &self.request_id {
            write!(f, ", request_id={request_id}")?;
        }
        if self.code.is_none() && self.message.is_none() {
            write!(f, ", body={}", self.body)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test_api_error {
    use rstest::rstest;

    use super::*;

    /// REST APIとトークンエンドポイントのどちらのエラー形式も、codeとmessageに正規化することを確認
    #[rstest]
    #[case::rest_api(
        r#"{"type":"error","status":404,"code":"not_found","message":"Not Found","request_id":"abc"}"#,
        Some("not_found"),
        Some("Not Found"),
        Some("abc")
    )]
    #[case::token_endpoint(
        r#"{"error":"invalid_grant","error_description":"Invalid JWT"}"#,
        Some("invalid_grant"),
        Some("Invalid JWT"),
        None
    )]
    #[case::not_json("Bad Gateway", None, None, None)]
    #[case::empty("", None, None, None)]
    fn test_new(
        #[case] body: &str,
        #[case] code: Option<&str>,
        #[case] message: Option<&str>,
        #[case] request_id: Option<&str>,
    ) {
        let error = ApiError::new(StatusCode::NOT_FOUND, body.to_owned());

        assert_eq!(error.code.as_deref(), code);
        assert_eq!(error.message.as_deref(), message);
        assert_eq!(error.request_id.as_deref(), request_id);
        assert_eq!(error.body, body);
    }

    /// codeとmessageを解釈できない場合は、原因を調べられるようにボディを表示することを確認
    #[rstest]
    #[case::parsed(
        r#"{"code":"not_found","message":"Not Found","request_id":"abc"}"#,
        "status=404 Not Found, code=not_found, message=Not Found, request_id=abc"
    )]
    #[case::not_parsed("Bad Gateway", "status=404 Not Found, body=Bad Gateway")]
    fn test_display(#[case] body: &str, #[case] expected: &str) {
        let error = ApiError::new(StatusCode::NOT_FOUND, body.to_owned());

        assert_eq!(error.to_string(), expected);
    }
}
