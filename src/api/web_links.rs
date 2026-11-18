//! Webリンクに関するAPI
//!
//! BOXのドキュメントは、Webリンクの更新（`PUT /web_links/{web_link_id}`）が`If-Match`ヘッダーを受け付けることを明記していない。
//! しかし、Webリンクの`ETag`は`If-Match`ヘッダーで使用すると記載しているため、
//! Webリンクを更新するメソッドは、ファイルおよびフォルダと同じく、`options.etag`を指定した場合に`If-Match`ヘッダーで送信する。

use garde::Validate;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, FolderId, FolderItem, Idempotency, ItemStatus,
    MAX_DESCRIPTION_CHARS, MoveDestination, PathCollection, UserMini, WebLinkId,
    api::types::{
        MAX_NAME_CHARS, MoveBody, MoveParent, ParentFolder, RenameBody, SetDescriptionBody,
        UpdateItemOptions, update_item_request, validate_optional_item_name, validated,
    },
    api::updates::{
        Nullable, NullableBody, SharedLinkBody, SharedLinkSettings, validate_common_attributes,
    },
    request::ApiRequest,
};

/// Webリンク情報
///
/// フォルダ情報およびファイル情報と同様に、BOXがnullを返したり省略したりしても取得を失敗させないため、
/// IDおよび名前以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct WebLink {
    /// WebリンクID
    pub id: WebLinkId,

    /// Webリンクのバージョンを識別する`ETag`
    pub etag: Option<String>,

    /// Webリンクの変更を識別するシーケンスID
    pub sequence_id: Option<String>,

    /// Webリンク名
    pub name: String,

    /// Webリンクが指すURL
    pub url: Option<String>,

    /// Webリンクの説明
    pub description: Option<String>,

    /// ルートフォルダからこのWebリンクの親フォルダまでのパス
    pub path_collection: Option<PathCollection>,

    /// BOX上でWebリンクを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// BOX上でWebリンクを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// Webリンクをゴミ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// ゴミ箱に移動したWebリンクを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// Webリンクを作成したユーザー
    pub created_by: Option<UserMini>,

    /// Webリンクを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// Webリンクを所有するユーザー
    pub owned_by: Option<UserMini>,

    /// 親フォルダ
    pub parent: Option<FolderItem>,

    /// Webリンクの状態
    pub item_status: Option<ItemStatus>,
}

/// Webリンクの作成のオプション
///
/// # Examples
///
/// ```
/// use box_client::CreateWebLinkOptions;
///
/// let options = CreateWebLinkOptions::default()
///     .with_name("Example")
///     .with_description("Example site");
/// assert_eq!(options.name, Some("Example"));
/// assert_eq!(options.description, Some("Example site"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreateWebLinkOptions<'a> {
    /// Webリンク名
    ///
    /// `None`の場合、BOXはURLをWebリンク名にする。
    pub name: Option<&'a str>,

    /// Webリンクの説明
    ///
    /// [`MAX_DESCRIPTION_CHARS`]文字以下で指定する。
    pub description: Option<&'a str>,
}

impl<'a> CreateWebLinkOptions<'a> {
    /// Webリンク名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// Webリンクの説明を設定する。
    #[must_use]
    pub fn with_description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }
}

/// Webリンク作成リクエストのボディ
#[derive(Debug, Clone, Serialize, Validate)]
struct CreateWebLinkBody<'a> {
    /// WebリンクのURL
    ///
    /// `url`ルールでURLを解釈できることを検証する。
    /// `url`ルールはすべてのスキームを受け付けるが、BOXは`http://`または`https://`で始まるURLだけを受け付けるため、
    /// `pattern`ルールでスキームも検証する。URLのスキームは大文字と小文字を区別しないため、大文字と小文字を区別せずに検証する。
    #[garde(url, pattern(r"(?i-u)^https?://"))]
    url: &'a str,

    /// Webリンクを作成する親フォルダ
    #[garde(skip)]
    parent: ParentFolder<'a>,

    /// Webリンク名
    ///
    /// `None`の場合は送信せず、BOXはURLをWebリンク名にする。
    /// `garde`は`None`の場合に`length`を検証しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, min = 1, max = MAX_NAME_CHARS), custom(validate_optional_item_name))]
    name: Option<&'a str>,

    /// Webリンクの説明
    ///
    /// `None`の場合は送信しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, max = MAX_DESCRIPTION_CHARS))]
    description: Option<&'a str>,
}

impl<'a> CreateWebLinkBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// URL、Webリンク名または説明をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    fn new(
        url: &'a str,
        parent_id: &'a FolderId,
        options: &CreateWebLinkOptions<'a>,
    ) -> BoxClientResult<Self> {
        validated(Self {
            url,
            parent: ParentFolder::new(parent_id),
            name: options.name,
            description: options.description,
        })
    }
}

/// WebリンクのURL変更リクエストのボディ
#[derive(Debug, Clone, Serialize, Validate)]
struct UpdateWebLinkUrlBody<'a> {
    /// 新しいURL
    ///
    /// [`CreateWebLinkBody::url`]と同じ規則で検証する。
    #[garde(url, pattern(r"(?i-u)^https?://"))]
    url: &'a str,
}

impl<'a> UpdateWebLinkUrlBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// URLを解釈できない場合、またはURLが`http://`もしくは`https://`で始まらない場合は、
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    fn new(url: &'a str) -> BoxClientResult<Self> {
        validated(Self { url })
    }
}

/// Webリンクの更新で変更する属性
///
/// `None`の属性は送信せず、BOXは変更しない。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// # Examples
///
/// ```
/// use box_client::WebLinkUpdate;
///
/// let update = WebLinkUpdate::default()
///     .with_url("https://example.com/new")
///     .with_name("Example");
/// assert_eq!(update.url, Some("https://example.com/new"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebLinkUpdate<'a> {
    /// 新しいURL
    ///
    /// `http://`または`https://`で始まるURLを指定する。
    pub url: Option<&'a str>,

    /// 新しいWebリンク名
    pub name: Option<&'a str>,

    /// Webリンクの説明
    ///
    /// [`MAX_DESCRIPTION_CHARS`]文字以下で指定する。空文字列を指定すると、説明を削除する。
    pub description: Option<&'a str>,

    /// 移動先
    pub parent: Option<&'a MoveDestination>,

    /// 共有リンク
    ///
    /// [`Nullable::Value`]を指定すると、共有リンクを作成または更新する。
    /// Webリンクの共有リンクでは、[`SharedLinkSettings::can_download`]を指定できない。
    /// BOXのドキュメントは、Webリンクの更新で共有リンクにnullを指定した場合の動作を記載していないが、
    /// ファイルおよびフォルダと同じく、[`Nullable::Null`]を指定するとnullを送信する。
    pub shared_link: Option<Nullable<SharedLinkSettings<'a>>>,
}

impl<'a> WebLinkUpdate<'a> {
    /// 新しいURLを設定する。
    #[must_use]
    pub fn with_url(mut self, url: &'a str) -> Self {
        self.url = Some(url);
        self
    }

    /// 新しいWebリンク名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// Webリンクの説明を設定する。
    #[must_use]
    pub fn with_description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    /// 移動先を設定する。
    #[must_use]
    pub fn with_parent(mut self, parent: &'a MoveDestination) -> Self {
        self.parent = Some(parent);
        self
    }

    /// 共有リンクを作成または更新する。
    #[must_use]
    pub fn with_shared_link(mut self, shared_link: SharedLinkSettings<'a>) -> Self {
        self.shared_link = Some(Nullable::Value(shared_link));
        self
    }

    /// 共有リンクを削除する。
    #[must_use]
    pub fn with_shared_link_removed(mut self) -> Self {
        self.shared_link = Some(Nullable::Null);
        self
    }
}

/// Webリンクの更新リクエストのボディ
///
/// 共有リンクのパスワードを含みうるため、`Debug`を実装しない。
#[derive(Serialize)]
struct UpdateWebLinkBody<'a> {
    /// 新しいURL
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,

    /// 新しいWebリンク名
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,

    /// Webリンクの説明
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,

    /// 移動先の親フォルダ
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<MoveParent<'a>>,

    /// 共有リンク
    #[serde(skip_serializing_if = "Option::is_none")]
    shared_link: Option<NullableBody<SharedLinkBody<'a>>>,
}

impl<'a> UpdateWebLinkBody<'a> {
    /// 変更する属性からボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// 移動先のユーザーIDが空文字列の場合、または共有リンクで`can_download`を指定した場合は、
    /// [`BoxClientError::InvalidArgument`]を返す。
    /// URLまたはWebリンク名をBOXが拒否する場合、または説明が長すぎる場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    fn new(update: &WebLinkUpdate<'a>) -> BoxClientResult<Self> {
        validate_common_attributes(update.name, update.description, None)?;
        if let Some(url) = update.url {
            UpdateWebLinkUrlBody::new(url)?;
        }
        if let Some(Nullable::Value(shared_link)) = update.shared_link
            && shared_link.can_download.is_some()
        {
            return Err(BoxClientError::InvalidArgument(
                "can_download cannot be set on the shared link of a web link",
            ));
        }

        Ok(Self {
            url: update.url,
            name: update.name,
            description: update.description,
            parent: update.parent.map(MoveParent::new).transpose()?,
            shared_link: update
                .shared_link
                .map(|shared_link| shared_link.to_body(SharedLinkBody::new)),
        })
    }
}

impl BoxClient {
    /// Webリンク情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 情報を取得するWebリンクのID
    ///
    /// # Errors
    ///
    /// Webリンクが存在しない場合、またはWebリンクにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn web_link_information(&self, web_link_id: &WebLinkId) -> BoxClientResult<WebLink> {
        self.send_json(ApiRequest::get(["web_links", web_link_id.as_ref()]))
            .await
    }

    /// Webリンクを作成する。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、同じWebリンクを重複して作成するおそれがあるためである。
    ///
    /// # Arguments
    ///
    /// - `url`: WebリンクのURL。`http://`または`https://`で始まるURLを指定する。
    /// - `parent_id`: Webリンクを作成する親フォルダのID
    /// - `options`: Webリンク名および説明のオプション
    ///
    /// # Errors
    ///
    /// URLを解釈できない場合、URLが`http://`または`https://`で始まらない場合、Webリンク名をBOXが拒否する場合、
    /// または説明が[`MAX_DESCRIPTION_CHARS`]文字より長い場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンクを作成した可能性がある。
    /// したがって、[`BoxClient::list_items_in_folder`]で親フォルダを確認する。
    pub async fn create_web_link(
        &self,
        url: &str,
        parent_id: &FolderId,
        options: &CreateWebLinkOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let body = CreateWebLinkBody::new(url, parent_id, options)?;
        let request = ApiRequest::post(["web_links"]).json(&body)?;

        let web_link: WebLink = self.send_json(request).await?;

        tracing::debug!(
            web_link_id = web_link.id.as_ref(),
            parent_id = parent_id.as_ref(),
            "created web link"
        );

        Ok(web_link)
    }

    /// Webリンクの情報を更新する。
    ///
    /// BOXのWebリンクの更新API（`PUT /web_links/{web_link_id}`）が受け付けるすべての属性を、`update`で指定できる。
    /// `update`で指定した属性だけを送信し、BOXは指定しない属性を変更しない。
    /// 名前の変更、説明の設定、移動またはURLの変更だけを行う場合は、[`BoxClient::rename_web_link`]、
    /// [`BoxClient::set_web_link_description`]、[`BoxClient::move_web_link`]または
    /// [`BoxClient::update_web_link_url`]も使用できる。
    ///
    /// [`BoxClient::rename_web_link`]と同様に、HTTPメソッドがPUTであっても冪等でないリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 更新するWebリンクのID
    /// - `update`: 変更する属性
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// `update.parent`のユーザーIDが空文字列の場合、または共有リンクで[`SharedLinkSettings::can_download`]を指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// URLを解釈できない場合、URLが`http://`もしくは`https://`で始まらない場合、Webリンク名をBOXが拒否する場合、
    /// または説明が[`MAX_DESCRIPTION_CHARS`]文字より長い場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンクを更新した可能性がある。
    /// したがって、[`BoxClient::web_link_information`]でWebリンクの状態を確認する。
    pub async fn update_web_link(
        &self,
        web_link_id: &WebLinkId,
        update: &WebLinkUpdate<'_>,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let body = UpdateWebLinkBody::new(update)?;
        let request = update_item_request("web_links", web_link_id.as_ref(), &body, options.etag)?;

        let web_link = self.send_json(request).await?;

        tracing::debug!(web_link_id = web_link_id.as_ref(), "updated web link");

        Ok(web_link)
    }

    /// Webリンク名を変更する。
    ///
    /// [`BoxClient::rename_file`]と同様に、`options.etag`を指定した場合に再送すると412を受け取り、
    /// 変更の結果を誤って判断するおそれがあるため、HTTPメソッドがPUTであっても冪等でないリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 名前を変更するWebリンクのID
    /// - `name`: 新しいWebリンク名
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// Webリンク名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンク名を変更した可能性がある。
    /// したがって、[`BoxClient::web_link_information`]でWebリンク名を確認する。
    pub async fn rename_web_link(
        &self,
        web_link_id: &WebLinkId,
        name: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let body = RenameBody::new(name)?;
        let request = update_item_request("web_links", web_link_id.as_ref(), &body, options.etag)?;

        let web_link: WebLink = self.send_json(request).await?;

        tracing::debug!(
            web_link_id = web_link_id.as_ref(),
            name = web_link.name,
            "renamed web link"
        );

        Ok(web_link)
    }

    /// Webリンクの説明を設定する。
    ///
    /// 説明を削除する場合は、`description`に空文字列を指定する。
    ///
    /// [`BoxClient::set_file_description`]と同様に、HTTPメソッドがPUTであっても冪等でないリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 説明を設定するWebリンクのID
    /// - `description`: Webリンクの説明。[`MAX_DESCRIPTION_CHARS`]文字以下で指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// 説明が[`MAX_DESCRIPTION_CHARS`]文字より長い場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXが説明を設定した可能性がある。
    /// したがって、[`BoxClient::web_link_information`]でWebリンクの説明を確認する。
    pub async fn set_web_link_description(
        &self,
        web_link_id: &WebLinkId,
        description: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let body = SetDescriptionBody::new(description)?;
        let request = update_item_request("web_links", web_link_id.as_ref(), &body, options.etag)?;

        let web_link = self.send_json(request).await?;

        tracing::debug!(
            web_link_id = web_link_id.as_ref(),
            "set web link description"
        );

        Ok(web_link)
    }

    /// Webリンクを別のフォルダへ移動する。
    ///
    /// [`BoxClient::move_file`]と同様に、HTTPメソッドがPUTであっても冪等でないリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 移動するWebリンクのID
    /// - `destination`: 移動先。フォルダID、またはルートフォルダへ移動するユーザーのIDで指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// `destination`のユーザーIDが空文字列の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンクを移動した可能性がある。
    /// したがって、[`BoxClient::web_link_information`]でWebリンクの親フォルダを確認する。
    pub async fn move_web_link(
        &self,
        web_link_id: &WebLinkId,
        destination: &MoveDestination,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let body = MoveBody::new(destination)?;
        let request = update_item_request("web_links", web_link_id.as_ref(), &body, options.etag)?;

        let web_link = self.send_json(request).await?;

        tracing::debug!(
            web_link_id = web_link_id.as_ref(),
            ?destination,
            "moved web link"
        );

        Ok(web_link)
    }

    /// WebリンクのURLを変更する。
    ///
    /// [`BoxClient::rename_web_link`]と同様に、HTTPメソッドがPUTであっても冪等でないリクエストとして送信する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: URLを変更するWebリンクのID
    /// - `url`: 新しいURL。`http://`または`https://`で始まるURLを指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// URLを解釈できない場合、またはURLが`http://`もしくは`https://`で始まらない場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがURLを変更した可能性がある。
    /// したがって、[`BoxClient::web_link_information`]でWebリンクのURLを確認する。
    pub async fn update_web_link_url(
        &self,
        web_link_id: &WebLinkId,
        url: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let body = UpdateWebLinkUrlBody::new(url)?;
        let request = update_item_request("web_links", web_link_id.as_ref(), &body, options.etag)?;

        let web_link = self.send_json(request).await?;

        tracing::debug!(web_link_id = web_link_id.as_ref(), "updated web link url");

        Ok(web_link)
    }

    /// Webリンクを削除する。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがWebリンクの削除を完了している可能性がある。
    /// BOXが削除した後にリクエストを再送すると、削除済みのWebリンクに対する削除となるため、BOXはエラーを返しうる。
    /// その結果、削除に成功したにもかかわらず、削除に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// BOXがリクエストを処理していないことが明らかな429、エラーコードが`operation_blocked_temporary`の409
    /// および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 削除するWebリンクのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。
    /// 公式ドキュメントは、このAPIが返すエラーのステータスを個別に記載していない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンクを削除した可能性があるため、
    /// [`BoxClient::web_link_information`]でWebリンクの状態を確認する。
    pub async fn delete_web_link(&self, web_link_id: &WebLinkId) -> BoxClientResult<()> {
        let request = ApiRequest::delete(["web_links", web_link_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);

        self.send_empty(request).await?;

        tracing::debug!(web_link_id = web_link_id.as_ref(), "deleted web link");

        Ok(())
    }
}

#[cfg(test)]
mod test_web_link {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;
    use crate::FolderId;

    /// 日時をRFC 3339形式で解析し、URL、パス、親フォルダおよびユーザーをデシリアライズすることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        let web_link: WebLink = serde_json::from_value(json!({
            "type": "web_link",
            "id": "12345",
            "etag": "1",
            "sequence_id": "3",
            "name": "Example",
            "url": "https://www.example.com/?a=1&b=2",
            "description": "Example web site",
            "path_collection": {
                "total_count": 2,
                "entries": [
                    {"type": "folder", "id": "0", "sequence_id": null, "etag": null, "name": "All Files"},
                    {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Links"},
                ],
            },
            "created_at": "2012-12-12T10:53:43-08:00",
            "modified_at": "2012-12-12T11:15:04+09:00",
            "trashed_at": null,
            "purged_at": null,
            "created_by": user,
            "modified_by": user,
            "owned_by": user,
            "shared_link": null,
            "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Links"},
            "item_status": "active",
        }))?;

        let user = UserMini {
            id: crate::UserId::new("33224"),
            user_type: "user".to_owned(),
            name: Some("Test User".to_owned()),
            login: Some("user@example.com".to_owned()),
        };
        let links = FolderItem {
            id: FolderId::new("111"),
            name: "Links".to_owned(),
            etag: Some("0".to_owned()),
        };
        assert_eq!(
            web_link,
            WebLink {
                id: WebLinkId::new("12345"),
                etag: Some("1".to_owned()),
                sequence_id: Some("3".to_owned()),
                name: "Example".to_owned(),
                url: Some("https://www.example.com/?a=1&b=2".to_owned()),
                description: Some("Example web site".to_owned()),
                path_collection: Some(PathCollection {
                    total_count: 2,
                    entries: vec![
                        FolderItem {
                            id: FolderId::root(),
                            name: "All Files".to_owned(),
                            etag: None,
                        },
                        links.clone(),
                    ],
                }),
                created_at: Some(datetime!(2012-12-12 10:53:43 -08:00)),
                modified_at: Some(datetime!(2012-12-12 11:15:04 +09:00)),
                trashed_at: None,
                purged_at: None,
                created_by: Some(user.clone()),
                modified_by: Some(user.clone()),
                owned_by: Some(user),
                parent: Some(links),
                item_status: Some(ItemStatus::Active),
            }
        );

        Ok(())
    }

    /// IDおよび名前以外の属性が欠落しているWebリンクをデシリアライズすることを確認
    #[test]
    fn test_deserialize_minimal() -> anyhow::Result<()> {
        let web_link: WebLink = serde_json::from_value(json!({
            "type": "web_link",
            "id": "12345",
            "name": "Example",
        }))?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(web_link.name, "Example");
        assert_eq!(web_link.url, None);
        assert_eq!(web_link.created_at, None);
        assert_eq!(web_link.parent, None);

        Ok(())
    }

    /// RFC 3339形式でない日時の場合は、デシリアライズに失敗することを確認
    #[test]
    fn test_deserialize_invalid_datetime() {
        let result = serde_json::from_value::<WebLink>(json!({
            "id": "12345",
            "name": "Example",
            "created_at": "2012/12/12 10:53:43",
        }));

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_web_link_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError, FolderId,
        test_support::{create_client, mount_token, not_found_response, short_retry},
    };

    /// `GET /web_links/{web_link_id}`を呼び出し、Webリンク情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_web_link_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/web_links/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "web_link",
                "id": "12345",
                "etag": "1",
                "name": "Example",
                "url": "https://www.example.com/",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let web_link = client
            .web_link_information(&WebLinkId::new("12345"))
            .await?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(web_link.name, "Example");
        assert_eq!(web_link.url.as_deref(), Some("https://www.example.com/"));
        assert_eq!(web_link.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// Webリンクが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_web_link_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/web_links/999"))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.web_link_information(&WebLinkId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_create_web_link {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, create_client, mount_token, request_count, short_retry,
    };

    /// WebリンクのAPIのパス
    const WEB_LINKS_PATH: &str = "/2.0/web_links";

    /// URLと親フォルダ、および指定した場合だけ名前と説明を送信し、作成したWebリンクを返すことを確認
    #[rstest]
    #[case::minimal(
        CreateWebLinkOptions::default(),
        json!({"url": "https://example.com", "parent": {"id": "456"}})
    )]
    #[case::name_and_description(
        CreateWebLinkOptions::default().with_name("Example").with_description("Example site"),
        json!({
            "url": "https://example.com",
            "parent": {"id": "456"},
            "name": "Example",
            "description": "Example site",
        })
    )]
    #[tokio::test]
    async fn test_create_web_link(
        #[case] options: CreateWebLinkOptions<'static>,
        #[case] expected_body: serde_json::Value,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(WEB_LINKS_PATH))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "web_link",
                "id": "12345",
                "name": "Example",
                "url": "https://example.com",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let web_link = client
            .create_web_link("https://example.com", &FolderId::new("456"), &options)
            .await?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(web_link.url.as_deref(), Some("https://example.com"));

        Ok(())
    }

    /// BOXが拒否するURL、名前または説明を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::ftp_url("ftp://example.com", None, None)]
    #[case::no_scheme("example.com", None, None)]
    #[case::no_host("https://", None, None)]
    #[case::space_in_host("https://exa mple.com", None, None)]
    #[case::invalid_port("https://example.com:99999", None, None)]
    #[case::empty_name("https://example.com", Some(""), None)]
    #[case::invalid_name("https://example.com", Some("a/b"), None)]
    #[case::too_long_description(
        "https://example.com",
        None,
        Some(&*"あ".repeat(MAX_DESCRIPTION_CHARS + 1))
    )]
    #[tokio::test]
    async fn test_create_web_link_rejects_without_request(
        #[case] url: &str,
        #[case] name: Option<&str>,
        #[case] description: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let options = CreateWebLinkOptions { name, description };

        let result = client
            .create_web_link(url, &FolderId::new("456"), &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 作成は冪等でないため、5xxを再試行せずにそのまま返すことを確認
    #[tokio::test]
    async fn test_create_web_link_error_is_not_retried() -> anyhow::Result<()> {
        assert_not_retried("POST", WEB_LINKS_PATH, 500, async |client: &BoxClient| {
            client
                .create_web_link(
                    "https://example.com",
                    &FolderId::new("456"),
                    &CreateWebLinkOptions::default(),
                )
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_update_web_link {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        UserId,
        test_support::{
            assert_not_retried, create_client, if_match_headers, mount_token, request_count,
            short_retry,
        },
    };

    /// WebリンクのAPIのパス
    const WEB_LINK_PATH: &str = "/2.0/web_links/12345";

    /// Webリンクを更新するメソッド
    #[derive(Debug, Clone)]
    enum Update {
        /// 名前の変更
        Rename(&'static str),

        /// 説明の設定
        Description(String),

        /// 移動
        Move(MoveDestination),

        /// URLの変更
        Url(&'static str),
    }

    impl Update {
        /// 更新するメソッドを呼び出す。
        async fn call(
            &self,
            client: &BoxClient,
            options: &UpdateItemOptions<'_>,
        ) -> BoxClientResult<WebLink> {
            let id = WebLinkId::new("12345");
            match self {
                Self::Rename(name) => client.rename_web_link(&id, name, options).await,
                Self::Description(description) => {
                    client
                        .set_web_link_description(&id, description, options)
                        .await
                }
                Self::Move(destination) => client.move_web_link(&id, destination, options).await,
                Self::Url(url) => client.update_web_link_url(&id, url, options).await,
            }
        }
    }

    /// 更新する属性だけをボディで送信し、`ETag`を指定した場合だけIf-Matchヘッダーを送信することを確認
    #[rstest]
    #[case::rename(Update::Rename("Example"), json!({"name": "Example"}), None)]
    #[case::rename_with_etag(Update::Rename("Example"), json!({"name": "Example"}), Some("1"))]
    #[case::description(
        Update::Description("Example site".to_owned()),
        json!({"description": "Example site"}),
        None
    )]
    #[case::clear_description(Update::Description(String::new()), json!({"description": ""}), None)]
    #[case::move_to_folder(
        Update::Move(MoveDestination::Folder(FolderId::new("456"))),
        json!({"parent": {"id": "456"}}),
        None
    )]
    #[case::move_to_user_root(
        Update::Move(MoveDestination::User(UserId::new("789"))),
        json!({"parent": {"id": "0", "user_id": "789"}}),
        None
    )]
    #[case::url(Update::Url("http://example.org"), json!({"url": "http://example.org"}), Some("1"))]
    #[case::uppercase_scheme(
        Update::Url("HTTPS://example.org"),
        json!({"url": "HTTPS://example.org"}),
        None
    )]
    #[tokio::test]
    async fn test_update_web_link(
        #[case] update: Update,
        #[case] expected_body: serde_json::Value,
        #[case] etag: Option<&'static str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(WEB_LINK_PATH))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "web_link",
                "id": "12345",
                "etag": "2",
                "name": "Example",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UpdateItemOptions { etag };

        let web_link = update.call(&client, &options).await?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(
            if_match_headers(&server, "PUT", WEB_LINK_PATH).await,
            vec![etag.map(str::to_owned)]
        );

        Ok(())
    }

    /// BOXが拒否する値を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::empty_name(Update::Rename(""))]
    #[case::invalid_name(Update::Rename("a/b"))]
    #[case::too_long_description(Update::Description("あ".repeat(MAX_DESCRIPTION_CHARS + 1)))]
    #[case::empty_user_id(Update::Move(MoveDestination::User(UserId::new(""))))]
    #[case::invalid_url(Update::Url("ftp://example.com"))]
    #[case::unparsable_url(Update::Url("https://exa mple.com"))]
    #[tokio::test]
    async fn test_update_web_link_rejects_without_request(
        #[case] update: Update,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = update.call(&client, &UpdateItemOptions::default()).await;

        assert!(
            matches!(
                result,
                Err(BoxClientError::InvalidRequestBody(_) | BoxClientError::InvalidArgument(_))
            ),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 更新は冪等でないため、BOXのエラーレスポンスを再試行せずにそのまま返すことを確認
    #[rstest]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[tokio::test]
    async fn test_update_web_link_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("PUT", WEB_LINK_PATH, status, async |client: &BoxClient| {
            Update::Rename("Example")
                .call(client, &UpdateItemOptions::default())
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_delete_web_link {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_retried_after, count_requests, create_client, mount_token,
        operation_blocked_response, short_retry,
    };

    /// WebリンクのAPIのパス
    const WEB_LINK_PATH: &str = "/2.0/web_links/123";

    /// `DELETE /web_links/{web_link_id}`を呼び出し、204を受け取ると成功することを確認
    #[tokio::test]
    async fn test_delete_web_link() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(WEB_LINK_PATH))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client.delete_web_link(&WebLinkId::new("123")).await?;

        assert_eq!(count_requests(&server, "DELETE", WEB_LINK_PATH).await, 1);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、削除済みのWebリンクに対してエラーを受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_delete_web_link_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried(
            "DELETE",
            WEB_LINK_PATH,
            status,
            async |client: &BoxClient| client.delete_web_link(&WebLinkId::new("123")).await,
        )
        .await
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(operation_blocked_response())]
    #[tokio::test]
    async fn test_delete_web_link_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        assert_retried_after(
            "DELETE",
            WEB_LINK_PATH,
            first_response,
            ResponseTemplate::new(204),
            async |client: &BoxClient| client.delete_web_link(&WebLinkId::new("123")).await,
        )
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod test_update_web_link_attributes {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        SharedLinkAccess,
        test_support::{
            assert_not_retried, create_client, is_invalid_argument, is_invalid_request_body,
            mount_token, request_count, short_retry,
        },
    };

    /// WebリンクのAPIのパス
    const WEB_LINK_PATH: &str = "/2.0/web_links/123";

    /// 期待するエラーであるかを判定する関数
    type IsExpectedError = fn(&BoxClientError) -> bool;

    /// 指定したすべての属性を送信することを確認
    #[tokio::test]
    async fn test_sends_all_attributes() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(WEB_LINK_PATH))
            .and(body_json(json!({
                "url": "https://example.com/new",
                "name": "Example",
                "description": "Example site",
                "parent": {"id": "456"},
                "shared_link": {"access": "open", "password": "Secret-Passw0rd", "vanity_name": "example-vanity"},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "web_link",
                "id": "123",
                "name": "Example",
                "url": "https://example.com/new",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let destination = MoveDestination::Folder(FolderId::new("456"));
        let update = WebLinkUpdate::default()
            .with_url("https://example.com/new")
            .with_name("Example")
            .with_description("Example site")
            .with_parent(&destination)
            .with_shared_link(
                SharedLinkSettings::default()
                    .with_access(SharedLinkAccess::Open)
                    .with_password("Secret-Passw0rd")
                    .with_vanity_name("example-vanity"),
            );

        let web_link = client
            .update_web_link(
                &WebLinkId::new("123"),
                &update,
                &UpdateItemOptions::default(),
            )
            .await?;

        assert_eq!(web_link.url.as_deref(), Some("https://example.com/new"));

        Ok(())
    }

    /// BOXが拒否する更新は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_rejects_invalid_update() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let web_link_id = WebLinkId::new("123");
        let options = UpdateItemOptions::default();

        let cases: [(WebLinkUpdate<'_>, IsExpectedError); 2] = [
            (
                WebLinkUpdate::default()
                    .with_shared_link(SharedLinkSettings::default().with_can_download(true)),
                is_invalid_argument,
            ),
            (
                WebLinkUpdate::default().with_url("ftp://example.com"),
                is_invalid_request_body,
            ),
        ];
        for (update, is_expected) in cases {
            let result = client
                .update_web_link(&web_link_id, &update, &options)
                .await;

            assert!(
                result.as_ref().is_err_and(is_expected),
                "update: {update:?}, result: {result:?}"
            );
        }
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 5xxを受け取った場合は、再試行せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_server_error_is_not_retried() -> anyhow::Result<()> {
        assert_not_retried("PUT", WEB_LINK_PATH, 500, async |client: &BoxClient| {
            client
                .update_web_link(
                    &WebLinkId::new("123"),
                    &WebLinkUpdate::default().with_shared_link_removed(),
                    &UpdateItemOptions::default(),
                )
                .await
        })
        .await
    }
}
