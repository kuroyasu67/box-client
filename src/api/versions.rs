//! ファイルのバージョンに関するAPI
//!
//! BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, DeleteItemOptions, FileId, FileVersionId,
    Idempotency, UserMini,
    api::folders::{MAX_ITEMS_LIMIT, validate_limit},
    request::ApiRequest,
};

/// ファイルのバージョン
///
/// ファイル情報に含まれるバージョンは、IDとSHA-1ハッシュ値だけを持つ。
/// [`BoxClient::file_version`]で取得したバージョンは、それ以外の属性も持つ。
/// どちらも同じ型で受け取れるように、IDおよびSHA-1ハッシュ値以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct FileVersion {
    /// バージョンのID
    pub id: FileVersionId,

    /// このバージョンのファイルの内容のSHA-1ハッシュ値
    pub sha1: Option<String>,

    /// このバージョンのファイル名
    pub name: Option<String>,

    /// このバージョンのファイルのバイト数
    pub size: Option<u64>,

    /// バージョン番号
    ///
    /// BOXは、バージョン番号を数字の文字列で返す。
    pub version_number: Option<String>,

    /// バージョンを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// バージョンを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// バージョンを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// バージョンをゴミ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// バージョンをゴミ箱に移動したユーザー
    pub trashed_by: Option<UserMini>,

    /// バージョンをゴミ箱から復元した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub restored_at: Option<OffsetDateTime>,

    /// バージョンをゴミ箱から復元したユーザー
    pub restored_by: Option<UserMini>,

    /// ゴミ箱に移動したバージョンを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// バージョンをアップロードしたユーザーの表示名
    ///
    /// 通常は、アップロードしたときのユーザー名である。
    /// ファイルリクエストで匿名のユーザーがアップロードした場合は、メールアドレス、`File Request`または`Someone`になる。
    pub uploader_display_name: Option<String>,
}

/// ファイルのバージョンのリストで指定できるオフセットの最大値
///
/// BOXは、オフセットがこの値を超えるリクエストを400で拒否する。
pub const MAX_FILE_VERSIONS_OFFSET: u64 = 10_000;

/// ファイルのバージョンのリストのオプション
///
/// ```
/// use box_client::ListFileVersionsOptions;
///
/// let options = ListFileVersionsOptions::default().with_limit(100);
/// assert_eq!(options.limit, Some(100));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListFileVersionsOptions {
    /// 1ページで取得するバージョン数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

impl ListFileVersionsOptions {
    /// 1ページで取得するバージョン数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// ファイルのバージョンの1ページ
///
/// BOXは、ファイルのバージョンのリストをオフセット方式でページ送りする。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct FileVersionsPage {
    /// このページのバージョン
    pub entries: Vec<FileVersion>,

    /// 最後のバージョンのオフセットに1を加えた値
    ///
    /// BOXのドキュメントによると、実際のバージョン数はこの値より少ない場合がある。
    pub total_count: u64,

    /// このページの取得に使用したバージョン数の上限
    pub limit: u32,

    /// このページの最初のバージョンのオフセット（0始まり）
    pub offset: u64,
}

/// ファイルのバージョンの昇格リクエストのボディ
#[derive(Debug, Serialize)]
struct PromoteFileVersionBody<'a> {
    /// 昇格するバージョンのID
    id: &'a FileVersionId,

    /// 昇格する項目の種類
    ///
    /// BOXは、`file_version`だけを受け付ける。
    #[serde(rename = "type")]
    item_type: &'static str,
}

/// 削除したファイルのバージョンの復元リクエストのボディ
#[derive(Debug, Serialize)]
struct RestoreFileVersionBody {
    /// ゴミ箱に移動した日時
    ///
    /// BOXは、`null`を指定するとバージョンを復元する。
    trashed_at: Option<OffsetDateTime>,
}

impl BoxClient {
    /// ファイルの特定のバージョンの情報を取得する。
    ///
    /// BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ファイルのID
    /// - `file_version_id`: 情報を取得するバージョンのID。
    ///   ファイルの現在のバージョンのIDは、[`BoxClient::file_information`]で取得した[`File::file_version`](crate::File::file_version)で確認できる。
    ///
    /// # Errors
    ///
    /// ファイルまたはバージョンが存在しない場合、またはファイルにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn file_version(
        &self,
        file_id: &FileId,
        file_version_id: &FileVersionId,
    ) -> BoxClientResult<FileVersion> {
        self.send_json(ApiRequest::get([
            "files",
            file_id.as_ref(),
            "versions",
            file_version_id.as_ref(),
        ]))
        .await
    }

    /// ファイルの過去のバージョンを1ページ分リストする。
    ///
    /// BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。
    /// BOXのドキュメントは、このAPIが過去のバージョンを返すと記載している。
    /// 現在のバージョンのIDは、[`BoxClient::file_information`]で取得した[`File::file_version`](crate::File::file_version)で確認できる。
    ///
    /// 1回の呼び出しで返すバージョンは最大で`limit`件である。次のページを取得する場合は、
    /// 前のページの`offset`に、前のページの`entries`の数を加えた値を`offset`に指定して再度呼び出す。
    /// すべてのバージョンが必要な場合は、[`BoxClient::list_all_file_versions`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ファイルのID
    /// - `offset`: 取得する最初のバージョンのオフセット（0始まり）。[`MAX_FILE_VERSIONS_OFFSET`]以下を指定する。
    /// - `options`: バージョン数の上限のオプション
    ///
    /// # Errors
    ///
    /// `offset`が[`MAX_FILE_VERSIONS_OFFSET`]を超える場合、または`options.limit`が範囲外の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ファイルが存在しない場合、またはファイルにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_file_versions(
        &self,
        file_id: &FileId,
        offset: u64,
        options: &ListFileVersionsOptions,
    ) -> BoxClientResult<FileVersionsPage> {
        if MAX_FILE_VERSIONS_OFFSET < offset {
            return Err(BoxClientError::InvalidArgument(
                "offset must be less than or equal to 10000",
            ));
        }

        let mut request = ApiRequest::get(["files", file_id.as_ref(), "versions"])
            .query("offset", offset.to_string());
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }

        self.send_json(request).await
    }

    /// ファイルの過去のすべてのバージョンを、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。
    /// 取得したバージョンの数が`total_count`に達した場合、またはBOXが空のページを返した場合に終了する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ファイルのID
    /// - `options`: バージョン数の上限のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 次のページのオフセットが[`MAX_FILE_VERSIONS_OFFSET`]を超え、残りのバージョンを取得できない場合は、
    /// 一部のバージョンだけを返さないように[`BoxClientError::UnexpectedResponse`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みのバージョンを破棄してエラーを返す。
    pub async fn list_all_file_versions(
        &self,
        file_id: &FileId,
        options: &ListFileVersionsOptions,
    ) -> BoxClientResult<Vec<FileVersion>> {
        let options = ListFileVersionsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let mut versions = Vec::new();
        let mut offset = 0;
        let mut pages: u32 = 0;
        loop {
            let page = self.list_file_versions(file_id, offset, &options).await?;
            pages += 1;
            let count = page.entries.len() as u64;
            versions.extend(page.entries);
            // BOXが返す`offset`ではなく、要求した`offset`から次のページの位置を求める。
            // BOXが`offset`を無視して同じページを返し続けても、位置を進めて必ずループを終える。
            offset += count;

            if count == 0 || page.total_count <= offset {
                break;
            }
            if MAX_FILE_VERSIONS_OFFSET < offset {
                return Err(BoxClientError::UnexpectedResponse(
                    "file versions cannot be listed beyond the maximum offset",
                ));
            }
        }

        tracing::debug!(
            file_id = file_id.as_ref(),
            pages,
            versions = versions.len(),
            "listed all file versions"
        );

        Ok(versions)
    }

    /// ファイルの過去のバージョンを昇格し、現在のバージョンにする。
    ///
    /// BOXは、指定したバージョンの複製を新しいバージョンとして作成し、バージョン履歴の先頭に置く。
    /// 新しいバージョンは、指定したバージョンと同じ内容、ハッシュ値、`ETag`および名前を持つ。
    /// コメントなどの他の属性は、昇格したバージョンの時点の値に戻らない。
    /// BOXは、Box Notesの復元にこのAPIを使用しないように案内している。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、同じバージョンの複製を重複して作成するおそれがあるためである。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ファイルのID
    /// - `file_version_id`: 昇格する過去のバージョンのID。[`BoxClient::list_file_versions`]で取得できる。
    ///
    /// # Returns
    ///
    /// 昇格によって作成された、現在のバージョンを返す。昇格したバージョンとは異なるIDを持つ。
    ///
    /// # Errors
    ///
    /// ファイルまたはバージョンが存在しない場合、またはファイルを操作する権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがバージョンを昇格した可能性がある。
    /// したがって、[`BoxClient::file_information`]でファイルの現在のバージョンを確認する。
    pub async fn promote_file_version(
        &self,
        file_id: &FileId,
        file_version_id: &FileVersionId,
    ) -> BoxClientResult<FileVersion> {
        let request = ApiRequest::post(["files", file_id.as_ref(), "versions", "current"]).json(
            &PromoteFileVersionBody {
                id: file_version_id,
                item_type: "file_version",
            },
        )?;
        let version: FileVersion = self.send_json(request).await?;

        tracing::debug!(
            file_id = file_id.as_ref(),
            promoted_version_id = file_version_id.as_ref(),
            new_version_id = version.id.as_ref(),
            "promoted file version"
        );

        Ok(version)
    }

    /// 削除したファイルのバージョンを復元する。
    ///
    /// BOXは、削除したバージョンをゴミ箱に移動している。このメソッドは、ゴミ箱に移動したバージョンを
    /// ファイルのバージョン履歴に戻す。ファイル自体をゴミ箱から復元する場合は、
    /// [`BoxClient::restore_file`]を使用する。
    /// BOXは、Box Notesの復元にこのAPIを使用しないように案内している。
    ///
    /// 復元したバージョンを再び復元しても状態は変わらないため、HTTPメソッドがPUTであることに従い、
    /// 冪等なリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ファイルのID
    /// - `file_version_id`: 復元するバージョンのID
    ///
    /// # Returns
    ///
    /// 復元したバージョンを返す。
    ///
    /// # Errors
    ///
    /// ファイルまたはバージョンが存在しない場合、またはファイルを操作する権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn restore_file_version(
        &self,
        file_id: &FileId,
        file_version_id: &FileVersionId,
    ) -> BoxClientResult<FileVersion> {
        let request = ApiRequest::put([
            "files",
            file_id.as_ref(),
            "versions",
            file_version_id.as_ref(),
        ])
        .json(&RestoreFileVersionBody { trashed_at: None })?;
        let version = self.send_json(request).await?;

        tracing::debug!(
            file_id = file_id.as_ref(),
            file_version_id = file_version_id.as_ref(),
            "restored file version"
        );

        Ok(version)
    }

    /// ファイルのバージョンを削除する。
    ///
    /// BOXは、削除したバージョンをゴミ箱に移動する。ゴミ箱に移動したバージョンは、
    /// [`BoxClient::restore_file_version`]でファイルのバージョン履歴に戻せる。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがバージョンの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのバージョンに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、[`BoxClient::delete_file`]と同様に、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: ファイルのID
    /// - `file_version_id`: 削除するバージョンのID
    /// - `options`: `ETag`のオプション。`ETag`を指定した場合は`If-Match`ヘッダーで送信し、
    ///   ファイルが`ETag`の取得後に変更されていると、BOXはバージョンを削除せずに412を返す。
    ///
    /// # Errors
    ///
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: ファイルまたはバージョンが存在しない、またはファイルにアクセスする権限がない。
    /// - 412: `options.etag`がファイルの現在の`ETag`と一致しない。
    ///
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがバージョンを削除した可能性があるため、
    /// [`BoxClient::file_version`]でバージョンの`trashed_at`を確認する。
    pub async fn delete_file_version(
        &self,
        file_id: &FileId,
        file_version_id: &FileVersionId,
        options: &DeleteItemOptions<'_>,
    ) -> BoxClientResult<()> {
        let request = ApiRequest::delete([
            "files",
            file_id.as_ref(),
            "versions",
            file_version_id.as_ref(),
        ])
        .idempotency(Idempotency::NonIdempotent)
        .if_match(options.etag)?;

        self.send_empty(request).await?;

        tracing::debug!(
            file_id = file_id.as_ref(),
            file_version_id = file_version_id.as_ref(),
            "deleted file version"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_file_version {
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        UserId, UserMini,
        test_support::{create_client, mount_token, not_found_response, short_retry},
    };

    /// `GET /files/{file_id}/versions/{file_version_id}`を呼び出し、バージョンの属性をデシリアライズすることを確認
    #[tokio::test]
    async fn test_file_version() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        Mock::given(method("GET"))
            .and(path("/2.0/files/12345/versions/67890"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file_version",
                "id": "67890",
                "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
                "name": "Contract.pdf",
                "size": 629_644,
                "version_number": "2",
                "created_at": "2012-12-12T10:53:43-08:00",
                "modified_at": "2012-12-12T11:15:04-08:00",
                "modified_by": user,
                "trashed_at": null,
                "trashed_by": null,
                "restored_at": null,
                "restored_by": null,
                "purged_at": null,
                "uploader_display_name": "Test User",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let version = client
            .file_version(&FileId::new("12345"), &FileVersionId::new("67890"))
            .await?;

        assert_eq!(
            version,
            FileVersion {
                id: FileVersionId::new("67890"),
                sha1: Some("85136c79cbf9fe36bb9d05d0639c70c265c18d37".to_owned()),
                name: Some("Contract.pdf".to_owned()),
                size: Some(629_644),
                version_number: Some("2".to_owned()),
                created_at: Some(datetime!(2012-12-12 10:53:43 -08:00)),
                modified_at: Some(datetime!(2012-12-12 11:15:04 -08:00)),
                modified_by: Some(UserMini {
                    id: UserId::new("33224"),
                    user_type: "user".to_owned(),
                    name: Some("Test User".to_owned()),
                    login: Some("user@example.com".to_owned()),
                }),
                trashed_at: None,
                trashed_by: None,
                restored_at: None,
                restored_by: None,
                purged_at: None,
                uploader_display_name: Some("Test User".to_owned()),
            }
        );

        Ok(())
    }

    /// バージョンが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_file_version_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/12345/versions/999"))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .file_version(&FileId::new("12345"), &FileVersionId::new("999"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_file_versions {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, Request, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    use super::*;
    use crate::test_support::{
        count_requests_to_path, create_client, mount_token, request_count, short_retry,
    };

    /// ファイルのバージョンのAPIのパス
    const VERSIONS_PATH: &str = "/2.0/files/12345/versions";

    /// バージョンのJSONを返す。
    fn version_json(id: u64) -> serde_json::Value {
        json!({"type": "file_version", "id": id.to_string(), "sha1": "abc"})
    }

    /// `offset`から`limit`件のバージョンを、全体が`total`件として返すレスポンスを作成する。
    fn page_response(request: &Request, total: u64) -> ResponseTemplate {
        let param = |name: &str| {
            request
                .url
                .query_pairs()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.parse::<u64>().ok())
        };
        let offset = param("offset").unwrap_or(0);
        let limit = param("limit").unwrap_or(1000);
        let end = total.min(offset + limit);
        let entries: Vec<_> = (offset..end).map(version_json).collect();
        ResponseTemplate::new(200).set_body_json(json!({
            "total_count": total,
            "limit": limit,
            "offset": offset,
            "order": [{"by": "created_at", "direction": "DESC"}],
            "entries": entries,
        }))
    }

    /// `offset`と`limit`をクエリパラメーターで送信し、バージョンの1ページをデシリアライズすることを確認
    #[tokio::test]
    async fn test_list_file_versions() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(VERSIONS_PATH))
            .and(query_param("offset", "2"))
            .and(query_param("limit", "2"))
            .respond_with(|request: &Request| page_response(request, 5))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let page = client
            .list_file_versions(
                &FileId::new("12345"),
                2,
                &ListFileVersionsOptions::default().with_limit(2),
            )
            .await?;

        assert_eq!(page.total_count, 5);
        assert_eq!(page.limit, 2);
        assert_eq!(page.offset, 2);
        let ids: Vec<_> = page.entries.iter().map(|v| v.id.as_ref()).collect();
        assert_eq!(ids, ["2", "3"]);

        Ok(())
    }

    /// オフセットまたは項目数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::offset_above_max(MAX_FILE_VERSIONS_OFFSET + 1, None)]
    #[case::zero_limit(0, Some(0))]
    #[case::limit_above_max(0, Some(MAX_ITEMS_LIMIT + 1))]
    #[tokio::test]
    async fn test_list_file_versions_rejects_without_request(
        #[case] offset: u64,
        #[case] limit: Option<u32>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .list_file_versions(
                &FileId::new("12345"),
                offset,
                &ListFileVersionsOptions { limit },
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// `total_count`に達するまでページを送り、すべてのバージョンを順に返すことを確認
    ///
    /// `total_count`が上限の倍数でない場合、および倍数の場合のページ数も確認する。
    #[rstest]
    #[case::partial_last_page(5, 2, 3)]
    #[case::full_last_page(4, 2, 2)]
    #[case::no_versions(0, 2, 1)]
    #[tokio::test]
    async fn test_list_all_file_versions(
        #[case] total: u64,
        #[case] limit: u32,
        #[case] expected_pages: usize,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(VERSIONS_PATH))
            .respond_with(move |request: &Request| page_response(request, total))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let versions = client
            .list_all_file_versions(
                &FileId::new("12345"),
                &ListFileVersionsOptions::default().with_limit(limit),
            )
            .await?;

        let ids: Vec<String> = versions.iter().map(|v| v.id.to_string()).collect();
        let expected: Vec<String> = (0..total).map(|i| i.to_string()).collect();
        assert_eq!(ids, expected);
        let pages = count_requests_to_path(&server, VERSIONS_PATH).await;
        assert_eq!(pages, expected_pages);

        Ok(())
    }

    /// BOXが要求と異なる`offset`を返し続けても、要求した`offset`から位置を進めてループを終えることを確認
    #[tokio::test]
    async fn test_list_all_file_versions_ignores_returned_offset() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // BOXは、要求した`offset`によらず、常に`offset`が0で1件のバージョンを返す。
        Mock::given(method("GET"))
            .and(path(VERSIONS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_count": 3,
                "offset": 0,
                "limit": 1,
                "entries": [version_json(0)],
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let versions = client
            .list_all_file_versions(
                &FileId::new("12345"),
                &ListFileVersionsOptions::default().with_limit(1),
            )
            .await?;

        assert_eq!(versions.len(), 3);

        Ok(())
    }

    /// 次のページのオフセットが最大値を超える場合は、一部のバージョンだけを返さずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_file_versions_exceeds_max_offset() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(VERSIONS_PATH))
            .respond_with(|request: &Request| page_response(request, 20_000))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_file_versions(&FileId::new("12345"), &ListFileVersionsOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_promote_file_version {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{assert_not_retried, create_client, mount_token, short_retry};

    /// ファイルの現在のバージョンのAPIのパス
    const CURRENT_VERSION_PATH: &str = "/2.0/files/12345/versions/current";

    /// 昇格するバージョンのIDと種類を送信し、作成された現在のバージョンを返すことを確認
    #[tokio::test]
    async fn test_promote_file_version() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(CURRENT_VERSION_PATH))
            .and(body_json(json!({"id": "67890", "type": "file_version"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "file_version",
                "id": "99999",
                "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
                "name": "Contract.pdf",
                "version_number": "3",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let version = client
            .promote_file_version(&FileId::new("12345"), &FileVersionId::new("67890"))
            .await?;

        assert_eq!(version.id, FileVersionId::new("99999"));
        assert_eq!(version.version_number.as_deref(), Some("3"));

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 昇格は冪等でないため、5xxの後に再送すると、同じバージョンの複製を重複して作成するおそれがある。
    #[rstest]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_promote_file_version_error_is_not_retried(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        assert_not_retried(
            "POST",
            CURRENT_VERSION_PATH,
            status,
            async |client: &BoxClient| {
                client
                    .promote_file_version(&FileId::new("12345"), &FileVersionId::new("67890"))
                    .await
            },
        )
        .await
    }
}

#[cfg(test)]
mod test_restore_file_version {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// ファイルのバージョンのAPIのパス
    const VERSION_PATH: &str = "/2.0/files/12345/versions/67890";

    /// `trashed_at`に`null`を指定してPUTで送信し、復元したバージョンを返すことを確認
    #[tokio::test]
    async fn test_restore_file_version() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(VERSION_PATH))
            .and(body_json(json!({"trashed_at": null})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file_version",
                "id": "67890",
                "version_number": "1",
                "trashed_at": null,
                "restored_at": "2012-12-12T10:53:43-08:00",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let version = client
            .restore_file_version(&FileId::new("12345"), &FileVersionId::new("67890"))
            .await?;

        assert_eq!(version.id, FileVersionId::new("67890"));
        assert!(version.trashed_at.is_none());
        assert!(version.restored_at.is_some());

        Ok(())
    }

    /// 復元は冪等であるため、5xxの後に再試行し、4xxは再試行せずにそのまま返すことを確認
    #[rstest]
    #[case::not_found(404, 1)]
    #[case::internal_server_error(500, 3)]
    #[tokio::test]
    async fn test_restore_file_version_retries_only_server_errors(
        #[case] status: u16,
        #[case] expected_requests: u64,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(VERSION_PATH))
            .respond_with(ResponseTemplate::new(status))
            .expect(expected_requests)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .restore_file_version(&FileId::new("12345"), &FileVersionId::new("67890"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_file_version {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, create_client, if_match_headers, mount_token, short_retry,
    };

    /// ファイルのバージョンのAPIのパス
    const VERSION_PATH: &str = "/2.0/files/12345/versions/67890";

    /// `ETag`を指定した場合だけ、If-Matchヘッダーを送信してバージョンを削除することを確認
    #[rstest]
    #[case::without_etag(None)]
    #[case::with_etag(Some("1"))]
    #[tokio::test]
    async fn test_delete_file_version(#[case] etag: Option<&'static str>) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(VERSION_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client
            .delete_file_version(
                &FileId::new("12345"),
                &FileVersionId::new("67890"),
                &DeleteItemOptions { etag },
            )
            .await?;

        assert_eq!(
            if_match_headers(&server, "DELETE", VERSION_PATH).await,
            vec![etag.map(str::to_owned)]
        );

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 削除は冪等でないため、5xxの後にも再送しない。
    #[rstest]
    #[case::not_found(404)]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_file_version_error_is_not_retried(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        assert_not_retried(
            "DELETE",
            VERSION_PATH,
            status,
            async |client: &BoxClient| {
                client
                    .delete_file_version(
                        &FileId::new("12345"),
                        &FileVersionId::new("67890"),
                        &DeleteItemOptions::default(),
                    )
                    .await
            },
        )
        .await
    }
}
