use std::{fmt, path::PathBuf, time::Duration};

use reqwest::StatusCode;
use serde::Deserialize;

/// BOXクライアントエラー型
///
/// 原因となったエラーを[`std::error::Error::source`]で返し、メッセージには原因を含めない。
/// 原因をメッセージにも含めると、`anyhow`の`{:#}`などで原因の連鎖を表示したときに同じ文言が二重に出力されるためである。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BoxClientError {
    /// BOX設定ファイルのオープンエラー
    #[error("Failed to open config file: {path}")]
    OpenConfigFile {
        /// BOX設定ファイルのパス
        path: PathBuf,

        /// 原因となったエラー
        #[source]
        source: std::io::Error,
    },

    /// BOX設定ファイルの解析エラー
    #[error("Failed to parse config file: {path}")]
    ParseConfig {
        /// BOX設定ファイルのパス
        path: PathBuf,

        /// 原因となったエラー
        #[source]
        source: serde_json::Error,
    },

    /// 再試行設定の不正エラー
    #[error("Invalid retry policy: {0}")]
    InvalidRetryPolicy(&'static str),

    /// アクセストークンを有効期限のどれだけ前に取得し直すかの設定が上限を超えたエラー
    #[error("refresh_before_expiry {refresh_before_expiry:?} exceeds max {max:?}")]
    InvalidRefreshBeforeExpiry {
        /// 指定された値
        refresh_before_expiry: Duration,

        /// 上限
        max: Duration,
    },

    /// アップロードするファイルの読み込みエラー
    #[error("Failed to read file: {path}")]
    ReadFile {
        /// アップロードするファイルのパス
        path: PathBuf,

        /// 原因となったエラー
        #[source]
        source: std::io::Error,
    },

    /// ダウンロードしたファイルの書き込みエラー
    #[error("Failed to write file: {path}")]
    WriteFile {
        /// ダウンロードしたファイルを保存するパス
        path: PathBuf,

        /// 原因となったエラー
        #[source]
        source: std::io::Error,
    },

    /// 分割アップロードで並列にアップロードする部分の数が範囲外であるエラー
    #[error("upload_concurrency {upload_concurrency} is out of range {min}..={max}")]
    InvalidUploadConcurrency {
        /// 指定された値
        upload_concurrency: usize,

        /// 最小値
        min: usize,

        /// 最大値
        max: usize,
    },

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
    #[error("Invalid header value")]
    InvalidHeaderValue(#[source] reqwest::header::InvalidHeaderValue),

    /// リクエストボディの検証エラー
    ///
    /// BOXに送信する前に、BOXが拒否する値をクライアント側で検出した場合に返す。
    #[error("Invalid request body")]
    InvalidRequestBody(#[source] ValidationError),

    /// リクエストボディのシリアライズエラー
    #[error("Failed to serialize request body")]
    SerializeBody(#[source] serde_json::Error),

    /// 秘密鍵の復号エラー
    ///
    /// 復号した秘密鍵をPEM形式に変換できない場合も、このエラーを返す。
    #[error("Failed to decrypt private key")]
    DecryptPrivateKey(#[source] CryptoError),

    /// 復号した秘密鍵から署名鍵を作成できないエラー
    #[error("Failed to create signing key")]
    CreateSigningKey(#[source] CryptoError),

    /// JWTアサーションの署名エラー
    #[error("Failed to sign JWT assertion")]
    SignAssertion(#[source] CryptoError),

    /// システム時刻の取得エラー
    #[error("Failed to get system time")]
    SystemTime(#[from] std::time::SystemTimeError),

    /// HTTP通信エラー
    #[error("HTTP request failed")]
    Http(#[from] reqwest::Error),

    /// 成功レスポンスのボディのデシリアライズエラー
    #[error("Failed to deserialize response body")]
    DeserializeResponse(#[source] serde_json::Error),

    /// BOXのエラーレスポンス
    ///
    /// エラー型のサイズを小さく保つため、詳細をヒープに確保する。
    /// [`ApiError`]は原因となったエラーではなくこのエラーの内容であるため、メッセージに含める。
    #[error("BOX API request failed: {0}")]
    Api(Box<ApiError>),

    /// `Retry-After`ヘッダーが示す待機時間が上限を超えたエラー
    ///
    /// 呼び出し元がステータス、エラーコードおよびリクエストIDを調べられるように、
    /// `Retry-After`ヘッダーを返したリクエストのエラーを原因として保持する。
    #[error("Retry-After {retry_after:?} exceeds max delay {max_delay:?}")]
    RetryAfterTooLong {
        /// `Retry-After`ヘッダーが示す待機時間
        retry_after: Duration,

        /// 待機時間の上限
        max_delay: Duration,

        /// `Retry-After`ヘッダーを返したリクエストのエラー
        ///
        /// エラー型が自身を含むため、ヒープに確保する。
        #[source]
        source: Box<BoxClientError>,
    },
}

/// BOXクライアントの結果型
pub type BoxClientResult<T> = Result<T, BoxClientError>;

/// 秘密鍵の復号、署名鍵の作成またはJWTアサーションの署名で発生したエラー
///
/// 原因となった外部クレート（`pkcs8`および`jsonwebtoken`）のエラーの型を公開すると、
/// それらのクレートのメジャーバージョンを上げたときに、このクレートの破壊的変更となる。
/// 利用者は原因の型を調べる必要がないため、型を隠して保持し、メッセージと原因の連鎖だけを公開する。
/// メッセージと[`std::error::Error::source`]は、原因となったエラーのものをそのまま返す。
#[derive(Debug)]
pub struct CryptoError(Box<dyn std::error::Error + Send + Sync + 'static>);

impl CryptoError {
    /// 外部クレートのエラーを包む。
    pub(crate) fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for CryptoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

/// リクエストボディの検証で発生したエラー
///
/// 原因となった`garde`のエラーの型を公開すると、`garde`のメジャーバージョンを上げたときに、
/// このクレートの破壊的変更となる。利用者は原因の型を調べる必要がないため、型を隠して保持し、
/// メッセージだけを公開する。メッセージは`name: ...`のように、検証に失敗したフィールドを含む。
#[derive(Debug)]
pub struct ValidationError(garde::Report);

impl ValidationError {
    /// `garde`の検証結果を包む。
    pub(crate) fn new(report: garde::Report) -> Self {
        Self(report)
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ValidationError {}

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

#[cfg(test)]
mod test_box_client_error {
    use std::error::Error as _;

    use rstest::rstest;

    use super::*;

    /// 不正なヘッダーの値のエラーを返す。
    fn invalid_header_value() -> anyhow::Result<BoxClientError> {
        match reqwest::header::HeaderValue::from_str("1\n") {
            Ok(_) => anyhow::bail!("header value must be invalid"),
            Err(e) => Ok(BoxClientError::InvalidHeaderValue(e)),
        }
    }

    /// JSONのシリアライズエラーを返す。
    ///
    /// 文字列以外をキーとするマップはJSONにシリアライズできない。
    fn serialize_body() -> anyhow::Result<BoxClientError> {
        let map = std::collections::HashMap::from([((1, 2), 3)]);
        match serde_json::to_vec(&map) {
            Ok(_) => anyhow::bail!("serialization must fail"),
            Err(e) => Ok(BoxClientError::SerializeBody(e)),
        }
    }

    /// 署名鍵の作成エラーを返す。
    fn create_signing_key() -> anyhow::Result<BoxClientError> {
        match jsonwebtoken::EncodingKey::from_rsa_pem(b"invalid") {
            Ok(_) => anyhow::bail!("signing key must be invalid"),
            Err(e) => Ok(BoxClientError::CreateSigningKey(CryptoError::new(e))),
        }
    }

    /// BOX設定ファイルのオープンエラーを返す。
    #[expect(
        clippy::unnecessary_wraps,
        reason = "rstestのケースに渡す関数ポインターの型を、失敗しうる他の関数と揃えるため"
    )]
    fn open_config_file() -> anyhow::Result<BoxClientError> {
        Ok(BoxClientError::OpenConfigFile {
            path: PathBuf::from("config.json"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        })
    }

    /// BOX設定ファイルの解析エラーを返す。
    fn parse_config() -> anyhow::Result<BoxClientError> {
        match serde_json::from_str::<serde_json::Value>("not json") {
            Ok(_) => anyhow::bail!("parsing must fail"),
            Err(source) => Ok(BoxClientError::ParseConfig {
                path: PathBuf::from("config.json"),
                source,
            }),
        }
    }

    /// 原因となったエラーを`source()`で返し、メッセージに原因の文言を含めないことを確認
    #[rstest]
    #[case::open_config_file(open_config_file)]
    #[case::parse_config(parse_config)]
    #[case::invalid_header_value(invalid_header_value)]
    #[case::serialize_body(serialize_body)]
    #[case::create_signing_key(create_signing_key)]
    fn test_source(#[case] error: fn() -> anyhow::Result<BoxClientError>) -> anyhow::Result<()> {
        let error = error()?;

        let Some(source) = error.source() else {
            anyhow::bail!("source must be set: {error:?}");
        };
        assert!(
            !error.to_string().contains(&source.to_string()),
            "message: {error}, source: {source}"
        );

        Ok(())
    }

    /// BOX設定ファイルのエラーのメッセージに、ファイルのパスを含めることを確認
    #[rstest]
    #[case::open_config_file(open_config_file, "Failed to open config file: config.json")]
    #[case::parse_config(parse_config, "Failed to parse config file: config.json")]
    fn test_config_file_message(
        #[case] error: fn() -> anyhow::Result<BoxClientError>,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        assert_eq!(error()?.to_string(), expected);

        Ok(())
    }

    /// BOXのエラーレスポンスは原因ではなくエラーの内容であるため、メッセージに含めて`source()`では返さないことを確認
    #[test]
    fn test_api_error_is_not_source() {
        let error = BoxClientError::Api(Box::new(ApiError::new(
            StatusCode::NOT_FOUND,
            r#"{"code":"not_found","message":"Not Found"}"#.to_owned(),
        )));

        assert!(error.source().is_none());
        assert_eq!(
            error.to_string(),
            "BOX API request failed: status=404 Not Found, code=not_found, message=Not Found"
        );
    }
}

#[cfg(test)]
mod test_crypto_error {
    use std::error::Error as _;

    use super::*;

    /// 原因となったエラーのメッセージと`source()`をそのまま返すことを確認
    #[test]
    fn test_delegates_message_and_source() {
        let inner = BoxClientError::OpenConfigFile {
            path: PathBuf::from("config.json"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        let expected_message = inner.to_string();
        let expected_source = inner.source().map(ToString::to_string);

        let error = CryptoError::new(inner);

        assert_eq!(error.to_string(), expected_message);
        assert_eq!(error.source().map(ToString::to_string), expected_source);
    }
}
