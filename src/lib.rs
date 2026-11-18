//! BOXのREST APIを呼び出すクライアント
//!
//! [`BoxClient`]は、JWT認証でエンタープライズのサービスアカウントとして認証し、BOXのREST APIを呼び出す。
//! アクセストークンの取得と更新、および失敗したリクエストの再試行は、[`BoxClient`]が自動で行う。
//! 対応しているAPI、アクセストークンの管理および再試行の仕組みは、リポジトリのREADMEを参照する。
//!
//! ```no_run
//! use box_client::{BoxClient, BoxConfig, FolderId};
//!
//! # async fn example() -> box_client::BoxClientResult<()> {
//! let config = BoxConfig::from_file("config.json")?;
//! let client = BoxClient::builder(config).build().await?;
//!
//! let folder = client.folder_information(&FolderId::root()).await?;
//! println!("{}", folder.name);
//! # Ok(())
//! # }
//! ```

mod api;
mod assertion_claims;
mod auth;
mod client;
mod config;
mod errors;
mod request;
mod retry;

#[cfg(test)]
mod test_support;

pub use api::files::{File, FileVersion};
pub use api::folders::{
    DeleteFolderOptions, Folder, ItemSort, ItemsPage, ListItemsOptions,
    MAX_FOLDER_DESCRIPTION_CHARS, MAX_ITEMS_LIMIT, MoveFolderDestination, SortDirection,
    UpdateFolderOptions,
};
pub use api::trash::{ListTrashedItemsOptions, RestoreItemOptions};
pub use api::types::{
    BoxItem, FileId, FileItem, FolderId, FolderItem, ItemStatus, ParentFolder, PathCollection,
    UserId, WebLinkId, WebLinkItem,
};
pub use api::users::{User, UserMini};
pub use api::web_links::WebLink;
pub use client::{BoxClient, BoxClientBuilder, BoxClientTimeout, MAX_REFRESH_BEFORE_EXPIRY};
pub use config::{BoxAppAuth, BoxAppSettings, BoxConfig};
pub use errors::{ApiError, BoxClientError, BoxClientResult, CryptoError, ValidationError};
pub use request::ApiRequest;
pub use retry::{Idempotency, RetryPolicy};
