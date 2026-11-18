//! ファイルに関するAPI

use garde::Validate;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use reqwest::header::IF_MATCH;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, FileId, FileVersion, FileVersionId, FolderId,
    FolderItem, Idempotency, ItemStatus, MoveDestination, ParentFolder, PathCollection, UserMini,
    ValidationError,
    api::types::{
        MoveBody, RenameBody, SetDescriptionBody, UpdateItemOptions, validate_optional_item_name,
    },
    request::ApiRequest,
};

/// ファイル情報
///
/// フォルダ情報と同様に、BOXがnullを返したり省略したりしても取得を失敗させないため、
/// IDおよび名前以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct File {
    /// ファイルID
    pub id: FileId,

    /// ファイルのバージョンを識別する`ETag`
    pub etag: Option<String>,

    /// ファイルの変更を識別するシーケンスID
    pub sequence_id: Option<String>,

    /// ファイル名
    pub name: String,

    /// ファイルの内容のSHA-1ハッシュ値
    pub sha1: Option<String>,

    /// ファイルの現在のバージョン
    pub file_version: Option<FileVersion>,

    /// ファイルの説明
    pub description: Option<String>,

    /// ファイルのバイト数
    pub size: Option<u64>,

    /// ルートフォルダからこのファイルの親フォルダまでのパス
    pub path_collection: Option<PathCollection>,

    /// BOX上でファイルを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// BOX上でファイルを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// ファイルをゴミ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// ゴミ箱に移動したファイルを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// ファイルの内容を作成した日時
    ///
    /// アップロードしたクライアントが指定した日時であり、BOX上での作成日時と異なる場合がある。
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_created_at: Option<OffsetDateTime>,

    /// ファイルの内容を最後に更新した日時
    ///
    /// アップロードしたクライアントが指定した日時であり、BOX上での更新日時と異なる場合がある。
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_modified_at: Option<OffsetDateTime>,

    /// ファイルを作成したユーザー
    pub created_by: Option<UserMini>,

    /// ファイルを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// ファイルを所有するユーザー
    pub owned_by: Option<UserMini>,

    /// 親フォルダ
    pub parent: Option<FolderItem>,

    /// ファイルの状態
    pub item_status: Option<ItemStatus>,
}

/// ファイルのコピーのオプション
///
/// # Examples
///
/// ```
/// use box_client::{CopyFileOptions, FileVersionId};
///
/// let version_id = FileVersionId::new("67890");
/// let options = CopyFileOptions::default()
///     .with_name("Contract (copy).pdf")
///     .with_version(&version_id);
/// assert_eq!(options.name, Some("Contract (copy).pdf"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CopyFileOptions<'a> {
    /// コピーしたファイルの名前
    ///
    /// `None`の場合、BOXはコピー元のファイルと同じ名前を付ける。
    pub name: Option<&'a str>,

    /// コピーするバージョンのID
    ///
    /// `None`の場合、BOXはファイルの現在のバージョンをコピーする。
    pub version: Option<&'a FileVersionId>,
}

impl<'a> CopyFileOptions<'a> {
    /// コピーしたファイルの名前を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// コピーするバージョンのIDを設定する。
    #[must_use]
    pub fn with_version(mut self, version: &'a FileVersionId) -> Self {
        self.version = Some(version);
        self
    }
}

/// ファイルコピーリクエストのボディ
#[derive(Debug, Clone, Serialize, Validate)]
struct CopyFileBody<'a> {
    /// コピー先の親フォルダ
    #[garde(skip)]
    parent: ParentFolder,

    /// コピーしたファイルの名前
    ///
    /// `None`の場合は送信せず、BOXはコピー元のファイルと同じ名前を付ける。
    /// `garde`は`None`の場合に`length`を検証しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, min = 1, max = 255), custom(validate_optional_item_name))]
    name: Option<&'a str>,

    /// コピーするバージョンのID
    ///
    /// `None`の場合は送信せず、BOXはファイルの現在のバージョンをコピーする。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    version: Option<&'a FileVersionId>,
}

impl<'a> CopyFileBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// ファイル名をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    fn new(parent_id: &FolderId, options: &CopyFileOptions<'a>) -> BoxClientResult<Self> {
        let body = Self {
            parent: ParentFolder {
                id: parent_id.clone(),
            },
            name: options.name,
            version: options.version,
        };
        body.validate()
            .map_err(|report| BoxClientError::InvalidRequestBody(ValidationError::new(report)))?;
        Ok(body)
    }
}

impl BoxClient {
    /// ファイル情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 情報を取得するファイルのID
    ///
    /// # Errors
    ///
    /// ファイルが存在しない場合、またはファイルにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn file_information(&self, file_id: &FileId) -> BoxClientResult<File> {
        self.send_json(ApiRequest::get(["files", file_id.as_ref()]))
            .await
    }

    /// ファイルを別のフォルダへコピーする。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、BOXがコピーを完了していた場合は409を受け取り、
    /// 名前が異なる場合はファイルを重複してコピーするおそれがあるためである。
    /// したがって、429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: コピーするファイルのID
    /// - `parent_id`: コピー先の親フォルダのID。ルートフォルダへコピーする場合は[`FolderId::root`]を指定する。
    /// - `options`: コピーしたファイルの名前、およびコピーするバージョンのオプション
    ///
    /// # Errors
    ///
    /// `options.name`をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXは使用できない文字を取り除いて受け付けるが、意図しない名前でコピーしないように、クライアントで拒否する。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: パラメーターが不足している、または不正である。
    /// - 403: ファイルをコピーする権限がない、または情報バリアがコピーを禁止している。
    /// - 404: コピー元のファイルまたはコピー先の親フォルダが存在しない、またはアクセスする権限がない。
    /// - 409: 同じ名前の項目がコピー先の親フォルダに存在する（`item_name_in_use`）。
    ///   または、移動、コピーなど、他の操作がコピー元またはコピー先のフォルダをロックしている（`operation_blocked_temporary`）。
    ///   後者の場合、クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルをコピーした可能性がある。
    /// したがって、[`BoxClient::list_items_in_folder`]でコピー先の親フォルダを確認する。
    pub async fn copy_file(
        &self,
        file_id: &FileId,
        parent_id: &FolderId,
        options: &CopyFileOptions<'_>,
    ) -> BoxClientResult<File> {
        let body = CopyFileBody::new(parent_id, options)?;
        let request = ApiRequest::post(["files", file_id.as_ref(), "copy"]).json(&body)?;

        let file: File = self.send_json(request).await?;

        tracing::debug!(
            file_id = file_id.as_ref(),
            parent_id = parent_id.as_ref(),
            copied_file_id = file.id.as_ref(),
            "copied file"
        );

        Ok(file)
    }

    /// ファイル名を変更する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがファイル名の変更を完了している可能性がある。
    /// `options.etag`を指定した場合は、名前の変更によって`ETag`が変わるため、再送すると412を受け取る。
    /// その結果、変更に成功したにもかかわらず、変更に失敗したと誤って判断するおそれがある。
    /// したがって、[`BoxClient::rename_folder`]と同様に、HTTPメソッドがPUTであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 名前を変更するファイルのID
    /// - `name`: 新しいファイル名
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// ファイル名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: ファイルを操作する権限がない。
    /// - 404: ファイルが存在しない、またはファイルにアクセスする権限がない。
    /// - 409: 同じ名前の項目が親フォルダに存在する（`item_name_in_use`）。
    ///   または、他の操作がファイルをロックしている（`operation_blocked_temporary`）。
    ///   後者の場合、クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    /// - 412: `options.etag`がファイルの現在の`ETag`と一致しない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイル名を変更した可能性がある。
    /// したがって、[`BoxClient::file_information`]でファイル名を確認する。
    pub async fn rename_file(
        &self,
        file_id: &FileId,
        name: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<File> {
        let body = RenameBody::new(name)?;
        let mut request = ApiRequest::put(["files", file_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent)
            .json(&body)?;
        if let Some(etag) = options.etag {
            request = request.header(IF_MATCH, etag)?;
        }

        let file: File = self.send_json(request).await?;

        tracing::debug!(file_id = file_id.as_ref(), name = file.name, "renamed file");

        Ok(file)
    }

    /// ファイルの説明を設定する。
    ///
    /// 説明を削除する場合は、`description`に空文字列を指定する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXが説明の設定を完了している可能性がある。
    /// `options.etag`を指定した場合は、説明の設定によって`ETag`が変わるため、再送すると412を受け取る。
    /// その結果、設定に成功したにもかかわらず、設定に失敗したと誤って判断するおそれがある。
    /// したがって、[`BoxClient::set_folder_description`]と同様に、HTTPメソッドがPUTであっても
    /// 冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 説明を設定するファイルのID
    /// - `description`: ファイルの説明。[`MAX_DESCRIPTION_CHARS`](crate::MAX_DESCRIPTION_CHARS)文字以下で指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// 説明が[`MAX_DESCRIPTION_CHARS`](crate::MAX_DESCRIPTION_CHARS)文字より長い場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: ファイルを操作する権限がない。
    /// - 404: ファイルが存在しない、またはファイルにアクセスする権限がない。
    /// - 412: `options.etag`がファイルの現在の`ETag`と一致しない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXが説明を設定した可能性がある。
    /// したがって、[`BoxClient::file_information`]でファイルの説明を確認する。
    pub async fn set_file_description(
        &self,
        file_id: &FileId,
        description: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<File> {
        let body = SetDescriptionBody::new(description)?;
        let mut request = ApiRequest::put(["files", file_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent)
            .json(&body)?;
        if let Some(etag) = options.etag {
            request = request.header(IF_MATCH, etag)?;
        }

        let file = self.send_json(request).await?;

        tracing::debug!(file_id = file_id.as_ref(), "set file description");

        Ok(file)
    }

    /// ファイルを別のフォルダへ移動する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがファイルの移動を完了している可能性がある。
    /// `options.etag`を指定した場合は、移動によって`ETag`が変わるため、再送すると412を受け取る。
    /// その結果、移動に成功したにもかかわらず、移動に失敗したと誤って判断するおそれがある。
    /// したがって、[`BoxClient::move_folder`]と同様に、HTTPメソッドがPUTであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 移動するファイルのID
    /// - `destination`: 移動先。フォルダID、またはルートフォルダへ移動するユーザーのIDで指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// `destination`のユーザーIDが空文字列の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: 移動先のフォルダにアクセスする権限がない（`access_denied_insufficient_permissions`）。
    /// - 404: ファイルが存在しない、またはファイルにアクセスする権限がない。
    /// - 409: 同じ名前の項目が移動先のフォルダに存在する。
    /// - 412: `options.etag`がファイルの現在の`ETag`と一致しない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルを移動した可能性がある。
    /// したがって、[`BoxClient::file_information`]でファイルの親フォルダを確認する。
    pub async fn move_file(
        &self,
        file_id: &FileId,
        destination: &MoveDestination,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<File> {
        let body = MoveBody::new(destination)?;
        let mut request = ApiRequest::put(["files", file_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent)
            .json(&body)?;
        if let Some(etag) = options.etag {
            request = request.header(IF_MATCH, etag)?;
        }

        let file = self.send_json(request).await?;

        tracing::debug!(file_id = file_id.as_ref(), ?destination, "moved file");

        Ok(file)
    }

    /// ファイルを削除する。
    ///
    /// BOXは、エンタープライズの設定に従って、ファイルをゴミ箱に移動するか、完全に削除する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがファイルの削除を完了している可能性がある。
    /// BOXは削除済みのファイルに対して404を返すため、BOXが削除した後にリクエストを再送すると、
    /// 削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 削除するファイルのID
    /// - `etag`: 最後に取得したファイルの`ETag`。指定した場合は`If-Match`ヘッダーで送信し、
    ///   ファイルが`ETag`の取得後に変更されていると、BOXはファイルを削除せずに412を返す。
    ///
    /// # Errors
    ///
    /// `etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    /// 主なステータスは次のとおり。
    ///
    /// - 404: ファイルが存在しない、ファイルが削除済みである、またはファイルにアクセスする権限がない。
    /// - 412: `etag`がファイルの現在の`ETag`と一致しない。ファイルが`ETag`の取得後に変更されている。
    ///
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルを削除した可能性があるため、
    /// [`BoxClient::file_information`]でファイルの状態を確認する。
    pub async fn delete_file(&self, file_id: &FileId, etag: Option<&str>) -> BoxClientResult<()> {
        let mut request =
            ApiRequest::delete(["files", file_id.as_ref()]).idempotency(Idempotency::NonIdempotent);
        if let Some(etag) = etag {
            request = request.header(IF_MATCH, etag)?;
        }

        self.send_empty(request).await?;

        tracing::debug!(file_id = file_id.as_ref(), "deleted file");

        Ok(())
    }
}

#[cfg(test)]
mod test_file {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;
    use crate::{FileVersionId, FolderId};

    /// 日時をRFC 3339形式で解析し、バージョン、パス、親フォルダおよびユーザーをデシリアライズすることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        let file: File = serde_json::from_value(json!({
            "type": "file",
            "id": "12345",
            "etag": "1",
            "sequence_id": "3",
            "name": "Contract.pdf",
            "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
            "file_version": {
                "type": "file_version",
                "id": "67890",
                "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
            },
            "description": "Contract for the project",
            "size": 629_644,
            "path_collection": {
                "total_count": 2,
                "entries": [
                    {"type": "folder", "id": "0", "sequence_id": null, "etag": null, "name": "All Files"},
                    {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
                ],
            },
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T11:15:04+09:00",
            "trashed_at": null,
            "purged_at": null,
            "content_created_at": "2012-12-12T10:53:43Z",
            "content_modified_at": "2012-12-12T11:15:04Z",
            "created_by": user,
            "modified_by": user,
            "owned_by": user,
            "shared_link": null,
            "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
            "item_status": "active",
        }))?;

        let user = UserMini {
            id: crate::UserId::new("33224"),
            user_type: "user".to_owned(),
            name: Some("Test User".to_owned()),
            login: Some("user@example.com".to_owned()),
        };
        let legal = FolderItem {
            id: FolderId::new("111"),
            name: "Legal".to_owned(),
            etag: Some("0".to_owned()),
        };
        assert_eq!(
            file,
            File {
                id: FileId::new("12345"),
                etag: Some("1".to_owned()),
                sequence_id: Some("3".to_owned()),
                name: "Contract.pdf".to_owned(),
                sha1: Some("85136c79cbf9fe36bb9d05d0639c70c265c18d37".to_owned()),
                file_version: Some(FileVersion {
                    id: FileVersionId::new("67890"),
                    sha1: Some("85136c79cbf9fe36bb9d05d0639c70c265c18d37".to_owned()),
                    name: None,
                    size: None,
                    version_number: None,
                    created_at: None,
                    modified_at: None,
                    modified_by: None,
                    trashed_at: None,
                    trashed_by: None,
                    restored_at: None,
                    restored_by: None,
                    purged_at: None,
                    uploader_display_name: None,
                }),
                description: Some("Contract for the project".to_owned()),
                size: Some(629_644),
                path_collection: Some(PathCollection {
                    total_count: 2,
                    entries: vec![
                        FolderItem {
                            id: FolderId::root(),
                            name: "All Files".to_owned(),
                            etag: None,
                        },
                        legal.clone(),
                    ],
                }),
                created_at: Some(datetime!(2012-12-12 10:53:43 -08:00)),
                modified_at: Some(datetime!(2012-12-12 11:15:04 +09:00)),
                trashed_at: None,
                purged_at: None,
                content_created_at: Some(datetime!(2012-12-12 10:53:43 UTC)),
                content_modified_at: Some(datetime!(2012-12-12 11:15:04 UTC)),
                created_by: Some(user.clone()),
                modified_by: Some(user.clone()),
                owned_by: Some(user),
                parent: Some(legal),
                item_status: Some(ItemStatus::Active),
            }
        );

        Ok(())
    }

    /// IDおよび名前以外の属性が欠落しているファイルをデシリアライズすることを確認
    #[test]
    fn test_deserialize_minimal() -> anyhow::Result<()> {
        let file: File = serde_json::from_value(json!({
            "type": "file",
            "id": "12345",
            "name": "Contract.pdf",
        }))?;

        assert_eq!(file.id, FileId::new("12345"));
        assert_eq!(file.name, "Contract.pdf");
        assert_eq!(file.file_version, None);
        assert_eq!(file.created_at, None);
        assert_eq!(file.parent, None);

        Ok(())
    }

    /// RFC 3339形式でない日時の場合は、デシリアライズに失敗することを確認
    #[test]
    fn test_deserialize_invalid_datetime() {
        let result = serde_json::from_value::<File>(json!({
            "id": "12345",
            "name": "Contract.pdf",
            "modified_at": "2012/12/12 11:15:04",
        }));

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_file_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError, FolderId,
        test_support::{create_client, mount_token, short_retry},
    };

    /// `GET /files/{file_id}`を呼び出し、ファイル情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_file_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file",
                "id": "12345",
                "etag": "1",
                "name": "Contract.pdf",
                "size": 629_644,
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client.file_information(&FileId::new("12345")).await?;

        assert_eq!(file.id, FileId::new("12345"));
        assert_eq!(file.name, "Contract.pdf");
        assert_eq!(file.size, Some(629_644));
        assert_eq!(file.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// ファイルが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_file_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.file_information(&FileId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_copy_file {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, request_count, short_retry};

    /// ファイルのコピーのAPIのパス
    const COPY_PATH: &str = "/2.0/files/12345/copy";

    /// コピーするバージョンのID
    const VERSION_ID: &str = "67890";

    /// コピー先の親フォルダと、指定した場合だけ名前とバージョンを送信し、コピーしたファイルを返すことを確認
    #[rstest]
    #[case::default(None, None, json!({"parent": {"id": "456"}}))]
    #[case::with_name(
        Some("Contract (copy).pdf"),
        None,
        json!({"parent": {"id": "456"}, "name": "Contract (copy).pdf"})
    )]
    #[case::with_version(
        None,
        Some(VERSION_ID),
        json!({"parent": {"id": "456"}, "version": VERSION_ID})
    )]
    #[tokio::test]
    async fn test_copy_file(
        #[case] name: Option<&'static str>,
        #[case] version: Option<&'static str>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(COPY_PATH))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "file",
                "id": "99999",
                "etag": "0",
                "name": name.unwrap_or("Contract.pdf"),
                "parent": {"type": "folder", "id": "456", "etag": "0", "name": "Archive"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let version = version.map(FileVersionId::new);
        let options = CopyFileOptions {
            name,
            version: version.as_ref(),
        };

        let file = client
            .copy_file(&FileId::new("12345"), &FolderId::new("456"), &options)
            .await?;

        assert_eq!(file.id, FileId::new("99999"));
        assert_eq!(file.parent.map(|p| p.id), Some(FolderId::new("456")));

        Ok(())
    }

    /// BOXが拒否する名前を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    ///
    /// 使用できない文字の詳細な検証は、`validate_item_name`のテストで確認する。
    #[rstest]
    #[case::empty("")]
    #[case::too_long(&"あ".repeat(256))]
    #[case::invalid_char("a/b")]
    #[tokio::test]
    async fn test_copy_file_rejects_invalid_name(#[case] name: &str) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .copy_file(
                &FileId::new("12345"),
                &FolderId::new("456"),
                &CopyFileOptions::default().with_name(name),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// コピーは冪等でないため、5xxの後に再送すると、ファイルを重複してコピーするおそれがある。
    #[rstest]
    #[case::name_in_use(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_copy_file_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(COPY_PATH))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .copy_file(
                &FileId::new("12345"),
                &FolderId::new("456"),
                &CopyFileOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_rename_file {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, request_count, short_retry};

    /// ファイルのAPIのパス
    const FILE_PATH: &str = "/2.0/files/12345";

    /// ファイルのAPIへのPUTリクエストのIf-Matchヘッダーを返す。
    async fn rename_requests(server: &MockServer) -> Vec<Option<String>> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "PUT" && r.url.path() == FILE_PATH)
            .map(|r| {
                r.headers
                    .get("if-match")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            })
            .collect()
    }

    /// 新しい名前だけをボディで送信し、オプションに応じてIf-Matchヘッダーを送信することを確認
    #[rstest]
    #[case::default(UpdateItemOptions::default(), None)]
    #[case::etag(UpdateItemOptions { etag: Some("1") }, Some("1"))]
    #[tokio::test]
    async fn test_rename_file(
        #[case] options: UpdateItemOptions<'static>,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FILE_PATH))
            .and(body_json(json!({"name": "Renamed.pdf"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file",
                "id": "12345",
                "etag": "2",
                "name": "Renamed.pdf",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .rename_file(&FileId::new("12345"), "Renamed.pdf", &options)
            .await?;

        assert_eq!(file.name, "Renamed.pdf");
        assert_eq!(
            rename_requests(&server).await,
            vec![expected_if_match.map(str::to_owned)]
        );

        Ok(())
    }

    /// BOXが拒否するファイル名を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    ///
    /// 使用できない文字の詳細な検証は、`validate_item_name`のテストで確認する。
    #[rstest]
    #[case::empty("")]
    #[case::too_long(&"あ".repeat(256))]
    #[case::invalid_char("a/b")]
    #[tokio::test]
    async fn test_rename_file_rejects_invalid_name(#[case] name: &str) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .rename_file(&FileId::new("12345"), name, &UpdateItemOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    #[rstest]
    #[case::name_in_use(409)]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_rename_file_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .rename_file(
                &FileId::new("12345"),
                "Renamed.pdf",
                &UpdateItemOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(rename_requests(&server).await.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_set_file_description {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, request_count, short_retry};

    /// ファイルのAPIのパス
    const FILE_PATH: &str = "/2.0/files/12345";

    /// ファイルのAPIへのPUTリクエストのIf-Matchヘッダーを返す。
    async fn update_requests(server: &MockServer) -> Vec<Option<String>> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "PUT" && r.url.path() == FILE_PATH)
            .map(|r| {
                r.headers
                    .get("if-match")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            })
            .collect()
    }

    /// 説明だけをボディで送信し、オプションに応じてIf-Matchヘッダーを送信することを確認
    ///
    /// 説明を削除する空文字列と、上限の文字数の説明も送信できることを確認する。
    #[rstest]
    #[case::default("Contract for the project", UpdateItemOptions::default(), None)]
    #[case::etag(
        "Contract for the project",
        UpdateItemOptions { etag: Some("1") },
        Some("1")
    )]
    #[case::empty("", UpdateItemOptions::default(), None)]
    #[case::max_japanese(
        &"あ".repeat(crate::MAX_DESCRIPTION_CHARS),
        UpdateItemOptions::default(),
        None
    )]
    #[tokio::test]
    async fn test_set_file_description(
        #[case] description: &str,
        #[case] options: UpdateItemOptions<'static>,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FILE_PATH))
            .and(body_json(json!({"description": description})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file",
                "id": "12345",
                "etag": "2",
                "name": "Contract.pdf",
                "description": description,
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .set_file_description(&FileId::new("12345"), description, &options)
            .await?;

        assert_eq!(file.description.as_deref(), Some(description));
        assert_eq!(
            update_requests(&server).await,
            vec![expected_if_match.map(str::to_owned)]
        );

        Ok(())
    }

    /// 説明が上限の文字数より長い場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_set_file_description_rejects_too_long() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .set_file_description(
                &FileId::new("12345"),
                &"あ".repeat(crate::MAX_DESCRIPTION_CHARS + 1),
                &UpdateItemOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    #[rstest]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_set_file_description_error_is_not_retried(
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .set_file_description(
                &FileId::new("12345"),
                "Contract for the project",
                &UpdateItemOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(update_requests(&server).await.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_move_file {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        FolderId, UserId,
        test_support::{create_client, mount_token, request_count, short_retry},
    };

    /// ファイルのAPIのパス
    const FILE_PATH: &str = "/2.0/files/12345";

    /// ファイルのAPIへのPUTリクエストのIf-Matchヘッダーを返す。
    async fn move_requests(server: &MockServer) -> Vec<Option<String>> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "PUT" && r.url.path() == FILE_PATH)
            .map(|r| {
                r.headers
                    .get("if-match")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            })
            .collect()
    }

    /// 移動先に応じた親フォルダだけをボディで送信し、オプションに応じてIf-Matchヘッダーを送信することを確認
    #[rstest]
    #[case::folder(
        MoveDestination::Folder(FolderId::new("456")),
        UpdateItemOptions::default(),
        json!({"parent": {"id": "456"}}),
        None
    )]
    #[case::folder_with_etag(
        MoveDestination::Folder(FolderId::new("456")),
        UpdateItemOptions { etag: Some("1") },
        json!({"parent": {"id": "456"}}),
        Some("1")
    )]
    #[case::user_root(
        MoveDestination::User(UserId::new("789")),
        UpdateItemOptions::default(),
        json!({"parent": {"id": "0", "user_id": "789"}}),
        None
    )]
    #[tokio::test]
    async fn test_move_file(
        #[case] destination: MoveDestination,
        #[case] options: UpdateItemOptions<'static>,
        #[case] expected_body: serde_json::Value,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FILE_PATH))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file",
                "id": "12345",
                "etag": "2",
                "name": "Contract.pdf",
                "parent": {"type": "folder", "id": "456", "etag": "0", "name": "Archive"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .move_file(&FileId::new("12345"), &destination, &options)
            .await?;

        assert_eq!(file.id, FileId::new("12345"));
        assert_eq!(
            move_requests(&server).await,
            vec![expected_if_match.map(str::to_owned)]
        );

        Ok(())
    }

    /// ユーザーIDが空文字列の場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_move_file_rejects_empty_user_id() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .move_file(
                &FileId::new("12345"),
                &MoveDestination::User(UserId::new("")),
                &UpdateItemOptions::default(),
            )
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    #[rstest]
    #[case::forbidden(403)]
    #[case::name_in_use(409)]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[tokio::test]
    async fn test_move_file_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .move_file(
                &FileId::new("12345"),
                &MoveDestination::Folder(FolderId::new("456")),
                &UpdateItemOptions::default(),
            )
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(move_requests(&server).await.len(), 1);

        Ok(())
    }
}

#[cfg(test)]
mod test_delete_file {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError,
        test_support::{create_client, mount_token, request_count, short_retry},
    };

    /// ファイルのAPIのパス
    const FILE_PATH: &str = "/2.0/files/123";

    /// ファイルのAPIへのDELETEリクエストのIf-Matchヘッダーを返す。
    async fn delete_requests(server: &MockServer) -> Vec<Option<String>> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "DELETE" && r.url.path() == FILE_PATH)
            .map(|r| {
                r.headers
                    .get("if-match")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned)
            })
            .collect()
    }

    /// `ETag`を指定した場合だけ、If-Matchヘッダーを送信することを確認
    #[rstest]
    #[case::without_etag(None)]
    #[case::with_etag(Some("1"))]
    #[tokio::test]
    async fn test_delete_file(#[case] etag: Option<&str>) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client.delete_file(&FileId::new("123"), etag).await?;

        assert_eq!(
            delete_requests(&server).await,
            vec![etag.map(str::to_owned)]
        );

        Ok(())
    }

    /// `ETag`に使用できない文字が含まれる場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_delete_file_invalid_etag() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .delete_file(&FileId::new("123"), Some("1\r\nX-Injected: 1"))
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidHeaderValue(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのファイルに対して404を受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::not_found(404)]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_file_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client.delete_file(&FileId::new("123"), None).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(delete_requests(&server).await.len(), 1);

        Ok(())
    }

    /// 429の場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[tokio::test]
    async fn test_delete_file_retries_too_many_requests() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client.delete_file(&FileId::new("123"), None).await?;

        assert_eq!(delete_requests(&server).await.len(), 2);

        Ok(())
    }
}
