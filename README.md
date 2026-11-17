# BOXクライアントクレート

## 対応APIリスト

`BoxClient`のメソッドから呼び出せるBOXのREST APIを、リソースごとに示す。

### ユーザー

#### 認証中のユーザーの取得

- メソッド: `BoxClient::me`
- BOX API: `GET /users/me`
- 認証中のユーザーを取得する。
  JWT認証でエンタープライズとして認証している場合は、アプリケーションのサービスアカウントを返す。

### フォルダ

#### フォルダ情報の取得

- メソッド: `BoxClient::folder_information`
- BOX API: `GET /folders/{folder_id}`
- フォルダ情報を取得する。
  ルートフォルダの情報を取得する場合は、フォルダIDに`FolderId::root()`を指定する。

#### フォルダ内の項目のリスト

- メソッド: `BoxClient::list_items_in_folder`および`BoxClient::list_all_items_in_folder`
- BOX API: `GET /folders/{folder_id}/items`
- フォルダ内のファイル、フォルダおよびWebリンクをリストする。
- `list_items_in_folder`は、マーカー方式のページ送りで1ページ分の項目を返す。
  `list_all_items_in_folder`は、ページを送りながらすべての項目を返す。
- 1ページで取得する項目数の上限、並び替えに使用する属性および並び替えの方向を、`ListItemsOptions`で指定する。

#### フォルダの削除

- メソッド: `BoxClient::delete_folder`
- BOX API: `DELETE /folders/{folder_id}`
- フォルダを削除する。
  BOXは、ゴミ箱が有効な場合はフォルダをゴミ箱に移動し、無効な場合はフォルダを完全に削除する。
- 空でないフォルダを削除するかどうか、およびETagを、`DeleteFolderOptions`で指定する。
- ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

### ファイル

#### ファイル情報の取得

- メソッド: `BoxClient::file_information`
- BOX API: `GET /files/{file_id}`
- ファイル情報を取得する。

#### ファイルの削除

- メソッド: `BoxClient::delete_file`
- BOX API: `DELETE /files/{file_id}`
- ファイルを削除する。
  BOXは、エンタープライズの設定に従って、ファイルをゴミ箱に移動するか、完全に削除する。
- ETagを指定した場合は、`If-Match`ヘッダーで送信する。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

### Webリンク

#### Webリンク情報の取得

- メソッド: `BoxClient::web_link_information`
- BOX API: `GET /web_links/{web_link_id}`
- Webリンク情報を取得する。

#### Webリンクの削除

- メソッド: `BoxClient::delete_web_link`
- BOX API: `DELETE /web_links/{web_link_id}`
- Webリンクを削除する。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

### ゴミ箱

#### ゴミ箱内の項目のリスト

- メソッド: `BoxClient::list_trashed_items`および`BoxClient::list_all_trashed_items`
- BOX API: `GET /folders/trash/items`
- ゴミ箱に直接移動したファイル、フォルダおよびWebリンクをリストする。
  ゴミ箱に直接移動したフォルダの子孫にあたる項目は返さない。
- `list_trashed_items`は、マーカー方式のページ送りで1ページ分の項目を返す。
  `list_all_trashed_items`は、ページを送りながらすべての項目を返す。
- 1ページで取得する項目数の上限および並び替えの方向を、`ListTrashedItemsOptions`で指定する。
  BOXはマーカー方式のページ送りと並び替えに使用する属性の指定を併用できないため、並び替えに使用する属性は指定できない。

#### ゴミ箱内のフォルダ情報の取得

- メソッド: `BoxClient::trashed_folder_information`
- BOX API: `GET /folders/{folder_id}/trash`
- ゴミ箱に直接移動したフォルダの情報を取得する。
  ゴミ箱に直接移動したフォルダの子孫フォルダを指定した場合、BOXは404を返す。
- ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返す。

#### ゴミ箱内のファイル情報の取得

- メソッド: `BoxClient::trashed_file_information`
- BOX API: `GET /files/{file_id}/trash`
- ゴミ箱に直接移動したファイルの情報を取得する。
  ゴミ箱に直接移動したフォルダの子孫にあたるファイルを指定した場合、BOXは404を返す。

#### ゴミ箱内のWebリンク情報の取得

- メソッド: `BoxClient::trashed_web_link_information`
- BOX API: `GET /web_links/{web_link_id}/trash`
- ゴミ箱内のWebリンクの情報を取得する。
  Webリンクがゴミ箱にない場合、BOXは404を返す。

#### ゴミ箱内のフォルダの復元

- メソッド: `BoxClient::restore_folder`
- BOX API: `POST /folders/{folder_id}`
- ゴミ箱に直接移動したフォルダを、元の親フォルダに復元する。
- 復元後のフォルダ名、および元の親フォルダが存在しない場合の復元先の親フォルダを、`RestoreItemOptions`で指定する。
- ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返す。
- 復元に成功したフォルダはゴミ箱からなくなるため、BOXが復元した後に再送すると、BOXは404を返す。
  したがって、冪等でないリクエストとして送信する。

#### ゴミ箱内のファイルの復元

- メソッド: `BoxClient::restore_file`
- BOX API: `POST /files/{file_id}`
- ゴミ箱に直接移動したファイルを、元の親フォルダに復元する。
- 復元後のファイル名、および元の親フォルダが存在しない場合の復元先の親フォルダを、`RestoreItemOptions`で指定する。
- 復元に成功したファイルはゴミ箱からなくなるため、BOXが復元した後に再送すると、BOXは404を返す。
  したがって、冪等でないリクエストとして送信する。

#### ゴミ箱内のWebリンクの復元

- メソッド: `BoxClient::restore_web_link`
- BOX API: `POST /web_links/{web_link_id}`
- ゴミ箱内のWebリンクを、元の親フォルダに復元する。
- 復元後のWebリンク名、および元の親フォルダが存在しない場合の復元先の親フォルダを、`RestoreItemOptions`で指定する。
- 復元に成功したWebリンクはゴミ箱からなくなるため、BOXが復元した後に再送すると、BOXは404を返す。
  したがって、冪等でないリクエストとして送信する。

## アクセストークンの管理

`BoxClient`は、JWT認証で取得したアクセストークンをキャッシュし、アクセストークンの有効期限が近づくと、アクセストークンを自動で取得し直す。
BOXが発行するアクセストークンの有効期間は60分である。

### アクセストークンを取得し直す時期と方式

`BoxClient`は、アクセストークンを取得し直す時期を、有効期限までの残り時間と`refresh_before_expiry`から決める。

| キャッシュしたアクセストークンの状態 | `BoxClient`の動作 |
| --- | --- |
| 有効期限までの残り時間が`refresh_before_expiry`より長い | `BoxClient`は、キャッシュしたアクセストークンを使用する。 |
| 有効期限までの残り時間が`refresh_before_expiry`以下で、アクセストークンがまだ期限切れでない | `BoxClient`は、バックグラウンドのタスクで新しいアクセストークンを取得する。`BoxClient`は取得を終えるまでキャッシュしたアクセストークンを使用するため、APIの呼び出しは取得を待たない。 |
| アクセストークンが期限切れ、またはキャッシュがない | APIの呼び出しは、`BoxClient`がアクセストークンを取得し終えるまで待つ。 |

複数のタスクが同時にAPIを呼び出しても、`BoxClient`はアクセストークンを取得するタスクを1つに限る。

バックグラウンドでの取得に失敗した場合、`BoxClient`はWARNログを出力し、キャッシュしたアクセストークンを期限が切れるまで使用し続ける。
次にAPIが呼び出されたとき、`BoxClient`は再びバックグラウンドでアクセストークンを取得し直す。

### `refresh_before_expiry`の決め方

`refresh_before_expiry`は、アクセストークンを有効期限のどれだけ前に取得し直すかを表す設定値である。
利用者は、アクセストークンの有効期間60分を考慮して、`refresh_before_expiry`を次の範囲で決める。

- `refresh_before_expiry`は、再試行を含めてアクセストークンの取得にかかる時間より長くする。
  バックグラウンドでの取得が期限切れまでに終わらないと、APIの呼び出しがアクセストークンの取得を待つためである。
- `refresh_before_expiry`は、`MAX_REFRESH_BEFORE_EXPIRY`（30分）以下にする。
  `refresh_before_expiry`に有効期間の60分以上を指定すると、`BoxClient`は、取得した直後からアクセストークンを取得し直す必要があると判定する。
  その結果、`BoxClient`は、APIが呼び出されるたびにトークンエンドポイントへリクエストする。
  したがって、`MAX_REFRESH_BEFORE_EXPIRY`を有効期間の半分とし、`BoxClient`に、取得したアクセストークンを少なくとも有効期間の半分は再利用させる。
  上限を超える値を指定すると、`BoxClientBuilder::build()`は`BoxClientError::InvalidRefreshBeforeExpiry`を返す。

`refresh_before_expiry`の既定値は60秒であり、`BoxClient`は発行から約59分後にアクセストークンを取得し直す。
通常、`BoxClient`はアクセストークンを1秒未満で取得できる。
したがって、トークンエンドポイントが一時的に失敗し、`BoxClient`が数回再試行しても、`BoxClient`は期限内にアクセストークンを取得し終えられる。
再試行設定の待機時間やタイムアウトを長くした場合、利用者は`refresh_before_expiry`もそれに合わせて長くする。

```rust
use std::time::Duration;

use box_client::BoxClient;

let client = BoxClient::builder(config)
    .refresh_before_expiry(Duration::from_secs(5 * 60))
    .build()
    .await?;
```

## 再試行の仕組み

`BoxClient`は、BOXのREST APIおよびトークンエンドポイントへのリクエストが失敗すると、失敗の種類とリクエストの冪等性から再試行するかどうかを判定する。
再試行する場合は、待機した後に同じリクエストを送信し直す。

### 処理の流れ

1. リクエストを送信する。
2. 成功レスポンスを受け取った場合は、結果を返して終了する。
3. 失敗した場合は、失敗を「再試行できるエラー」と「再試行できないエラー」に分類する。
4. 再試行できないエラーの場合は、エラーを返して終了する。
5. 再試行回数が上限に達している場合は、最後のエラーを返して終了する。
6. 待機時間を決定して待機し、手順1に戻る。

### 再試行するかどうかの判定

冪等でないリクエストを、BOXが処理したかもしれない失敗の後に再送すると、リソースを二重に作成するおそれがある。
したがって、失敗の種類だけでなく、リクエストの冪等性に応じて再試行するかどうかを変える。

| 失敗の種類 | 冪等なリクエスト | 冪等でないリクエスト | 理由 |
| --- | --- | --- | --- |
| 429 Too Many Requests | 再試行する | 再試行する | BOXはリクエストを処理せずに拒否している。 |
| エラーコードが`operation_blocked_temporary`の409 Conflict | 再試行する | 再試行する | 移動、コピー、削除または復元の操作が項目をロックしているため、BOXはリクエストを処理せずに拒否している。ロックが解除されれば成功する可能性がある。 |
| 5xx | 再試行する | 再試行しない | BOXがリクエストを処理した可能性がある。 |
| 上記以外の4xx（401を除く） | 再試行しない | 再試行しない | リクエストの内容に問題があり、再送しても結果は変わらない。名前の重複による409なども含む。 |
| 接続エラー | 再試行する | 再試行する | リクエストを送信する前に失敗している。 |
| タイムアウト | 再試行する | 再試行しない | BOXがリクエストを処理した可能性がある。 |
| リクエストの送信中またはレスポンスボディの受信中の通信エラー | 再試行する | 再試行しない | BOXがリクエストを処理した可能性がある。 |
| レスポンスのJSONを解釈できない | 再試行しない | 再試行しない | 再送しても同じレスポンスを受け取る可能性が高い。`BoxClientError::DeserializeResponse`を返す。 |

再試行しない場合や、再試行回数が上限に達した場合、BOXのエラーレスポンスは`BoxClientError::Api`として返る。
`BoxClientError::Api`は、ステータスとレスポンスボディを保持する。

### 401 Unauthorizedの扱い

BOXがアクセストークンを拒否して401を返した場合は、キャッシュしているアクセストークンを破棄する。
その後、アクセストークンを取得し直して、リクエストを1回だけ再送する。
401はBOXがリクエストを処理していないことを示すため、冪等でないリクエストも再送する。

この再送は、再試行回数に数えない。
再送しても401を受け取った場合は、再試行せずに`BoxClientError::Api`を返す。

### 冪等性の決定

リクエストの冪等性は、HTTPメソッドから次のように決定する。

- 冪等: GET、HEAD、PUT、DELETE、OPTIONSおよびTRACE
- 冪等でない: POST、PATCHなど、上記以外のメソッド

ただし、HTTPメソッドの意味とAPIの実際の振る舞いが一致しない場合がある。
したがって、`ApiRequest::idempotency()`で冪等性を上書きできる。
このクレートでは、次のリクエストで冪等性を上書きしている。

| リクエスト | HTTPメソッド | 冪等性 | 理由 |
| --- | --- | --- | --- |
| アクセストークンの取得 | POST | 冪等 | アクセストークンを二重に発行しても害はない。また、JWTアサーションは試行ごとに作り直す。 |
| フォルダの削除（`BoxClient::delete_folder`） | DELETE | 冪等でない | BOXは削除に600秒以上かかると503を返すが、削除処理はその後も継続する。再送すると409や404を受け取り、削除の結果を誤って判断するおそれがある。 |
| ファイルの削除（`BoxClient::delete_file`） | DELETE | 冪等でない | BOXは削除済みのファイルに対して404を返す。BOXが削除した後に再送すると404を受け取り、削除の結果を誤って判断するおそれがある。 |
| Webリンクの削除（`BoxClient::delete_web_link`） | DELETE | 冪等でない | BOXが削除した後に再送すると、削除済みのWebリンクに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。 |

フォルダの削除で503を受け取った場合は、`BoxClient::folder_information`でフォルダの状態を確認する。
ファイルの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::file_information`でファイルの状態を確認する。
Webリンクの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::web_link_information`でWebリンクの状態を確認する。

### 待機時間の決定

再試行する前の待機時間は、次のように決定する。

- レスポンスに`Retry-After`ヘッダーがある場合
  - `Retry-After`ヘッダーが示す秒数にジッターを加算した時間だけ待機する。
    BOXが指定した時刻より早く再試行しないように、ジッターは加算だけを行う。
  - `Retry-After`ヘッダーが示す秒数が待機時間の上限より大きい場合は、再試行を中止して`BoxClientError::RetryAfterTooLong`を返す。
  - `Retry-After`ヘッダーを秒数として解釈できない場合は、ヘッダーがない場合と同じく指数バックオフで待機する。
- レスポンスに`Retry-After`ヘッダーがない場合
  - 指数バックオフで算出した時間にジッターを加算した時間だけ待機する。
  - n回目の再試行前の待機時間は、`initial_delay * 2^(n - 1)`を`max_delay`で切り詰めた時間とする。
  - 多数のクライアントが同時に再試行しないように、`0`以上`max_jitter`以下の乱数の時間を加算する。
    ジッターを加算した後は`max_delay`で切り詰めないため、待機時間の最大値は`max_delay + max_jitter`となる。

### 再試行設定

再試行設定は`RetryPolicy`で指定する。
`RetryPolicy`は、REST APIとトークンエンドポイントの両方のリクエストに適用する。

| 設定項目 | 意味 | 既定値 |
| --- | --- | --- |
| `max_retries` | 最大再試行回数。最初のリクエストは回数に含めない。`0`の場合は再試行しない。 | 5回 |
| `initial_delay` | 1回目の再試行前に待機する時間 | 0.5秒 |
| `max_delay` | 指数バックオフで算出する待機時間の上限、および`Retry-After`ヘッダーが示す待機時間の許容値 | 30秒 |
| `max_jitter` | 待機時間に加算するジッターの上限 | 1秒 |

既定値を使用した場合、ジッターを加算する前の待機時間は、0.5秒、1秒、2秒、4秒、8秒と倍増する。

`RetryPolicy::new()`は、次の条件を満たさない設定値に対して`BoxClientError::InvalidRetryPolicy`を返す。

- `initial_delay`が`0`より大きい。
  `0`の場合は、待機せずに再試行し続けるためである。
- `initial_delay`が`max_delay`以下である。
- `max_jitter`が`max_delay`以下である。

再試行設定とタイムアウトは、`BoxClientBuilder`で指定する。

```rust
use std::time::Duration;

use box_client::{BoxClient, BoxClientTimeout, RetryPolicy};

let client = BoxClient::builder(config)
    .retry(RetryPolicy::new(
        3,
        Duration::from_secs(1),
        Duration::from_secs(10),
        Duration::from_millis(500),
    )?)
    .timeout(BoxClientTimeout {
        request_timeout: Duration::from_secs(30),
        connect_timeout: Duration::from_secs(10),
    })
    .build()
    .await?;
```

### ログ

再試行に関するイベントは、`tracing`で構造化ログとして出力する。
401を受け取ったとき、およびアクセストークンのバックグラウンドでの取得に失敗したときのイベントを除き、各イベントは処理名`operation`および試行回数`attempts`を含む。
`operation`は、REST APIへのリクエストでは`box_api_request`、アクセストークンの取得では`fetch_access_token`である。
`retrying`以外のイベントは、最初のリクエストからの経過時間`elapsed_ms`も含む。

| レベル | メッセージ | 出力する場面 |
| --- | --- | --- |
| INFO | `succeeded` | リクエストが成功した。 |
| WARN | `retrying` | 再試行する。待機時間`delay_ms`と、待機時間を決めた方法`delay_source`を含む。`delay_source`は`retry_after`または`backoff`である。 |
| WARN | `access token was rejected; fetching a new one and resending` | 401を受け取り、アクセストークンを取得し直して再送する。 |
| WARN | `failed to refresh access token in background; using the cached token until it expires` | `BoxClient`が、アクセストークンをバックグラウンドで取得できなかった。`BoxClient`は、キャッシュしたアクセストークンを期限が切れるまで使用する。このイベントは、原因のエラー`error`を含む。 |
| ERROR | `failed with non-retryable error` | 再試行できないエラーが発生した。 |
| ERROR | `failed after reaching max retries` | 再試行回数が上限に達した。 |
| ERROR | `gave up retrying` | `Retry-After`ヘッダーが示す待機時間が上限を超えたため、再試行を中止した。 |

### キャンセル

リクエストを送信するfutureは、待機中または送信中に破棄しても状態を残さない。
したがって、`tokio::time::timeout`などで処理全体の時間を制限できる。
ただし、送信中に破棄した場合は、BOXがリクエストを処理したかどうかを判断できない。

アクセストークンをバックグラウンドで取得し直すタスクは、リクエストを送信するfutureを破棄しても中断せず、取得の成功または失敗まで実行する。

## 開発環境

### prek

Gitフックを導入する。

```shell
cargo install cargo-deny
cargo install prek
prek install
```
