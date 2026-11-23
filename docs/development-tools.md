# 開発ツール

本文書は、Rustの版、テスト、CI、依存関係の自動更新およびクレートの版の指定の方針を記述する。
Gitのフックの導入は、READMEの「開発環境」に記述する。

## Rustの版

本クレートでは、Rustの版を次の2か所で指定する。
2つの版は、目的が異なるため、揃えない。

| 指定 | 目的 |
| --- | --- |
| `rust-toolchain.toml`の`channel` | 開発者とCIが使うRustの版を固定し、`cargo fmt`やClippyの結果を揃える |
| `Cargo.toml`の`rust-version` | 本クレートの利用者がビルドできる、最小のRustの版（MSRV）を示す |

- `rustup`は、`rust-toolchain.toml`に指定した版のツールチェーンを、初めて`cargo`を実行したときに自動で導入する。
- 最新の安定版への追随は、[依存関係の自動更新](#依存関係の自動更新)で、`rust-toolchain.toml`を更新して行う。
- 本クレートは、利用者が依存して使うライブラリーである。
  `rust-version`を開発に使う版まで上げると、古いRustを使う利用者がビルドできなくなるため、`rust-version`は、必要になったときのみ手動で上げる。
- resolver 3は、`rust-version`の版でビルドできる依存クレートの版を、`Cargo.lock`に選ぶ。
  したがって、`Cargo.lock`を更新しても、MSRVでビルドできない版は取り込まれにくい。
- CIの`msrv`ジョブが、`rust-version`の版のツールチェーンで、ライブラリーをビルドできることを確認する。
  テストとサンプルの依存は、利用者のビルドに影響しないため、MSRVでは検査しない。

## テスト

テストランナーには、[`cargo-nextest`](https://nexte.st/)を使う。
`cargo-nextest`は、テストを別々のプロセスで並行して実行するため、`cargo test`より速く、失敗したテストの出力が読みやすい。

```shell
cargo nextest run --all-features
```

`cargo-nextest`は、ドキュメントテストを実行しない。
したがって、ドキュメントテストは、`cargo test`で別に実行する。

```shell
cargo test --doc --all-features
```

テストは、プッシュ時のGitのフックとCIで実行する。

## CI

CIは、GitHub Actionsの`.github/workflows/ci.yml`で実行する。
`main`ブランチへのプッシュと、`main`ブランチへのPull Requestで、次のジョブを並行して実行する。
同じブランチで新しい実行が始まった場合は、古い実行を取り消す。

Gitのフックは、手元で省略できる。
したがって、CIで同じ検査を実行し、CIを最終の関門とする。

検査の結果に影響しない変更でCIを実行すると、結果の変わらない検査に時間を使い、ランナーの実行時間を消費する。
したがって、CIは、検査の結果に影響する次のファイルを変更した場合のみ実行する。
次のファイルを1つでも含む変更では、すべてのジョブを実行する。
READMEや`docs/`などのドキュメント、ライセンスおよびmarkdownlintの設定のみの変更では、CIを実行しない。

- Rustのコードとテストの資材である`src/`、`examples/`、`tests/`、`benches/`、`build.rs`および`resources/tests/`
- `Cargo.toml`、`Cargo.lock`および`rust-toolchain.toml`
- 検査の設定である`deny.toml`、`prek.toml`および`taplo.toml`
- CIの定義である`.github/workflows/ci.yml`

対象のファイルは、ワークフローの`paths`に列挙する。
ファイルを追加して検査の対象が増えた場合は、`paths`にも加える。
対象のファイルを変更していない場合でも、GitHubの画面からCIを手動で実行できる。

| ジョブ | 内容 |
| --- | --- |
| `hooks` | `prek run --all-files`で、コミット時のGitのフックを実行する。Clippyは`clippy`ジョブで実行するため省略する |
| `clippy` | `cargo clippy --all-targets --all-features -- -D warnings`を実行する |
| `tests` | `cargo nextest run`と`cargo test --doc`で、単体テストとドキュメントテストを実行する |
| `msrv` | `Cargo.toml`の`rust-version`の版で、`cargo check --lib`を実行する |
| `deny` | `cargo deny check`で、依存クレートの勧告、ライセンスおよび取得元を検査する |

- 第三者のActionsは、タグを差し替えられても内容が変わらないよう、コミットのハッシュで指定する。
- ジョブごとに、`timeout-minutes`で実行時間の上限を設ける。
- `cargo`のコマンドには`--locked`を指定し、`Cargo.lock`と`Cargo.toml`が食い違う場合は失敗させる。
- `deny`ジョブは、Dependabotが`Cargo.lock`を更新しない間に公開された勧告も検出する。

## 依存関係の自動更新

依存関係は、手動で追随すると更新が滞り、脆弱性の修正を取り込み遅れる。
したがって、GitHubの[Dependabot](https://docs.github.com/ja/code-security/dependabot)で、更新のPull Requestを自動で作成し、CIで検査してからマージする。
設定は、`.github/dependabot.yml`に記述する。

| 対象 | 更新の方法 |
| --- | --- |
| クレート | Dependabotは、`Cargo.toml`の版の範囲内の更新では`Cargo.lock`のみを更新し、範囲を超える更新では`Cargo.toml`と`Cargo.lock`を更新する。マイナーとパッチの更新は、1つのPull Requestにまとめる |
| Rustの版 | Dependabotは、`rust-toolchain.toml`を更新する。`Cargo.toml`の`rust-version`は更新しない |
| GitHub Actions | Dependabotは、コミットのハッシュと、コメントの版を更新する。マイナーとパッチの更新は、1つのPull Requestにまとめる |

その他の方針は、次のとおりである。

- Dependabotは、`Asia/Tokyo`の時刻で毎週月曜日の早朝に、更新を作成する。
- Dependabotは、公開から3日を経過していない版を取り込まない。
  公開の直後に取り下げられた版や、乗っ取られて公開された版を、取り込まないためである。
- Dependabotは、直接の依存のみを更新し、間接的な依存は更新しない。
  したがって、開発者は、間接的な依存を含む`Cargo.lock`の全体を、`cargo update`で手動で更新する。
- Dependabotは、ワークフローの`prek-version`のような、Actionsの入力に書いたツールの版を更新しない。
  したがって、開発者は、これらの版を手動で更新する。

## クレートの版の指定

本クレートは、利用者が依存して使うライブラリーである。
Cargoは、利用者のプロジェクトのすべての依存を通じて、1つのクレートにつき、互換性のある版を1つに揃える。
したがって、`Cargo.toml`の版の下限を高くすると、利用者が使える版の組み合わせを狭める。

- 版は、キャレット指定で書く。キャレット指定は、演算子を付けない指定である。
  - `=`による完全固定は、利用者の依存と衝突し、脆弱性の修正版も取り込めなくなるため、使わない。
  - 下限は、本クレートが使う機能を含む版にする。
    `Cargo.lock`の版に揃えて上げることはしない。
- `Cargo.lock`はリポジトリに含め、開発とCIで使う版を固定する。
  `Cargo.lock`は、利用者のビルドには使われない。
- 既定のフィーチャーを無効にする場合や、特定のフィーチャーを選ぶ場合は、理由をコメントに記述する。
- 使っていない依存は、`cargo`が警告するため、削除する。
  テストとサンプルのみで使うクレートは、`[dev-dependencies]`に置く。
