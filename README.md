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
  BOXはルートフォルダでマーカー方式のページ送りと並び替えに使用する属性の指定を併用できないため、ルートフォルダで並び替えに使用する属性を指定した場合は、リクエストを送信せずにエラーを返す。

#### フォルダの削除

- メソッド: `BoxClient::delete_folder`
- BOX API: `DELETE /folders/{folder_id}`
- フォルダを削除する。
  BOXは、ゴミ箱が有効な場合はフォルダをゴミ箱に移動し、無効な場合はフォルダを完全に削除する。
- 空でないフォルダを削除するかどうか、およびETagを、`DeleteFolderOptions`で指定する。
- ルートフォルダを指定した場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### フォルダの移動

- メソッド: `BoxClient::move_folder`
- BOX API: `PUT /folders/{folder_id}`
- フォルダを、フォルダ内のすべての項目とともに別のフォルダへ移動する。
- 移動先は`MoveDestination`で指定する。
  - `MoveDestination::Folder`: フォルダIDで指定したフォルダへ移動する。
    認証中のユーザーのルートフォルダへ移動する場合は、`FolderId::root()`を指定する。
  - `MoveDestination::User`: ユーザーIDで指定したユーザーのルートフォルダへ移動する。
    BOXはユーザーIDを指定した場合にルートフォルダ以外への移動を受け付けないため、ルートフォルダのIDとユーザーIDを送信する。
    ユーザーIDが空文字列の場合は、リクエストを送信せずにエラーを返す。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
- BOXは移動先の親フォルダと他の属性を同時に指定すると拒否するため、ボディでは移動先の親フォルダだけを送信する。
- ルートフォルダを移動しようとした場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### フォルダ名の変更

- メソッド: `BoxClient::rename_folder`
- BOX API: `PUT /folders/{folder_id}`
- フォルダ名を変更する。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
- ルートフォルダを指定した場合、またはBOXが拒否するフォルダ名を指定した場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### フォルダの説明の設定

- メソッド: `BoxClient::set_folder_description`
- BOX API: `PUT /folders/{folder_id}`
- フォルダの説明を設定する。
  説明を削除する場合は、空文字列を指定する。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
- ルートフォルダを指定した場合、または説明が256文字（`MAX_DESCRIPTION_CHARS`）より長い場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### フォルダのコピー

- メソッド: `BoxClient::copy_folder`
- BOX API: `POST /folders/{folder_id}/copy`
- フォルダを、フォルダ内のすべての項目とともに別のフォルダへコピーする。
- コピーしたフォルダの名前を`CopyFolderOptions`で指定できる。
  指定しない場合、BOXはコピー元のフォルダと同じ名前を付ける。
- BOXは、項目数が500を超えるフォルダを非同期にコピーし、コピーの完了を待たずにフォルダを返す。
  BOXはコピーの完了を確認するAPIを提供していない。
- ルートフォルダを指定した場合、またはBOXが拒否するフォルダ名を指定した場合は、リクエストを送信せずにエラーを返す。
  BOXはルートフォルダのコピーに500を返すため、クライアントで検出する。
- HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。

### ファイル

#### ファイル情報の取得

- メソッド: `BoxClient::file_information`
- BOX API: `GET /files/{file_id}`
- ファイル情報を取得する。

#### ファイルのバージョン情報の取得

- メソッド: `BoxClient::file_version`
- BOX API: `GET /files/{file_id}/versions/{file_version_id}`
- ファイルの特定のバージョンの情報を取得する。
  BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。
- ファイル情報に含まれるバージョン（`File::file_version`）と同じ`FileVersion`型で返す。
  ファイル情報に含まれるバージョンはIDとSHA-1ハッシュ値だけを持つため、それ以外の属性は`Option`で受け取る。

#### ファイルのバージョンのリスト

- メソッド: `BoxClient::list_file_versions`および`BoxClient::list_all_file_versions`
- BOX API: `GET /files/{file_id}/versions`
- ファイルの過去のバージョンをリストする。
  BOXは、プレミアムアカウントのユーザーのファイルだけにバージョンを記録する。
  現在のバージョンのIDは、`BoxClient::file_information`で取得したファイル情報の`file_version`で確認できる。
- BOXは、バージョンのリストをオフセット方式でページ送りする。
  - `list_file_versions`は、指定したオフセットから1ページ分のバージョンを返す。
  - `list_all_file_versions`は、取得したバージョンの数が`total_count`に達するまでページを送り、すべてのバージョンを返す。
- 1ページで取得するバージョン数の上限を、`ListFileVersionsOptions`で指定する。
  範囲外の値を指定した場合は、リクエストを送信せずにエラーを返す。
- BOXはオフセットが10000（`MAX_FILE_VERSIONS_OFFSET`）を超えるリクエストを拒否するため、範囲外のオフセットを指定した場合は、リクエストを送信せずにエラーを返す。
  `list_all_file_versions`で次のページのオフセットが10000を超える場合は、一部のバージョンだけを返さないようにエラーを返す。

#### ファイルのバージョンの昇格

- メソッド: `BoxClient::promote_file_version`
- BOX API: `POST /files/{file_id}/versions/current`
- ファイルの過去のバージョンを昇格し、現在のバージョンにする。
  BOXは、指定したバージョンの複製を新しいバージョンとして作成し、バージョン履歴の先頭に置く。
  新しいバージョンは、指定したバージョンと同じ内容、ハッシュ値、ETagおよび名前を持つ。
- 昇格によって作成された現在のバージョンを返す。昇格したバージョンとは異なるIDを持つ。
- HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。

#### ファイルのバージョンの削除

- メソッド: `BoxClient::delete_file_version`
- BOX API: `DELETE /files/{file_id}/versions/{file_version_id}`
- ファイルのバージョンを削除する。
  BOXは、削除したバージョンをゴミ箱に移動する。ゴミ箱に移動したバージョンは、`BoxClient::restore_file_version`で復元できる。
- ETagを指定した場合は、`If-Match`ヘッダーで送信する。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### 削除したファイルのバージョンの復元

- メソッド: `BoxClient::restore_file_version`
- BOX API: `PUT /files/{file_id}/versions/{file_version_id}`
- 削除してゴミ箱に移動したファイルのバージョンを、ファイルのバージョン履歴に戻す。
  ファイル自体をゴミ箱から復元する場合は、`BoxClient::restore_file`を使用する。
- `trashed_at`に`null`を指定して送信し、復元したバージョンを返す。
- 復元したバージョンを再び復元しても状態は変わらないため、冪等なリクエストとして送信する。

#### ファイル名の変更

- メソッド: `BoxClient::rename_file`
- BOX API: `PUT /files/{file_id}`
- ファイル名を変更する。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
- BOXが拒否するファイル名を指定した場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### ファイルの移動

- メソッド: `BoxClient::move_file`
- BOX API: `PUT /files/{file_id}`
- ファイルを別のフォルダへ移動する。
- 移動先は、フォルダの移動と同じく`MoveDestination`で指定する。
  - `MoveDestination::Folder`: フォルダIDで指定したフォルダへ移動する。
  - `MoveDestination::User`: ユーザーIDで指定したユーザーのルートフォルダへ移動する。
    ユーザーIDが空文字列の場合は、リクエストを送信せずにエラーを返す。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### ファイルのコピー

- メソッド: `BoxClient::copy_file`
- BOX API: `POST /files/{file_id}/copy`
- ファイルを別のフォルダへコピーする。
- コピーしたファイルの名前と、コピーするバージョンを`CopyFileOptions`で指定できる。
  - 名前を指定しない場合、BOXはコピー元のファイルと同じ名前を付ける。
  - バージョンを指定しない場合、BOXはファイルの現在のバージョンをコピーする。
- BOXが拒否する名前を指定した場合は、リクエストを送信せずにエラーを返す。
  BOXは使用できない文字を取り除いて受け付けるが、意図しない名前でコピーしないように、クライアントで拒否する。
- HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。

#### ファイルの説明の設定

- メソッド: `BoxClient::set_file_description`
- BOX API: `PUT /files/{file_id}`
- ファイルの説明を設定する。
  説明を削除する場合は、空文字列を指定する。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
- 説明が256文字（`MAX_DESCRIPTION_CHARS`）より長い場合は、リクエストを送信せずにエラーを返す。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### ファイルの削除

- メソッド: `BoxClient::delete_file`
- BOX API: `DELETE /files/{file_id}`
- ファイルを削除する。
  BOXは、エンタープライズの設定に従って、ファイルをゴミ箱に移動するか、完全に削除する。
- ETagを指定した場合は、`If-Match`ヘッダーで送信する。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

#### ファイルのアップロード

- メソッド: `BoxClient::upload_file`
- BOX API
  - `POST /files/content`
  - `POST /files/upload_sessions`
  - `PUT /files/upload_sessions/{upload_session_id}`
  - `POST /files/upload_sessions/{upload_session_id}/commit`
  - `DELETE /files/upload_sessions/{upload_session_id}`
- ローカルのファイルを、指定したフォルダにアップロードする。
  BOXに保存するファイル名を`UploadFileOptions`で指定しない場合は、パスのファイル名を使用する。
- BOXは、アップロードをREST APIとは別のホスト（`upload.box.com`）で受け付ける。
- 分割アップロードに切り替えるファイルサイズは、`BoxClientBuilder::chunked_upload_threshold`で指定する（既定値は`DEFAULT_CHUNKED_UPLOAD_THRESHOLD`の50MiB）。
  20MiB（`MIN_CHUNKED_UPLOAD_THRESHOLD`）以上50MiB（`MAX_CHUNKED_UPLOAD_THRESHOLD`）以下で指定し、範囲外の値は`BoxClientBuilder::chunked_upload_threshold`でエラーになる。
  分割アップロードのAPIは20MB未満のファイルを拒否するため、最小値を20MiBとする。
  また、BOXは50MBを超えるファイルに分割アップロードを推奨しているため、最大値を50MiBとする。
- ファイルサイズが分割アップロードに切り替えるファイルサイズ以下の場合は、ファイル全体をメモリに読み込み、1回のリクエストでアップロードする。
  メモリの使用量を抑える場合、利用者は、分割アップロードに切り替えるファイルサイズを小さくする。
  - `attributes`パート（ファイル名と親フォルダ）、`file`パートの順に、multipartで送信する。
    BOXは、`attributes`パートが`file`パートより後にあるリクエストを拒否する。
  - ファイルの内容のSHA-1を`Content-MD5`ヘッダーで送信し、BOXに転送中の破損を検出させる。
- ファイルサイズが分割アップロードに切り替えるファイルサイズを超える場合は、分割アップロードのAPIを使用する。
  1. アップロードセッションを作成する。BOXは、部分のサイズと部分の数を返す。
  2. ファイルを先頭から部分のサイズごとに読み込み、アップロードする。
     部分は、`BoxClientBuilder::upload_concurrency`で指定した数（既定値は`DEFAULT_UPLOAD_CONCURRENCY`の4）まで並列にアップロードする。
     並列数は1（`MIN_UPLOAD_CONCURRENCY`）以上5（`MAX_UPLOAD_CONCURRENCY`）以下で指定し、範囲外の値は`BoxClientBuilder::upload_concurrency`でエラーになる。
     BOXは、3から5の部分を並列にアップロードすることを推奨している。
     メモリには、並列にアップロードしている部分だけを読み込むため、並列数と部分のサイズの積のメモリを使用する。
     いずれかの部分のアップロードに失敗した場合は、アップロード中の部分を取り消す。
     部分のSHA-1を`Digest`ヘッダーで、部分の範囲を`Content-Range`ヘッダーで送信する。
  3. ファイル全体のSHA-1を`Digest`ヘッダーで送信し、アップロードセッションをコミットする。
     部分は完了した順に並ぶため、ファイルの先頭からの位置の順に並べ替えてから送信する。
     BOXが部分の処理を終えていないことを示す202を返した場合は、`Retry-After`ヘッダーが示す時間だけ待機してから、コミットを再送する。
     再送する回数の上限は、`BoxClientBuilder::max_commit_polls`で指定する（既定値は`DEFAULT_MAX_COMMIT_POLLS`の30）。
     上限には100（`MAX_COMMIT_POLLS_LIMIT`）以下を指定し、範囲外の値は`BoxClientBuilder::max_commit_polls`でエラーになる。
  4. 部分のアップロードまたはコミットに失敗した場合は、アップロードセッションを中止してからエラーを返す。
- ファイル名をBOXが拒否する場合、またはパスからファイル名を取得できない場合は、リクエストを送信せずにエラーを返す。
- 1回のリクエストでのアップロード、アップロードセッションの作成およびコミットは、HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。
  部分のアップロードは、同じ範囲を同じ内容で送り直すため、冪等なリクエストとして送信する。
- 1回のリクエストで送信するデータが大きいため、利用者は、`BoxClientTimeout::request_timeout`を、通信速度に応じて、分割アップロードに切り替えるファイルサイズまたは部分のサイズを送信できる長さにする。

#### アップロードのプリフライトチェック

- メソッド: `BoxClient::preflight_upload_file`
- BOX API: `OPTIONS /files/content`
- ファイルの内容を送信せずに、BOXがファイルを受け付けるかを確認する。
  BOXは、ファイル名の重複、ファイルサイズの上限、およびフォルダとアカウントのストレージの容量を、実際のアップロードと同じように検査する。
- 引数は`BoxClient::upload_file`と同じであり、ファイル名とファイルサイズも同じ方法で決める。
  ファイルの内容は読み込まず、ファイルのメタデータだけを読み込む。
- アップロードと異なり、REST APIのホスト（`api.box.com`）で受け付ける。
- 確認に合格した場合は、BOXが返すアップロード用のURL（`upload_url`）とアクセストークン（`upload_token`）を`UploadUrl`として返す。
  `BoxClient::upload_file`はこれらを使用しない。
  アクセストークンはログやデバッグ出力に残らないように`SecretString`で保持する。
  BOXが成功レスポンスのボディを返さない場合は、どちらも`None`とする。
- BOXがファイルを受け付けない場合は、BOXのエラーレスポンスを返す。
  例えば、同じ名前のファイルが親フォルダに存在する場合は409を返し、レスポンスボディの`context_info.conflicts`に名前が重複するファイルが含まれる。
- 確認に合格しても、その後のアップロードが成功するとは限らない。
- 分割アップロードでは、BOXがアップロードセッションの作成時に同じ検査を行う。

#### ファイルの内容の更新のプリフライトチェック

- メソッド: `BoxClient::preflight_update_file_content`
- BOX API: `OPTIONS /files/{file_id}/content`
- ファイルの内容を送信せずに、BOXが既存のファイルの新しいバージョンを受け付けるかを確認する。
- 引数は`BoxClient::update_file_content`と同じであり、ファイルサイズと、指定した場合は新しいファイル名を送信する。
  BOXのドキュメントは、このAPIが`If-Match`ヘッダーを受け付けることを記載していないため、`UpdateFileContentOptions`のETagは送信しない。
- 戻り値、エラーおよび注意点は、「[アップロードのプリフライトチェック](#アップロードのプリフライトチェック)」と同じである。

#### ファイルの内容の更新

- メソッド: `BoxClient::update_file_content`
- BOX API
  - `POST /files/{file_id}/content`
  - `POST /files/{file_id}/upload_sessions`
  - `PUT /files/upload_sessions/{upload_session_id}`
  - `POST /files/upload_sessions/{upload_session_id}/commit`
  - `DELETE /files/upload_sessions/{upload_session_id}`
- ローカルのファイルの内容で、BOXのファイルの内容を更新する。
  BOXは、ファイルの新しいバージョンとして内容を保存し、以前の内容をバージョン履歴に残す。
- 新しいファイル名とETagを、`UpdateFileContentOptions`で指定できる。
  - 新しいファイル名を指定した場合、BOXは内容の更新とともにファイル名を変更する。
    指定しない場合は、ファイル名を変更しない。
  - ETagを指定した場合は、1回のリクエストでのアップロード、または分割アップロードのコミットで、`If-Match`ヘッダーとして送信する。
- ファイルサイズによるアップロード方法の切り替え、分割アップロードの進め方、および冪等性は、「[ファイルのアップロード](#ファイルのアップロード)」と同じである。
  分割アップロードでは、アップロードセッションを`POST /files/{file_id}/upload_sessions`で作成する。
- 新しいファイル名をBOXが拒否する場合は、リクエストを送信せずにエラーを返す。

#### ファイルのダウンロード

- メソッド: `BoxClient::download_file`
- BOX API: `GET /files/{file_id}/content`
- ファイルの内容をダウンロードし、ローカルのファイルに保存する。
  保存したファイルのバイト数を返す。
- ダウンロードするバージョンを`DownloadFileOptions`で指定できる。
  指定しない場合は、ファイルの現在のバージョンをダウンロードする。
- BOXは、`dl.boxcloud.com`のダウンロード用のURLへリダイレクトする302を返す。
  HTTPクライアントはリダイレクトを自動で追跡する。
  HTTPクライアントは、ホストが異なるURLへリダイレクトするときに`Authorization`ヘッダーを削除するため、`BoxClient`はアクセストークンを`dl.boxcloud.com`へ送信しない。
- `BoxClient`は、内容をメモリに読み込まずに、受信した順に保存先と同じディレクトリの一時ファイルへ書き込む。
  すべての内容を受信した後に、一時ファイルを保存先に移動する。
  - ダウンロードに失敗した場合、`BoxClient`は一時ファイルを削除し、保存先に存在したファイルも変更しない。
  - 保存先にファイルが存在する場合、`BoxClient`はダウンロードしたファイルで置き換える。
  - 保存先の親ディレクトリは、呼び出し元が作成しておく。
- BOXは、アップロードの直後などでダウンロードの準備ができていない場合に、202と`Retry-After`ヘッダーを返す。
  この場合、`BoxClient`は`Retry-After`ヘッダーが示す時間だけ待機してから、リクエストを再送する。
  再送する回数の上限は、`BoxClientBuilder::max_download_polls`で指定する（既定値は`DEFAULT_MAX_DOWNLOAD_POLLS`の10）。
  上限には100（`MAX_DOWNLOAD_POLLS_LIMIT`）以下を指定し、範囲外の値は`BoxClientBuilder::max_download_polls`でエラーになる。
  `BoxClient`は、上限に達した場合にエラーを返す。
- HTTPメソッドがGETであるため、冪等なリクエストとして送信する。
  受信の途中で失敗して再試行した場合、`BoxClient`は一時ファイルを作成し直し、内容を先頭から書き込む。
- ダウンロードの時間はファイルサイズによって大きく変わるため、`BoxClient`は、ダウンロードに`BoxClientTimeout::request_timeout`を適用しない。
  代わりに、データを受信しない状態が`BoxClientTimeout::download_read_timeout`（既定値は30秒）だけ続いた場合に、`BoxClient`はタイムアウトさせる。
  - このタイムアウトは、リクエストを送信してからレスポンスヘッダーを受信するまでの時間と、レスポンスボディのデータを受信する間隔に適用し、データを受信するたびに計測をやり直す。
  - `BoxClient`は、ダウンロードに、ほかのAPIとは別のHTTPクライアントを使用する。
    reqwestのタイムアウトはHTTPクライアントごとに設定するためである。
  - ダウンロード全体の時間を制限する場合、利用者は`tokio::time::timeout`などで制限する。
    futureを破棄した場合も、`BoxClient`は一時ファイルを削除する。

### Webリンク

#### Webリンク情報の取得

- メソッド: `BoxClient::web_link_information`
- BOX API: `GET /web_links/{web_link_id}`
- Webリンク情報を取得する。

#### Webリンクの作成

- メソッド: `BoxClient::create_web_link`
- BOX API: `POST /web_links`
- 指定したURLを指すWebリンクを、指定したフォルダに作成する。
- Webリンク名と説明を`CreateWebLinkOptions`で指定できる。
  Webリンク名を指定しない場合、BOXはURLをWebリンク名にする。
- URLが`http://`または`https://`で始まらない場合、Webリンク名をBOXが拒否する場合、または説明が256文字（`MAX_DESCRIPTION_CHARS`）より長い場合は、リクエストを送信せずにエラーを返す。
  Webリンク名の制限は、ファイル名およびフォルダ名と同じ検証を共有する。
- HTTPメソッドがPOSTであるため、冪等でないリクエストとして送信する。

#### Webリンクの更新

- メソッド
  - `BoxClient::rename_web_link`: Webリンク名を変更する。
  - `BoxClient::set_web_link_description`: Webリンクの説明を設定する。説明を削除する場合は、空文字列を指定する。
  - `BoxClient::move_web_link`: Webリンクを別のフォルダへ移動する。移動先は、ファイルおよびフォルダの移動と同じく`MoveDestination`で指定する。
  - `BoxClient::update_web_link_url`: WebリンクのURLを変更する。
- BOX API: `PUT /web_links/{web_link_id}`
- 名前、説明および移動先の検証とリクエストのボディは、ファイルおよびフォルダの同じ操作と共有する。
  URLは、`http://`または`https://`で始まることを検証する。
- ETagを`UpdateItemOptions`で指定した場合は、`If-Match`ヘッダーで送信する。
  BOXのドキュメントは、このAPIが`If-Match`ヘッダーを受け付けることを明記していないが、WebリンクのETagを`If-Match`ヘッダーで使用すると記載している。
- 冪等でないリクエストとして送信する。理由は「[冪等性の決定](#冪等性の決定)」を参照する。

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
- 1ページで取得する項目数の上限を、`ListTrashedItemsOptions`で指定する。
  BOXは、マーカー方式のページ送りで並び替えに使用する属性または並び替えの方向を指定すると、400を返す。
  したがって、並び替えに使用する属性および並び替えの方向は指定できない。

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
失敗してから一定の間隔が経過した後にAPIが呼び出されると、`BoxClient`は再びバックグラウンドでアクセストークンを取得し直す。
失敗した直後からAPIが呼び出されるたびに取得し直すと、アプリケーションの認可が取り消された場合など、再試行しても成功しない失敗で、トークンエンドポイントへのリクエストが期限切れまで続くためである。
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
| フォルダの移動（`BoxClient::move_folder`） | PUT | 冪等でない | BOXは移動に600秒以上かかると503を返すが、移動処理はその後も継続する。再送すると409を受け取り、ETagを指定した場合は412を受け取るため、移動の結果を誤って判断するおそれがある。 |
| フォルダ名の変更（`BoxClient::rename_folder`） | PUT | 冪等でない | ETagを指定した場合、BOXが名前を変更した後に再送すると、ETagが変わっているため412を受け取る。その結果、変更の結果を誤って判断するおそれがある。 |
| ファイルの移動（`BoxClient::move_file`） | PUT | 冪等でない | ETagを指定した場合、BOXがファイルを移動した後に再送すると、ETagが変わっているため412を受け取る。その結果、移動の結果を誤って判断するおそれがある。 |
| ファイル名の変更（`BoxClient::rename_file`） | PUT | 冪等でない | ETagを指定した場合、BOXが名前を変更した後に再送すると、ETagが変わっているため412を受け取る。その結果、変更の結果を誤って判断するおそれがある。 |
| ファイルの説明の設定（`BoxClient::set_file_description`） | PUT | 冪等でない | ETagを指定した場合、BOXが説明を設定した後に再送すると、ETagが変わっているため412を受け取る。その結果、設定の結果を誤って判断するおそれがある。 |
| Webリンクの更新（`BoxClient::rename_web_link`など） | PUT | 冪等でない | ETagを指定した場合、BOXが更新した後に再送すると、ETagが変わっているため412を受け取る。その結果、更新の結果を誤って判断するおそれがある。 |
| フォルダの説明の設定（`BoxClient::set_folder_description`） | PUT | 冪等でない | ETagを指定した場合、BOXが説明を設定した後に再送すると、ETagが変わっているため412を受け取る。その結果、設定の結果を誤って判断するおそれがある。 |
| ファイルの削除（`BoxClient::delete_file`） | DELETE | 冪等でない | BOXは削除済みのファイルに対して404を返す。BOXが削除した後に再送すると404を受け取り、削除の結果を誤って判断するおそれがある。 |
| ファイルのバージョンの削除（`BoxClient::delete_file_version`） | DELETE | 冪等でない | BOXが削除した後に再送すると、削除済みのバージョンに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。 |
| Webリンクの削除（`BoxClient::delete_web_link`） | DELETE | 冪等でない | BOXが削除した後に再送すると、削除済みのWebリンクに対する削除となり、エラーを受け取りうる。その結果、削除の結果を誤って判断するおそれがある。 |

フォルダの削除で503を受け取った場合は、`BoxClient::folder_information`でフォルダの状態を確認する。
フォルダの移動で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::folder_information`でフォルダの親フォルダを確認する。
フォルダ名の変更で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::folder_information`でフォルダ名を確認する。
フォルダの説明の設定で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::folder_information`でフォルダの説明を確認する。
ファイルの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::file_information`でファイルの状態を確認する。
Webリンクの削除で5xxを受け取った場合、またはタイムアウトした場合は、`BoxClient::web_link_information`でWebリンクの状態を確認する。

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
    .timeout(
        BoxClientTimeout::default()
            .with_request_timeout(Duration::from_secs(30))
            .with_connect_timeout(Duration::from_secs(10))
            .with_download_read_timeout(Duration::from_secs(30)),
    )
    .build()
    .await?;
```

### ログ

再試行に関するイベントは、`tracing`で構造化ログとして出力する。
ライブラリが利用者のアプリケーションのログを埋めないように、成功を知らせるイベントはDEBUGで出力し、利用者が対処を検討すべきイベントだけをWARNで出力する。
このクレートはERRORのイベントを出力しない。
エラーはすべて呼び出し元に返すため、エラーの重さは呼び出し元が判断し、必要に応じて記録する。
再試行できないエラーは、再試行せずにそのまま呼び出し元に返すため、DEBUGで出力する。
404のように呼び出し元が想定しているエラーもあるため、エラーを異常とみなすかどうかは呼び出し元が判断する。
フォルダの削除やゴミ箱からの復元など、各APIの処理が完了したことを知らせるイベントも、DEBUGで出力する。
401を受け取ったとき、JWTアサーションの有効期限を理由に拒否されたとき、およびアクセストークンのバックグラウンドでの取得に失敗したときのイベントを除き、各イベントは処理名`operation`および試行回数`attempts`を含む。
`operation`は、REST APIへのリクエストでは`box_api_request`、アクセストークンの取得では`fetch_access_token`である。
`retrying`以外のイベントは、最初のリクエストからの経過時間`elapsed_ms`も含む。

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

### キャンセル

リクエストを送信するfutureは、待機中または送信中に破棄しても状態を残さない。
したがって、`tokio::time::timeout`などで処理全体の時間を制限できる。
ただし、送信中に破棄した場合は、BOXがリクエストを処理したかどうかを判断できない。

アクセストークンをバックグラウンドで取得し直すタスクは、リクエストを送信するfutureを破棄しても中断せず、取得の成功または失敗まで実行する。

## 接続先、プロキシおよびUser-Agent

### User-Agent

`BoxClient`は、すべてのリクエストで、`User-Agent`ヘッダーにこのクレートの名前とバージョン（`box-client/<version>`）を送信する。
対象には、アクセストークンの取得とファイルのダウンロードも含む。
利用者が`BoxClientBuilder::user_agent`でアプリケーションの名前を指定した場合、`BoxClient`は、その後ろに空白とこのクレートの名前とバージョンを続けた`<user_agent> box-client/<version>`を送信する。
アプリケーションの名前を指定すると、BOXのログで呼び出し元のアプリケーションを識別しやすくなる。

空文字列、空白だけの文字列、およびヘッダーの値に使用できない文字を含む文字列は、`BoxClientBuilder::user_agent`でエラーになる。

### プロキシ

利用者がプロキシを指定しない場合、`BoxClient`は環境変数（`HTTPS_PROXY`、`HTTP_PROXY`、`ALL_PROXY`および`NO_PROXY`など）に従う。

- 利用者が`BoxClientBuilder::proxy`でプロキシのURLを指定した場合、`BoxClient`は、すべてのリクエストを指定したプロキシ経由で送信し、環境変数のプロキシを使用しない。
  対象には、アクセストークンの取得とファイルのダウンロードも含む。
- 利用者が`BoxClientBuilder::no_proxy`を呼び出した場合、`BoxClient`は、環境変数でプロキシが設定されていても、プロキシを使用せずに直接送信する。
- プロキシに認証が必要な場合、利用者は、`http://user:password@proxy.example.com:8080`のように、URLにユーザー名とパスワードを含める。
  `BoxClient`は、ユーザー名とパスワードを`Proxy-Authorization`ヘッダーで送信する。
  ユーザー名とパスワードに`@`や`:`などを含める場合、利用者はパーセントエンコードする。
- プロキシのURLのスキームは、`http`または`https`とする。
  クエリまたはフラグメントを含むURL、およびホストを含まないURLは、`BoxClientBuilder::proxy`でエラーになる。
- プロキシのURLは認証情報を含みうるため、`BoxClient`は、URLをDebug出力やエラーに含めない。

```rust
let client = BoxClient::builder(config)
    .proxy("http://user:password@proxy.example.com:8080")?
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
条件を満たさないURLは、各メソッドでエラーになる。

- スキームが`https`である。
  ただし、ホストが`localhost`またはループバックアドレス（`127.0.0.0/8`、`::1`）の場合は、モックサーバーに接続できるように`http`も受け付ける。
- ユーザー名とパスワード、クエリおよびフラグメントを含まない。

`BoxClient`は、JWTアサーションの`aud`クレームを、トークンエンドポイントのURLの設定によらず、BOXのトークンエンドポイントのURL（`https://api.box.com/oauth2/token`）とする。
BOXは、`aud`クレームがBOXのトークンエンドポイントのURLと一致することを求めるためである。
したがって、利用者がBOXに転送するプロキシをトークンエンドポイントに指定しても、BOXはJWTアサーションを受け付ける。

```rust
let client = BoxClient::builder(config)
    .user_agent("my-app/1.0")?
    .api_base_url("http://127.0.0.1:8080/2.0")?
    .upload_base_url("http://127.0.0.1:8080/api/2.0")?
    .token_url("http://127.0.0.1:8080/oauth2/token")?
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

```shell
cargo install cargo-deny
cargo install prek
prek install
```
