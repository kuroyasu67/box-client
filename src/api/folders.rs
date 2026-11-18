//! フォルダに関するAPI

use garde::Validate;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, BoxItem, FolderId, FolderItem, Idempotency,
    ItemStatus, MoveDestination, UserMini,
    api::pagination::{
        ItemSort, ItemsPage, MAX_ITEMS_LIMIT, SortDirection, collect_all_pages, marker_page_request,
    },
    api::types::{
        MAX_NAME_CHARS, MoveBody, ParentFolder, PathCollection, RenameBody, SetDescriptionBody,
        UpdateItemOptions, ensure_not_root, update_item_request, validate_item_name,
        validate_optional_item_name, validated,
    },
    request::ApiRequest,
};

/// フォルダ情報
///
/// ルートフォルダでは、`ETag`、作成日時、親フォルダなどの多くの属性をBOXがnullで返すため、
/// IDおよび名前以外の属性を`Option`で受け取る。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Folder {
    /// フォルダID
    pub id: FolderId,

    /// フォルダのバージョンを識別する`ETag`
    pub etag: Option<String>,

    /// フォルダの変更を識別するシーケンスID
    pub sequence_id: Option<String>,

    /// フォルダ名
    pub name: String,

    /// フォルダの説明
    pub description: Option<String>,

    /// フォルダ内のすべてのファイルの合計バイト数
    pub size: Option<u64>,

    /// ルートフォルダからこのフォルダの親フォルダまでのパス
    pub path_collection: Option<PathCollection>,

    /// BOX上でフォルダを作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,

    /// BOX上でフォルダを最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,

    /// フォルダをゴミ箱に移動した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub trashed_at: Option<OffsetDateTime>,

    /// ゴミ箱に移動したフォルダを完全に削除する予定の日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub purged_at: Option<OffsetDateTime>,

    /// フォルダの内容を作成した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_created_at: Option<OffsetDateTime>,

    /// フォルダの内容を最後に更新した日時
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub content_modified_at: Option<OffsetDateTime>,

    /// フォルダを作成したユーザー
    pub created_by: Option<UserMini>,

    /// フォルダを最後に更新したユーザー
    pub modified_by: Option<UserMini>,

    /// フォルダを所有するユーザー
    pub owned_by: Option<UserMini>,

    /// 親フォルダ
    ///
    /// ルートフォルダの場合は`None`になる。
    pub parent: Option<FolderItem>,

    /// フォルダの状態
    pub item_status: Option<ItemStatus>,
}

/// フォルダ内の項目をリストするときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// ```
/// use box_client::{ItemSort, ListItemsOptions, SortDirection};
///
/// let options = ListItemsOptions::default()
///     .with_limit(500)
///     .with_sort(ItemSort::Name)
///     .with_direction(SortDirection::Desc);
/// assert_eq!(options.limit, Some(500));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListItemsOptions {
    /// 1ページで取得する項目数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,

    /// 並び替えに使用する属性
    ///
    /// BOXは、ルートフォルダでマーカー方式のページ送りと並び替えに使用する属性を併用すると、
    /// エラーを返さずに属性の指定を無視する。BOXクライアントは常にマーカー方式でページを送るため、
    /// 指定した並び順で項目を受け取ったと利用者が誤解しないように、ルートフォルダでは指定できないようにする。
    pub sort: Option<ItemSort>,

    /// 並び替えの方向
    ///
    /// 並び替えに使用する属性と異なり、BOXはルートフォルダでも並び替えの方向を反映するため、ルートフォルダでも指定できる。
    pub direction: Option<SortDirection>,
}

impl ListItemsOptions {
    /// 1ページで取得する項目数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// 並び替えに使用する属性を設定する。
    ///
    /// ルートフォルダでは指定できない。指定した場合は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_sort(mut self, sort: ItemSort) -> Self {
        self.sort = Some(sort);
        self
    }

    /// 並び替えの方向を設定する。
    #[must_use]
    pub fn with_direction(mut self, direction: SortDirection) -> Self {
        self.direction = Some(direction);
        self
    }
}

/// フォルダ作成リクエストのボディ
#[derive(Debug, Clone, Serialize, Validate)]
struct CreateFolderBody<'a> {
    /// フォルダ名
    ///
    /// BOXは、名前の長さを文字数で数える。
    #[garde(length(chars, min = 1, max = MAX_NAME_CHARS), custom(validate_item_name))]
    name: &'a str,

    /// 親フォルダ
    #[garde(skip)]
    parent: ParentFolder<'a>,
}

impl<'a> CreateFolderBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// フォルダ名をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    fn new(name: &'a str, parent_id: &'a FolderId) -> BoxClientResult<Self> {
        validated(Self {
            name,
            parent: ParentFolder::new(parent_id),
        })
    }
}

/// フォルダのコピーのオプション
///
/// # Examples
///
/// ```
/// use box_client::CopyFolderOptions;
///
/// let options = CopyFolderOptions::default().with_name("Contracts (copy)");
/// assert_eq!(options.name, Some("Contracts (copy)"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CopyFolderOptions<'a> {
    /// コピーしたフォルダの名前
    ///
    /// `None`の場合、BOXはコピー元のフォルダと同じ名前を付ける。
    pub name: Option<&'a str>,
}

impl<'a> CopyFolderOptions<'a> {
    /// コピーしたフォルダの名前を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }
}

/// フォルダコピーリクエストのボディ
#[derive(Debug, Clone, Serialize, Validate)]
struct CopyFolderBody<'a> {
    /// コピー先の親フォルダ
    #[garde(skip)]
    parent: ParentFolder<'a>,

    /// コピーしたフォルダの名前
    ///
    /// `None`の場合は送信せず、BOXはコピー元のフォルダと同じ名前を付ける。
    /// `garde`は`None`の場合に`length`を検証しない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[garde(length(chars, min = 1, max = MAX_NAME_CHARS), custom(validate_optional_item_name))]
    name: Option<&'a str>,
}

impl<'a> CopyFolderBody<'a> {
    /// ボディを作成して検証する。
    ///
    /// # Errors
    ///
    /// フォルダ名をBOXが拒否する場合は、[`BoxClientError::InvalidRequestBody`]を返す。
    fn new(parent_id: &'a FolderId, name: Option<&'a str>) -> BoxClientResult<Self> {
        validated(Self {
            parent: ParentFolder::new(parent_id),
            name,
        })
    }
}

/// フォルダを削除するときのオプション
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// ```
/// use box_client::DeleteFolderOptions;
///
/// let options = DeleteFolderOptions::default()
///     .with_recursive(true)
///     .with_etag("1");
/// assert!(options.recursive);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeleteFolderOptions<'a> {
    /// 空でないフォルダを、フォルダ内のすべての項目とともに削除することを指示するフラグ
    ///
    /// `false`の場合に空でないフォルダを削除しようとすると、BOXは400（`folder_not_empty`）を返す。
    pub recursive: bool,

    /// 最後に取得したフォルダの`ETag`
    ///
    /// 指定した場合は、`If-Match`ヘッダーで送信する。フォルダが`ETag`の取得後に変更されていると、
    /// BOXはフォルダを削除せずに412を返す。
    pub etag: Option<&'a str>,
}

impl<'a> DeleteFolderOptions<'a> {
    /// 空でないフォルダを、フォルダ内のすべての項目とともに削除するかどうかを設定する。
    #[must_use]
    pub fn with_recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    /// 最後に取得したフォルダの`ETag`を設定する。
    #[must_use]
    pub fn with_etag(mut self, etag: &'a str) -> Self {
        self.etag = Some(etag);
        self
    }
}

impl BoxClient {
    /// フォルダ情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 情報を取得するフォルダのID。ルートフォルダの場合は[`FolderId::root`]を指定する。
    ///
    /// # Errors
    ///
    /// フォルダが存在しない場合、またはフォルダにアクセスする権限がない場合は、
    /// [`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn folder_information(&self, folder_id: &FolderId) -> BoxClientResult<Folder> {
        self.send_json(ApiRequest::get(["folders", folder_id.as_ref()]))
            .await
    }

    /// フォルダ内の項目を1ページ分リストする。
    ///
    /// 1回の呼び出しで返す項目は最大で`limit`件であるため、フォルダに後続の項目が残っている
    /// 場合がある。後続の項目がある場合は、戻り値の[`ItemsPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定して再度呼び出すと、次のページを取得できる。
    /// すべての項目が必要な場合は、[`BoxClient::list_all_items_in_folder`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 項目をリストするフォルダのID
    /// - `marker`: 前のページの[`ItemsPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: 項目数の上限、並び替えなどのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ルートフォルダで`options.sort`を指定した場合も、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_items_in_folder(
        &self,
        folder_id: &FolderId,
        marker: Option<&str>,
        options: &ListItemsOptions,
    ) -> BoxClientResult<ItemsPage> {
        if folder_id.is_root() && options.sort.is_some() {
            return Err(BoxClientError::InvalidArgument(
                "sort is not supported for marker-based pagination in the root folder",
            ));
        }

        let request = marker_page_request(
            ["folders", folder_id.as_ref(), "items"],
            marker,
            options.limit,
            options.sort,
            options.direction,
        )?;

        self.send_json(request).await
    }

    /// フォルダ内のすべての項目を、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。すべての項目をメモリに保持するため、
    /// 項目数が非常に多いフォルダでは[`BoxClient::list_items_in_folder`]でページごとに処理する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 項目をリストするフォルダのID
    /// - `options`: 項目数の上限、並び替えなどのオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// ルートフォルダで`options.sort`を指定した場合も、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXが以前に返したマーカーを再び返した場合、このメソッドは無限ループを避けるため[`BoxClientError::UnexpectedResponse`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みの項目を破棄してエラーを返す。
    pub async fn list_all_items_in_folder(
        &self,
        folder_id: &FolderId,
        options: &ListItemsOptions,
    ) -> BoxClientResult<Vec<BoxItem>> {
        let options = ListItemsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
            ..*options
        };
        let (items, pages) = collect_all_pages(async |marker| {
            self.list_items_in_folder(folder_id, marker, &options).await
        })
        .await?;

        tracing::debug!(
            folder_id = folder_id.as_ref(),
            pages,
            items = items.len(),
            "listed all items in folder"
        );

        Ok(items)
    }

    /// フォルダを作成する。
    ///
    /// # Arguments
    ///
    /// - `name`: 作成するフォルダの名前
    /// - `parent_id`: 作成するフォルダの親フォルダのID
    ///
    /// # Errors
    ///
    /// フォルダ名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// 同じ名前の項目が親フォルダに存在する場合は、[`BoxClientError::Api`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn create_folder(&self, name: &str, parent_id: &FolderId) -> BoxClientResult<Folder> {
        let body = CreateFolderBody::new(name, parent_id)?;
        let folder: Folder = self
            .send_json(ApiRequest::post(["folders"]).json(&body)?)
            .await?;

        tracing::debug!(
            folder_id = folder.id.as_ref(),
            parent_id = parent_id.as_ref(),
            "created folder"
        );

        Ok(folder)
    }

    /// フォルダを、フォルダ内のすべての項目とともに別のフォルダへコピーする。
    ///
    /// BOXは、項目数が500以下のフォルダを同期的にコピーし、作成したフォルダを返す。
    /// 項目数が500を超えるフォルダは非同期にコピーし、コピーの完了を待たずにフォルダを返す。
    /// BOXはコピーの完了を確認するAPIを提供していない。
    ///
    /// コピーの間、BOXはコピー元のフォルダとその子孫、およびコピー先の親フォルダをロックする。
    /// ロックの間は、これらのフォルダを移動、コピー、削除または復元できない。
    ///
    /// HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
    /// 5xxまたはタイムアウトの後に再送すると、BOXがコピーを完了していた場合は409を受け取り、
    /// 名前が異なる場合はフォルダを重複してコピーするおそれがあるためである。
    /// したがって、429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: コピーするフォルダのID
    /// - `parent_id`: コピー先の親フォルダのID。ルートフォルダへコピーする場合は[`FolderId::root`]を指定する。
    /// - `options`: コピーしたフォルダの名前のオプション
    ///
    /// # Errors
    ///
    /// `folder_id`にルートフォルダを指定した場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXはルートフォルダのコピーに500を返すため、クライアントで検出する。
    /// `options.name`をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: パラメーターが不足している、または不正である。
    /// - 403: フォルダをコピーする権限がない、または情報バリアがコピーを禁止している。
    /// - 404: コピー元のフォルダまたはコピー先の親フォルダが存在しない、またはアクセスする権限がない。
    /// - 409: 同じ名前の項目がコピー先の親フォルダに存在する（`item_name_in_use`）。
    ///   または、移動、コピーなど、他の操作がコピー元のフォルダまたはコピー先の親フォルダをロックしている
    ///   （`operation_blocked_temporary`）。後者の場合、クライアントは再試行設定に従って再試行し、
    ///   再試行回数の上限に達してもロックが解除されない場合にこのエラーを返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダをコピーした可能性がある。
    /// したがって、[`BoxClient::list_items_in_folder`]でコピー先の親フォルダを確認する。
    pub async fn copy_folder(
        &self,
        folder_id: &FolderId,
        parent_id: &FolderId,
        options: &CopyFolderOptions<'_>,
    ) -> BoxClientResult<Folder> {
        ensure_not_root(folder_id, "the root folder cannot be copied")?;

        let body = CopyFolderBody::new(parent_id, options.name)?;
        let request = ApiRequest::post(["folders", folder_id.as_ref(), "copy"]).json(&body)?;

        let folder: Folder = self.send_json(request).await?;

        tracing::debug!(
            folder_id = folder_id.as_ref(),
            parent_id = parent_id.as_ref(),
            copied_folder_id = folder.id.as_ref(),
            "copied folder"
        );

        Ok(folder)
    }

    /// フォルダ名を変更する。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがフォルダ名の変更を完了している可能性がある。
    /// `options.etag`を指定した場合は、名前の変更によって`ETag`が変わるため、再送すると412を受け取る。
    /// その結果、変更に成功したにもかかわらず、変更に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがPUTであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 名前を変更するフォルダのID
    /// - `name`: 新しいフォルダ名
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// `folder_id`にルートフォルダを指定した場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// フォルダ名をBOXが拒否する場合は、リクエストを送信せずに[`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: フォルダを操作する権限がない。
    /// - 404: フォルダが存在しない、またはフォルダにアクセスする権限がない。
    /// - 409: 同じ名前の項目が親フォルダに存在する（`item_name_in_use`）。
    ///   または、移動、コピーなど、他の操作がフォルダをロックしている（`operation_blocked_temporary`）。
    ///   後者の場合、クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    /// - 412: `options.etag`がフォルダの現在の`ETag`と一致しない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダ名を変更した可能性がある。
    /// したがって、[`BoxClient::folder_information`]でフォルダ名を確認する。
    pub async fn rename_folder(
        &self,
        folder_id: &FolderId,
        name: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<Folder> {
        ensure_not_root(folder_id, "the root folder cannot be renamed")?;

        let body = RenameBody::new(name)?;
        let request = update_item_request("folders", folder_id.as_ref(), &body, options.etag)?;

        let folder: Folder = self.send_json(request).await?;

        tracing::debug!(
            folder_id = folder_id.as_ref(),
            name = folder.name,
            "renamed folder"
        );

        Ok(folder)
    }

    /// フォルダの説明を設定する。
    ///
    /// 説明を削除する場合は、`description`に空文字列を指定する。
    ///
    /// BOXクライアントは、冪等なリクエストであれば、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXが説明の設定を完了している可能性がある。
    /// `options.etag`を指定した場合は、説明の設定によって`ETag`が変わるため、再送すると412を受け取る。
    /// その結果、設定に成功したにもかかわらず、設定に失敗したと誤って判断するおそれがある。
    /// したがって、HTTPメソッドがPUTであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 説明を設定するフォルダのID
    /// - `description`: フォルダの説明。[`MAX_DESCRIPTION_CHARS`](crate::MAX_DESCRIPTION_CHARS)文字以下で指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// `folder_id`にルートフォルダを指定した場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 説明が[`MAX_DESCRIPTION_CHARS`](crate::MAX_DESCRIPTION_CHARS)文字より長い場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidRequestBody`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: フォルダを操作する権限がない。
    /// - 404: フォルダが存在しない、またはフォルダにアクセスする権限がない。
    /// - 409: 移動、コピーなど、他の操作がフォルダをロックしている（`operation_blocked_temporary`）。
    ///   クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    /// - 412: `options.etag`がフォルダの現在の`ETag`と一致しない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXが説明を設定した可能性がある。
    /// したがって、[`BoxClient::folder_information`]でフォルダの説明を確認する。
    pub async fn set_folder_description(
        &self,
        folder_id: &FolderId,
        description: &str,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<Folder> {
        ensure_not_root(
            folder_id,
            "the description of the root folder cannot be set",
        )?;

        let body = SetDescriptionBody::new(description)?;
        let request = update_item_request("folders", folder_id.as_ref(), &body, options.etag)?;

        let folder = self.send_json(request).await?;

        tracing::debug!(folder_id = folder_id.as_ref(), "set folder description");

        Ok(folder)
    }

    /// フォルダを、フォルダ内のすべての項目とともに別のフォルダへ移動する。
    ///
    /// 項目の多いフォルダの移動には時間がかかる。BOXは移動に600秒以上かかると503を返すが、移動処理はその後も継続する。
    /// ただし、[`BoxClientTimeout::request_timeout`](crate::BoxClientTimeout::request_timeout)の既定値は30秒であるため、
    /// 通常はBOXが503を返す前にクライアントがタイムアウトする。タイムアウトした場合も、BOXは移動処理を継続している可能性がある。
    ///
    /// 503を受け取った後、またはタイムアウトの後に再送すると、継続中の移動処理がフォルダをロックしているため409を受け取る。
    /// また、`options.etag`を指定した場合は、移動によって`ETag`が変わるため412を受け取る。
    /// その結果、移動の結果を誤って判断するおそれがある。
    /// したがって、HTTPメソッドがPUTであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 移動するフォルダのID
    /// - `destination`: 移動先。フォルダID、またはルートフォルダへ移動するユーザーのIDで指定する。
    /// - `options`: `ETag`のオプション
    ///
    /// # Errors
    ///
    /// `folder_id`にルートフォルダを指定した場合、または`destination`のユーザーIDが空文字列の場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: パラメーターが不正である、またはフォルダのロックが移動を禁止している。
    /// - 403: フォルダまたは移動先の親フォルダを操作する権限がない、またはフォルダのロックが移動を禁止している。
    /// - 404: フォルダまたは移動先の親フォルダが存在しない、またはアクセスする権限がない。
    /// - 409: 同じ名前の項目が移動先の親フォルダに存在する（`item_name_in_use`）。
    ///   または、移動、コピーなど、他の操作がフォルダをロックしている（`operation_blocked_temporary`）。
    ///   後者の場合、クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    /// - 412: `options.etag`がフォルダの現在の`ETag`と一致しない。
    /// - 503: 移動に600秒以上かかっている。移動処理は継続している。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// タイムアウトした場合は、`is_timeout()`が`true`を返す[`BoxClientError::Http`]を返す。
    ///
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダを移動したか、移動処理を継続している可能性がある。
    /// したがって、[`BoxClient::folder_information`]でフォルダの親フォルダを確認する。
    pub async fn move_folder(
        &self,
        folder_id: &FolderId,
        destination: &MoveDestination,
        options: &UpdateItemOptions<'_>,
    ) -> BoxClientResult<Folder> {
        ensure_not_root(folder_id, "the root folder cannot be moved")?;

        let body = MoveBody::new(destination)?;
        let request = update_item_request("folders", folder_id.as_ref(), &body, options.etag)?;

        let folder = self.send_json(request).await?;

        tracing::debug!(folder_id = folder_id.as_ref(), ?destination, "moved folder");

        Ok(folder)
    }

    /// フォルダを削除する。
    ///
    /// BOXは、ゴミ箱が有効な場合はフォルダをゴミ箱に移動し、無効な場合はフォルダを完全に削除する。
    /// BOXでは、ゴミ箱は既定で有効である。
    ///
    /// 項目の多いフォルダの削除には時間がかかる。BOXは削除に600秒以上かかると503を返すが、削除処理はその後も継続する。
    /// ただし、[`BoxClientTimeout::request_timeout`](crate::BoxClientTimeout::request_timeout)の既定値は30秒であるため、
    /// 通常はBOXが503を返す前にクライアントがタイムアウトする。タイムアウトした場合も、BOXは削除処理を継続している可能性がある。
    /// 503を受け取るまで待つ場合は、[`BoxClientBuilder::timeout`](crate::BoxClientBuilder::timeout)で
    /// `request_timeout`を600秒より長くする。
    ///
    /// 503を受け取った後、またはタイムアウトの後に再送すると、削除済みのフォルダに対して404を受け取る可能性がある。
    /// したがって、HTTPメソッドがDELETEであっても冪等でないリクエストとして送信し、
    /// 429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 削除するフォルダのID
    /// - `options`: 再帰的に削除するかどうか、および`ETag`のオプション
    ///
    /// # Errors
    ///
    /// ルートフォルダを指定した場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// `options.etag`に使用できない文字が含まれる場合は、リクエストを送信せずに
    /// [`BoxClientError::InvalidHeaderValue`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 400: `options.recursive`が`false`で、フォルダが空でない。
    /// - 403: フォルダを削除する権限がない、またはフォルダがロックされている。
    /// - 404: フォルダが存在しない、またはフォルダにアクセスする権限がない。
    /// - 409: 移動、コピーなど、他の操作がフォルダをロックしている。エラーコードが`operation_blocked_temporary`の
    ///   場合、クライアントは再試行設定に従って再試行し、再試行回数の上限に達してもロックが解除されない場合に
    ///   このエラーを返す。
    /// - 412: `options.etag`がフォルダの現在の`ETag`と一致しない。
    /// - 500: BOXの内部エラー。BOXがフォルダを削除した可能性がある。
    /// - 503: 削除に600秒以上かかっている。削除処理は継続している。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// タイムアウトした場合は、`is_timeout()`が`true`を返す[`BoxClientError::Http`]を返す。
    ///
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダを削除したか、削除処理を継続している可能性がある。
    /// したがって、[`BoxClient::folder_information`]でフォルダの状態を確認する。
    /// ゴミ箱が有効な場合は、[`BoxClient::trashed_folder_information`]でゴミ箱に移動したことも確認できる。
    pub async fn delete_folder(
        &self,
        folder_id: &FolderId,
        options: &DeleteFolderOptions<'_>,
    ) -> BoxClientResult<()> {
        ensure_not_root(folder_id, "the root folder cannot be deleted")?;

        let mut request = ApiRequest::delete(["folders", folder_id.as_ref()])
            .idempotency(Idempotency::NonIdempotent);
        if options.recursive {
            request = request.query("recursive", "true");
        }
        request = request.if_match(options.etag)?;

        self.send_empty(request).await?;

        tracing::debug!(
            folder_id = folder_id.as_ref(),
            recursive = options.recursive,
            "deleted folder"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_folder_id {
    use super::*;

    /// ルートフォルダのIDが0であることを確認
    #[test]
    fn test_root() {
        assert_eq!(FolderId::root().as_ref(), "0");
    }
}

#[cfg(test)]
mod test_folder {
    use serde_json::json;
    use time::macros::datetime;

    use super::*;

    /// 日時をRFC 3339形式で解析し、パス、親フォルダおよびユーザーをデシリアライズすることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let user = json!({"type": "user", "id": "33224", "name": "Test User", "login": "user@example.com"});
        let folder: Folder = serde_json::from_value(json!({
            "type": "folder",
            "id": "12345",
            "etag": "1",
            "sequence_id": "3",
            "name": "Contracts",
            "description": "Legal contracts",
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
            "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
            "item_status": "active",
            "item_collection": {"total_count": 0, "entries": []},
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
            folder,
            Folder {
                id: FolderId::new("12345"),
                etag: Some("1".to_owned()),
                sequence_id: Some("3".to_owned()),
                name: "Contracts".to_owned(),
                description: Some("Legal contracts".to_owned()),
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

    /// 日時、親フォルダなどがnullまたは欠落しているルートフォルダをデシリアライズすることを確認
    #[test]
    fn test_deserialize_root() -> anyhow::Result<()> {
        let folder: Folder = serde_json::from_value(json!({
            "type": "folder",
            "id": "0",
            "etag": null,
            "sequence_id": null,
            "name": "All Files",
            "created_at": null,
            "modified_at": null,
            "path_collection": {"total_count": 0, "entries": []},
            "parent": null,
            "item_status": "active",
        }))?;

        assert_eq!(folder.id, FolderId::root());
        assert_eq!(folder.etag, None);
        assert_eq!(folder.created_at, None);
        // レスポンスに存在しない日時の属性も`None`として受け入れる。
        assert_eq!(folder.trashed_at, None);
        assert_eq!(folder.parent, None);
        assert_eq!(folder.path_collection.map(|p| p.entries.len()), Some(0));

        Ok(())
    }

    /// RFC 3339形式でない日時の場合は、デシリアライズに失敗することを確認
    #[test]
    fn test_deserialize_invalid_datetime() {
        let result = serde_json::from_value::<Folder>(json!({
            "id": "12345",
            "name": "Contracts",
            "created_at": "2012/12/12 10:53:43",
        }));

        assert!(result.is_err(), "result: {result:?}");
    }
}

#[cfg(test)]
mod test_folder_information {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, not_found_response, short_retry};

    /// `GET /folders/{folder_id}`を呼び出し、フォルダ情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_folder_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "folder",
                "id": "12345",
                "etag": "1",
                "name": "Contracts",
                "created_at": "2012-12-12T10:53:43-08:00",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let folder = client.folder_information(&FolderId::new("12345")).await?;

        assert_eq!(folder.id, FolderId::new("12345"));
        assert_eq!(folder.name, "Contracts");
        assert_eq!(folder.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// フォルダが存在しない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_folder_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/999"))
            .respond_with(not_found_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.folder_information(&FolderId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_items_in_folder {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, request_queries, short_retry};

    /// 項目のAPIのパス
    const ITEMS_PATH: &str = "/2.0/folders/123/items";

    /// マーカーとオプションをクエリパラメーターに変換し、ページを返すことを確認
    #[rstest]
    #[case::default(None, ListItemsOptions::default(), "usemarker=true")]
    #[case::all(
        Some("m1"),
        ListItemsOptions {
            limit: Some(1000),
            sort: Some(ItemSort::Name),
            direction: Some(SortDirection::Desc),
        },
        "usemarker=true&limit=1000&marker=m1&sort=name&direction=DESC"
    )]
    #[tokio::test]
    async fn test_list_items_in_folder(
        #[case] marker: Option<&str>,
        #[case] options: ListItemsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "file", "id": "1", "name": "a.txt", "etag": "0"}],
                "limit": 100,
                "next_marker": "m2",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let page = client
            .list_items_in_folder(&FolderId::new("123"), marker, &options)
            .await?;

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(
            request_queries(&server, ITEMS_PATH).await,
            vec![expected_query]
        );

        Ok(())
    }

    /// 項目数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::zero(0)]
    #[case::over_max(1001)]
    #[tokio::test]
    async fn test_list_items_in_folder_invalid_limit(#[case] limit: u32) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListItemsOptions {
            limit: Some(limit),
            ..ListItemsOptions::default()
        };

        let result = client
            .list_items_in_folder(&FolderId::new("123"), None, &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(
            request_queries(&server, ITEMS_PATH).await,
            Vec::<String>::new()
        );

        Ok(())
    }

    /// ルートフォルダの項目のAPIのパス
    const ROOT_ITEMS_PATH: &str = "/2.0/folders/0/items";

    /// ルートフォルダで並び替えに使用する属性を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_items_in_folder_root_with_sort() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListItemsOptions {
            sort: Some(ItemSort::Name),
            ..ListItemsOptions::default()
        };

        let result = client
            .list_items_in_folder(&FolderId::root(), None, &options)
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(
            request_queries(&server, ROOT_ITEMS_PATH).await,
            Vec::<String>::new()
        );

        Ok(())
    }

    /// ルートフォルダでも、並び替えに使用する属性を指定しなければ、並び替えの方向とともにリストできることを確認
    #[tokio::test]
    async fn test_list_items_in_folder_root_without_sort() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ROOT_ITEMS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [],
                "limit": 100,
                "next_marker": null,
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListItemsOptions {
            direction: Some(SortDirection::Desc),
            ..ListItemsOptions::default()
        };

        let page = client
            .list_items_in_folder(&FolderId::root(), None, &options)
            .await?;

        assert_eq!(page.entries, Vec::<BoxItem>::new());
        assert_eq!(
            request_queries(&server, ROOT_ITEMS_PATH).await,
            vec!["usemarker=true&direction=DESC"]
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_items_in_folder {
    use rstest::rstest;
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::test_support::{create_client, mount_token, short_retry};

    /// 項目のAPIのパス
    const ITEMS_PATH: &str = "/2.0/folders/123/items";

    /// 指定した名前のファイルだけを含むページのレスポンスを返す。
    fn page_response(name: &str, next_marker: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"type": "file", "id": name, "name": name, "etag": "0"}],
            "limit": 1000,
            "next_marker": next_marker,
        }))
    }

    /// 項目の名前を返す。
    fn item_name(item: &BoxItem) -> Option<&str> {
        match item {
            BoxItem::File(file) => Some(&file.name),
            BoxItem::Folder(folder) => Some(&folder.name),
            BoxItem::WebLink(link) => Some(&link.name),
            BoxItem::Unknown => None,
        }
    }

    /// `next_marker`がnullまたは空文字列になるまでページを送り、すべての項目を返すことを確認
    #[rstest]
    #[case::null(Value::Null)]
    #[case::empty(json!(""))]
    #[tokio::test]
    async fn test_list_all_items_in_folder(#[case] last_marker: Value) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // 項目数の上限を省略した場合は、上限の1000件ずつ取得する。
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", &json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", &last_marker))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let items = client
            .list_all_items_in_folder(&FolderId::new("123"), &ListItemsOptions::default())
            .await?;

        let names: Vec<_> = items.iter().filter_map(item_name).collect();
        assert_eq!(names, vec!["first", "second"]);

        Ok(())
    }

    /// BOXが同じマーカーを続けて返す場合は、無限ループせずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_items_in_folder_marker_not_advanced() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(ITEMS_PATH))
            .respond_with(page_response("same", &json!("m1")))
            .expect(2)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_items_in_folder(&FolderId::new("123"), &ListItemsOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_create_folder_body {
    use rstest::rstest;

    use super::*;

    /// BOXが受け付けるフォルダ名で、ボディを作成できることを確認
    #[rstest]
    #[case::ascii("folder")]
    #[case::max_ascii(&"a".repeat(255))]
    #[case::max_japanese(&"あ".repeat(255))]
    fn test_valid_name(#[case] name: &str) -> anyhow::Result<()> {
        let parent_id = FolderId::new("0");
        let body = CreateFolderBody::new(name, &parent_id)?;

        assert_eq!(body.name, name);

        Ok(())
    }

    /// BOXが拒否するフォルダ名を、検証エラーとして返すことを確認
    ///
    /// 使用できない文字の詳細な検証は、`validate_item_name`のテストで確認する。
    /// ここでは、長さの検証と、`validate_item_name`をフィールドに適用していることを確認する。
    #[rstest]
    #[case::empty("")]
    #[case::too_long(&"a".repeat(256))]
    #[case::too_long_japanese(&"あ".repeat(256))]
    #[case::invalid_char("a/b")]
    fn test_invalid_name(#[case] name: &str) {
        let parent_id = FolderId::new("0");
        let result = CreateFolderBody::new(name, &parent_id);

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
    }

    /// 検証エラーのメッセージに、検証に失敗したフィールドを含むことを確認
    #[test]
    fn test_error_message_contains_field() -> anyhow::Result<()> {
        let Err(BoxClientError::InvalidRequestBody(error)) =
            CreateFolderBody::new("a/b", &FolderId::new("0"))
        else {
            anyhow::bail!("validation must fail");
        };

        let message = error.to_string();
        assert!(message.contains("name"), "message: {message}");

        Ok(())
    }

    /// BOXが求める形式のJSONにシリアライズすることを確認
    #[test]
    fn test_serialize() -> anyhow::Result<()> {
        let parent_id = FolderId::new("123");
        let body = CreateFolderBody::new("folder", &parent_id)?;

        assert_eq!(
            serde_json::to_value(&body)?,
            serde_json::json!({"name": "folder", "parent": {"id": "123"}})
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_create_folder {
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

    /// フォルダのAPIのパス
    const FOLDERS_PATH: &str = "/2.0/folders";

    /// `POST /folders`に名前と親フォルダを送信し、作成したフォルダをデシリアライズすることを確認
    #[tokio::test]
    async fn test_create_folder() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDERS_PATH))
            .and(body_json(
                json!({"name": "Contracts", "parent": {"id": "0"}}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "folder",
                "id": "12345",
                "etag": "0",
                "name": "Contracts",
                "created_at": "2012-12-12T10:53:43-08:00",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let folder = client.create_folder("Contracts", &FolderId::root()).await?;

        assert_eq!(folder.id, FolderId::new("12345"));
        assert_eq!(folder.name, "Contracts");
        assert_eq!(folder.parent.map(|p| p.id), Some(FolderId::root()));

        Ok(())
    }

    /// BOXが拒否するフォルダ名を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_create_folder_rejects_invalid_name() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client.create_folder("a/b", &FolderId::root()).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 作成は冪等でないため、5xxの後に再送すると同じフォルダを重複して作成するおそれがある。
    #[rstest]
    #[case::name_conflict(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_create_folder_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("POST", FOLDERS_PATH, status, async |client: &BoxClient| {
            client.create_folder("Contracts", &FolderId::root()).await
        })
        .await
    }
}

#[cfg(test)]
mod test_copy_folder {
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

    /// フォルダのコピーのAPIのパス
    const COPY_PATH: &str = "/2.0/folders/123/copy";

    /// コピー先の親フォルダと、指定した場合だけ名前を送信し、コピーしたフォルダをデシリアライズすることを確認
    #[rstest]
    #[case::without_name(
        CopyFolderOptions::default(),
        json!({"parent": {"id": "456"}}),
        "Contracts"
    )]
    #[case::with_name(
        CopyFolderOptions::default().with_name("Contracts (copy)"),
        json!({"parent": {"id": "456"}, "name": "Contracts (copy)"}),
        "Contracts (copy)"
    )]
    #[tokio::test]
    async fn test_copy_folder(
        #[case] options: CopyFolderOptions<'static>,
        #[case] expected_body: serde_json::Value,
        #[case] copied_name: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(COPY_PATH))
            .and(body_json(expected_body))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "type": "folder",
                "id": "789",
                "etag": "0",
                "name": copied_name,
                "parent": {"type": "folder", "id": "456", "etag": "0", "name": "Archive"},
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let folder = client
            .copy_folder(&FolderId::new("123"), &FolderId::new("456"), &options)
            .await?;

        assert_eq!(folder.id, FolderId::new("789"));
        assert_eq!(folder.name, copied_name);
        assert_eq!(folder.parent.map(|p| p.id), Some(FolderId::new("456")));

        Ok(())
    }

    /// ルートフォルダ、またはBOXが拒否する名前を指定した場合は、リクエストを送信せずにエラーを返すことを確認
    ///
    /// 使用できない文字の詳細な検証は、`validate_item_name`のテストで確認する。
    /// ここでは、名前を指定した場合に長さと`validate_item_name`の検証を適用していることを確認する。
    #[rstest]
    #[case::root(FolderId::root(), None, "InvalidArgument")]
    #[case::empty_name(FolderId::new("123"), Some(""), "InvalidRequestBody")]
    #[case::too_long_name(FolderId::new("123"), Some(&*"あ".repeat(256)), "InvalidRequestBody")]
    #[case::invalid_char(FolderId::new("123"), Some("a/b"), "InvalidRequestBody")]
    #[tokio::test]
    async fn test_copy_folder_rejects_without_request(
        #[case] folder_id: FolderId,
        #[case] name: Option<&str>,
        #[case] expected: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;
        let options = CopyFolderOptions { name };

        let result = client
            .copy_folder(&folder_id, &FolderId::new("456"), &options)
            .await;

        let matched = match expected {
            "InvalidArgument" => matches!(result, Err(BoxClientError::InvalidArgument(_))),
            _ => matches!(result, Err(BoxClientError::InvalidRequestBody(_))),
        };
        assert!(matched, "result: {result:?}");
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// コピーは冪等でないため、5xxの後に再送すると、フォルダを重複してコピーするおそれがある。
    #[rstest]
    #[case::name_in_use(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_copy_folder_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("POST", COPY_PATH, status, async |client: &BoxClient| {
            client
                .copy_folder(
                    &FolderId::new("123"),
                    &FolderId::new("456"),
                    &CopyFolderOptions::default(),
                )
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_update_folder {
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
            assert_not_retried, create_client, if_match_headers, is_invalid_argument,
            is_invalid_request_body, mount_token, request_count, short_retry,
        },
    };

    /// フォルダのAPIのパス
    const FOLDER_PATH: &str = "/2.0/folders/123";

    /// フォルダを更新するメソッド
    #[derive(Debug, Clone)]
    enum Update {
        /// 名前の変更
        Rename(String),

        /// 説明の設定
        Description(String),

        /// 移動
        Move(MoveDestination),
    }

    impl Update {
        /// `folder_id`のフォルダを更新するメソッドを呼び出す。
        async fn call(
            &self,
            client: &BoxClient,
            folder_id: &FolderId,
            options: &UpdateItemOptions<'_>,
        ) -> BoxClientResult<Folder> {
            match self {
                Self::Rename(name) => client.rename_folder(folder_id, name, options).await,
                Self::Description(description) => {
                    client
                        .set_folder_description(folder_id, description, options)
                        .await
                }
                Self::Move(destination) => {
                    client.move_folder(folder_id, destination, options).await
                }
            }
        }

        /// BOXが返す、更新した属性を含むフォルダのレスポンスを返す。
        fn response(&self) -> ResponseTemplate {
            let mut folder = json!({
                "type": "folder",
                "id": "123",
                "etag": "2",
                "name": "Contracts",
            });
            match self {
                Self::Rename(name) => folder["name"] = json!(name),
                Self::Description(description) => folder["description"] = json!(description),
                Self::Move(MoveDestination::Folder(folder_id)) => {
                    folder["parent"] = json!({
                        "type": "folder",
                        "id": folder_id.as_ref(),
                        "etag": "0",
                        "name": "Archive",
                    });
                }
                Self::Move(MoveDestination::User(_)) => {
                    folder["parent"] =
                        json!({"type": "folder", "id": "0", "etag": null, "name": "All Files"});
                }
            }
            ResponseTemplate::new(200).set_body_json(folder)
        }

        /// 更新後のフォルダに、更新した属性が反映されていることを確認する。
        fn assert_applied(&self, folder: &Folder) {
            let parent_id = folder.parent.as_ref().map(|p| &p.id);
            match self {
                Self::Rename(name) => assert_eq!(&folder.name, name),
                Self::Description(description) => {
                    assert_eq!(folder.description.as_ref(), Some(description));
                }
                Self::Move(MoveDestination::Folder(folder_id)) => {
                    assert_eq!(parent_id, Some(folder_id));
                }
                Self::Move(MoveDestination::User(_)) => {
                    assert_eq!(parent_id, Some(&FolderId::root()));
                }
            }
        }
    }

    /// 更新する属性だけをボディで送信し、`ETag`を指定した場合だけIf-Matchヘッダーを送信することを確認
    ///
    /// 説明を削除する空文字列と、上限の文字数の説明も送信できることを確認する。
    /// ユーザーを移動先に指定した場合は、ルートフォルダのIDとユーザーIDを送信することを確認する。
    #[rstest]
    #[case::rename(Update::Rename("Renamed".to_owned()), json!({"name": "Renamed"}), None)]
    #[case::rename_with_etag(
        Update::Rename("Renamed".to_owned()),
        json!({"name": "Renamed"}),
        Some("1")
    )]
    #[case::description(
        Update::Description("Legal contracts".to_owned()),
        json!({"description": "Legal contracts"}),
        None
    )]
    #[case::description_with_etag(
        Update::Description("Legal contracts".to_owned()),
        json!({"description": "Legal contracts"}),
        Some("1")
    )]
    #[case::clear_description(Update::Description(String::new()), json!({"description": ""}), None)]
    #[case::max_japanese_description(
        Update::Description("あ".repeat(crate::MAX_DESCRIPTION_CHARS)),
        json!({"description": "あ".repeat(crate::MAX_DESCRIPTION_CHARS)}),
        None
    )]
    #[case::move_to_folder(
        Update::Move(MoveDestination::Folder(FolderId::new("456"))),
        json!({"parent": {"id": "456"}}),
        None
    )]
    #[case::move_to_folder_with_etag(
        Update::Move(MoveDestination::Folder(FolderId::new("456"))),
        json!({"parent": {"id": "456"}}),
        Some("1")
    )]
    #[case::move_to_user_root(
        Update::Move(MoveDestination::User(UserId::new("789"))),
        json!({"parent": {"id": "0", "user_id": "789"}}),
        None
    )]
    #[tokio::test]
    async fn test_update_folder(
        #[case] update: Update,
        #[case] expected_body: serde_json::Value,
        #[case] etag: Option<&'static str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("PUT"))
            .and(path(FOLDER_PATH))
            .and(body_json(expected_body))
            .respond_with(update.response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = UpdateItemOptions { etag };

        let folder = update
            .call(&client, &FolderId::new("123"), &options)
            .await?;

        assert_eq!(folder.id, FolderId::new("123"));
        update.assert_applied(&folder);
        assert_eq!(
            if_match_headers(&server, "PUT", FOLDER_PATH).await,
            vec![etag.map(str::to_owned)]
        );

        Ok(())
    }

    /// ルートフォルダを更新しようとした場合、またはBOXが拒否する値を指定した場合は、
    /// リクエストを送信せずにエラーを返すことを確認
    ///
    /// 使用できない文字の詳細な検証は、`validate_item_name`のテストで確認する。
    #[rstest]
    #[case::rename_root(
        FolderId::root(),
        Update::Rename("Renamed".to_owned()),
        is_invalid_argument
    )]
    #[case::empty_name(
        FolderId::new("123"),
        Update::Rename(String::new()),
        is_invalid_request_body
    )]
    #[case::too_long_name(
        FolderId::new("123"),
        Update::Rename("あ".repeat(256)),
        is_invalid_request_body
    )]
    #[case::invalid_char_name(
        FolderId::new("123"),
        Update::Rename("a/b".to_owned()),
        is_invalid_request_body
    )]
    #[case::describe_root(
        FolderId::root(),
        Update::Description("Legal contracts".to_owned()),
        is_invalid_argument
    )]
    #[case::too_long_description(
        FolderId::new("123"),
        Update::Description("あ".repeat(crate::MAX_DESCRIPTION_CHARS + 1)),
        is_invalid_request_body
    )]
    #[case::move_root(
        FolderId::root(),
        Update::Move(MoveDestination::Folder(FolderId::new("456"))),
        is_invalid_argument
    )]
    #[case::empty_user_id(
        FolderId::new("123"),
        Update::Move(MoveDestination::User(UserId::new(""))),
        is_invalid_argument
    )]
    #[tokio::test]
    async fn test_update_folder_rejects_without_request(
        #[case] folder_id: FolderId,
        #[case] update: Update,
        #[case] is_expected_error: fn(&BoxClientError) -> bool,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = update
            .call(&client, &folder_id, &UpdateItemOptions::default())
            .await;

        assert!(
            result.as_ref().is_err_and(is_expected_error),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// 更新は冪等でないため、BOXのエラーレスポンスを再試行せずにそのまま返すことを確認
    ///
    /// 移動の503は移動処理が継続していることを示すため、再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::rename_name_in_use(Update::Rename("Renamed".to_owned()), 409)]
    #[case::rename_etag_mismatch(Update::Rename("Renamed".to_owned()), 412)]
    #[case::rename_internal_server_error(Update::Rename("Renamed".to_owned()), 500)]
    #[case::rename_service_unavailable(Update::Rename("Renamed".to_owned()), 503)]
    #[case::description_etag_mismatch(Update::Description("Legal contracts".to_owned()), 412)]
    #[case::description_internal_server_error(
        Update::Description("Legal contracts".to_owned()),
        500
    )]
    #[case::description_service_unavailable(
        Update::Description("Legal contracts".to_owned()),
        503
    )]
    #[case::move_name_in_use(Update::Move(MoveDestination::Folder(FolderId::new("456"))), 409)]
    #[case::move_etag_mismatch(Update::Move(MoveDestination::Folder(FolderId::new("456"))), 412)]
    #[case::move_internal_server_error(
        Update::Move(MoveDestination::Folder(FolderId::new("456"))),
        500
    )]
    #[case::move_still_moving(Update::Move(MoveDestination::Folder(FolderId::new("456"))), 503)]
    #[tokio::test]
    async fn test_update_folder_error_is_not_retried(
        #[case] update: Update,
        #[case] status: u16,
    ) -> anyhow::Result<()> {
        assert_not_retried("PUT", FOLDER_PATH, status, async |client: &BoxClient| {
            update
                .call(client, &FolderId::new("123"), &UpdateItemOptions::default())
                .await
        })
        .await
    }
}

#[cfg(test)]
mod test_delete_folder {
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::test_support::{
        assert_not_retried, assert_retried_after, create_client, if_match, mount_token,
        operation_blocked_response, request_count, requests, short_retry,
    };

    /// フォルダのAPIのパス
    const FOLDER_PATH: &str = "/2.0/folders/123";

    /// フォルダのAPIへのDELETEリクエストのクエリ文字列とIf-Matchヘッダーを返す。
    async fn delete_requests(server: &MockServer) -> Vec<(String, Option<String>)> {
        requests(server, "DELETE", FOLDER_PATH)
            .await
            .iter()
            .map(|r| (r.url.query().unwrap_or_default().to_owned(), if_match(r)))
            .collect()
    }

    /// オプションに応じて、recursiveクエリパラメーターとIf-Matchヘッダーを送信することを確認
    #[rstest]
    #[case::default(DeleteFolderOptions::default(), "", None)]
    #[case::recursive(
        DeleteFolderOptions { recursive: true, etag: None },
        "recursive=true",
        None
    )]
    #[case::etag(
        DeleteFolderOptions { recursive: false, etag: Some("1") },
        "",
        Some("1")
    )]
    #[case::recursive_and_etag(
        DeleteFolderOptions { recursive: true, etag: Some("1") },
        "recursive=true",
        Some("1")
    )]
    #[tokio::test]
    async fn test_delete_folder(
        #[case] options: DeleteFolderOptions<'static>,
        #[case] expected_query: &str,
        #[case] expected_if_match: Option<&str>,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("DELETE"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        client
            .delete_folder(&FolderId::new("123"), &options)
            .await?;

        assert_eq!(
            delete_requests(&server).await,
            vec![(
                expected_query.to_owned(),
                expected_if_match.map(str::to_owned)
            )]
        );

        Ok(())
    }

    /// ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_delete_folder_rejects_root() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .delete_folder(&FolderId::root(), &DeleteFolderOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 503は削除処理が継続していることを示すため、再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::not_empty(400)]
    #[case::not_found(404)]
    #[case::locked(409)]
    #[case::etag_mismatch(412)]
    #[case::internal_server_error(500)]
    #[case::still_deleting(503)]
    #[tokio::test]
    async fn test_delete_folder_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        assert_not_retried("DELETE", FOLDER_PATH, status, async |client: &BoxClient| {
            client
                .delete_folder(&FolderId::new("123"), &DeleteFolderOptions::default())
                .await
        })
        .await
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(operation_blocked_response())]
    #[tokio::test]
    async fn test_delete_folder_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        assert_retried_after(
            "DELETE",
            FOLDER_PATH,
            first_response,
            ResponseTemplate::new(204),
            async |client: &BoxClient| {
                client
                    .delete_folder(&FolderId::new("123"), &DeleteFolderOptions::default())
                    .await
            },
        )
        .await?;

        Ok(())
    }
}
