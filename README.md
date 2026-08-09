# Discord 活動時間記録 Bot

鳥人間チームのメンバーが Discord の Slash Command から活動時間を記録する Rust 製 Bot である。SQLite のみを使用し、各利用者は自分の記録を開始・終了・継続・閲覧・修正・削除できる。

## 実装済みコマンド

- `/attendance start [at] [note]`
- `/attendance end [at] [note]`
- `/attendance continue`
- `/attendance status`
- `/attendance history [limit]`
- `/attendance month [target]`
- `/attendance edit record [start] [end] [note]`
- `/attendance delete record confirm`

`at` は `HH:MM`、`target` は `YYYY-MM`、編集日時は `YYYY-MM-DD HH:MM` 形式で入力する。時刻入力と表示は日本時間、SQLite 内部では UTC Unix timestamp を使用する。

## 必要環境

- Rust 1.85 以上
- Discord Application / Bot token
- Bot を追加できる Discord サーバー

## Discord 側の準備

1. Discord Developer Portal で Application を作成する。
2. Bot を作成して token を取得する。
3. Installation で `bot` と `applications.commands` を使用して対象サーバーへ追加する。
4. Discord の Developer Mode を有効にし、開発用サーバーIDをコピーする。

Bot は message content を読まないため、Privileged Gateway Intents は不要である。

## セットアップ

```powershell
Copy-Item .env.example .env
```

`.env` を編集する。テスト用と本番用のGuild IDおよびSQLite DBを分ける。

```dotenv
DISCORD_TOKEN=...
DISCORD_TEST_GUILD_ID=...
DISCORD_RELEASE_GUILD_ID=...
DATABASE_URL_TEST=sqlite://attendance-test.db
DATABASE_URL_RELEASE=sqlite://attendance-release.db
ATTENDANCE_STATUS_CHANNEL_ID_TEST=...
ATTENDANCE_STATUS_CHANNEL_ID_RELEASE=...
RUST_LOG=discord_attendance_bot=info,poise=info,serenity=info
```

`test` と `release` の起動引数によって、使用するGuild IDとDBが切り替わる。どちらもGuild commandとして登録されるため、指定したサーバーだけで利用できる。

各サーバーに活動状況表示用のテキストチャンネルを1つ用意し、そのチャンネルIDを設定する。Botには対象チャンネルのTopicを編集できる `Manage Channels` 権限が必要である。可能であればサーバー全体ではなく、専用チャンネルへの権限上書きで付与する。

## 起動

テスト環境:

```powershell
cargo run -- test
```

本番環境:

```powershell
cargo run --release -- release
```

引数は `test` または `release` のいずれかが必須である。不正な引数や未指定の場合は起動しない。

初回起動時に、選択したモードのSQLite databaseとmigration tableが自動作成される。

BotのActivityは活動記録の変更時に即時更新する。専用チャンネルのTopicはBot起動時および10分ごとに更新し、その周期更新時にはActivityも同時に更新する。

Activityの種別は、Botが活動状況を監視している意味に合わせて `Watching` を使用する。

```text
:green_circle: 現在2名活動中
(1) Bem130
(2) Alice
(最終更新: 2026年8月9日 21:30)
```

ActivityはDiscordの表示上限に合わせ、128文字を超える部分を省略する。Topicは1024文字まで保持する。

## テストと静的検査

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## データベース

テスト環境では `attendance-test.db`、本番環境では `attendance-release.db` が作成される。WAL modeを使用するため、実行中はそれぞれのDBに対応する `-wal` と `-shm` ファイルが存在する場合がある。

バックアップは Bot 停止中に `attendance.db` をコピーするのが簡単である。稼働中に取得する場合は SQLite CLI の `.backup` または `VACUUM INTO` を使用する。

```sql
VACUUM INTO 'attendance-backup.db';
```

## 挙動上の注意

- 重複 `start` は既存の活動中記録を表示し、DBを変更しない。
- 重複 `end` は直近の終了済み記録を表示し、DBを変更しない。
- `continue` は直近の終了済み記録の終了時刻を取り消す。
- `edit` で `end` を空文字として入力すると活動中へ戻せる。ただし、別の活動中記録がある場合は拒否する。
- `delete` は `confirm:true` が必要で、DB上では soft delete する。
- 月次集計は月境界および日境界で分割し、日本時間基準で算出する。

## ディレクトリ

```text
src/
├── main.rs
├── lib.rs
├── commands/
├── attendance/
├── repository/
├── presentation/
└── time.rs
migrations/
SPECIFICATION.html
```

詳細仕様は [SPECIFICATION.html](./SPECIFICATION.html) を参照する。
