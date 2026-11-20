//! ページ送りに関する共通の実装
//!
//! BOXの一覧を取得するAPIは、マーカー方式またはオフセット方式でページを送る。
//! このモジュールは、両方式のリクエストの作成、オフセットの検証、およびすべてのページの取得を共有する。

use std::collections::HashSet;

use serde::Deserialize;

use crate::{BoxClientError, BoxClientResult, BoxItem, request::ApiRequest};

/// 1回のリクエストで取得できる項目数の上限を表す整数リテラルに展開する。
///
/// `concat!`は定数を受け付けないため、[`MAX_ITEMS_LIMIT`]とエラーのメッセージで、このマクロを共有する。
/// これにより、上限を変更したときに、メッセージの値も追随させる。
macro_rules! max_items_limit {
    () => {
        1000
    };
}

/// 1回のリクエストで取得できる項目数の上限
pub const MAX_ITEMS_LIMIT: u32 = max_items_limit!();

/// オフセット方式のページ送りで、BOXが受け付けるオフセットの最大値を表す整数リテラルに展開する。
///
/// `concat!`は定数を受け付けないため、[`MAX_OFFSET`]とエラーのメッセージで、このマクロを共有する。
/// これにより、最大値を変更したときに、メッセージの値も追随させる。
macro_rules! max_offset {
    () => {
        10_000
    };
}

/// オフセット方式のページ送りで、BOXが受け付けるオフセットの最大値
///
/// BOXは、オフセット方式でページを送るどのAPIでも、オフセットがこの値を超えるリクエストを400で拒否する。
/// そのため、BOXクライアントは、オフセット方式ですべてのページを取得するメソッドで、先頭からこの件数までのエントリーを返す。
/// 公開している`MAX_*_OFFSET`および`MAX_LISTED_*`などの定数は、すべてこの値である。
pub(crate) const MAX_OFFSET: u64 = max_offset!();

/// 項目の並び替えに使用する属性
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemSort {
    /// ID
    Id,

    /// 名前
    Name,

    /// 日時
    ///
    /// BOXのAPIリファレンスは、この属性がどの日時を指すかを記載していない。
    /// 2026年11月18日に確認したところ、BOXは、フォルダ内の項目を、作成日時（`created_at`）および
    /// 更新日時（`modified_at`）ではなく、内容の更新日時（`content_modified_at`）の順に並べた。
    /// 例えば、ファイルの説明を設定すると更新日時は変わるが、内容の更新日時は変わらないため、並び順も変わらない。
    /// ゴミ箱内の項目では、BOXクライアントが使用するマーカー方式のページ送りでBOXが400を返すため、指定できない。
    Date,

    /// サイズ
    Size,
}

impl ItemSort {
    /// クエリパラメーターに指定する文字列を返す。
    fn as_str(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::Date => "date",
            Self::Size => "size",
        }
    }
}

/// 並び替えの方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    /// 昇順
    Asc,

    /// 降順
    Desc,
}

impl SortDirection {
    /// クエリパラメーターに指定する文字列を返す。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// フォルダ内またはゴミ箱内の項目の1ページ
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct ItemsPage {
    /// このページの項目
    pub entries: Vec<BoxItem>,

    /// このページの取得に使用した項目数の上限
    pub limit: u32,

    /// 次のページを取得するためのマーカー
    ///
    /// 後続の項目がない場合は`None`になる。
    pub next_marker: Option<String>,
}

/// マーカー方式のページ送りで取得する1ページ
///
/// [`collect_all_pages`]が、ページの種類によらず、エントリーと次のページのマーカーを取り出すために使用する。
pub(crate) trait MarkerPage {
    /// ページに含まれるエントリーの型
    type Entry;

    /// ページを、エントリーと次のページを取得するためのマーカーに分解する。
    fn into_parts(self) -> (Vec<Self::Entry>, Option<String>);
}

impl MarkerPage for ItemsPage {
    type Entry = BoxItem;

    fn into_parts(self) -> (Vec<BoxItem>, Option<String>) {
        (self.entries, self.next_marker)
    }
}

/// オフセット方式のページ送りで取得する1ページ
///
/// [`collect_offset_pages_up_to`]が、ページの種類によらず、エントリーと総数を取り出すために使用する。
pub(crate) trait OffsetPage {
    /// ページに含まれるエントリーの型
    type Entry;

    /// ページを、エントリーと`total_count`に分解する。
    fn into_parts(self) -> (Vec<Self::Entry>, u64);
}

/// 1ページで取得する項目数の上限が、BOXが受け付ける範囲内であることを検証する。
///
/// # Errors
///
/// `limit`が1以上[`MAX_ITEMS_LIMIT`]以下でない場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn validate_limit(limit: u32) -> BoxClientResult<u32> {
    if (1..=MAX_ITEMS_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(BoxClientError::InvalidArgument(concat!(
            "limit must be between 1 and ",
            max_items_limit!()
        )))
    }
}

/// オフセットが、BOXが受け付ける最大値（[`MAX_OFFSET`]）以下であることを検証する。
///
/// 1ページ分を取得するメソッドが、BOXが受け付けるオフセットをそのまま受け付ける場合に使用する。
///
/// # Errors
///
/// `offset`が[`MAX_OFFSET`]を超える場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn validate_offset(offset: u64) -> BoxClientResult<u64> {
    if MAX_OFFSET < offset {
        return Err(BoxClientError::InvalidArgument(concat!(
            "offset must be less than or equal to ",
            max_offset!()
        )));
    }
    Ok(offset)
}

/// オフセットが、取得できるエントリー数の上限（[`MAX_OFFSET`]）より小さいことを検証する。
///
/// 1ページ分を取得するメソッドも、先頭から[`MAX_OFFSET`]件までしか返さない場合に使用する。
/// それ以降のエントリーは返さないため、[`MAX_OFFSET`]以上のオフセットを受け付けない。
///
/// # Errors
///
/// `offset`が[`MAX_OFFSET`]以上の場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn validate_listed_offset(offset: u64) -> BoxClientResult<u64> {
    if MAX_OFFSET <= offset {
        return Err(BoxClientError::InvalidArgument(concat!(
            "offset must be less than ",
            max_offset!()
        )));
    }
    Ok(offset)
}

/// 先頭から`max_entries`件までしか返さないように、`offset`から始まるページの、上限を超えるエントリーを取り除く。
pub(crate) fn truncate_to_max_entries<T>(entries: &mut Vec<T>, offset: u64, max_entries: u64) {
    entries.truncate(usize::try_from(max_entries.saturating_sub(offset)).unwrap_or(usize::MAX));
}

/// オフセット方式のページ送りで、1ページ分のエントリーを取得するリクエストを作成する。
///
/// `offset`はクエリパラメーター`offset`で常に送信し、`limit`は指定した場合だけ送信する。
/// `offset`は呼び出し元が検証する。APIによって、[`validate_offset`]と[`validate_listed_offset`]を使い分けるためである。
///
/// # Errors
///
/// `limit`が範囲外の場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn offset_page_request<'a>(
    path: impl IntoIterator<Item = &'a str>,
    offset: u64,
    limit: Option<u32>,
) -> BoxClientResult<ApiRequest<'a>> {
    let mut request = ApiRequest::get(path).query("offset", offset.to_string());
    if let Some(limit) = limit {
        request = request.query("limit", validate_limit(limit)?.to_string());
    }
    Ok(request)
}

/// マーカー方式のページ送りで、1ページ分の項目を取得するリクエストを作成する。
///
/// マーカー方式でページを送るすべてのAPIで共有する。
/// 指定した引数だけを、同じ名前のクエリパラメーター（`limit`、`marker`、`sort`および`direction`）で送信する。
///
/// # Arguments
///
/// - `path`: ベースURLに続けるパス要素
/// - `marker`: 前のページの`next_marker`。最初のページを取得する場合は`None`
/// - `limit`: 1ページで取得する項目数の上限
/// - `sort`: 並び替えに使用する属性
/// - `direction`: 並び替えの方向
///
/// # Errors
///
/// `limit`が範囲外の場合は、[`BoxClientError::InvalidArgument`]を返す。
pub(crate) fn marker_page_request<'a>(
    path: impl IntoIterator<Item = &'a str>,
    marker: Option<&'a str>,
    limit: Option<u32>,
    sort: Option<ItemSort>,
    direction: Option<SortDirection>,
) -> BoxClientResult<ApiRequest<'a>> {
    let mut request = ApiRequest::get(path).query("usemarker", "true");
    if let Some(limit) = limit {
        request = request.query("limit", validate_limit(limit)?.to_string());
    }
    if let Some(marker) = marker {
        request = request.query("marker", marker);
    }
    if let Some(sort) = sort {
        request = request.query("sort", sort.as_str());
    }
    if let Some(direction) = direction {
        request = request.query("direction", direction.as_str());
    }
    Ok(request)
}

/// マーカー方式のページ送りで、すべてのページのエントリーを取得する。
///
/// `fetch_page`には前のページの`next_marker`を渡す。最初のページでは`None`を渡す。
/// 戻り値は、すべてのエントリーと取得したページ数である。
///
/// # Errors
///
/// BOXが以前に返したマーカーを再び返した場合、この関数は無限ループを避けるため[`BoxClientError::UnexpectedResponse`]を返す。
/// `fetch_page`がエラーを返した場合、この関数は取得済みのエントリーを破棄してそのエラーを返す。
pub(crate) async fn collect_all_pages<P, F>(
    mut fetch_page: F,
) -> BoxClientResult<(Vec<P::Entry>, u32)>
where
    P: MarkerPage,
    F: AsyncFnMut(Option<&str>) -> BoxClientResult<P>,
{
    let mut items = Vec::new();
    let mut marker: Option<String> = None;
    // `m1 → m2 → m1`のように、BOXが以前のマーカーを再び返す循環も検出するため、使用したマーカーを記録する。
    // 現在のマーカーは`marker`が保持するため、このセットには現在のマーカーより前に使用したマーカーだけを記録する。
    let mut used_markers: HashSet<String> = HashSet::new();
    let mut pages: u32 = 0;

    loop {
        let (entries, next_marker) = fetch_page(marker.as_deref()).await?.into_parts();
        pages += 1;
        items.extend(entries);

        // BOXは最終ページで`next_marker`をnullにするが、空文字列を返す場合もあるため、どちらも終端として扱う。
        match next_marker {
            Some(next) if !next.is_empty() => {
                if marker.as_deref() == Some(next.as_str()) || used_markers.contains(&next) {
                    return Err(BoxClientError::UnexpectedResponse(
                        "next_marker was already returned",
                    ));
                }
                if let Some(previous) = marker.replace(next) {
                    used_markers.insert(previous);
                }
            }
            _ => break,
        }
    }

    Ok((items, pages))
}

/// オフセット方式のページ送りで、先頭から`max_entries`件までのエントリーを取得する。
///
/// オフセット方式でページを送るすべてのAPIで共有する。BOXはオフセットに上限（[`MAX_OFFSET`]）を設けているため、
/// 上限より後ろのエントリーを取得できない。そこで、エントリーが`max_entries`件より多くてもエラーにせず、
/// 取得済みのエントリーを捨てずに先頭から`max_entries`件を返す。
/// `fetch_page`には取得するページの最初のエントリーのオフセットを渡す。最初のページでは`0`を渡す。
/// 取得したエントリーの数が`total_count`もしくは`max_entries`に達した場合、またはBOXが空のページを返した場合に終了する。
/// 戻り値は、取得したエントリーと取得したページ数である。
///
/// # Arguments
///
/// - `max_entries`: 取得するエントリーの最大数
/// - `fetch_page`: 指定したオフセットから1ページ分を取得するクロージャー
///
/// # Errors
///
/// `fetch_page`がエラーを返した場合、この関数は取得済みのエントリーを破棄してそのエラーを返す。
pub(crate) async fn collect_offset_pages_up_to<P, F>(
    max_entries: u64,
    mut fetch_page: F,
) -> BoxClientResult<(Vec<P::Entry>, u32)>
where
    P: OffsetPage,
    F: AsyncFnMut(u64) -> BoxClientResult<P>,
{
    let mut entries = Vec::new();
    let mut offset = 0;
    let mut pages: u32 = 0;
    while offset < max_entries {
        let (mut page_entries, total_count) = fetch_page(offset).await?.into_parts();
        pages += 1;
        // 最後のページの、上限を超えるエントリーを取り除く。
        truncate_to_max_entries(&mut page_entries, offset, max_entries);
        let count = page_entries.len() as u64;
        entries.extend(page_entries);
        // BOXが返す`offset`ではなく、要求した`offset`から次のページの位置を求める。
        // BOXが`offset`を無視して同じページを返し続けても、位置を進めて必ずループを終える。
        offset += count;

        if count == 0 || total_count <= offset {
            break;
        }
    }

    Ok((entries, pages))
}

#[cfg(test)]
mod test_items_page {
    use serde_json::json;

    use crate::{
        FileItem, FolderItem, WebLinkItem,
        api::types::{FileId, FolderId, WebLinkId},
    };

    use super::*;

    /// type属性に従って項目をデシリアライズし、知らない種類の項目を受け入れることを確認
    #[test]
    fn test_deserialize() -> anyhow::Result<()> {
        let page: ItemsPage = serde_json::from_value(json!({
            "entries": [
                {"type": "file", "id": "1", "name": "a.txt", "etag": "0"},
                {"type": "folder", "id": "2", "name": "docs", "etag": null},
                {"type": "web_link", "id": "3", "name": "link", "etag": "1"},
                {"type": "hubs", "id": "4", "name": "hub"},
            ],
            "limit": 100,
            "next_marker": "m1",
        }))?;

        assert_eq!(
            page,
            ItemsPage {
                entries: vec![
                    BoxItem::File(FileItem {
                        id: FileId::new("1"),
                        name: "a.txt".to_owned(),
                        etag: Some("0".to_owned()),
                    }),
                    BoxItem::Folder(FolderItem {
                        id: FolderId::new("2"),
                        name: "docs".to_owned(),
                        etag: None,
                    }),
                    BoxItem::WebLink(WebLinkItem {
                        id: WebLinkId::new("3"),
                        name: "link".to_owned(),
                        etag: Some("1".to_owned()),
                    }),
                    BoxItem::Unknown,
                ],
                limit: 100,
                next_marker: Some("m1".to_owned()),
            }
        );

        Ok(())
    }

    /// `next_marker`がnullの場合と省略された場合に、後続がないと判定することを確認
    #[test]
    fn test_deserialize_without_next_marker() -> anyhow::Result<()> {
        let null_marker: ItemsPage =
            serde_json::from_value(json!({"entries": [], "limit": 100, "next_marker": null}))?;
        let missing_marker: ItemsPage =
            serde_json::from_value(json!({"entries": [], "limit": 100}))?;

        assert_eq!(null_marker.next_marker, None);
        assert_eq!(missing_marker.next_marker, None);

        Ok(())
    }
}

#[cfg(test)]
mod test_validate_limit {
    use rstest::rstest;

    use super::*;

    /// 1以上1000以下の項目数の上限だけを受け付けることを確認
    #[rstest]
    #[case::zero(0, false)]
    #[case::min(1, true)]
    #[case::max(1000, true)]
    #[case::over_max(1001, false)]
    fn test_validate_limit(#[case] limit: u32, #[case] valid: bool) {
        let result = validate_limit(limit);

        if valid {
            assert!(matches!(result, Ok(l) if l == limit), "result: {result:?}");
        } else {
            assert!(
                matches!(result, Err(BoxClientError::InvalidArgument(_))),
                "result: {result:?}"
            );
        }
    }
}

#[cfg(test)]
mod test_collect_all_pages {
    use rstest::rstest;

    use super::*;

    /// `next_markers`の順に`next_marker`を返すページを取得し、`fetch_page`に渡されたマーカーを記録する。
    ///
    /// 用意したページを使い切った後に呼び出された場合は、無限ループでテストが終わらなくなるのを防ぐため、エラーを返す。
    async fn collect(
        next_markers: &[Option<&str>],
    ) -> (BoxClientResult<(Vec<BoxItem>, u32)>, Vec<Option<String>>) {
        let mut received_markers = Vec::new();
        let mut remaining = next_markers.iter();
        let result = collect_all_pages(async |marker: Option<&str>| {
            received_markers.push(marker.map(str::to_owned));
            let Some(next_marker) = remaining.next() else {
                return Err(BoxClientError::InvalidArgument("no more pages"));
            };
            Ok(ItemsPage {
                entries: vec![BoxItem::Unknown],
                limit: 1,
                next_marker: next_marker.map(str::to_owned),
            })
        })
        .await;

        (result, received_markers)
    }

    /// 終端までマーカーが重複しない場合は、すべてのページの項目とページ数を返し、前のページのマーカーを渡すことを確認
    #[rstest]
    #[case::single_page(vec![None], vec![None])]
    #[case::empty_marker_terminates(vec![Some("")], vec![None])]
    #[case::three_pages(
        vec![Some("m1"), Some("m2"), None],
        vec![None, Some("m1"), Some("m2")]
    )]
    #[tokio::test]
    async fn test_collect_all_pages(
        #[case] next_markers: Vec<Option<&str>>,
        #[case] expected_markers: Vec<Option<&str>>,
    ) -> anyhow::Result<()> {
        let (result, received_markers) = collect(&next_markers).await;

        let (items, pages) = result?;
        assert_eq!(items.len(), next_markers.len());
        assert_eq!(pages as usize, next_markers.len());
        assert_eq!(
            received_markers,
            expected_markers
                .into_iter()
                .map(|m| m.map(str::to_owned))
                .collect::<Vec<_>>()
        );

        Ok(())
    }

    /// BOXが以前に返したマーカーを再び返した場合は、そのマーカーでページを取得せずにエラーを返すことを確認
    #[rstest]
    // 直前のマーカーと同じマーカーを返す
    #[case::same_as_previous(vec![Some("m1"), Some("m1")], 2)]
    // 2つ前のマーカーに戻る
    #[case::two_page_cycle(vec![Some("m1"), Some("m2"), Some("m1")], 3)]
    // 最初のマーカーに戻る
    #[case::back_to_first(vec![Some("m1"), Some("m2"), Some("m3"), Some("m1")], 4)]
    #[tokio::test]
    async fn test_collect_all_pages_detects_repeated_marker(
        #[case] next_markers: Vec<Option<&str>>,
        #[case] expected_fetches: usize,
    ) {
        let (result, received_markers) = collect(&next_markers).await;

        assert!(
            matches!(result, Err(BoxClientError::UnexpectedResponse(_))),
            "result: {result:?}"
        );
        assert_eq!(received_markers.len(), expected_fetches);
    }
}
