//! ファイルのアップロードに関するAPI
//!
//! BOXは、ファイルのアップロードを、REST APIとは別のホスト（`upload.box.com`）で受け付ける。
//! 小さいファイルは1回のリクエストでアップロードし、大きいファイルは分割アップロードのAPIで部分ごとにアップロードする。

use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::Bytes;
use futures_util::stream::{FuturesUnordered, StreamExt as _};
use garde::Validate;
use reqwest::{
    Response, StatusCode,
    header::{CONTENT_RANGE, HeaderName, HeaderValue, IF_MATCH},
};
use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};
use tokio::io::AsyncReadExt as _;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, File, FileId, FolderId, ParentFolder,
    ValidationError,
    api::types::validate_item_name,
    request::ApiRequest,
    retry::{AttemptError, Idempotency, parse_retry_after, read_json},
};

/// 分割アップロードに切り替えるファイルサイズ（バイト）
///
/// この値を超えるファイルは、分割アップロードのAPIでアップロードする。
/// BOXは、50MBを超えるファイルに分割アップロードを推奨している。
/// また、分割アップロードのAPIは20MB未満のファイルを拒否するため、この値は20MB以上である必要がある。
pub const CHUNKED_UPLOAD_THRESHOLD: u64 = 50 * 1024 * 1024;

/// 分割アップロードのコミットで、BOXが処理中を示す202を返した場合に、コミットを再送する回数の上限
const MAX_COMMIT_POLLS: u32 = 30;

/// 内容のハッシュを送信するヘッダー
///
/// BOXは、ヘッダー名に反して、MD5ではなくSHA-1のハッシュを16進数で受け取る。
const CONTENT_MD5: HeaderName = HeaderName::from_static("content-md5");

/// 内容のダイジェストを`sha=<Base64で符号化したSHA-1>`の形式で送信するヘッダー
const DIGEST: HeaderName = HeaderName::from_static("digest");

/// ファイルのアップロードのオプション
///
/// # Examples
///
/// ```
/// use box_client::UploadFileOptions;
///
/// let options = UploadFileOptions::default().with_name("report.pdf");
/// assert_eq!(options.name, Some("report.pdf"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UploadFileOptions<'a> {
    /// BOXに保存するファイル名
    ///
    /// `None`の場合は、アップロードするファイルのパスのファイル名を使用する。
    pub name: Option<&'a str>,
}

impl<'a> UploadFileOptions<'a> {
    /// BOXに保存するファイル名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }
}

/// ファイルの内容の更新のオプション
///
/// # Examples
///
/// ```
/// use box_client::UpdateFileContentOptions;
///
/// let options = UpdateFileContentOptions::default()
///     .with_name("report-v2.pdf")
///     .with_etag("1");
/// assert_eq!(options.name, Some("report-v2.pdf"));
/// assert_eq!(options.etag, Some("1"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpdateFileContentOptions<'a> {
    /// 新しいファイル名
    ///
    /// 指定した場合、BOXは内容の更新とともにファイル名を変更する。`None`の場合はファイル名を変更しない。
    pub name: Option<&'a str>,

    /// 最後に取得したファイルの`ETag`
    ///
    /// 指定した場合は、`If-Match`ヘッダーで送信する。ファイルが`ETag`の取得後に変更されていると、
    /// BOXはファイルの内容を更新せずに412を返す。
    pub etag: Option<&'a str>,
}

impl<'a> UpdateFileContentOptions<'a> {
    /// 新しいファイル名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// 最後に取得したファイルの`ETag`を設定する。
    #[must_use]
    pub fn with_etag(mut self, etag: &'a str) -> Self {
        self.etag = Some(etag);
        self
    }
}

/// アップロードの対象
///
/// 新しいファイルのアップロードと、既存のファイルの内容の更新では、呼び出すAPIと送信する属性だけが異なる。
#[derive(Debug, Clone, Copy)]
enum UploadTarget<'a> {
    /// 新しいファイル
    NewFile {
        /// アップロード先の親フォルダのID
        parent_id: &'a FolderId,

        /// ファイル名
        name: &'a str,
    },

    /// 既存のファイルの新しいバージョン
    NewVersion {
        /// 内容を更新するファイルのID
        file_id: &'a FileId,

        /// 新しいファイル名。`None`の場合はファイル名を変更しない。
        name: Option<&'a str>,

        /// 最後に取得したファイルの`ETag`
        etag: Option<&'a str>,
    },
}

impl UploadTarget<'_> {
    /// 1回のリクエストでアップロードするAPIのパス要素を返す。
    fn direct_upload_path(&self) -> Vec<&str> {
        match self {
            Self::NewFile { .. } => vec!["files", "content"],
            Self::NewVersion { file_id, .. } => vec!["files", file_id.as_ref(), "content"],
        }
    }

    /// 1回のリクエストでアップロードする場合の、`attributes`パートのJSONを返す。
    ///
    /// # Errors
    ///
    /// JSONにシリアライズできない場合は、[`BoxClientError::SerializeBody`]を返す。
    fn direct_upload_attributes(&self) -> BoxClientResult<Vec<u8>> {
        match *self {
            Self::NewFile { parent_id, name } => serde_json::to_vec(&UploadAttributes {
                name,
                parent: ParentFolder {
                    id: parent_id.clone(),
                },
            }),
            Self::NewVersion { name, .. } => serde_json::to_vec(&UploadVersionAttributes { name }),
        }
        .map_err(BoxClientError::SerializeBody)
    }

    /// アップロードセッションの作成リクエストを返す。
    ///
    /// # Errors
    ///
    /// ボディをJSONにシリアライズできない場合は、[`BoxClientError::SerializeBody`]を返す。
    fn create_upload_session_request(&self, file_size: u64) -> BoxClientResult<ApiRequest<'_>> {
        match *self {
            Self::NewFile { parent_id, name } => ApiRequest::post(["files", "upload_sessions"])
                .upload_host()
                .json(&CreateUploadSessionBody {
                    folder_id: parent_id,
                    file_size,
                    file_name: name,
                }),
            Self::NewVersion { file_id, name, .. } => {
                ApiRequest::post(["files", file_id.as_ref(), "upload_sessions"])
                    .upload_host()
                    .json(&CreateVersionUploadSessionBody {
                        file_size,
                        file_name: name,
                    })
            }
        }
    }

    /// `If-Match`ヘッダーで送信する`ETag`を返す。
    fn etag(&self) -> Option<&str> {
        match *self {
            Self::NewFile { .. } => None,
            Self::NewVersion { etag, .. } => etag,
        }
    }
}

/// BOXに保存するファイル名
#[derive(Debug, Validate)]
struct FileName<'a> {
    /// ファイル名
    ///
    /// BOXは、名前の長さを文字数で数える。
    #[garde(length(chars, min = 1, max = 255), custom(validate_item_name))]
    name: &'a str,
}

/// ファイル名を検証する。
///
/// # Errors
///
/// ファイル名をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
fn validate_file_name(name: &str) -> BoxClientResult<()> {
    FileName { name }
        .validate()
        .map_err(|report| BoxClientError::InvalidRequestBody(ValidationError::new(report)))
}

/// 1回のリクエストでアップロードする場合の、ファイルの属性
#[derive(Debug, Serialize)]
struct UploadAttributes<'a> {
    /// ファイル名
    name: &'a str,

    /// アップロード先の親フォルダ
    parent: ParentFolder,
}

/// 1回のリクエストで新しいバージョンをアップロードする場合の、ファイルの属性
#[derive(Debug, Serialize)]
struct UploadVersionAttributes<'a> {
    /// 新しいファイル名
    ///
    /// `None`の場合は送信せず、BOXはファイル名を変更しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// アップロードしたファイルのリスト
#[derive(Debug, Deserialize)]
struct FileCollection {
    /// アップロードしたファイル
    entries: Vec<File>,
}

impl FileCollection {
    /// 最初のファイルを返す。
    ///
    /// # Errors
    ///
    /// ファイルが含まれていない場合は、[`BoxClientError::UnexpectedResponse`]を返す。
    fn into_first(self) -> BoxClientResult<File> {
        self.entries
            .into_iter()
            .next()
            .ok_or(BoxClientError::UnexpectedResponse(
                "upload response contains no file",
            ))
    }
}

/// アップロードセッションの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateUploadSessionBody<'a> {
    /// アップロード先の親フォルダのID
    folder_id: &'a FolderId,

    /// ファイルのバイト数
    file_size: u64,

    /// ファイル名
    file_name: &'a str,
}

/// 既存のファイルの新しいバージョンをアップロードする、アップロードセッションの作成リクエストのボディ
#[derive(Debug, Serialize)]
struct CreateVersionUploadSessionBody<'a> {
    /// ファイルのバイト数
    file_size: u64,

    /// 新しいファイル名
    ///
    /// `None`の場合は送信せず、BOXはファイル名を変更しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<&'a str>,
}

/// アップロードセッション
#[derive(Debug, Deserialize)]
struct UploadSession {
    /// アップロードセッションのID
    id: String,

    /// 最後の部分を除く、すべての部分のバイト数
    part_size: u64,

    /// 部分の数
    total_parts: u64,
}

/// アップロードした部分
///
/// コミットでは、この内容をそのままBOXに送り返す。
/// BOXは部分のハッシュ（`sha1`または`sha512`）を省略できるため、送り返さない。
/// ファイル全体の内容は、コミットの`Digest`ヘッダーで検証される。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct UploadedPart {
    /// 部分のID
    part_id: String,

    /// ファイルの先頭からの部分の位置（バイト）
    offset: u64,

    /// 部分のバイト数
    size: u64,
}

/// 部分のアップロードのレスポンス
///
/// BOXのAPIリファレンスは部分を`part`属性で包んで返すと記載し、ガイドは部分を直接返すと記載している。
/// どちらの形式でも受け付ける。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UploadPartResponse {
    /// `part`属性で包んだ部分
    Wrapped {
        /// アップロードした部分
        part: UploadedPart,
    },

    /// 部分
    Bare(UploadedPart),
}

impl UploadPartResponse {
    /// アップロードした部分を返す。
    fn into_part(self) -> UploadedPart {
        match self {
            Self::Wrapped { part } | Self::Bare(part) => part,
        }
    }
}

/// アップロードセッションのコミットのリクエストのボディ
#[derive(Debug, Serialize)]
struct CommitUploadSessionBody<'a> {
    /// アップロードしたすべての部分
    parts: &'a [UploadedPart],
}

/// アップロードセッションのコミットの成功レスポンスのボディ
///
/// BOXのAPIリファレンスはファイルのリストを返すと記載し、ガイドはファイルを直接返すと記載している。
/// どちらの形式でも受け付ける。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CommitResponse {
    /// ファイルのリスト
    Files(FileCollection),

    /// ファイル
    File(Box<File>),
}

impl CommitResponse {
    /// コミットしたファイルを返す。
    ///
    /// # Errors
    ///
    /// ファイルのリストにファイルが含まれていない場合は、[`BoxClientError::UnexpectedResponse`]を返す。
    fn into_file(self) -> BoxClientResult<File> {
        match self {
            Self::Files(files) => files.into_first(),
            Self::File(file) => Ok(*file),
        }
    }
}

/// アップロードセッションのコミットの結果
#[derive(Debug)]
enum CommitOutcome {
    /// BOXがファイルを作成した。
    Committed(CommitResponse),

    /// BOXが部分を処理しているため、コミットを再送する必要がある。
    Processing {
        /// `Retry-After`ヘッダーが示す待機時間
        retry_after: Option<std::time::Duration>,
    },
}

/// アップロードセッションのコミットの成功レスポンスを読み込む。
///
/// BOXは、すべての部分を受け取ったが処理を終えていない場合に、202と`Retry-After`ヘッダーを返す。
async fn read_commit_response(
    response: Response,
    idempotency: Idempotency,
) -> Result<CommitOutcome, AttemptError> {
    if response.status() == StatusCode::ACCEPTED {
        return Ok(CommitOutcome::Processing {
            retry_after: parse_retry_after(response.headers()),
        });
    }
    read_json(response, idempotency)
        .await
        .map(CommitOutcome::Committed)
}

/// ファイルの読み込みエラーを、パスを含むエラーに変換する関数を返す。
fn read_file_error(path: &Path) -> impl FnOnce(std::io::Error) -> BoxClientError + '_ {
    move |source| BoxClientError::ReadFile {
        path: path.to_owned(),
        source,
    }
}

/// `Digest`ヘッダーの値を返す。
fn sha1_digest_header(digest: &[u8]) -> String {
    format!("sha={}", BASE64.encode(digest))
}

impl BoxClient {
    /// ファイルをアップロードする。
    ///
    /// ファイルサイズが[`CHUNKED_UPLOAD_THRESHOLD`]以下の場合は、ファイル全体をメモリに読み込み、
    /// 1回のリクエストでアップロードする（`POST /files/content`）。
    /// [`CHUNKED_UPLOAD_THRESHOLD`]を超える場合は、分割アップロードのAPIを使用する。
    /// 分割アップロードでは、アップロードセッションを作成し、BOXが指定したサイズの部分をアップロードしてから、
    /// アップロードセッションをコミットする。部分は、[`BoxClient::upload_concurrency`]個まで並列にアップロードする。
    /// メモリには、並列にアップロードしている部分だけを読み込む。
    ///
    /// 1回のリクエストでのアップロードとコミットは、HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 部分のアップロードは、HTTPメソッドがPUTであり、同じ範囲を同じ内容で送り直すため、冪等なリクエストとして送信する。
    ///
    /// 部分のアップロードまたはコミットに失敗した場合は、アップロードセッションを中止してからエラーを返す。
    ///
    /// 1回のリクエストで送信するデータが大きいため、
    /// [`BoxClientTimeout::request_timeout`](crate::BoxClientTimeout::request_timeout)は、
    /// 通信速度に応じて、[`CHUNKED_UPLOAD_THRESHOLD`]または部分のサイズを送信できる長さにする。
    ///
    /// # Arguments
    ///
    /// - `path`: アップロードするファイルのパス
    /// - `parent_id`: アップロード先の親フォルダのID
    /// - `options`: BOXに保存するファイル名のオプション
    ///
    /// # Errors
    ///
    /// `options.name`を指定せず、パスからUTF-8のファイル名を取得できない場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ファイル名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// ファイルを読み込めない場合は、[`BoxClientError::ReadFile`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: ファイル名が不正である。
    /// - 403: ストレージの上限を超える。
    /// - 404: 親フォルダが存在しない、またはアクセスする権限がない。
    /// - 409: 同じ名前の項目が親フォルダに存在する、またはストレージの空き容量がない。
    ///
    /// 分割アップロードのコミットで、BOXが部分の処理を終えないまま、コミットの再送の上限に達した場合は、
    /// [`BoxClientError::UnexpectedResponse`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルを作成した可能性がある。
    /// したがって、[`BoxClient::list_items_in_folder`]で親フォルダを確認する。
    pub async fn upload_file(
        &self,
        path: impl AsRef<Path>,
        parent_id: &FolderId,
        options: &UploadFileOptions<'_>,
    ) -> BoxClientResult<File> {
        self.upload_file_with_threshold(path.as_ref(), parent_id, options, CHUNKED_UPLOAD_THRESHOLD)
            .await
    }

    /// 分割アップロードに切り替えるファイルサイズを指定して、ファイルをアップロードする。
    ///
    /// テストで大きなファイルを作成せずに分割アップロードを確認できるように、閾値を引数で受け取る。
    async fn upload_file_with_threshold(
        &self,
        path: &Path,
        parent_id: &FolderId,
        options: &UploadFileOptions<'_>,
        threshold: u64,
    ) -> BoxClientResult<File> {
        let name = match options.name {
            Some(name) => name,
            None => path.file_name().and_then(|name| name.to_str()).ok_or(
                BoxClientError::InvalidArgument("the file name cannot be taken from the path"),
            )?,
        };
        validate_file_name(name)?;

        self.upload(path, UploadTarget::NewFile { parent_id, name }, threshold)
            .await
    }

    /// ファイルの内容を更新する。
    ///
    /// BOXは、ファイルの新しいバージョンとして内容を保存し、以前の内容をバージョン履歴に残す。
    ///
    /// ファイルサイズが[`CHUNKED_UPLOAD_THRESHOLD`]以下の場合は、ファイル全体をメモリに読み込み、
    /// 1回のリクエストでアップロードする（`POST /files/{file_id}/content`）。
    /// [`CHUNKED_UPLOAD_THRESHOLD`]を超える場合は、分割アップロードのAPIを使用する
    /// （`POST /files/{file_id}/upload_sessions`で作成したアップロードセッションに部分をアップロードしてコミットする）。
    /// 分割アップロードの進め方、冪等性および失敗した場合の扱いは、[`BoxClient::upload_file`]と同じである。
    ///
    /// `options.etag`を指定した場合は、1回のリクエストでのアップロード、または分割アップロードのコミットで、
    /// `If-Match`ヘッダーとして送信する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 内容を更新するファイルのID
    /// - `path`: 新しい内容のファイルのパス
    /// - `options`: 新しいファイル名および`ETag`のオプション
    ///
    /// # Errors
    ///
    /// `options.name`をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、[`BoxClientError::InvalidHeaderValue`]を返す。
    /// ファイルを読み込めない場合は、[`BoxClientError::ReadFile`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: ファイルが存在しない、またはアクセスする権限がない。
    /// - 409: `options.name`と同じ名前の項目が親フォルダに存在する、またはストレージの空き容量がない。
    /// - 412: `options.etag`がファイルの現在の`ETag`と一致しない。
    ///
    /// 分割アップロードのコミットで、BOXが部分の処理を終えないまま、コミットの再送の上限に達した場合は、
    /// [`BoxClientError::UnexpectedResponse`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルの内容を更新した可能性がある。
    /// したがって、[`BoxClient::file_information`]でファイルのバージョンを確認する。
    pub async fn update_file_content(
        &self,
        file_id: &FileId,
        path: impl AsRef<Path>,
        options: &UpdateFileContentOptions<'_>,
    ) -> BoxClientResult<File> {
        self.update_file_content_with_threshold(
            file_id,
            path.as_ref(),
            options,
            CHUNKED_UPLOAD_THRESHOLD,
        )
        .await
    }

    /// 分割アップロードに切り替えるファイルサイズを指定して、ファイルの内容を更新する。
    ///
    /// テストで大きなファイルを作成せずに分割アップロードを確認できるように、閾値を引数で受け取る。
    async fn update_file_content_with_threshold(
        &self,
        file_id: &FileId,
        path: &Path,
        options: &UpdateFileContentOptions<'_>,
        threshold: u64,
    ) -> BoxClientResult<File> {
        if let Some(name) = options.name {
            validate_file_name(name)?;
        }

        let target = UploadTarget::NewVersion {
            file_id,
            name: options.name,
            etag: options.etag,
        };
        self.upload(path, target, threshold).await
    }

    /// ファイルサイズに応じて、1回のリクエストまたは分割アップロードのAPIでアップロードする。
    async fn upload(
        &self,
        path: &Path,
        target: UploadTarget<'_>,
        threshold: u64,
    ) -> BoxClientResult<File> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(read_file_error(path))?;
        let size = file.metadata().await.map_err(read_file_error(path))?.len();

        let uploaded = if size <= threshold {
            self.upload_file_directly(path, &mut file, &target).await?
        } else {
            self.upload_file_in_chunks(path, &mut file, size, &target)
                .await?
        };

        tracing::debug!(
            path = %path.display(),
            size,
            ?target,
            file_id = uploaded.id.as_ref(),
            chunked = threshold < size,
            "uploaded file"
        );

        Ok(uploaded)
    }

    /// ファイルを1回のリクエストでアップロードする。
    ///
    /// BOXは、`attributes`パートが`file`パートより前にない場合に400（`metadata_after_file_contents`）を返す。
    /// したがって、`attributes`パート、`file`パートの順に送信する。
    /// 再試行のたびにボディを作り直さないように、multipartのボディをバイト列として組み立てる。
    async fn upload_file_directly(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        target: &UploadTarget<'_>,
    ) -> BoxClientResult<File> {
        let attributes = target.direct_upload_attributes()?;
        let boundary = format!("box-client-{}", uuid::Uuid::new_v4().simple());

        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\n\
                 Content-Disposition: form-data; name=\"attributes\"\r\n\
                 Content-Type: application/json\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&attributes);
        // BOXはファイル名を`attributes`から取得するため、`filename`には固定の値を指定し、ファイル名のエスケープを避ける。
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\n\
                 Content-Disposition: form-data; name=\"file\"; filename=\"file\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        // ファイルの内容をボディに直接読み込み、ファイルの内容とボディを別々にメモリに保持しない。
        let content_start = body.len();
        file.read_to_end(&mut body)
            .await
            .map_err(read_file_error(path))?;
        let content_sha1 = format!("{:x}", Sha1::digest(&body[content_start..]));
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let content_type =
            HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
                .map_err(BoxClientError::InvalidHeaderValue)?;
        let mut request = ApiRequest::post(target.direct_upload_path())
            .upload_host()
            .header(CONTENT_MD5, &content_sha1)?
            .body(content_type, Bytes::from(body));
        if let Some(etag) = target.etag() {
            request = request.header(IF_MATCH, etag)?;
        }

        let files: FileCollection = self.send_json(request).await?;
        files.into_first()
    }

    /// ファイルを分割アップロードのAPIでアップロードする。
    ///
    /// 部分のアップロードまたはコミットに失敗した場合は、アップロードセッションを中止してからエラーを返す。
    async fn upload_file_in_chunks(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        size: u64,
        target: &UploadTarget<'_>,
    ) -> BoxClientResult<File> {
        let request = target.create_upload_session_request(size)?;
        let session: UploadSession = self.send_json(request).await?;
        tracing::debug!(
            upload_session_id = session.id,
            part_size = session.part_size,
            total_parts = session.total_parts,
            "created upload session"
        );

        let result = self
            .upload_parts_and_commit(path, file, size, &session, target.etag())
            .await;
        if result.is_err() {
            self.abort_upload_session(&session.id).await;
        }
        result
    }

    /// ファイルを部分ごとにアップロードし、アップロードセッションをコミットする。
    ///
    /// ファイルを先頭から順に読み込みながら、[`BoxClient::upload_concurrency`]個までの部分を並列にアップロードする。
    /// いずれかの部分のアップロードに失敗した場合は、アップロード中の部分を取り消してエラーを返す。
    async fn upload_parts_and_commit(
        &self,
        path: &Path,
        file: &mut tokio::fs::File,
        size: u64,
        session: &UploadSession,
        etag: Option<&str>,
    ) -> BoxClientResult<File> {
        // 部分のサイズが0の場合は、ファイルを読み進められず無限ループになるため、BOXの応答を不正として扱う。
        if session.part_size == 0 {
            return Err(BoxClientError::UnexpectedResponse(
                "upload session part_size is 0",
            ));
        }

        let concurrency = self.upload_concurrency();
        // ファイルを先頭から順に読み込むため、部分を読み込むたびにファイル全体のハッシュも計算する。
        let mut file_hasher = Sha1::new();
        let mut parts = Vec::new();
        let mut uploading = FuturesUnordered::new();
        let mut offset = 0;
        loop {
            // 空きがある限り、次の部分を読み込んでアップロードを始める。
            // 読み込みを待つ間はアップロード中の部分をポーリングしないが、ローカルのファイルの読み込みは
            // 部分の送信より十分に短いため、送信の遅れは無視できる。
            while uploading.len() < concurrency && offset < size {
                let part_size = session.part_size.min(size - offset);
                let mut part = vec![
                    0;
                    usize::try_from(part_size).map_err(|_| {
                        BoxClientError::UnexpectedResponse("upload session part_size is too large")
                    })?
                ];
                file.read_exact(&mut part)
                    .await
                    .map_err(read_file_error(path))?;
                file_hasher.update(&part);
                uploading.push(self.upload_part(session, offset, size, Bytes::from(part)));
                offset += part_size;
            }

            // アップロード中の部分がなければ、すべての部分のアップロードを終えている。
            let Some(uploaded) = uploading.next().await else {
                break;
            };
            let part = uploaded?;
            tracing::debug!(
                upload_session_id = session.id,
                offset = part.offset,
                size = part.size,
                uploaded_parts = parts.len() + 1,
                total_parts = session.total_parts,
                "uploaded part"
            );
            parts.push(part);
        }

        // 部分は完了した順に並ぶため、コミットの前にファイルの先頭からの位置の順に並べ替える。
        parts.sort_by_key(|part| part.offset);
        let digest = sha1_digest_header(&file_hasher.finalize());
        self.commit_upload_session(session, &digest, &parts, etag)
            .await
    }

    /// 部分をアップロードする。
    ///
    /// # Arguments
    ///
    /// - `session`: アップロードセッション
    /// - `offset`: ファイルの先頭からの部分の位置（バイト）
    /// - `file_size`: ファイルのバイト数
    /// - `part`: 部分の内容
    async fn upload_part(
        &self,
        session: &UploadSession,
        offset: u64,
        file_size: u64,
        part: Bytes,
    ) -> BoxClientResult<UploadedPart> {
        let digest = sha1_digest_header(&Sha1::digest(&part));
        let last = offset + part.len() as u64 - 1;
        let content_range = format!("bytes {offset}-{last}/{file_size}");
        let request = ApiRequest::put(["files", "upload_sessions", &session.id])
            .upload_host()
            .header(DIGEST, &digest)?
            .header(CONTENT_RANGE, &content_range)?
            .body(HeaderValue::from_static("application/octet-stream"), part);
        let uploaded: UploadPartResponse = self.send_json(request).await?;
        Ok(uploaded.into_part())
    }

    /// アップロードセッションをコミットする。
    ///
    /// BOXが部分の処理を終えていないことを示す202を返した場合は、`Retry-After`ヘッダーが示す時間だけ待機してから、
    /// コミットを再送する。`Retry-After`ヘッダーがない場合、または待機時間の上限を超える場合は、
    /// 再試行設定に従って待機時間を決める。
    ///
    /// `etag`を指定した場合は、`If-Match`ヘッダーで送信する。
    async fn commit_upload_session(
        &self,
        session: &UploadSession,
        digest: &str,
        parts: &[UploadedPart],
        etag: Option<&str>,
    ) -> BoxClientResult<File> {
        let body = CommitUploadSessionBody { parts };
        let mut polls = 0;
        loop {
            let mut request = ApiRequest::post(["files", "upload_sessions", &session.id, "commit"])
                .upload_host()
                .header(DIGEST, digest)?
                .json(&body)?;
            if let Some(etag) = etag {
                request = request.header(IF_MATCH, etag)?;
            }
            let retry_after = match self.send(request, read_commit_response).await? {
                CommitOutcome::Committed(response) => return response.into_file(),
                CommitOutcome::Processing { retry_after } => retry_after,
            };

            if MAX_COMMIT_POLLS <= polls {
                return Err(BoxClientError::UnexpectedResponse(
                    "upload session was not committed before reaching max commit polls",
                ));
            }
            polls += 1;

            // `ThreadRng`は`Send`を実装していないため、待機時間の計算と待機を別の文に分け、`ThreadRng`を待機の前に破棄する。
            let policy = self.retry_policy();
            let delay = policy
                .retry_delay(polls, retry_after, &mut rand::rng())
                .unwrap_or_else(|_| policy.max_delay());
            tracing::debug!(
                upload_session_id = session.id,
                polls,
                delay_ms = delay.as_millis(),
                "upload session is being processed; resending commit"
            );
            tokio::time::sleep(delay).await;
        }
    }

    /// アップロードセッションを中止する。
    ///
    /// アップロードの失敗を呼び出し元に返すことを優先するため、中止に失敗した場合はWARNで出力するだけにする。
    /// BOXは、中止しなかったアップロードセッションを7日後に破棄する。
    async fn abort_upload_session(&self, session_id: &str) {
        let request = ApiRequest::delete(["files", "upload_sessions", session_id]).upload_host();
        match self.send_empty(request).await {
            Ok(()) => tracing::debug!(upload_session_id = session_id, "aborted upload session"),
            Err(error) => tracing::warn!(
                upload_session_id = session_id,
                error = &error as &(dyn std::error::Error + 'static),
                "failed to abort upload session"
            ),
        }
    }
}

#[cfg(test)]
mod test_upload_file {
    use std::{path::PathBuf, time::Duration};

    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    use super::*;
    use crate::test_support::{
        UPLOAD_BASE_PATH, create_client, create_client_with_upload_concurrency, mount_token,
        request_count, short_retry,
    };

    /// 分割アップロードに切り替えるファイルサイズ
    ///
    /// 大きなファイルを作成せずに分割アップロードを確認できるように、小さい値にする。
    const THRESHOLD: u64 = 20;

    /// 分割アップロードでアップロードするファイルの内容
    ///
    /// 部分のサイズを10バイトとし、最後の部分が部分のサイズより小さくなる長さにする。
    const CHUNKED_CONTENT: &[u8] = b"abcdefghijklmnopqrstuvwxy";

    /// 分割アップロードの部分のサイズ
    const PART_SIZE: usize = 10;

    /// アップロードセッションのID
    const SESSION_ID: &str = "S1";

    /// アップロードのAPIのパスを返す。
    fn upload_path(suffix: &str) -> String {
        format!("{UPLOAD_BASE_PATH}{suffix}")
    }

    /// 一時ディレクトリに、指定した名前と内容のファイルを作成する。
    fn create_file(name: &str, content: &[u8]) -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(name);
        std::fs::write(&path, content)?;
        Ok((dir, path))
    }

    /// ファイル情報のJSONを返す。
    fn file_json(name: &str) -> serde_json::Value {
        json!({"type": "file", "id": "11", "name": name})
    }

    /// 指定したメソッドとパスのリクエストを返す。
    async fn requests(server: &MockServer, http_method: &str, request_path: &str) -> Vec<Request> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == http_method && r.url.path() == request_path)
            .collect()
    }

    /// 1回のリクエストで、`attributes`パート、`file`パートの順に送信し、内容のSHA-1をヘッダーで送信することを確認
    ///
    /// ファイル名を指定しない場合は、パスのファイル名を使用することも確認する。
    #[rstest]
    #[case::name_from_path(None, "report.txt")]
    #[case::name_from_options(Some("renamed.txt"), "renamed.txt")]
    #[tokio::test]
    async fn test_upload_file_directly(
        #[case] option_name: Option<&'static str>,
        #[case] expected_name: &str,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("report.txt", b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(upload_path("/files/content")))
            // "hello"のSHA-1
            .and(header(
                "content-md5",
                "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json(expected_name)],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UploadFileOptions { name: option_name };

        let file = client
            .upload_file_with_threshold(&file_path, &FolderId::root(), &options, THRESHOLD)
            .await?;

        assert_eq!(file.name, expected_name);
        let requests = requests(&server, "POST", &upload_path("/files/content")).await;
        let [request] = requests.as_slice() else {
            anyhow::bail!("expected one request, got {}", requests.len());
        };
        let content_type = request
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("multipart/form-data; boundary="),
            "content-type: {content_type}"
        );
        let body = String::from_utf8_lossy(&request.body);
        let attributes = format!(r#"{{"name":"{expected_name}","parent":{{"id":"0"}}}}"#);
        let attributes_at = body.find(&attributes);
        let file_at = body.find("name=\"file\"");
        let content_at = body.find("\r\n\r\nhello\r\n");
        assert!(
            matches!((attributes_at, file_at, content_at), (Some(a), Some(f), Some(c)) if a < f && f < c),
            "body: {body}"
        );

        Ok(())
    }

    /// BOXが拒否するファイル名、またはパスから取得できないファイル名の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::invalid_name(Some("a/b"), "InvalidRequestBody")]
    #[case::too_long_name(Some(&*"あ".repeat(256)), "InvalidRequestBody")]
    #[case::name_from_dot_dot_path(None, "InvalidArgument")]
    #[tokio::test]
    async fn test_upload_file_rejects_name_without_request(
        #[case] option_name: Option<&str>,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        let (dir, _) = create_file("report.txt", b"hello")?;
        // `..`で終わるパスからは、ファイル名を取得できない。
        let file_path = dir.path().join("..");
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let options = UploadFileOptions { name: option_name };

        let result = client
            .upload_file_with_threshold(&file_path, &FolderId::root(), &options, THRESHOLD)
            .await;

        let matched = match expected {
            "InvalidArgument" => matches!(result, Err(BoxClientError::InvalidArgument(_))),
            _ => matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
        };
        assert!(matched, "result: {result:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// ファイルが存在しない場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_upload_file_not_found() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .upload_file(
                dir.path().join("missing.txt"),
                &FolderId::root(),
                &UploadFileOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::ReadFile { .. })),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// アップロードセッションの作成と、すべての部分のアップロードのモックを登録する。
    ///
    /// BOXのAPIリファレンスとガイドで部分のレスポンスの形式が異なるため、最後の部分だけ`part`属性で包まずに返す。
    async fn mount_session_and_parts(server: &MockServer) {
        mount_session_and_delayed_parts(server, [Duration::ZERO; 3]).await;
    }

    /// アップロードセッションの作成と、指定した時間だけ遅れて応答する部分のアップロードのモックを登録する。
    async fn mount_session_and_delayed_parts(server: &MockServer, part_delays: [Duration; 3]) {
        Mock::given(method("POST"))
            .and(path(upload_path("/files/upload_sessions")))
            .and(body_json(json!({
                "folder_id": "0",
                "file_size": CHUNKED_CONTENT.len(),
                "file_name": "big.bin",
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "upload_session",
                "id": SESSION_ID,
                "part_size": PART_SIZE,
                "total_parts": 3,
                "num_parts_processed": 0,
            })))
            .expect(1)
            .mount(server)
            .await;

        let chunks: Vec<&[u8]> = CHUNKED_CONTENT.chunks(PART_SIZE).collect();
        let last = chunks.len() - 1;
        for ((i, chunk), delay) in chunks.into_iter().enumerate().zip(part_delays) {
            let offset = i * PART_SIZE;
            let part = json!({
                "part_id": format!("P{i}"),
                "offset": offset,
                "size": chunk.len(),
                "sha1": format!("{:x}", Sha1::digest(chunk)),
            });
            let body = if i == last {
                part
            } else {
                json!({"part": part})
            };
            Mock::given(method("PUT"))
                .and(path(upload_path(&format!(
                    "/files/upload_sessions/{SESSION_ID}"
                ))))
                .and(header(
                    "content-range",
                    format!(
                        "bytes {offset}-{}/{}",
                        offset + chunk.len() - 1,
                        CHUNKED_CONTENT.len()
                    )
                    .as_str(),
                ))
                .and(header(
                    "digest",
                    sha1_digest_header(&Sha1::digest(chunk)).as_str(),
                ))
                .and(header("content-type", "application/octet-stream"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(body)
                        .set_delay(delay),
                )
                .expect(1)
                .mount(server)
                .await;
        }
    }

    /// コミットで送信するボディを返す。
    fn commit_body() -> serde_json::Value {
        json!({"parts": [
            {"part_id": "P0", "offset": 0, "size": 10},
            {"part_id": "P1", "offset": 10, "size": 10},
            {"part_id": "P2", "offset": 20, "size": 5},
        ]})
    }

    /// 閾値を超えるファイルを、部分ごとにアップロードしてからコミットすることを確認
    #[tokio::test]
    async fn test_upload_file_in_chunks() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_parts(&server).await;
        Mock::given(method("POST"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}/commit"
            ))))
            .and(header(
                "digest",
                sha1_digest_header(&Sha1::digest(CHUNKED_CONTENT)).as_str(),
            ))
            .and(body_json(commit_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json("big.bin")],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.name, "big.bin");
        let aborts = requests(
            &server,
            "DELETE",
            &upload_path(&format!("/files/upload_sessions/{SESSION_ID}")),
        )
        .await;
        assert!(aborts.is_empty());

        Ok(())
    }

    /// コミットでBOXが202を返した場合は、待機してからコミットを再送することを確認
    ///
    /// BOXのAPIリファレンスとガイドでコミットのレスポンスの形式が異なるため、ファイルを直接返す形式も確認する。
    #[tokio::test]
    async fn test_upload_file_in_chunks_waits_for_processing() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_parts(&server).await;
        let commit_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}/commit"));
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .respond_with(ResponseTemplate::new(202).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .respond_with(ResponseTemplate::new(201).set_body_json(file_json("big.bin")))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.name, "big.bin");
        assert_eq!(requests(&server, "POST", &commit_path).await.len(), 2);

        Ok(())
    }

    /// 部分の応答の遅延
    const PART_DELAY: Duration = Duration::from_millis(300);

    /// 並列数に応じて部分を並列にアップロードし、完了の順によらず位置の順に部分をコミットすることを確認
    ///
    /// 各部分の応答を同じ時間だけ遅らせ、アップロードにかかった時間から並列にアップロードしたかを判定する。
    /// 並列数が部分の数以上の場合は、最初の部分の応答を最も遅らせて、完了の順を位置の順と逆にする。
    /// 並列数が1の場合は、すべての部分を順にアップロードするため、遅延の合計以上の時間がかかる。
    #[rstest]
    #[case::sequential(1, [PART_DELAY; 3], PART_DELAY * 3, None)]
    #[case::parallel(
        3,
        [PART_DELAY * 2, PART_DELAY, Duration::ZERO],
        PART_DELAY * 2,
        Some(PART_DELAY * 3)
    )]
    #[tokio::test]
    async fn test_upload_file_in_chunks_concurrently(
        #[case] concurrency: usize,
        #[case] part_delays: [Duration; 3],
        #[case] min_elapsed: Duration,
        #[case] max_elapsed: Option<Duration>,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        mount_session_and_delayed_parts(&server, part_delays).await;
        Mock::given(method("POST"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}/commit"
            ))))
            .and(body_json(commit_body()))
            .respond_with(ResponseTemplate::new(201).set_body_json(file_json("big.bin")))
            .expect(1)
            .mount(&server)
            .await;
        let client =
            create_client_with_upload_concurrency(&server, short_retry(0)?, concurrency).await?;

        let started_at = std::time::Instant::now();
        client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await?;
        let elapsed = started_at.elapsed();

        assert!(min_elapsed <= elapsed, "elapsed: {elapsed:?}");
        if let Some(max_elapsed) = max_elapsed {
            assert!(elapsed < max_elapsed, "elapsed: {elapsed:?}");
        }

        Ok(())
    }

    /// 部分のアップロードに失敗した場合は、アップロードセッションを中止してからエラーを返すことを確認
    #[tokio::test]
    async fn test_upload_file_in_chunks_aborts_session_on_failure() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file("big.bin", CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let session_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}"));
        Mock::given(method("POST"))
            .and(path(upload_path("/files/upload_sessions")))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "upload_session",
                "id": SESSION_ID,
                "part_size": PART_SIZE,
                "total_parts": 3,
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(session_path.as_str()))
            .respond_with(ResponseTemplate::new(416))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(session_path.as_str()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .upload_file_with_threshold(
                &file_path,
                &FolderId::root(),
                &UploadFileOptions::default(),
                THRESHOLD,
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 416),
            "result: {result:?}"
        );
        assert_eq!(requests(&server, "DELETE", &session_path).await.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_update_file_content {
    use std::path::PathBuf;

    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    use super::*;
    use crate::test_support::{
        UPLOAD_BASE_PATH, create_client, mount_token, request_count, short_retry,
    };

    /// 分割アップロードに切り替えるファイルサイズ
    const THRESHOLD: u64 = 20;

    /// 分割アップロードでアップロードするファイルの内容
    const CHUNKED_CONTENT: &[u8] = b"abcdefghijklmnopqrstuvwxy";

    /// 分割アップロードの部分のサイズ
    const PART_SIZE: usize = 10;

    /// アップロードセッションのID
    const SESSION_ID: &str = "S1";

    /// アップロードのAPIのパスを返す。
    fn upload_path(suffix: &str) -> String {
        format!("{UPLOAD_BASE_PATH}{suffix}")
    }

    /// 一時ディレクトリに、指定した内容のファイルを作成する。
    fn create_file(content: &[u8]) -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("report.txt");
        std::fs::write(&path, content)?;
        Ok((dir, path))
    }

    /// ファイル情報のJSONを返す。
    fn file_json(name: &str) -> serde_json::Value {
        json!({"type": "file", "id": "123", "etag": "2", "name": name})
    }

    /// 指定したメソッドとパスのリクエストを返す。
    async fn requests(server: &MockServer, http_method: &str, request_path: &str) -> Vec<Request> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == http_method && r.url.path() == request_path)
            .collect()
    }

    /// リクエストの`If-Match`ヘッダーを返す。
    fn if_match(request: &Request) -> Option<String> {
        request
            .headers
            .get("if-match")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    /// 1回のリクエストで、`POST /files/{file_id}/content`に新しい内容を送信することを確認
    ///
    /// 新しいファイル名を指定した場合だけ`attributes`パートに名前を含め、`ETag`を指定した場合だけ
    /// `If-Match`ヘッダーを送信することも確認する。
    #[rstest]
    #[case::default(UpdateFileContentOptions::default(), "{}", None)]
    #[case::name_and_etag(
        UpdateFileContentOptions::default().with_name("report-v2.txt").with_etag("1"),
        r#"{"name":"report-v2.txt"}"#,
        Some("1")
    )]
    #[tokio::test]
    async fn test_update_file_content_directly(
        #[case] options: UpdateFileContentOptions<'static>,
        #[case] expected_attributes: &str,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let (_dir, file_path) = create_file(b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let content_path = upload_path("/files/123/content");
        Mock::given(method("POST"))
            .and(path(content_path.as_str()))
            // "hello"のSHA-1
            .and(header(
                "content-md5",
                "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json("report.txt")],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .update_file_content_with_threshold(
                &FileId::new("123"),
                &file_path,
                &options,
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.id, FileId::new("123"));
        let requests = requests(&server, "POST", &content_path).await;
        let [request] = requests.as_slice() else {
            anyhow::bail!("expected one request, got {}", requests.len());
        };
        assert_eq!(if_match(request).as_deref(), expected_if_match);
        let body = String::from_utf8_lossy(&request.body);
        let attributes = format!("\r\n\r\n{expected_attributes}\r\n");
        let attributes_at = body.find(&attributes);
        let content_at = body.find("\r\n\r\nhello\r\n");
        assert!(
            matches!((attributes_at, content_at), (Some(a), Some(c)) if a < c),
            "body: {body}"
        );

        Ok(())
    }

    /// BOXが拒否する新しいファイル名を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_update_file_content_rejects_invalid_name() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file(b"hello")?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .update_file_content(
                &FileId::new("123"),
                &file_path,
                &UpdateFileContentOptions::default().with_name("a/b"),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 閾値を超える場合は、`POST /files/{file_id}/upload_sessions`でアップロードセッションを作成し、
    /// 部分をアップロードしてから、`If-Match`ヘッダーを付けてコミットすることを確認
    #[tokio::test]
    async fn test_update_file_content_in_chunks() -> anyhow::Result<()> {
        let (_dir, file_path) = create_file(CHUNKED_CONTENT)?;
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(upload_path("/files/123/upload_sessions")))
            .and(body_json(json!({
                "file_size": CHUNKED_CONTENT.len(),
                "file_name": "report-v2.txt",
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "upload_session",
                "id": SESSION_ID,
                "part_size": PART_SIZE,
                "total_parts": 3,
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(upload_path(&format!(
                "/files/upload_sessions/{SESSION_ID}"
            ))))
            .respond_with(|request: &Request| {
                // `Content-Range`ヘッダーから部分の位置とサイズを取り出し、アップロードした部分として返す。
                let range = request
                    .headers
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                let (start, end) = range
                    .trim_start_matches("bytes ")
                    .split_once('/')
                    .and_then(|(r, _)| r.split_once('-'))
                    .and_then(|(s, e)| Some((s.parse::<u64>().ok()?, e.parse::<u64>().ok()?)))
                    .unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(json!({"part": {
                    "part_id": format!("P{start}"),
                    "offset": start,
                    "size": end - start + 1,
                }}))
            })
            .expect(3)
            .mount(&server)
            .await;
        let commit_path = upload_path(&format!("/files/upload_sessions/{SESSION_ID}/commit"));
        Mock::given(method("POST"))
            .and(path(commit_path.as_str()))
            .and(header("if-match", "1"))
            .and(header(
                "digest",
                sha1_digest_header(&Sha1::digest(CHUNKED_CONTENT)).as_str(),
            ))
            .and(body_json(json!({"parts": [
                {"part_id": "P0", "offset": 0, "size": 10},
                {"part_id": "P10", "offset": 10, "size": 10},
                {"part_id": "P20", "offset": 20, "size": 5},
            ]})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "total_count": 1,
                "entries": [file_json("report-v2.txt")],
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UpdateFileContentOptions::default()
            .with_name("report-v2.txt")
            .with_etag("1");

        let file = client
            .update_file_content_with_threshold(
                &FileId::new("123"),
                &file_path,
                &options,
                THRESHOLD,
            )
            .await?;

        assert_eq!(file.name, "report-v2.txt");
        // アップロードセッションの作成では、`If-Match`ヘッダーを送信しない。
        let sessions = requests(&server, "POST", &upload_path("/files/123/upload_sessions")).await;
        assert!(sessions.iter().all(|r| if_match(r).is_none()));

        Ok(())
    }
}
