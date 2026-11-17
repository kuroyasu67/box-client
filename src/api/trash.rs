//! ゴミ箱に関するAPI

use serde::Serialize;

use crate::{
    BoxClient, BoxClientError, BoxClientResult, BoxItem, FileId, Folder, FolderId, WebLink,
    WebLinkId,
    api::files::File,
    api::folders::{ItemsPage, MAX_ITEMS_LIMIT, collect_all_pages, validate_limit},
    request::ApiRequest,
};

/// ゴミ箱内の項目をリストするときのオプション
///
/// BOXは、マーカー方式のページ送りで`sort`または`direction`クエリパラメーターを指定すると、
/// 400（`invalid_parameter`）を返す。このクライアントは常にマーカー方式でページを送るため、
/// 並び替えに使用する属性および並び替えの方向を指定できない。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// ```
/// use box_client::ListTrashedItemsOptions;
///
/// let options = ListTrashedItemsOptions::default().with_limit(500);
/// assert_eq!(options.limit, Some(500));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListTrashedItemsOptions {
    /// 1ページで取得する項目数の上限
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。`None`の場合はBOXの既定値を使用する。
    pub limit: Option<u32>,
}

/// ゴミ箱内のフォルダ、ファイルまたはWebリンクを復元するときのオプション
///
/// BOXはフォルダ、ファイルおよびWebリンクの復元で同じ形式のリクエストボディを受け付けるため、
/// [`BoxClient::restore_folder`]、[`BoxClient::restore_file`]および[`BoxClient::restore_web_link`]で共有する。
///
/// 将来フィールドを追加しても利用者のコードが壊れないように、`#[non_exhaustive]`を付けている。
/// そのため、クレートの外では構造体リテラルで作成できない。[`Default`]で作成し、`with_*`メソッドで値を設定する。
///
/// ```
/// use box_client::{FolderId, RestoreItemOptions};
///
/// let parent_id = FolderId::new("123");
/// let options = RestoreItemOptions::default()
///     .with_name("restored.txt")
///     .with_parent_id(&parent_id);
/// assert_eq!(options.name, Some("restored.txt"));
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RestoreItemOptions<'a> {
    /// 復元後のフォルダ名、ファイル名またはWebリンク名
    ///
    /// 復元先に同じ名前の項目がある場合に、別の名前で復元するために指定する。
    /// `None`の場合は、ゴミ箱に移動する前の名前で復元する。
    pub name: Option<&'a str>,

    /// 元の親フォルダが存在しない場合の復元先の親フォルダのID
    ///
    /// BOXは、元の親フォルダが存在しない場合にだけこのIDを使用する。
    /// 元の親フォルダが存在する場合は、このIDを指定しても元の親フォルダに復元する。
    pub parent_id: Option<&'a FolderId>,
}

impl ListTrashedItemsOptions {
    /// 1ページで取得する項目数の上限を設定する。
    ///
    /// 1以上[`MAX_ITEMS_LIMIT`]以下を指定する。範囲外の値は、APIを呼び出したときにエラーになる。
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

impl<'a> RestoreItemOptions<'a> {
    /// 復元後のフォルダ名、ファイル名またはWebリンク名を設定する。
    #[must_use]
    pub fn with_name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    /// 元の親フォルダが存在しない場合の復元先の親フォルダのIDを設定する。
    #[must_use]
    pub fn with_parent_id(mut self, parent_id: &'a FolderId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }
}

/// フォルダ、ファイルまたはWebリンクを復元するリクエストのボディ
#[derive(Debug, Serialize)]
struct RestoreItemBody<'a> {
    /// 復元後のフォルダ名、ファイル名またはWebリンク名
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,

    /// 元の親フォルダが存在しない場合の復元先の親フォルダ
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<ParentReference<'a>>,
}

/// リクエストボディで親フォルダを指定する参照
#[derive(Debug, Serialize)]
struct ParentReference<'a> {
    /// 親フォルダのID
    id: &'a str,
}

impl<'a> From<&RestoreItemOptions<'a>> for RestoreItemBody<'a> {
    fn from(options: &RestoreItemOptions<'a>) -> Self {
        Self {
            name: options.name,
            parent: options
                .parent_id
                .map(|id| ParentReference { id: id.as_ref() }),
        }
    }
}

impl BoxClient {
    /// ゴミ箱内のフォルダの情報を取得する。
    ///
    /// 取得できるのは、ゴミ箱に直接移動したフォルダだけである。
    /// フォルダをゴミ箱に移動すると、そのフォルダの子フォルダ、孫フォルダなど、すべての子孫フォルダもゴミ箱に入る。
    /// ただし、子孫フォルダはゴミ箱に直接移動したフォルダではないため、子孫フォルダを指定するとBOXは404を返す。
    /// 子孫フォルダの状態を確認する場合は、その子孫フォルダの祖先のうち、ゴミ箱に直接移動したフォルダの情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 情報を取得するゴミ箱内のフォルダのID
    ///
    /// # Errors
    ///
    /// ルートフォルダはゴミ箱に移動できないため、ルートフォルダを指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: フォルダがゴミ箱に直接移動されていない、フォルダが存在しない、
    ///   またはフォルダにアクセスする権限がない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn trashed_folder_information(
        &self,
        folder_id: &FolderId,
    ) -> BoxClientResult<Folder> {
        if folder_id.is_root() {
            return Err(BoxClientError::InvalidArgument(
                "the root folder cannot be in the trash",
            ));
        }

        self.send_json(ApiRequest::get(["folders", folder_id.as_ref(), "trash"]))
            .await
    }

    /// ゴミ箱内のファイルの情報を取得する。
    ///
    /// 取得できるのは、ゴミ箱に直接移動したファイルだけである。
    /// フォルダをゴミ箱に移動すると、そのフォルダの子孫にあたるすべてのファイルもゴミ箱に入る。
    /// ただし、子孫にあたるファイルはゴミ箱に直接移動したファイルではないため、そのファイルを指定するとBOXは404を返す。
    /// 子孫にあたるファイルの状態を確認する場合は、そのファイルの祖先のうち、ゴミ箱に直接移動したフォルダの情報を
    /// [`BoxClient::trashed_folder_information`]で取得する。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 情報を取得するゴミ箱内のファイルのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: ファイルがゴミ箱に直接移動されていない、ファイルが存在しない、
    ///   またはファイルにアクセスする権限がない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn trashed_file_information(&self, file_id: &FileId) -> BoxClientResult<File> {
        self.send_json(ApiRequest::get(["files", file_id.as_ref(), "trash"]))
            .await
    }

    /// ゴミ箱内のWebリンクの情報を取得する。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 情報を取得するゴミ箱内のWebリンクのID
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 404: Webリンクがゴミ箱にない。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn trashed_web_link_information(
        &self,
        web_link_id: &WebLinkId,
    ) -> BoxClientResult<WebLink> {
        self.send_json(ApiRequest::get([
            "web_links",
            web_link_id.as_ref(),
            "trash",
        ]))
        .await
    }

    /// ゴミ箱内のフォルダを復元する。
    ///
    /// 復元できるのは、ゴミ箱に直接移動したフォルダだけである。
    /// BOXはフォルダを元の親フォルダに復元する。元の親フォルダが存在しない場合に限り、
    /// `options.parent_id`で指定したフォルダに復元する。
    ///
    /// 復元中は、復元するフォルダ、復元するフォルダの子孫にあたるすべての項目および復元先のフォルダをBOXがロックする。
    /// ロックしている間は、それらのフォルダを対象とする移動、コピー、削除および復元を実行できない。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがフォルダの復元を完了している可能性がある。
    /// 復元に成功したフォルダはゴミ箱からなくなるため、BOXが復元した後にリクエストを再送すると、BOXは404を返す。
    /// その結果、復元に成功したにもかかわらず、復元に失敗したと誤って判断するおそれがある。
    /// したがって、このリクエストを冪等でないリクエストとして送信し、BOXがリクエストを処理していないことが
    /// 明らかな429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    /// POSTリクエストは既定で冪等でないリクエストとして扱われるため、冪等性を上書きしていない。
    ///
    /// # Arguments
    ///
    /// - `folder_id`: 復元するゴミ箱内のフォルダのID
    /// - `options`: 復元後のフォルダ名および復元先の親フォルダのオプション
    ///
    /// # Errors
    ///
    /// ルートフォルダはゴミ箱に移動できないため、ルートフォルダを指定した場合は、
    /// リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: ゴミ箱からフォルダを復元する権限がない、または復元先のフォルダにアクセスする権限がない。
    /// - 404: フォルダがゴミ箱にない。
    /// - 409: 次のいずれかの理由で復元できない。
    ///   - 復元先に同じ名前のフォルダがある。`options.name`に別の名前を指定して再度呼び出す。
    ///   - 移動、コピー、削除または復元の操作が、復元するフォルダまたは復元先のフォルダをロックしている。
    ///     エラーコードは`operation_blocked_temporary`である。クライアントは再試行設定に従って再試行し、
    ///     再試行回数の上限に達してもロックが解除されない場合にこのエラーを返す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがフォルダを復元した可能性があるため、
    /// [`BoxClient::folder_information`]でフォルダの状態を確認する。
    pub async fn restore_folder(
        &self,
        folder_id: &FolderId,
        options: &RestoreItemOptions<'_>,
    ) -> BoxClientResult<Folder> {
        if folder_id.is_root() {
            return Err(BoxClientError::InvalidArgument(
                "the root folder cannot be in the trash",
            ));
        }

        let request = ApiRequest::post(["folders", folder_id.as_ref()])
            .json(&RestoreItemBody::from(options))?;
        let folder: Folder = self.send_json(request).await?;

        tracing::debug!(
            folder_id = folder_id.as_ref(),
            parent_id = folder.parent.as_ref().map(|p| p.id.as_ref()),
            "restored folder"
        );

        Ok(folder)
    }

    /// ゴミ箱内のファイルを復元する。
    ///
    /// 復元できるのは、ゴミ箱に直接移動したファイルだけである。
    /// BOXはファイルを元の親フォルダに復元する。元の親フォルダが存在しない場合に限り、
    /// `options.parent_id`で指定したフォルダに復元する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがファイルの復元を完了している可能性がある。
    /// 復元に成功したファイルはゴミ箱からなくなるため、BOXが復元した後にリクエストを再送すると、BOXは404を返す。
    /// その結果、復元に成功したにもかかわらず、復元に失敗したと誤って判断するおそれがある。
    /// したがって、このリクエストを冪等でないリクエストとして送信し、BOXがリクエストを処理していないことが
    /// 明らかな429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    /// POSTリクエストは既定で冪等でないリクエストとして扱われるため、冪等性を上書きしていない。
    ///
    /// # Arguments
    ///
    /// - `file_id`: 復元するゴミ箱内のファイルのID
    /// - `options`: 復元後のファイル名および復元先の親フォルダのオプション
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: ゴミ箱からファイルを復元する権限がない、または復元先のフォルダにアクセスする権限がない。
    /// - 404: ファイルがゴミ箱にない。
    /// - 409: 復元先に同じ名前のファイルがある。`options.name`に別の名前を指定して再度呼び出す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがファイルを復元した可能性があるため、
    /// [`BoxClient::file_information`]でファイルの状態を確認する。
    pub async fn restore_file(
        &self,
        file_id: &FileId,
        options: &RestoreItemOptions<'_>,
    ) -> BoxClientResult<File> {
        let request =
            ApiRequest::post(["files", file_id.as_ref()]).json(&RestoreItemBody::from(options))?;
        let file: File = self.send_json(request).await?;

        tracing::debug!(
            file_id = file_id.as_ref(),
            parent_id = file.parent.as_ref().map(|p| p.id.as_ref()),
            "restored file"
        );

        Ok(file)
    }

    /// ゴミ箱内のWebリンクを復元する。
    ///
    /// BOXはWebリンクを元の親フォルダに復元する。元の親フォルダが存在しない場合に限り、
    /// `options.parent_id`で指定したフォルダに復元する。
    ///
    /// クライアントは、5xxを受け取った場合、またはタイムアウトした場合に、同じリクエストを再送して再試行する。
    /// しかし、これらの場合は、BOXがWebリンクの復元を完了している可能性がある。
    /// 復元に成功したWebリンクはゴミ箱からなくなるため、BOXが復元した後にリクエストを再送すると、BOXは404を返す。
    /// その結果、復元に成功したにもかかわらず、復元に失敗したと誤って判断するおそれがある。
    /// したがって、このリクエストを冪等でないリクエストとして送信し、BOXがリクエストを処理していないことが
    /// 明らかな429、エラーコードが`operation_blocked_temporary`の409および接続エラーの後にのみ再試行する。
    /// POSTリクエストは既定で冪等でないリクエストとして扱われるため、冪等性を上書きしていない。
    ///
    /// # Arguments
    ///
    /// - `web_link_id`: 復元するゴミ箱内のWebリンクのID
    /// - `options`: 復元後のWebリンク名および復元先の親フォルダのオプション
    ///
    /// # Errors
    ///
    /// BOXがエラーレスポンスを返した場合は、[`BoxClientError::Api`]を返す。主なステータスは次のとおり。
    ///
    /// - 403: ゴミ箱からWebリンクを復元する権限がない、または復元先のフォルダにアクセスする権限がない。
    /// - 404: Webリンクがゴミ箱にない。
    /// - 409: 復元先に同じ名前のWebリンクがある。`options.name`に別の名前を指定して再度呼び出す。
    ///
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    /// 5xxまたはタイムアウトでエラーを返した場合は、BOXがWebリンクを復元した可能性があるため、
    /// [`BoxClient::web_link_information`]でWebリンクの状態を確認する。
    pub async fn restore_web_link(
        &self,
        web_link_id: &WebLinkId,
        options: &RestoreItemOptions<'_>,
    ) -> BoxClientResult<WebLink> {
        let request = ApiRequest::post(["web_links", web_link_id.as_ref()])
            .json(&RestoreItemBody::from(options))?;
        let web_link: WebLink = self.send_json(request).await?;

        tracing::debug!(
            web_link_id = web_link_id.as_ref(),
            parent_id = web_link.parent.as_ref().map(|p| p.id.as_ref()),
            "restored web link"
        );

        Ok(web_link)
    }

    /// ゴミ箱内の項目を1ページ分リストする。
    ///
    /// BOXは、ユーザーがゴミ箱に直接移動したファイル、フォルダおよびWebリンクを返す。
    /// ゴミ箱に直接移動したフォルダの子孫にあたる項目は、ゴミ箱に入っていても返さない。
    ///
    /// 1回の呼び出しで返す項目は最大で`limit`件であるため、ゴミ箱に後続の項目が残っている
    /// 場合がある。後続の項目がある場合は、戻り値の[`ItemsPage::next_marker`]が`Some`になる。
    /// そのマーカーを`marker`に指定して再度呼び出すと、次のページを取得できる。
    /// すべての項目が必要な場合は、[`BoxClient::list_all_trashed_items`]を使用する。
    ///
    /// # Arguments
    ///
    /// - `marker`: 前のページの[`ItemsPage::next_marker`]。最初のページを取得する場合は`None`
    /// - `options`: 項目数の上限のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// 再試行できないエラーが発生した場合、または再試行回数の上限に達した場合はエラーを返す。
    pub async fn list_trashed_items(
        &self,
        marker: Option<&str>,
        options: &ListTrashedItemsOptions,
    ) -> BoxClientResult<ItemsPage> {
        let mut request = ApiRequest::get(["folders", "trash", "items"]).query("usemarker", "true");
        if let Some(limit) = options.limit {
            request = request.query("limit", validate_limit(limit)?.to_string());
        }
        if let Some(marker) = marker {
            request = request.query("marker", marker);
        }

        self.send_json(request).await
    }

    /// ゴミ箱内のすべての項目を、ページを送りながらリストする。
    ///
    /// `options.limit`が`None`の場合は、リクエスト数を減らすために1ページあたり
    /// [`MAX_ITEMS_LIMIT`]件を取得する。すべての項目をメモリに保持するため、
    /// 項目数が非常に多いゴミ箱では[`BoxClient::list_trashed_items`]でページごとに処理する。
    ///
    /// # Arguments
    ///
    /// - `options`: 項目数の上限のオプション
    ///
    /// # Errors
    ///
    /// `options.limit`が範囲外の場合は、リクエストを送信せずに[`BoxClientError::InvalidArgument`]を返す。
    /// BOXが以前に返したマーカーを再び返した場合、このメソッドは無限ループを避けるため[`BoxClientError::UnexpectedResponse`]を返す。
    /// いずれかのページの取得に失敗した場合は、取得済みの項目を破棄してエラーを返す。
    pub async fn list_all_trashed_items(
        &self,
        options: &ListTrashedItemsOptions,
    ) -> BoxClientResult<Vec<BoxItem>> {
        let options = ListTrashedItemsOptions {
            limit: Some(options.limit.unwrap_or(MAX_ITEMS_LIMIT)),
        };
        let (items, pages) =
            collect_all_pages(async |marker| self.list_trashed_items(marker, &options).await)
                .await?;

        tracing::debug!(pages, items = items.len(), "listed all trashed items");

        Ok(items)
    }
}

#[cfg(test)]
mod test_trashed_folder_information {
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, request_count, short_retry},
    };

    /// `GET /folders/{folder_id}/trash`を呼び出し、ゴミ箱に移動した日時と状態を含むフォルダ情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_trashed_folder_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/12345/trash"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "folder",
                "id": "12345",
                "etag": "1",
                "sequence_id": "3",
                "name": "Contracts",
                "trashed_at": "2012-12-12T10:53:43-08:00",
                "purged_at": "2013-01-11T10:53:43-08:00",
                "parent": {"type": "folder", "id": "0", "etag": null, "name": "All Files"},
                "item_status": "trashed",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let folder = client
            .trashed_folder_information(&FolderId::new("12345"))
            .await?;

        assert_eq!(folder.id, FolderId::new("12345"));
        assert_eq!(folder.name, "Contracts");
        assert_eq!(
            folder.trashed_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(
            folder.purged_at,
            Some(datetime!(2013-01-11 10:53:43 -08:00))
        );
        assert_eq!(folder.item_status, Some(ItemStatus::Trashed));

        Ok(())
    }

    /// ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_trashed_folder_information_rejects_root() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client.trashed_folder_information(&FolderId::root()).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(request_count(&server).await, requests_before);

        Ok(())
    }

    /// フォルダがゴミ箱に直接移動されていない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_trashed_folder_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/folders/999/trash"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .trashed_folder_information(&FolderId::new("999"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_trashed_file_information {
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, short_retry},
    };

    /// `GET /files/{file_id}/trash`を呼び出し、ゴミ箱に移動した日時と状態を含むファイル情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_trashed_file_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/12345/trash"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "file",
                "id": "12345",
                "etag": "1",
                "sequence_id": "3",
                "name": "Contract.pdf",
                "sha1": "85136c79cbf9fe36bb9d05d0639c70c265c18d37",
                "size": 629_644,
                // BOXはゴミ箱内のファイルのパスを、ETagおよびシーケンスIDがnullのゴミ箱フォルダで返す。
                "path_collection": {
                    "total_count": 1,
                    "entries": [
                        {"type": "folder", "id": "1", "sequence_id": null, "etag": null, "name": "Trash"},
                    ],
                },
                "trashed_at": "2012-12-12T10:53:43-08:00",
                "purged_at": "2013-01-11T10:53:43-08:00",
                "shared_link": null,
                "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Legal"},
                "item_status": "trashed",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let file = client
            .trashed_file_information(&FileId::new("12345"))
            .await?;

        assert_eq!(file.id, FileId::new("12345"));
        assert_eq!(file.name, "Contract.pdf");
        assert_eq!(file.trashed_at, Some(datetime!(2012-12-12 10:53:43 -08:00)));
        assert_eq!(file.purged_at, Some(datetime!(2013-01-11 10:53:43 -08:00)));
        assert_eq!(file.item_status, Some(ItemStatus::Trashed));
        assert_eq!(
            file.path_collection.map(|p| p.entries[0].etag.clone()),
            Some(None)
        );
        assert_eq!(file.parent.map(|p| p.id), Some(FolderId::new("111")));

        Ok(())
    }

    /// ファイルがゴミ箱に直接移動されていない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_trashed_file_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/files/999/trash"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client.trashed_file_information(&FileId::new("999")).await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_trashed_web_link_information {
    use serde_json::json;
    use time::macros::datetime;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, short_retry},
    };

    /// `GET /web_links/{web_link_id}/trash`を呼び出し、ゴミ箱に移動した日時と状態を含むWebリンク情報をデシリアライズすることを確認
    #[tokio::test]
    async fn test_trashed_web_link_information() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/web_links/12345/trash"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "type": "web_link",
                "id": "12345",
                "etag": "1",
                "sequence_id": "3",
                "name": "Example",
                "url": "https://www.example.com/",
                "description": "Example web site",
                // BOXはゴミ箱内のWebリンクのパスを、ETagおよびシーケンスIDがnullのゴミ箱フォルダで返す。
                "path_collection": {
                    "total_count": 1,
                    "entries": [
                        {"type": "folder", "id": "1", "sequence_id": null, "etag": null, "name": "Trash"},
                    ],
                },
                "trashed_at": "2012-12-12T10:53:43-08:00",
                "purged_at": "2013-01-11T10:53:43-08:00",
                "shared_link": null,
                "parent": {"type": "folder", "id": "111", "sequence_id": "0", "etag": "0", "name": "Links"},
                "item_status": "trashed",
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let web_link = client
            .trashed_web_link_information(&WebLinkId::new("12345"))
            .await?;

        assert_eq!(web_link.id, WebLinkId::new("12345"));
        assert_eq!(web_link.name, "Example");
        assert_eq!(web_link.url.as_deref(), Some("https://www.example.com/"));
        assert_eq!(
            web_link.trashed_at,
            Some(datetime!(2012-12-12 10:53:43 -08:00))
        );
        assert_eq!(
            web_link.purged_at,
            Some(datetime!(2013-01-11 10:53:43 -08:00))
        );
        assert_eq!(web_link.item_status, Some(ItemStatus::Trashed));
        assert_eq!(web_link.parent.map(|p| p.id), Some(FolderId::new("111")));

        Ok(())
    }

    /// Webリンクがゴミ箱にない場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_trashed_web_link_information_not_found() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path("/2.0/web_links/999/trash"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "type": "error",
                "status": 404,
                "code": "not_found",
                "message": "Not Found",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .trashed_web_link_information(&WebLinkId::new("999"))
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 404),
            "result: {result:?}"
        );

        Ok(())
    }
}

#[cfg(test)]
mod test_restore_item_body {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::*;

    /// 指定したオプションだけをリクエストボディにシリアライズすることを確認
    #[rstest]
    #[case::default(None, None, json!({}))]
    #[case::name(Some("restored"), None, json!({"name": "restored"}))]
    #[case::parent(None, Some("111"), json!({"parent": {"id": "111"}}))]
    #[case::name_and_parent(
        Some("restored"),
        Some("111"),
        json!({"name": "restored", "parent": {"id": "111"}})
    )]
    fn test_serialize(
        #[case] name: Option<&str>,
        #[case] parent_id: Option<&str>,
        #[case] expected: Value,
    ) -> anyhow::Result<()> {
        let parent_id = parent_id.map(FolderId::new);
        let options = RestoreItemOptions {
            name,
            parent_id: parent_id.as_ref(),
        };

        let body = serde_json::to_value(RestoreItemBody::from(&options))?;

        assert_eq!(body, expected);

        Ok(())
    }
}

#[cfg(test)]
mod test_restore_folder {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, request_count, short_retry},
    };

    /// フォルダのAPIのパス
    const FOLDER_PATH: &str = "/2.0/folders/123";

    /// 復元したフォルダのレスポンスを返す。
    fn restored_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "folder",
            "id": "123",
            "etag": "2",
            "name": "restored",
            "trashed_at": null,
            "purged_at": null,
            "parent": {"type": "folder", "id": "111", "etag": "0", "name": "Legal"},
            "item_status": "active",
        }))
    }

    /// フォルダのAPIへのPOSTリクエストの数を返す。
    async fn post_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == FOLDER_PATH)
            .count()
    }

    /// `POST /folders/{folder_id}`にオプションをボディとして送信し、復元したフォルダ情報を返すことを確認
    #[tokio::test]
    async fn test_restore_folder() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .and(body_json(
                json!({"name": "restored", "parent": {"id": "111"}}),
            ))
            .respond_with(restored_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let parent_id = FolderId::new("111");
        let options = RestoreItemOptions {
            name: Some("restored"),
            parent_id: Some(&parent_id),
        };

        let folder = client
            .restore_folder(&FolderId::new("123"), &options)
            .await?;

        assert_eq!(folder.id, FolderId::new("123"));
        assert_eq!(folder.name, "restored");
        assert_eq!(folder.parent.map(|p| p.id), Some(parent_id));
        assert_eq!(folder.trashed_at, None);
        assert_eq!(folder.item_status, Some(ItemStatus::Active));

        Ok(())
    }

    /// ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_restore_folder_rejects_root() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let requests_before = request_count(&server).await;

        let result = client
            .restore_folder(&FolderId::root(), &RestoreItemOptions::default())
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
    /// 5xxの後に再送すると、復元済みのフォルダに対して404を受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::conflict_without_code(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_restore_folder_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .restore_folder(&FolderId::new("123"), &RestoreItemOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(post_count(&server).await, 1);

        Ok(())
    }

    /// 他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[tokio::test]
    async fn test_restore_folder_retries_operation_blocked_temporary() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "type": "error",
                "status": 409,
                "code": "operation_blocked_temporary",
                "message": "The operation is blocked by another on-going operation",
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(restored_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client
            .restore_folder(&FolderId::new("123"), &RestoreItemOptions::default())
            .await?;

        assert_eq!(post_count(&server).await, 2);

        Ok(())
    }

    /// 名前の重複で409を受け取った場合は、再試行せずにエラーを返すことを確認
    #[tokio::test]
    async fn test_restore_folder_name_conflict_is_not_retried() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "type": "error",
                "status": 409,
                "code": "item_name_in_use",
                "message": "Item with the same name already exists",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .restore_folder(&FolderId::new("123"), &RestoreItemOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e))
                if e.status == 409 && e.code.as_deref() == Some("item_name_in_use")),
            "result: {result:?}"
        );
        assert_eq!(post_count(&server).await, 1);

        Ok(())
    }

    /// 429の場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[tokio::test]
    async fn test_restore_folder_retries_too_many_requests() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(FOLDER_PATH))
            .and(body_json(json!({})))
            .respond_with(restored_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client
            .restore_folder(&FolderId::new("123"), &RestoreItemOptions::default())
            .await?;

        assert_eq!(post_count(&server).await, 2);

        Ok(())
    }
}

#[cfg(test)]
mod test_restore_file {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, short_retry},
    };

    /// ファイルのAPIのパス
    const FILE_PATH: &str = "/2.0/files/123";

    /// 復元したファイルのレスポンスを返す。
    fn restored_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "file",
            "id": "123",
            "etag": "2",
            "name": "restored.pdf",
            "trashed_at": null,
            "purged_at": null,
            "shared_link": null,
            "parent": {"type": "folder", "id": "111", "etag": "0", "name": "Legal"},
            "item_status": "active",
        }))
    }

    /// ファイルのAPIへのPOSTリクエストの数を返す。
    async fn post_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == FILE_PATH)
            .count()
    }

    /// `POST /files/{file_id}`にオプションをボディとして送信し、復元したファイル情報を返すことを確認
    #[tokio::test]
    async fn test_restore_file() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FILE_PATH))
            .and(body_json(
                json!({"name": "restored.pdf", "parent": {"id": "111"}}),
            ))
            .respond_with(restored_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let parent_id = FolderId::new("111");
        let options = RestoreItemOptions {
            name: Some("restored.pdf"),
            parent_id: Some(&parent_id),
        };

        let file = client.restore_file(&FileId::new("123"), &options).await?;

        assert_eq!(file.id, FileId::new("123"));
        assert_eq!(file.name, "restored.pdf");
        assert_eq!(file.parent.map(|p| p.id), Some(parent_id));
        assert_eq!(file.trashed_at, None);
        assert_eq!(file.item_status, Some(ItemStatus::Active));

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、復元済みのファイルに対して404を受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::name_conflict(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_restore_file_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FILE_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .restore_file(&FileId::new("123"), &RestoreItemOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(post_count(&server).await, 1);

        Ok(())
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(ResponseTemplate::new(409).set_body_json(json!({
        "type": "error",
        "status": 409,
        "code": "operation_blocked_temporary",
        "message": "The operation is blocked by another on-going operation",
    })))]
    #[tokio::test]
    async fn test_restore_file_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(FILE_PATH))
            .respond_with(first_response)
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(FILE_PATH))
            .and(body_json(json!({})))
            .respond_with(restored_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client
            .restore_file(&FileId::new("123"), &RestoreItemOptions::default())
            .await?;

        assert_eq!(post_count(&server).await, 2);

        Ok(())
    }
}

#[cfg(test)]
mod test_restore_web_link {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    use super::*;
    use crate::{
        ItemStatus,
        test_support::{create_client, mount_token, short_retry},
    };

    /// WebリンクのAPIのパス
    const WEB_LINK_PATH: &str = "/2.0/web_links/123";

    /// 復元したWebリンクのレスポンスを返す。
    fn restored_response() -> ResponseTemplate {
        ResponseTemplate::new(201).set_body_json(json!({
            "type": "web_link",
            "id": "123",
            "etag": "2",
            "name": "restored",
            "url": "https://www.example.com/",
            "trashed_at": null,
            "purged_at": null,
            "shared_link": null,
            "parent": {"type": "folder", "id": "111", "etag": "0", "name": "Links"},
            "item_status": "active",
        }))
    }

    /// WebリンクのAPIへのPOSTリクエストの数を返す。
    async fn post_count(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == WEB_LINK_PATH)
            .count()
    }

    /// `POST /web_links/{web_link_id}`にオプションをボディとして送信し、復元したWebリンク情報を返すことを確認
    #[tokio::test]
    async fn test_restore_web_link() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(WEB_LINK_PATH))
            .and(body_json(
                json!({"name": "restored", "parent": {"id": "111"}}),
            ))
            .respond_with(restored_response())
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let parent_id = FolderId::new("111");
        let options = RestoreItemOptions {
            name: Some("restored"),
            parent_id: Some(&parent_id),
        };

        let web_link = client
            .restore_web_link(&WebLinkId::new("123"), &options)
            .await?;

        assert_eq!(web_link.id, WebLinkId::new("123"));
        assert_eq!(web_link.name, "restored");
        assert_eq!(web_link.url.as_deref(), Some("https://www.example.com/"));
        assert_eq!(web_link.parent.map(|p| p.id), Some(parent_id));
        assert_eq!(web_link.trashed_at, None);
        assert_eq!(web_link.item_status, Some(ItemStatus::Active));

        Ok(())
    }

    /// BOXのエラーレスポンスを、再試行せずにそのまま返すことを確認
    ///
    /// 5xxの後に再送すると、復元済みのWebリンクに対して404を受け取るおそれがあるため、
    /// 再試行の上限に余裕があっても再送しない。
    #[rstest]
    #[case::forbidden(403)]
    #[case::not_found(404)]
    #[case::name_conflict(409)]
    #[case::internal_server_error(500)]
    #[case::service_unavailable(503)]
    #[tokio::test]
    async fn test_restore_web_link_error_is_not_retried(#[case] status: u16) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(WEB_LINK_PATH))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(2)?).await?;

        let result = client
            .restore_web_link(&WebLinkId::new("123"), &RestoreItemOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == status),
            "result: {result:?}"
        );
        assert_eq!(post_count(&server).await, 1);

        Ok(())
    }

    /// 429または他の操作によるロックで409を受け取った場合は、冪等でないリクエストとして送信しても再試行することを確認
    #[rstest]
    #[case::too_many_requests(ResponseTemplate::new(429))]
    #[case::operation_blocked_temporary(ResponseTemplate::new(409).set_body_json(json!({
        "type": "error",
        "status": 409,
        "code": "operation_blocked_temporary",
        "message": "The operation is blocked by another on-going operation",
    })))]
    #[tokio::test]
    async fn test_restore_web_link_retries_request_not_processed(
        #[case] first_response: ResponseTemplate,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("POST"))
            .and(path(WEB_LINK_PATH))
            .respond_with(first_response)
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(WEB_LINK_PATH))
            .and(body_json(json!({})))
            .respond_with(restored_response())
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(1)?).await?;

        client
            .restore_web_link(&WebLinkId::new("123"), &RestoreItemOptions::default())
            .await?;

        assert_eq!(post_count(&server).await, 2);

        Ok(())
    }
}

#[cfg(test)]
mod test_list_trashed_items {
    use rstest::rstest;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::{
        BoxClientError,
        test_support::{create_client, mount_token, short_retry},
    };

    /// ゴミ箱の項目のAPIのパス
    const TRASH_ITEMS_PATH: &str = "/2.0/folders/trash/items";

    /// ゴミ箱の項目のAPIへのリクエストのクエリ文字列を返す。
    async fn trash_queries(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path() == TRASH_ITEMS_PATH)
            .map(|r| r.url.query().unwrap_or_default().to_owned())
            .collect()
    }

    /// `GET /folders/trash/items`を呼び出し、マーカーとオプションをクエリパラメーターに変換することを確認
    #[rstest]
    #[case::default(None, ListTrashedItemsOptions::default(), "usemarker=true")]
    #[case::min_limit(
        None,
        ListTrashedItemsOptions { limit: Some(1) },
        "usemarker=true&limit=1"
    )]
    #[case::all(
        Some("m1"),
        ListTrashedItemsOptions { limit: Some(1000) },
        "usemarker=true&limit=1000&marker=m1"
    )]
    #[tokio::test]
    async fn test_list_trashed_items(
        #[case] marker: Option<&str>,
        #[case] options: ListTrashedItemsOptions,
        #[case] expected_query: &str,
    ) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [{"type": "folder", "id": "2", "name": "docs", "etag": "1"}],
                "limit": 100,
                "next_marker": "m2",
            })))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let page = client.list_trashed_items(marker, &options).await?;

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_marker.as_deref(), Some("m2"));
        assert_eq!(trash_queries(&server).await, vec![expected_query]);

        Ok(())
    }

    /// 項目数の上限が範囲外の場合は、リクエストを送信せずにエラーを返すことを確認
    #[rstest]
    #[case::zero(0)]
    #[case::over_max(1001)]
    #[tokio::test]
    async fn test_list_trashed_items_invalid_limit(#[case] limit: u32) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListTrashedItemsOptions { limit: Some(limit) };

        let result = client.list_trashed_items(None, &options).await;

        assert!(
            matches!(result, Err(BoxClientError::InvalidArgument(_))),
            "result: {result:?}"
        );
        assert_eq!(trash_queries(&server).await, Vec::<String>::new());

        Ok(())
    }
}

#[cfg(test)]
mod test_list_all_trashed_items {
    use rstest::rstest;
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    use super::*;
    use crate::{
        BoxClientError, FolderId, FolderItem,
        test_support::{create_client, mount_token, short_retry},
    };

    /// ゴミ箱の項目のAPIのパス
    const TRASH_ITEMS_PATH: &str = "/2.0/folders/trash/items";

    /// 指定したIDのフォルダだけを含むページのレスポンスを返す。
    fn page_response(id: &str, next_marker: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "entries": [{"type": "folder", "id": id, "name": id, "etag": "0"}],
            "limit": 1000,
            "next_marker": next_marker,
        }))
    }

    /// 指定したIDのフォルダの項目を返す。
    fn folder(id: &str) -> BoxItem {
        BoxItem::Folder(FolderItem {
            id: FolderId::new(id),
            name: id.to_owned(),
            etag: Some("0".to_owned()),
        })
    }

    /// `next_marker`がnullまたは空文字列になるまでページを送り、すべての項目を返すことを確認
    #[rstest]
    #[case::null(Value::Null)]
    #[case::empty(json!(""))]
    #[tokio::test]
    async fn test_list_all_trashed_items(#[case] last_marker: Value) -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        // 項目数の上限を省略した場合は、上限の1000件ずつ取得する。
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", &json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1000"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", &last_marker))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let items = client
            .list_all_trashed_items(&ListTrashedItemsOptions::default())
            .await?;

        assert_eq!(items, vec![folder("first"), folder("second")]);

        Ok(())
    }

    /// 項目数の上限を指定した場合は、すべてのページの取得に使用することを確認
    #[tokio::test]
    async fn test_list_all_trashed_items_with_options() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1"))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", &json!("m1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("limit", "1"))
            .and(query_param("marker", "m1"))
            .respond_with(page_response("second", &Value::Null))
            .expect(1)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;
        let options = ListTrashedItemsOptions { limit: Some(1) };

        let items = client.list_all_trashed_items(&options).await?;

        assert_eq!(items, vec![folder("first"), folder("second")]);

        Ok(())
    }

    /// BOXが同じマーカーを続けて返す場合は、無限ループせずにエラーを返すことを確認
    #[tokio::test]
    async fn test_list_all_trashed_items_marker_not_advanced() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .respond_with(page_response("same", &json!("m1")))
            .expect(2)
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_trashed_items(&ListTrashedItemsOptions::default())
            .await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );

        Ok(())
    }

    /// いずれかのページの取得に失敗した場合は、BOXのエラーレスポンスを返すことを確認
    #[tokio::test]
    async fn test_list_all_trashed_items_page_error() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        mount_token(&server).await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param_is_missing("marker"))
            .respond_with(page_response("first", &json!("m1")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(TRASH_ITEMS_PATH))
            .and(query_param("marker", "m1"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let client = create_client(&server, short_retry(0)?).await?;

        let result = client
            .list_all_trashed_items(&ListTrashedItemsOptions::default())
            .await;

        assert!(
            matches!(&result, Err(BoxClientError::Api(e)) if e.status == 403),
            "result: {result:?}"
        );

        Ok(())
    }
}
