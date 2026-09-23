# Discord 活動時間記録 Bot

日本語圏のクラブ・チーム向けに、メンバーの活動時間をDiscordのSlash Command・常設ボタンから記録するセルフホスト型Botです。1つの実行プロセスが1つのDiscord Guildを担当し、データは運用者が管理するSQLiteファイルへ保存します。

中央サービス型のマルチGuild SaaS、多言語対応、給与・法定勤怠管理を目的とした製品ではありません。法令上の勤怠・賃金計算に使用する場合は、必要な要件を別途確認してください。

記録用 `discord-attendance-bot` と表示用 `attendance-view` を別プロセス・別 Discord Application として実行します。
表示側は SQLite を readonly で参照し、CSV/PDF・ステータス表示の停止や再起動が記録側を停止させません。
設定・更新手順は [分離設計と起動手順](docs/process-separation.md) を参照してください。

## 主な機能

- 活動の開始、終了、再開、修正、削除、取り消し
- 日本語のephemeral Embedによる本人向け応答
- 月別集計とCSV・PDF出力
- SQLiteの所有者制約、区間重複防止、WAL競合対策
- 設定可能なタイムゾーンと自動終了時刻
- Activity／チャンネルTopicを、無効・人数のみ・名前表示から選択
- 本人によるプロフィール解除と全個人データの完全消去
- 稼働中SQLiteの整合スナップショット、検証、復元用の保守コマンド

## コマンド

- `/attendance start [at] [note]`、短縮名 `/join`
- `/attendance end [at] [note]`、短縮名 `/exit`
- `/attendance continue`
- `/attendance revert`
- `/attendanceview status`
- `/attendanceview list`
- `/attendanceview history [limit]`
- `/attendanceview month [target]`
- `/attendance edit record [start] [end] [note]`
- `/attendance delete record`
- `/attendance confirm id`
- `/attendance erase confirmation:DELETE`
- `/attendance help`
- `/attendance panel`（管理者が実行チャンネルへ共用 join・exit ボタンを設置）
- `/attendanceview export month [mode] [confirm_public] [format]`
- `/attendanceexport userconfig [generation] [real_name] [role] [name_reading]`
- `/attendanceexport clearuserconfig`
- `/attendanceexport help`
- `/attendanceview help`

`at`は`HH:MM`、`target`は`YYYY-MM`、編集日時は`YYYY-MM-DD HH:MM`形式です。入力と表示には`ATTENDANCE_TIMEZONE`を使用し、DBにはUTC Unix timestampを保存します。夏時間などの日付境界の扱いは[日時境界の規則](docs/time-policy.md)を参照してください。

パネルの操作・再設置・重複排除・処理済み情報の保持方針は [常設パネル](docs/recording-panel.md) を参照してください。

## 必要環境

推奨経路はDockerです。ソースから実行する場合は次が必要です。

- Rust 1.94以上
- 記録用 Discord Application と Bot token（表示・出力も使う場合は別 Application と token を追加）
- Botを追加できるDiscordサーバー
- PDF出力に使う日本語TTF・OTF・TTCフォント

## Discord側の準備

1. 記録用と表示用それぞれについて [Discord Developer Portal](https://discord.com/developers/applications)でApplicationとBotを作成し、tokenを取得します。
2. Installationで`bot`と`applications.commands`を使用して対象サーバーへ追加します。
3. Developer Modeを有効にしてGuild IDをコピーします。
4. ステータスTopicを使う場合だけ専用テキストチャンネルを作り、Channel IDをコピーします。

BotはMessage Contentを読み取らないため、Privileged Gateway Intentsは不要です。必要なBot権限は次のとおりです。

- 記録側の通常応答: View Channel、Send Messages、Embed Links
- 表示側の通常応答と帳票: View Channel、Send Messages、Embed Links、Attach Files
- ステータスTopicを使う場合のみ: 対象チャンネルのManage Channels

Manage Channelsは、可能ならサーバー全体ではなく専用チャンネルへの権限上書きで付与してください。月次exportの実行者にはDiscordのManage Guild権限が必要です。

## 設定

設定見本はルートの `.env.example` に統一しています。Docker Composeでは、これを `.env` へコピーし、ダミー値を置き換えます。`.env`はGitへ追加しないでください。

```powershell
Copy-Item .env.example .env
```

Docker Composeでは、記録用・表示用の値を同じ `.env` に設定します。共通設定は両者へ、トークンと専用設定は必要なサービスだけへ渡します。`.env` ファイル自体をコンテナへ渡すことはありません。表示用に別の設定見本や `.env.view` を用意する必要はありません。詳細は [起動手順](docs/process-separation.md) を参照してください。

主要設定は次のとおりです。

| 変数 | 必須 | 既定値 | 説明 |
|---|---:|---|---|
| `DISCORD_TOKEN` | 記録側 | なし | 記録用 Bot token |
| `DISCORD_VIEW_TOKEN` | 表示側 | なし | 別 Application の表示用 Bot token |
| `DISCORD_CORE_APPLICATION_ID` | 表示側 | なし | 記録用 Bot の Application ID（誤登録防止） |
| `DISCORD_GUILD_ID` | はい | なし | このプロセスが担当するGuild ID |
| `DATABASE_URL` | はい | なし | 永続SQLite URL。例:`sqlite://attendance.db` |
| `ATTENDANCE_TIMEZONE` | いいえ | `Asia/Tokyo` | IANA timezone |
| `ATTENDANCE_AUTO_END_TIME` | いいえ | `21:00` | `HH:MM`、または`disabled` |
| `ATTENDANCE_STATUS_MODE` | いいえ | Channel IDがあれば`count`、なければ`disabled` | `disabled`、`count`、`names` |
| `ATTENDANCE_STATUS_CHANNEL_ID` | 条件付き | なし | status modeが`count`か`names`の場合に必須 |
| `ATTENDANCE_STATUS_REFRESH_SECONDS` | いいえ | `600` | 60～86400秒 |
| `ATTENDANCE_SQLITE_SYNCHRONOUS` | いいえ | `full` | `full`推奨。`normal`は電源断時に直近commitを失う可能性あり |
| `ATTENDANCE_PDF_FONT_PATH` | いいえ | OS候補を検索 | 日本語フォントへのパス |
| `RUST_LOG` | いいえ | info相当 | ログフィルタ |

永続運用モードではSQLxのURL解釈に基づいてインメモリSQLiteを拒否し、接続後にも実ファイルを持つことをmigration前に検査します。表示側も実ファイルを必須とします。SQLite WALはネットワークファイルシステム向けではないため、DBは実行ホストのローカルディスクまたはDockerのローカルvolumeに置いてください。同じGuild・DB・tokenに対して複数のBotプロセスを同時起動しないでください。設定対象外のGuildから届くコマンドはcore・viewとも拒否します。Guildやモードを切り替えた場合も、旧Guildに残ったコマンドから現在のDBを操作できません。

以前の`test` / `release`分離運用も互換性のため利用できます。起動引数を付けると、`.env.example`末尾に記載したモード別変数を使用します。

## Dockerで起動する

```powershell
docker compose build
docker compose up -d
docker compose logs -f bot
```

上の手順は記録側のみを起動します。同じ `.env` の表示側専用項目を設定後、`docker compose --profile view up -d --build --no-deps view` で表示側を追加します。

DBとバックアップはDocker named volumeへ保存されます。コンテナを削除しても、volumeを明示的に削除しない限りデータは残ります。`docker compose down -v`はDBとバックアップvolumeを削除するため、通常運用では実行しないでください。

## ソースから起動する

```powershell
cargo build --locked --release -p discord-attendance-bot --bins
cargo run --locked --release -p discord-attendance-bot --bin discord-attendance-bot
```

ソースを直接実行する場合は、同じ `.env.example` の「共通」と「記録側専用」だけを記録側の `.env` に、「共通」と「表示側専用」だけを表示側の `.env.view` に設定します。表示側のファイルに `DISCORD_TOKEN` を含めないでください。このファイル分離は直接実行用であり、Composeでは不要です。

表示側は `cargo run --locked --release -p attendance-view --bin attendance-view` で起動します。

互換モードは次のように起動します。

```powershell
cargo run --locked -p discord-attendance-bot --bin discord-attendance-bot -- test
cargo run --locked --release -p discord-attendance-bot --bin discord-attendance-bot -- release
```

初回起動時にSQLiteファイルとmigration tableを自動作成します。Bot起動時には実行中SQLiteの版を検査し、既知のWAL-reset破損バグの影響を受ける版では起動しません。本リポジトリは修正版SQLite 3.51.3を同梱する`libsqlite3-sys`を固定しています。

## 自動終了とステータス表示

自動終了が有効な場合、設定タイムゾーンの毎日0時に処理します。開始または再開した時刻から見て次に到来する設定時刻を自動終了期限とし、Bot停止中の未処理分は次回起動時に補完します。

`ATTENDANCE_STATUS_MODE=count`は人数だけを表示し、`names`はDiscord表示名もTopicとActivityへ表示します。個人情報を最小化する場合は`count`または`disabled`を選んでください。

## 個人情報と削除

DBにはDiscord user ID、履歴上の表示名、活動時刻、任意の備考、任意の本名・役割・代・名前の読みを保存します。運用者はDBとバックアップへのアクセスを制限し、利用者へ保存目的と保持期間を説明してください。

- `/attendanceexport clearuserconfig`: 本名などのプロフィール情報だけを解除します。
- `/attendance erase confirmation:DELETE`: 本人の全セッション、変更履歴、確認要求、自動終了イベント、プロフィールを稼働DBから物理削除します。
- 通常の`/attendance delete`は取り消し可能にするため論理削除です。

完全消去後も、保持期間内のバックアップや、過去にDiscordへ公開したCSV・PDFには情報が残り得ます。運用者はバックアップ保持期限とDiscord上の削除手順を定めてください。

月次exportは `format:csv` でCSVのみ、既定の `format:all` でCSV・PDFを出力します。各ファイルを別メッセージで送り、CSVの送信後にPDFを生成します。PDFの失敗・サイズ超過は他のファイルの送信を妨げません。結果はファイルごとに本人へ通知し、再実行時には送信済みファイルが重複する場合があります。個別添付上限に加え、リクエスト全体の25 MiB制限への余裕を確保するため、ファイル当たり24 MiBを上限とします。

月次exportは既定で`preview`となり、実行者だけへ送信します。`publish`では全員分の本名と活動時間を公開するため、`confirm_public:true`の明示指定が必要で、実行を監査ログへ記録します。

## バックアップ

`attendance-maintenance`はDiscord tokenを読み取らず、SQLiteの`VACUUM INTO`で稼働中DBの整合したスナップショットを作成します。作成後に`PRAGMA integrity_check`・外部キー・migration台帳のversion/success/checksum・現行schema（テーブル、インデックス、trigger等）を検査します。検証済みの一時ファイルは出力先が存在しない場合だけ確定し、並行処理が先に作ったファイルも上書きしません。手動で変更されたschemaは検証を通過しません。

ソース実行例:

```powershell
New-Item -ItemType Directory -Force backups
$backupStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
cargo run --locked --release --bin attendance-maintenance -- backup attendance.db "backups\attendance-$backupStamp.db"
cargo run --locked --release --bin attendance-maintenance -- verify "backups\attendance-$backupStamp.db"
```

Docker例:

```powershell
$backupStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
docker compose run --rm maintenance backup /data/attendance.db "/backups/attendance-$backupStamp.db"
docker compose run --rm maintenance verify "/backups/attendance-$backupStamp.db"
```

Composeの`maintenance` serviceには`.env`を渡さないため、保守コマンドの環境へBot tokenは注入されません。

少なくとも日次バックアップ、複数世代保持、別ホストへの暗号化コピー、定期的な復元訓練を設定してください。必要な復旧時点と許容停止時間に合わせてRPO・RTOを決めます。

## 復元

復元は必ず記録 Bot と表示 Bot の両方を停止して実施します。

1. 記録 Bot と表示 Bot を停止します。
2. 復元対象バックアップを`verify`します。
3. 現在のDBと対応する`-wal`・`-shm`を、同じ復旧用ディレクトリへまとめて退避します。DBとWALを分離して使い回してはいけません。
4. 元のDBパスが存在しない状態で`restore`を実行します。保守コマンドは既存パスを上書きしません。
5. 復元DBをもう一度`verify`します。
6. 可能ならテストGuildで起動確認してから記録 Bot、表示 Bot の順に再開します。

```powershell
cargo run --locked --release --bin attendance-maintenance -- restore backups\attendance-YYYYMMDD-HHMMSS.db attendance.db
cargo run --locked --release --bin attendance-maintenance -- verify attendance.db
```

## 更新とmigration

- 更新前に検証済みバックアップを取得します。
- 現在は既存環境のない試作段階であり、schemaは `0001_initial_schema.sql` に統合します。後方互換は保証せず、新規DBを検証対象とします。データを維持する運用の開始後は初期migrationを固定し、連番で追加します。
- 記録側だけが `sqlx::migrate!` により起動時に未適用migrationを実行します。
- ロールバックが必要な場合は、Botを停止して更新前バックアップから復元します。
- 変更前の初期migrationを適用済みのテストDBは再利用対象外です。運用者が必要なデータを確認して新規DBを用意してください。Botが旧DBを自動削除・変換することはありません。

## データベース設計

- `attendance_sessions`: 活動記録。通常削除はsoft delete
- `attendance_changes`: 取り消し可能な変更履歴
- `pending_attendance_actions`: edit、delete、revertの確認要求
- `attendance_auto_end_events`: 自動終了・通知・訂正状態
- `attendance_user_profiles`: 任意の代、本名、役割、名前の読み

所有者を含む複合外部キー、同一ユーザーの区間重複を防ぐtrigger、`BEGIN IMMEDIATE`相当のwrite transaction、busy timeoutを使用します。詳細仕様は[仕様書](./SPECIFICATION.html)を参照してください。

## 開発と検証

```powershell
cargo fmt --check
cargo test --locked
cargo test --locked --doc
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --workspace --no-deps
cargo audit --deny warnings
python scripts/check-boundaries.py
docker build --tag discord-attendance-bot:local .
```

GitHub ActionsではRust 1.94を使い、Ubuntu、Windows、macOSで検査します。`cargo run`はDiscordへ接続するため、自動テストでは実行しません。

## セキュリティ報告とコントリビューション

脆弱性は公開Issueへ秘密値やDBを添付せず、[SECURITY.md](./SECURITY.md)の手順で報告してください。開発参加方法は[CONTRIBUTING.md](./CONTRIBUTING.md)、変更履歴は[CHANGELOG.md](./CHANGELOG.md)を参照してください。

## License

[MIT License](./LICENSE)
