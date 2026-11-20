# BOXクライアントクレート

## 対応APIリスト

`BoxClient`のメソッドから呼び出せるBOXのREST APIを、リソースごとに示す。
各メソッドの詳細は、メソッドのドキュメントを参照する。

### ユーザー

- 認証中のユーザーの取得
  - メソッド: `BoxClient::me`
  - BOX API: `GET /users/me`
- ユーザー情報の取得
  - メソッド: `BoxClient::user_information`
  - BOX API: `GET /users/{user_id}`
- エンタープライズのユーザーのリスト
  - メソッド: `BoxClient::list_enterprise_users`, `BoxClient::list_all_enterprise_users`
  - BOX API: `GET /users`
- ユーザーの作成
  - メソッド: `BoxClient::create_user`
  - BOX API: `POST /users`
- ユーザーの更新（エンタープライズからの除外を含む）
  - メソッド: `BoxClient::update_user`
  - BOX API: `PUT /users/{user_id}`
- ユーザーの削除
  - メソッド: `BoxClient::delete_user`
  - BOX API: `DELETE /users/{user_id}`

### フォルダ

- フォルダ情報の取得
  - メソッド: `BoxClient::folder_information`
  - BOX API: `GET /folders/{folder_id}`
- フォルダ内の項目のリスト
  - メソッド: `BoxClient::list_items_in_folder`, `BoxClient::list_all_items_in_folder`
  - BOX API: `GET /folders/{folder_id}/items`
- フォルダのコピー
  - メソッド: `BoxClient::copy_folder`
  - BOX API: `POST /folders/{folder_id}/copy`
- フォルダの作成
  - メソッド: `BoxClient::create_folder`
  - BOX API: `POST /folders`
- フォルダの更新
  - メソッド: `BoxClient::update_folder`
  - BOX API: `PUT /folders/{folder_id}`
- フォルダ名の変更
  - メソッド: `BoxClient::rename_folder`
  - BOX API: `PUT /folders/{folder_id}`
- フォルダの説明の設定
  - メソッド: `BoxClient::set_folder_description`
  - BOX API: `PUT /folders/{folder_id}`
- フォルダの移動
  - メソッド: `BoxClient::move_folder`
  - BOX API: `PUT /folders/{folder_id}`
- フォルダの削除
  - メソッド: `BoxClient::delete_folder`
  - BOX API: `DELETE /folders/{folder_id}`

### ファイル

- ファイル情報の取得
  - メソッド: `BoxClient::file_information`
  - BOX API: `GET /files/{file_id}`
- ファイルのコピー
  - メソッド: `BoxClient::copy_file`
  - BOX API: `POST /files/{file_id}/copy`
- ファイルの更新
  - メソッド: `BoxClient::update_file`
  - BOX API: `PUT /files/{file_id}`
- ファイル名の変更
  - メソッド: `BoxClient::rename_file`
  - BOX API: `PUT /files/{file_id}`
- ファイルの説明の設定
  - メソッド: `BoxClient::set_file_description`
  - BOX API: `PUT /files/{file_id}`
- ファイルの移動
  - メソッド: `BoxClient::move_file`
  - BOX API: `PUT /files/{file_id}`
- ファイルの削除
  - メソッド: `BoxClient::delete_file`
  - BOX API: `DELETE /files/{file_id}`
- ファイルのバージョン情報の取得
  - メソッド: `BoxClient::file_version`
  - BOX API: `GET /files/{file_id}/versions/{file_version_id}`
- ファイルのバージョンのリスト
  - メソッド: `BoxClient::list_file_versions`, `BoxClient::list_all_file_versions`
  - BOX API: `GET /files/{file_id}/versions`
- ファイルのバージョンの昇格
  - メソッド: `BoxClient::promote_file_version`
  - BOX API: `POST /files/{file_id}/versions/current`
- 削除したファイルのバージョンの復元
  - メソッド: `BoxClient::restore_file_version`
  - BOX API: `PUT /files/{file_id}/versions/{file_version_id}`
- ファイルのバージョンの削除
  - メソッド: `BoxClient::delete_file_version`
  - BOX API: `DELETE /files/{file_id}/versions/{file_version_id}`
- ファイルのアップロード
  - メソッド: `BoxClient::upload_file`
  - BOX API
    - `POST /files/content`
    - `POST /files/upload_sessions`
    - `PUT /files/upload_sessions/{upload_session_id}`
    - `POST /files/upload_sessions/{upload_session_id}/commit`
    - `DELETE /files/upload_sessions/{upload_session_id}`
- ファイルの内容の更新
  - メソッド: `BoxClient::update_file_content`
  - BOX API
    - `POST /files/{file_id}/content`
    - `POST /files/{file_id}/upload_sessions`
    - `PUT /files/upload_sessions/{upload_session_id}`
    - `POST /files/upload_sessions/{upload_session_id}/commit`
    - `DELETE /files/upload_sessions/{upload_session_id}`
- アップロードのプリフライトチェック
  - メソッド: `BoxClient::preflight_upload_file`
  - BOX API: `OPTIONS /files/content`
- ファイルの内容の更新のプリフライトチェック
  - メソッド: `BoxClient::preflight_update_file_content`
  - BOX API: `OPTIONS /files/{file_id}/content`
- ファイルのダウンロード
  - メソッド: `BoxClient::download_file`
  - BOX API: `GET /files/{file_id}/content`

### Webリンク

- Webリンク情報の取得
  - メソッド: `BoxClient::web_link_information`
  - BOX API: `GET /web_links/{web_link_id}`
- Webリンクの作成
  - メソッド: `BoxClient::create_web_link`
  - BOX API: `POST /web_links`
- Webリンクの更新
  - メソッド: `BoxClient::update_web_link`
  - BOX API: `PUT /web_links/{web_link_id}`
- Webリンク名の変更
  - メソッド: `BoxClient::rename_web_link`
  - BOX API: `PUT /web_links/{web_link_id}`
- Webリンクの説明の設定
  - メソッド: `BoxClient::set_web_link_description`
  - BOX API: `PUT /web_links/{web_link_id}`
- Webリンクの移動
  - メソッド: `BoxClient::move_web_link`
  - BOX API: `PUT /web_links/{web_link_id}`
- WebリンクのURLの変更
  - メソッド: `BoxClient::update_web_link_url`
  - BOX API: `PUT /web_links/{web_link_id}`
- Webリンクの削除
  - メソッド: `BoxClient::delete_web_link`
  - BOX API: `DELETE /web_links/{web_link_id}`

### コラボレーション

- コラボレーション情報の取得
  - メソッド: `BoxClient::collaboration_information`
  - BOX API: `GET /collaborations/{collaboration_id}`
- コラボレーションの作成
  - メソッド: `BoxClient::create_collaboration`
  - BOX API: `POST /collaborations`
- コラボレーションの更新（役割の変更、所有者の変更、保留中の招待の承認または拒否を含む）
  - メソッド: `BoxClient::update_collaboration`
  - BOX API: `PUT /collaborations/{collaboration_id}`
- コラボレーションの削除
  - メソッド: `BoxClient::delete_collaboration`
  - BOX API: `DELETE /collaborations/{collaboration_id}`
- ファイルのコラボレーションのリスト
  - メソッド: `BoxClient::list_file_collaborations`, `BoxClient::list_all_file_collaborations`
  - BOX API: `GET /files/{file_id}/collaborations`
- フォルダのコラボレーションのリスト
  - メソッド: `BoxClient::list_folder_collaborations`, `BoxClient::list_all_folder_collaborations`
  - BOX API: `GET /folders/{folder_id}/collaborations`
- グループのコラボレーションのリスト
  - メソッド: `BoxClient::list_group_collaborations`, `BoxClient::list_all_group_collaborations`
  - BOX API: `GET /groups/{group_id}/collaborations`
- 保留中のコラボレーションの招待のリスト
  - メソッド: `BoxClient::list_pending_collaborations`, `BoxClient::list_all_pending_collaborations`
  - BOX API: `GET /collaborations`

### ゴミ箱

- ゴミ箱内のファイル情報の取得
  - メソッド: `BoxClient::trashed_file_information`
  - BOX API: `GET /files/{file_id}/trash`
- ゴミ箱内のファイルの復元
  - メソッド: `BoxClient::restore_file`
  - BOX API: `POST /files/{file_id}`
- ゴミ箱内のフォルダ情報の取得
  - メソッド: `BoxClient::trashed_folder_information`
  - BOX API: `GET /folders/{folder_id}/trash`
- ゴミ箱内のフォルダの復元
  - メソッド: `BoxClient::restore_folder`
  - BOX API: `POST /folders/{folder_id}`
- ゴミ箱内の項目のリスト
  - メソッド: `BoxClient::list_trashed_items`, `BoxClient::list_all_trashed_items`
  - BOX API: `GET /folders/trash/items`
- ゴミ箱内のWebリンク情報の取得
  - メソッド: `BoxClient::trashed_web_link_information`
  - BOX API: `GET /web_links/{web_link_id}/trash`
- ゴミ箱内のWebリンクの復元
  - メソッド: `BoxClient::restore_web_link`
  - BOX API: `POST /web_links/{web_link_id}`

### ZIPダウンロード

- ZIPファイルのダウンロード
  - メソッド: `BoxClient::download_zip_content`
  - BOX API: `GET /zip_downloads/{zip_download_id}/content`
- ZIPダウンロードの状態の取得
  - メソッド: `BoxClient::zip_download_status`
  - BOX API: `GET /zip_downloads/{zip_download_id}/status`
- ZIPダウンロードの作成
  - メソッド: `BoxClient::create_zip_download`
  - BOX API: `POST /zip_downloads`
- ZIPファイルのダウンロード（作成、ダウンロードおよび状態の取得をまとめて実行）
  - メソッド: `BoxClient::download_zip`
  - BOX API
    - `POST /zip_downloads`
    - `GET /zip_downloads/{zip_download_id}/content`
    - `GET /zip_downloads/{zip_download_id}/status`

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
失敗してから一定の間隔が経過した後にAPIが呼び出されると、`BoxClient`は再びバックグラウンドでアクセストークンを取得し直す。
アプリケーションの認可が取り消された場合など、再試行しても成功しない失敗もある。
そのような失敗の直後から、APIが呼び出されるたびに取得し直すと、トークンエンドポイントへのリクエストが期限切れまで続く。
これを避けるためである。
間隔は、10秒と`refresh_before_expiry`の4分の1の短いほうである。
既定の`refresh_before_expiry`（60秒）では10秒になり、`refresh_before_expiry`を短く設定した場合も、期限が切れるまでに複数回取得し直す機会が残る。
アクセストークンが期限切れになった場合は、間隔に関係なく、APIの呼び出しはアクセストークンの取得を待つ。

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

### BOXとの時刻のずれ

`BoxClient`は、アクセストークンを取得するときに、署名したJWTアサーションをトークンエンドポイントに送信する。
BOXは、JWTアサーションの有効期限（`exp`クレーム）を発行から60秒以内とするよう求めるため、`BoxClient`は有効期限を45秒後とする。
システム時刻がBOXの時刻とずれていると、BOXは有効期限を理由に、エラーコードが`invalid_grant`の400を返してJWTアサーションを拒否する。

この場合、`BoxClient`は、レスポンスの`Date`ヘッダーが示すBOXの時刻とシステム時刻の差を求め、有効期限をBOXの時刻に合わせたJWTアサーションを作り直して、1回だけ再送する。
BOXはリクエストを処理せずに拒否しているため、この再送は再試行回数に数えない。
`BoxClient`は、求めた時刻の差を以後のJWTアサーションにも使用する。

BOXの公式ドキュメントはエラーの説明の文言を記載していないため、`BoxClient`は、BOXの旧版のPython SDKと同じく、エラーの説明に`exp`が含まれるかどうかで有効期限による拒否と判定する。
時刻の差は秒単位で求めるため、システム時刻はNTPなどでBOXの時刻と同期しておくことが望ましい。

## 再試行の仕組み

`BoxClient`は、BOXのREST APIおよびトークンエンドポイントへのリクエストが失敗すると、失敗の種類とリクエストの冪等性から再試行するかどうかを判定する。
再試行する場合、`BoxClient`は、待機した後に同じリクエストを送信し直す。

### 処理の流れ

1. リクエストを送信する。
2. 成功レスポンスを受け取った場合は、結果を返して終了する。
3. 失敗した場合は、失敗を「再試行できるエラー」と「再試行できないエラー」に分類する。
4. 再試行できないエラーの場合は、エラーを返して終了する。
5. 再試行回数が上限に達している場合は、最後のエラーを返して終了する。
6. 待機時間を決定して待機し、手順1に戻る。

### 再試行するかどうかの判定

冪等でないリクエストを、BOXが処理したかもしれない失敗の後に再送すると、リソースを二重に作成するおそれがある。
したがって、`BoxClient`は、失敗の種類だけでなく、リクエストの冪等性に応じて再試行するかどうかを変える。

- 429 Too Many Requests
  - 冪等なリクエスト: 再試行する
  - 冪等でないリクエスト: 再試行する
  - 理由: BOXはリクエストを処理せずに拒否している。
- エラーコードが`operation_blocked_temporary`の409 Conflict
  - 冪等なリクエスト: 再試行する
  - 冪等でないリクエスト: 再試行する
  - 理由: 移動、コピー、削除または復元の操作が項目をロックしているため、BOXはリクエストを処理せずに拒否している。ロックが解除されれば成功する可能性がある。
- 5xx
  - 冪等なリクエスト: 再試行する
  - 冪等でないリクエスト: 再試行しない
  - 理由: BOXがリクエストを処理した可能性がある。
- 上記以外の4xx（401を除く）
  - 冪等なリクエスト: 再試行しない
  - 冪等でないリクエスト: 再試行しない
  - 理由: リクエストの内容に問題があり、再送しても結果は変わらない。名前の重複による409なども含む。
- 接続エラー
  - 冪等なリクエスト: 再試行する
  - 冪等でないリクエスト: 再試行する
  - 理由: リクエストを送信する前に失敗している。
- タイムアウト
  - 冪等なリクエスト: 再試行する
  - 冪等でないリクエスト: 再試行しない
  - 理由: BOXがリクエストを処理した可能性がある。
- リクエストの送信中またはレスポンスボディの受信中の通信エラー
  - 冪等なリクエスト: 再試行する
  - 冪等でないリクエスト: 再試行しない
  - 理由: BOXがリクエストを処理した可能性がある。
- レスポンスのJSONを解釈できない
  - 冪等なリクエスト: 再試行しない
  - 冪等でないリクエスト: 再試行しない
  - 理由: 再送しても同じレスポンスを受け取る可能性が高い。`BoxClientError::DeserializeResponse`を返す。

再試行しない場合や、再試行回数が上限に達した場合、BOXのエラーレスポンスは`BoxClientError::Api`として返る。
`BoxClientError::Api`は、ステータスとレスポンスボディを保持する。

### 401 Unauthorizedの扱い

BOXがアクセストークンを拒否して401を返した場合、`BoxClient`は、キャッシュしているアクセストークンを破棄する。
その後、`BoxClient`は、アクセストークンを取得し直して、リクエストを1回だけ再送する。
401はBOXがリクエストを処理していないことを示すため、冪等でないリクエストも再送する。

この再送は、再試行回数に数えない。
再送しても401を受け取った場合は、再試行せずに`BoxClientError::Api`を返す。

### 冪等性の決定

`ApiRequest`は、リクエストの冪等性を、HTTPメソッドから次のように決定する。

- 冪等: GET、HEAD、PUT、DELETE、OPTIONSおよびTRACE
- 冪等でない: POST、PATCHなど、上記以外のメソッド

ただし、HTTPメソッドの意味とAPIの実際の振る舞いが一致しない場合がある。
したがって、`ApiRequest::idempotency()`で冪等性を上書きできる。
このクレートでは、次のリクエストで冪等性を上書きしている。

- アクセストークンの取得
  - HTTPメソッド: POST
  - 冪等性: 冪等
  - 理由: アクセストークンを二重に発行しても害はない。また、JWTアサーションは試行ごとに作り直す。
- ZIPダウンロードの作成（`BoxClient::create_zip_download`）
  - HTTPメソッド: POST
  - 冪等性: 冪等
  - 理由: ZIPダウンロードを重複して作成しても、BOXに項目は作成されず、使用しなかったZIPダウンロードは期限が切れて無効になる。
- ZIPファイルのダウンロード（`BoxClient::download_zip_content`）
  - HTTPメソッド: GET
  - 冪等性: 冪等でない
  - 理由: ダウンロード用のURLは一度しか使用できず、BOXは2回目以降のリクエストに404を返す。
- フォルダの削除（`BoxClient::delete_folder`）
  - HTTPメソッド: DELETE
  - 冪等性: 冪等でない
  - 理由: BOXは削除に600秒以上かかると503を返すが、削除処理はその後も継続する。再送すると409や404を受け取り、削除の結果を誤って判断するおそれがある。
- フォルダの更新（`BoxClient::update_folder`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが更新した後に再送すると、ETagが変わっているため412を受け取る。その結果、更新の結果を誤って判断するおそれがある。
- フォルダの移動（`BoxClient::move_folder`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: BOXは移動に600秒以上かかると503を返すが、移動処理はその後も継続する。再送すると409を受け取り、ETagを指定した場合は412を受け取るため、移動の結果を誤って判断するおそれがある。
- フォルダ名の変更（`BoxClient::rename_folder`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが名前を変更した後に再送すると、ETagが変わっているため412を受け取る。その結果、変更の結果を誤って判断するおそれがある。
- フォルダの説明の設定（`BoxClient::set_folder_description`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが説明を設定した後に再送すると、ETagが変わっているため412を受け取る。その結果、設定の結果を誤って判断するおそれがある。
- ファイルのバージョンの削除（`BoxClient::delete_file_version`）
  - HTTPメソッド: DELETE
  - 冪等性: 冪等でない
  - 理由: BOXが削除した後に再送すると、削除済みのバージョンに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。
- ファイルの更新（`BoxClient::update_file`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが更新した後に再送すると、ETagが変わっているため412を受け取る。その結果、更新の結果を誤って判断するおそれがある。
- ファイル名の変更（`BoxClient::rename_file`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが名前を変更した後に再送すると、ETagが変わっているため412を受け取る。その結果、変更の結果を誤って判断するおそれがある。
- ファイルの移動（`BoxClient::move_file`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXがファイルを移動した後に再送すると、ETagが変わっているため412を受け取る。その結果、移動の結果を誤って判断するおそれがある。
- ファイルの説明の設定（`BoxClient::set_file_description`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが説明を設定した後に再送すると、ETagが変わっているため412を受け取る。その結果、設定の結果を誤って判断するおそれがある。
- ファイルの削除（`BoxClient::delete_file`）
  - HTTPメソッド: DELETE
  - 冪等性: 冪等でない
  - 理由: BOXは削除済みのファイルに対して404を返す。BOXが削除した後に再送すると404を受け取り、削除の結果を誤って判断するおそれがある。
- Webリンクの更新（`BoxClient::update_web_link`、`BoxClient::rename_web_link`など）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: ETagを指定した場合、BOXが更新した後に再送すると、ETagが変わっているため412を受け取る。その結果、更新の結果を誤って判断するおそれがある。
- Webリンクの削除（`BoxClient::delete_web_link`）
  - HTTPメソッド: DELETE
  - 冪等性: 冪等でない
  - 理由: BOXが削除した後に再送すると、削除済みのWebリンクに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。
- コラボレーションの更新で、役割を所有者にする場合（`BoxClient::update_collaboration`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: 所有者を変更すると、BOXは更新したコラボレーションを削除して新しいコラボレーションを作成する。BOXが所有者を変更した後に再送すると、削除済みのコラボレーションに対する更新となり、404を受け取りうる。その結果、所有者の変更の結果を誤って判断するおそれがある。所有者の変更以外の更新は、HTTPメソッドのとおり冪等とする。
- コラボレーションの削除（`BoxClient::delete_collaboration`）
  - HTTPメソッド: DELETE
  - 冪等性: 冪等でない
  - 理由: BOXが削除した後に再送すると、削除済みのコラボレーションに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。
- ユーザーの更新で、ユーザーをエンタープライズから除外する場合（`BoxClient::update_user`）
  - HTTPメソッド: PUT
  - 冪等性: 冪等でない
  - 理由: BOXがユーザーを除外した後に再送すると、エンタープライズに属さないユーザーに対する更新となり、エラーを受け取りうる。その結果、除外の結果を誤って判断するおそれがある。除外以外の更新は、HTTPメソッドのとおり冪等とする。
- ユーザーの削除（`BoxClient::delete_user`）
  - HTTPメソッド: DELETE
  - 冪等性: 冪等でない
  - 理由: BOXが削除した後に再送すると、削除済みのユーザーに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。

フォルダの削除で503を受け取った場合は、`BoxClient::folder_information`でフォルダの状態を確認する。
フォルダの移動で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::folder_information`でフォルダの親フォルダを確認する。
フォルダ名の変更で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::folder_information`でフォルダ名を確認する。
フォルダの説明の設定で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::folder_information`でフォルダの説明を確認する。
ファイルの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::file_information`でファイルの状態を確認する。
Webリンクの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::web_link_information`でWebリンクの状態を確認する。
コラボレーションの所有者の変更で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::list_file_collaborations`または`BoxClient::list_folder_collaborations`で項目のコラボレーションを確認する。
コラボレーションの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::collaboration_information`でコラボレーションが存在するか確認する。
ユーザーのエンタープライズからの除外で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::user_information`でユーザーを確認する。
ユーザーの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::user_information`でユーザーが存在するか確認する。

### 待機時間の決定

再試行する前の待機時間は、次のように決定する。

- レスポンスに`Retry-After`ヘッダーがある場合
  - `Retry-After`ヘッダーが示す秒数にジッターを加算した時間だけ待機する。
    BOXが指定した時刻より早く再試行しないように、ジッターは加算だけを行う。
  - `Retry-After`ヘッダーが示す秒数が待機時間の上限より大きい場合は、再試行を中止して`BoxClientError::RetryAfterTooLong`を返す。
    `BoxClientError::RetryAfterTooLong`は、`Retry-After`ヘッダーを返したリクエストのエラーを`source()`で返すため、利用者はステータスやリクエストIDを調べられる。
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

再試行設定とタイムアウト設定は、`BoxClientBuilder`で指定する。
利用者は、どちらも`Default`で作成し、`with_*`メソッドで値を設定する。

`BoxClientBuilder::build()`は、次の条件を満たさない再試行設定に対して`BoxClientError::InvalidRetryPolicy`を返す。

- `initial_delay`が`0`より大きい。
  `0`の場合は、待機せずに再試行し続けるためである。
- `initial_delay`が`max_delay`以下である。
- `max_jitter`が`max_delay`以下である。

また、`BoxClientBuilder::build()`は、`request_timeout`、`connect_timeout`および`download_read_timeout`のいずれかが`0`のタイムアウト設定に対して`BoxClientError::InvalidTimeoutConfig`を返す。
`0`の場合は、すべてのリクエストが直ちにタイムアウトするためである。

```rust
use std::time::Duration;

use box_client::{BoxClient, RetryPolicy, TimeoutConfig};

let client = BoxClient::builder(config)
    .retry(
        RetryPolicy::default()
            .with_max_retries(3)
            .with_initial_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(10))
            .with_max_jitter(Duration::from_millis(500)),
    )
    .timeout(
        TimeoutConfig::default()
            .with_request_timeout(Duration::from_secs(30))
            .with_connect_timeout(Duration::from_secs(10))
            .with_download_read_timeout(Duration::from_secs(30)),
    )
    .build()
    .await?;
```

### ログ

このクレートは、再試行に関するイベントを、`tracing`で構造化ログとして出力する。
ライブラリが利用者のアプリケーションのログを埋めないように、成功を知らせるイベントはDEBUGで出力し、利用者が対処を検討すべきイベントだけをWARNで出力する。
このクレートはERRORのイベントを出力しない。
エラーはすべて呼び出し元に返すため、エラーの重さは呼び出し元が判断し、必要に応じて記録する。
再試行できないエラーは、再試行せずにそのまま呼び出し元に返すため、DEBUGで出力する。
404のように呼び出し元が想定しているエラーもあるため、エラーを異常とみなすかどうかは呼び出し元が判断する。
フォルダの削除やゴミ箱からの復元など、各APIの処理が完了したことを知らせるイベントも、DEBUGで出力する。
次の表のうち、再試行の結果を知らせるイベント（`succeeded`、`retrying`、`failed with non-retryable error`、`failed after reaching max retries`および`gave up retrying`）は、処理名`operation`および試行回数`attempts`を含む。
`operation`は、REST APIへのリクエストでは`box_api_request`、アクセストークンの取得では`fetch_access_token`である。
これらのイベントのうち、`retrying`以外のイベントは、最初のリクエストからの経過時間`elapsed_ms`も含む。

| レベル | メッセージ | 出力する場面 |
| --- | --- | --- |
| DEBUG | `succeeded` | リクエストが成功した。 |
| WARN | `retrying` | 再試行する。待機時間`delay_ms`と、待機時間を決めた方法`delay_source`を含む。`delay_source`は`retry_after`または`backoff`である。 |
| WARN | `access token was rejected; fetching a new one and resending` | 401を受け取り、アクセストークンを取得し直して再送する。 |
| WARN | `jwt assertion was rejected because of its expiration; adjusting to the box server time and resending` | JWTアサーションが有効期限を理由に拒否されたため、有効期限をBOXの時刻に合わせて再送する。BOXの時刻からシステム時刻を引いた秒数`clock_offset_secs`を含む。 |
| WARN | `failed to refresh access token in background; using the cached token until it expires` | `BoxClient`が、アクセストークンをバックグラウンドで取得できなかった。`BoxClient`は、キャッシュしたアクセストークンを期限が切れるまで使用する。このイベントは、原因のエラー`error`と、次にバックグラウンドで取得し直すまでの間隔`retry_interval_ms`を含む。 |
| DEBUG | `failed with non-retryable error` | 再試行できないエラーが発生した。 |
| WARN | `failed after reaching max retries` | 再試行回数が上限に達した。 |
| WARN | `gave up retrying` | `Retry-After`ヘッダーが示す待機時間が上限を超えたため、再試行を中止した。`Retry-After`ヘッダーが示す待機時間`retry_after_ms`と、待機時間の上限`max_delay_ms`を含む。 |
| WARN | `failed to abort upload session` | 分割アップロードに失敗した後、`BoxClient`がアップロードセッションを中止できなかった。アップロードセッションのID`upload_session_id`と、原因のエラー`error`を含む。 |

### キャンセル

リクエストを送信するfutureは、待機中または送信中に破棄しても、`BoxClient`の内部に状態を残さない。
したがって、利用者は、`tokio::time::timeout`などで処理全体の時間を制限できる。
ただし、送信中に破棄した場合は、利用者は、BOXがリクエストを処理したかどうかを判断できない。

分割アップロードの途中で`BoxClient::upload_file`または`BoxClient::update_file_content`のfutureを破棄した場合、`BoxClient`はアップロードセッションを中止しない。
BOXは、中止しなかったアップロードセッションを7日後に破棄する。

アクセストークンをバックグラウンドで取得し直すタスクは、リクエストを送信するfutureを破棄しても中断せず、取得の成功または失敗まで実行する。

## 接続先、プロキシおよびUser-Agent

### User-Agent

`BoxClient`は、すべてのリクエストで、`User-Agent`ヘッダーにこのクレートの名前とバージョン（`box-client/<version>`）を送信する。
対象には、アクセストークンの取得とファイルのダウンロードも含む。
利用者が`BoxClientBuilder::user_agent`でアプリケーションの名前を指定した場合、`BoxClient`は、その後ろに空白とこのクレートの名前とバージョンを続けた`<user_agent> box-client/<version>`を送信する。
アプリケーションの名前を指定すると、BOXのログで呼び出し元のアプリケーションを識別しやすくなる。

空文字列、空白だけの文字列、およびヘッダーの値に使用できない文字を含む文字列は、`BoxClientBuilder::build()`でエラーになる。

### プロキシ

利用者がプロキシを指定しない場合、`BoxClient`は環境変数（`HTTPS_PROXY`、`HTTP_PROXY`、`ALL_PROXY`および`NO_PROXY`など）に従う。

- 利用者が`BoxClientBuilder::proxy`でプロキシのURLを指定した場合、`BoxClient`は、すべてのリクエストを指定したプロキシ経由で送信し、環境変数のプロキシを使用しない。
  対象には、アクセストークンの取得とファイルのダウンロードも含む。
- 利用者が`BoxClientBuilder::no_proxy`を呼び出した場合、`BoxClient`は、環境変数でプロキシが設定されていても、プロキシを使用せずに直接送信する。
- プロキシに認証が必要な場合、利用者は、`http://user:password@proxy.example.com:8080`のように、URLにユーザー名とパスワードを含める。
  `BoxClient`は、ユーザー名とパスワードを`Proxy-Authorization`ヘッダーで送信する。
  ユーザー名とパスワードに`@`や`:`などを含める場合、利用者はパーセントエンコードする。
- プロキシのURLのスキームは、`http`または`https`とする。
  クエリまたはフラグメントを含むURL、およびホストを含まないURLは、`BoxClientBuilder::build()`でエラーになる。
- プロキシのURLは認証情報を含みうるため、`BoxClient`は、URLをDebug出力やエラーに含めない。

```rust
let client = BoxClient::builder(config)
    .proxy("http://user:password@proxy.example.com:8080")
    .build()
    .await?;
```

### 接続先のURL

利用者は、REST API、アップロードのAPIおよびトークンエンドポイントのURLを、`BoxClientBuilder`で変更できる。
利用者のテストで、BOXの代わりにモックサーバーへ接続する場合などに使用する。

| メソッド | 既定値 |
| --- | --- |
| `BoxClientBuilder::api_base_url` | `https://api.box.com/2.0` |
| `BoxClientBuilder::upload_base_url` | `https://upload.box.com/api/2.0` |
| `BoxClientBuilder::token_url` | `https://api.box.com/oauth2/token` |

`BoxClient`は、これらのURLにアクセストークンまたはJWTアサーションを送信する。
したがって、BOX以外のサーバーや暗号化しない通信で送信しないように、`BoxClient`は次の条件を満たすURLだけを受け付ける。
条件を満たさないURLは、`BoxClientBuilder::build()`でエラーになる。

- スキームが`https`である。
  ただし、ホストが`localhost`またはループバックアドレス（`127.0.0.0/8`、`::1`）の場合は、モックサーバーに接続できるように`http`も受け付ける。
- ユーザー名とパスワード、クエリおよびフラグメントを含まない。

`BoxClient`は、JWTアサーションの`aud`クレームを、トークンエンドポイントのURLの設定によらず、BOXのトークンエンドポイントのURL（`https://api.box.com/oauth2/token`）とする。
BOXは、`aud`クレームがBOXのトークンエンドポイントのURLと一致することを求めるためである。
したがって、利用者がBOXに転送するプロキシをトークンエンドポイントに指定しても、BOXはJWTアサーションを受け付ける。

```rust
let client = BoxClient::builder(config)
    .user_agent("my-app/1.0")
    .api_base_url("http://127.0.0.1:8080/2.0")
    .upload_base_url("http://127.0.0.1:8080/api/2.0")
    .token_url("http://127.0.0.1:8080/oauth2/token")
    .build()
    .await?;
```

## サンプル

`examples`ディレクトリのサンプルは、BOX設定ファイルを読み込んで認証する。
BOX設定ファイルは、BOXの開発者コンソールでJWT認証のアプリケーションの公開鍵を生成したときにダウンロードできるJSONファイルである。

サンプルは、環境変数`BOX_CONFIG_FILE`が示すパスからBOX設定ファイルを読み込む。
環境変数を指定しない場合は、`resources/secrets/config.json`を読み込む。
`resources/secrets`は`.gitignore`で除外しているため、BOX設定ファイルをこのディレクトリに置けば、誤ってコミットすることはない。

```shell
BOX_CONFIG_FILE=path/to/config.json RUST_LOG=info cargo run --quiet --example=me
```

各サンプルの実行方法は、各ファイルの`main`関数のドキュメントを参照する。

## 開発環境

### prek

Gitフックを導入する。
Gitフックは`cargo deny check`を実行するため、`cargo-deny`もインストールする。

```shell
cargo install cargo-deny
cargo install prek
prek install
```

## 履歴

### 0.1.0

- 2026-11-19
