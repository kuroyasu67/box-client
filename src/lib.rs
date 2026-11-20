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

pub use api::collaborations::{
    AcceptanceRequirementsStatus, AppItem, Collaboration, CollaborationGrantee, CollaborationItem,
    CollaborationRole, CollaborationStatus, CollaborationUpdate, CollaborationUpdateOutcome,
    CollaborationUser, CollaborationsOffsetPage, CollaborationsPage, Collaborator,
    CreateCollaborationOptions, ListFileCollaborationsOptions, ListFolderCollaborationsOptions,
    ListGroupCollaborationsOptions, ListPendingCollaborationsOptions, MAX_COLLABORATIONS_OFFSET,
    StrongPasswordRequirement, TermsOfService, TermsOfServiceRequirement,
    TwoFactorAuthenticationRequirement,
};
pub use api::downloads::DownloadFileOptions;
pub use api::files::{CopyFileOptions, File, FileDownloadPermission, FileLock, FileUpdate};
pub use api::folders::{
    CopyFolderOptions, DeleteFolderOptions, Folder, FolderUpdate, FolderUploadEmailAccess,
    ListItemsOptions, SyncState,
};
pub use api::groups::{
    CreateGroupOptions, Group, GroupAccessLevel, GroupMini, GroupPermissions, GroupType,
    GroupUpdate, GroupsPage, ListEnterpriseGroupsOptions, MAX_GROUP_DESCRIPTION_CHARS,
    MAX_GROUP_PROVENANCE_CHARS, MAX_LISTED_GROUPS,
};
pub use api::pagination::{ItemSort, ItemsPage, MAX_ITEMS_LIMIT, SortDirection};
pub use api::trash::{ListTrashedItemsOptions, RestoreItemOptions};
pub use api::types::{
    AppItemId, BoxItem, CollaborationId, DeleteItemOptions, FileId, FileItem, FileVersionId,
    FolderId, FolderItem, GroupId, ItemStatus, MAX_DESCRIPTION_CHARS, MAX_NAME_CHARS,
    MoveDestination, PathCollection, TermsOfServiceId, UpdateItemOptions, UserId, WebLinkId,
    WebLinkItem,
};
pub use api::updates::{
    MAX_TAGS, Nullable, OneTimePasscode, OtpAllowlistEntry, SharedLinkAccess, SharedLinkSettings,
};
pub use api::uploads::{UpdateFileContentOptions, UploadFileOptions, UploadUrl};
pub use api::users::{
    CreateUserOptions, DeleteUserOptions, ListEnterpriseUsersOptions, MAX_FILTERED_USERS,
    MAX_USER_ADDRESS_CHARS, MAX_USER_JOB_TITLE_CHARS, MAX_USER_NAME_CHARS, MAX_USER_PHONE_CHARS,
    NewUserAccount, NotificationEmail, TrackingCode, UNLIMITED_SPACE_AMOUNT, User, UserMini,
    UserRole, UserStatus, UserTypeFilter, UserUpdate, UsersPage,
};
pub use api::versions::{
    FileVersion, FileVersionsPage, ListFileVersionsOptions, MAX_FILE_VERSIONS_OFFSET,
};
pub use api::web_links::{CreateWebLinkOptions, WebLink, WebLinkUpdate};
pub use api::zip_downloads::{
    CreateZipDownloadOptions, MAX_ZIP_DOWNLOAD_ITEMS, ZipDownload, ZipDownloadItem,
    ZipDownloadItemType, ZipDownloadNameConflict, ZipDownloadState, ZipDownloadStatus,
    ZipDownloadSummary,
};
pub use client::{
    BoxClient, BoxClientBuilder, DEFAULT_CHUNKED_UPLOAD_THRESHOLD, DEFAULT_MAX_COMMIT_POLLS,
    DEFAULT_MAX_DOWNLOAD_POLLS, DEFAULT_REFRESH_BEFORE_EXPIRY, DEFAULT_UPLOAD_CONCURRENCY,
    MAX_CHUNKED_UPLOAD_THRESHOLD, MAX_COMMIT_POLLS_LIMIT, MAX_DOWNLOAD_POLLS_LIMIT,
    MAX_REFRESH_BEFORE_EXPIRY, MAX_UPLOAD_CONCURRENCY, MIN_CHUNKED_UPLOAD_THRESHOLD,
    MIN_UPLOAD_CONCURRENCY, TimeoutConfig,
};
pub use config::{BoxAppAuth, BoxAppSettings, BoxConfig};
pub use errors::{ApiError, BoxClientError, BoxClientResult, CryptoError, ValidationError};
pub use request::ApiRequest;
pub use retry::{Idempotency, RetryPolicy};
