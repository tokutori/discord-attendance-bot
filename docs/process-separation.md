# 記録系と表示系の分離（Issue #5）

## 採用した構成

記録系は既存の `discord-attendance-bot`、表示系は新しい `attendance-view` とする。
同一ホストの同一 SQLite DB を、別プロセス・別 Discord Application から利用する。
既存の記録コマンドと DB schema は変更しない。表示側の交換に IPC サーバーや記録プロセスの再起動は不要。

```text
discord-attendance-bot (記録用 Application)
  ├─ 記録 command / service / repository / migration / auto-end
  ├─ attendance-query の SELECT 関数
  └─ attendance-shared (モデル・時刻・設定・文字処理)
             │ read/write
          SQLite + WAL
             │ readonly
attendance-view (表示用 Application)
  ├─ status / history / month / list / CSV / PDF / topic / Activity
  ├─ attendance-query::ReadDatabase (非公開 pool)
  └─ attendance-shared
```

`attendance-view` と `attendance-query` は記録 crate に依存しない。
記録 crate は view や printpdf に依存しない。共通モデルには更新要求のデータ型もあるが、
mutation service・transaction・migration 実行機能は記録 crate にのみ置く。
CI の `scripts/check-boundaries.py` は間接依存も検査する。
記録操作への成功・エラー・確認 Embed とそのヘルプは、記録プロトコルの一部として記録側に残す。
これらは view のテンプレートに依存せず、表示側の変更で記録バイナリを更新する必要はない。

## Application とコマンドの所有権

Discord の bulk overwrite は Application ごとの登録一覧を置換する。
同一 Application を複数 Gateway プロセスで共有すると、登録・interaction 応答・presence の所有権が複雑になるため採用しない。
将来、単一 Application に戻す場合は interaction router と registration owner を別途設計する。

| 操作 | 所有者・新しいコマンド |
|---|---|
| start/end/continue/edit/delete/revert/confirm/erase、join/exit | 記録側、既存名を維持 |
| 本人プロフィール設定・解除 | 記録側 `/attendanceexport userconfig`、`clearuserconfig` を維持 |
| status/list/history/month | 表示側 `/attendanceview status` 等へ移動 |
| CSV/PDF export | 表示側 `/attendanceview export` へ移動 |
| topic/Activity | 表示用 Bot のみ。記録確定後の即時更新から定期読取へ変更 |
| 自動終了通知の既読更新 | 記録側の応答成功後のみ |

表示側は通知を閲覧しても DB を更新しない。そのため、記録側で通知が既読になるまで繰り返し表示する。
プロフィールの引数省略時の参照と記録用ヘルプは、安定した記録コマンドとの互換性のため記録側に残す。
表示 command が失敗しても自動終了・記録 command は表示側を呼び出さない。

表示側には `DISCORD_VIEW_TOKEN` と `DISCORD_CORE_APPLICATION_ID`（記録 Bot の Application ID、秘密値ではない）を設定する。
表示側は Ready の Bot ID が指定された記録 ID と一致する場合、コマンド登録前に拒否する。
誤った core ID まで検知する仕組みではないため、運用者は正しい ID と別 Application の token を設定する。
各プロセスは自分の Application の guild command のみ登録する。

## DB の保護と互換性

- 表示接続は `read_only(true)` と `query_only=ON`。URL 指定より強く readonly を適用する。
- URL からはファイル名だけを取り出す。`immutable=true` や `mode=rwc` を渡しても接続設定は変更できない。
- pool/connection/任意 SQL の実行 API を表示用 `ReadDatabase` から公開しない。
- 書き込み可能 pool を表示用 handle に変換する API は設けない。
- 読取は結果をメモリへ取得して接続を返してから表示・PDF 生成を行う。生成中に read transaction を保持しない。
- 起動時に SQLx migration 台帳が `version=1, success=true` の1行のみであることと、必要なテーブル・列を検査する。
  未作成・未移行・非対応の DB は表示側だけ起動失敗し、migration は実行しない。
- 将来 schema を変更するときは新 migration を追加し、query の対応 version とテストも更新する。
  schema 更新時は view を停止してから core を更新し、対応する view を起動する。
  起動済み view に対する schema の自動再交渉は行わない。

SQLx の readonly は接続単位の保護であり、同じ OS 権限で動く任意のプログラム全体の sandbox ではない。
Docker の view は DB volume を `:ro` でマウントし、バックアップ volume と記録 token を渡さない。
コンテナ root filesystem も readonly とし、メモリ・CPU・プロセス数を制限する。
ソースからの直接実行で同等の保護を得るには、別 OS ユーザーと DB ディレクトリの読み取り専用 ACL が必要。
ホスト障害、ディスク満杯、共通 Discord/ネットワーク障害、WAL を長期間保持する別 reader まで独立するわけではない。

WAL 方式の readonly reader は既存の `-wal`・`-shm` を読める必要がある。
core を先に起動して DB・migration・WAL の初期化を完了させる。
記録側は `WalAnchor` で pool 外の専用接続を保持する。最初の autocommit read で WAL を開き、
以後 command には貸し出さず、記録プロセス終了まで保持する。pool の idle timeout / max lifetime によって
全接続が回収されても、最後の SQLite 接続が閉じる状態を作らない。pool の最大5接続とは別に1接続を使う。
anchor は read transaction を保持しないため、checkpoint/TRUNCATE を妨げない。
プロセスの終了・DB ファイル交換・ストレージ障害からの継続を保証するものではなく、復元時は両プロセスを停止する。
view が先に起動して失敗しても core へは影響せず、Compose の restart policy で再試行する。
`immutable` は live DB の更新検知を無効にするため使わない。
DB はローカルディスクに置く。バックアップは従来どおり記録側の `attendance-maintenance` と `VACUUM INTO` を使う。
復元・DB ファイル交換時には **両プロセス**を停止する。稼働中の DB 本体だけをコピーしない。

## 既存運用からの移行

1. 従来の手順で検証済みバックアップを作成する。既存 DB に schema 変更はない。
2. 表示用 Discord Application を新規作成し、同じ Guild へ追加する。
   表示側には View Channel / Send Messages / Embed Links / Attach Files を付与する。
   Topic を使う場合だけ対象チャンネルの Manage Channels も付与する。
3. ルートの `.env.example` を唯一の見本とし、Compose用の `.env` に記録用・表示用の項目を設定する。
   既存の `.env` を上書きせず、不足する表示側専用項目を運用者が追加する。
   Composeは共通のGuild・timezone・auto-end policyを両者へ渡し、トークンなどの専用項目をサービス別に限定する。
   `env_file` でファイル全体を渡さず、`.env` 自体もコンテナへマウントしない。Compose用の `.env.view` は不要。
4. 記録用イメージを更新し `docker compose up -d --no-deps bot` で初回の切替を行う。
   この初回導入では記録プロセスの再起動が必要。再登録により旧表示 subcommand は記録 Bot から消える。
5. `docker compose --profile view build view`、続いて
   `docker compose --profile view up -d --no-deps view` で表示側を起動する。
6. 利用者へ `/attendanceview` への変更と表示用 Bot を案内する。
   `/join`・`/exit` を含む記録操作が view 停止中も利用できることをテスト Guild で確認する。

以後、表示側だけの更新は次の手順で行う。bot service は再作成しない。

```powershell
docker compose --profile view build view
docker compose --profile view up -d --no-deps view
# 表示側だけ停止・再開
docker compose --profile view stop view
docker compose --profile view start view
```

ソース運用でも別 OS ユーザー・別環境で各 binary を起動する。
記録側は従来の `.env`、表示側はカレントディレクトリの `.env.view` だけを読み込む。
DB URL には同じ DB の絶対パスを設定する。
見本は共通の `.env.example` を使い、記録側のファイルには「共通」と「記録側専用」、表示側のファイルには「共通」と「表示側専用」の項目だけを設定する。
view 用 `.env.view` に記録 token を含めない。直接実行ではComposeの環境変数制限が適用されないため、この運用上の分離を維持する。

```powershell
cargo build --locked --release -p discord-attendance-bot --bins
cargo build --locked --release -p attendance-view --bin attendance-view
```

`ATTENDANCE_STATUS_REFRESH_SECONDS` の既定は600秒。Activity/topic は最後の取得状態を表示するため、
view 停止中は古い topic が残り得る。再開後の定期取得で更新される。
個人データ消去後も、既存 topic は次の更新まで残り、過去の帳票は従来どおり削除対象外。

## 検証範囲

`python scripts/check-compose-config.py` は一時ディレクトリのダミー設定だけでComposeを展開する。ホストの `.env` やDockerの認証設定を読まず、プロセス環境も設定値を引き継がない。サービス別の変数一覧、共通設定の一致、viewのreadonly volume、maintenanceへのトークン非注入を検査する。Docker daemonやDiscord接続は不要。

ローカルの自動テストでは、実ファイル WAL DB を使い、readonly の全接続で DML/DDL 拒否、
live commit の可視性、view 再接続、通知が既読にならないこと、スキーマ拒否を検査する。
別子プロセスの panic/kill 後も既存の記録 service で終了・再開ができることを確認する。
既存の所有者制約・確認処理・取り消し・自動終了・帳票テストも各 crate で維持する。
これは Discord Gateway の end-to-end テストではない。

追加の `tests/wal_lifetime.rs` は idle timeout と max lifetime をそれぞれ短縮し、
物理 pool 接続数が実際に0になるまで待つ。記録操作を挟まず reader を起動し、3回の接続回収・再生成と
WAL truncate 成功を検証する。これは通常ファイルシステムでの SQLx テストであり、OS 権限の検証とは分ける。

CI の `scripts/check-container-runtime.py` はネットワークなし・token なしの専用 probe を実行する。
SQLx 0.9 / SQLite 3.51.3 を使用し、anchor なしでは idle 回収後の readonly 起動が失敗することを対照実験とする。
この組み合わせの Docker 実験では、DB 本体が存在しても最初の読取で `SQLITE_CANTOPEN (14)` を返した。
レビュー側の SQLite 3.46.1 実験の `SQLITE_READONLY_DIRECTORY (1544)` と、起動失敗という性質は同じだがエラーコードは異なる。
anchor ありでは実際の readonly volume mount で reader が起動できること、DB/WAL/SHM の書込用 open と
ファイル新規作成が OS に拒否されることを検査する。reader の停止・kill・再作成中も writer の PID を変えずに
記録を追加し、再開した reader が全3件を取得することを確認する。probe は記録 service を使うが Discord command の代用ではない。
PDF probe は production view と同じ runtime image のフォント・ユーザー・ライブラリを使い、
Compose 相当の512 MiB・1 CPU・128 PID制限のもとで2種類の PDF を生成する。フォント欠如時の skip はない。
PDF の外形検査であり、日本語の字形・レイアウトや Discord 添付の目視検証ではない。

## 本番 DB に接続する query の変更契約

`attendance-query` とその SQL・接続管理は **安定側** として扱う。表示だけの差し替えでは変更しない。
自由に交換する対象は、取得済み DTO に対する Embed、集計表、CSV/PDF レイアウトなどの処理とする。
SQL の追加・変更は記録系と同じレビュー対象とし、実データ規模で取得件数・実行時間・WAL 増加を検証する。
表示側から任意 SQL や transaction handle を公開しない現在の API を維持し、PDF 生成中は接続を保持しない。
運用者は WAL とディスク空き容量を監視し、異常な増加時にはまず view を停止する。

この契約は実装・運用上の制約であり、現在の API にクエリ時間・取得量の強制上限があるという意味ではない。
readonly reader も長い read transaction により WAL 回収を妨げ、ディスク満杯を通じて記録側へ影響し得る。
実験的 SQL を自由に実行する用途には、本番 DB を直接マウントせず、記録側で作った整合 snapshot を渡すか、
取得量・処理時間を制限する安定 query service を別途設計する。今回の変更にはその強い分離までは含めない。

## 依存監査への対応

比較元にも存在した `chacha20 0.10.1` の yank 警告を、互換 patch `0.10.2` への限定更新で解消する。
依存経路は `printpdf → lopdf → rand → chacha20`。上流の [変更履歴](https://github.com/RustCrypto/stream-ciphers/blob/master/chacha20/CHANGELOG.md)
と [修正 #580](https://github.com/RustCrypto/stream-ciphers/pull/580) で SSE2 backend の SSE4.1 命令使用の修正を確認した。
`cargo audit --deny warnings` は維持し、警告の無視設定は追加しない。

レビュー時の実機確認: 別 Application の登録・権限・添付ファイル・topic/Activity、
view 停止中の Discord 記録 command、復旧後の表示を確認する。
秘密情報を使う Discord 接続は実装時に実行しない。

## 参照

- [Issue #5](https://github.com/tokutori/discord-attendance-bot/issues/5)
- [Discord: Application Commands / Bulk Overwrite Guild Application Commands](https://docs.discord.com/developers/interactions/application-commands#bulk-overwrite-guild-application-commands)
- [SQLx: SqliteConnectOptions（read_only / immutable / journal_mode）](https://docs.rs/sqlx/0.9.0/sqlx/sqlite/struct.SqliteConnectOptions.html)
- [SQLite: Write-Ahead Logging / Read-Only Databases](https://www.sqlite.org/wal.html#read_only_databases)
