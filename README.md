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

`.env` を編集する。

```dotenv
DISCORD_TOKEN=...
DISCORD_TEST_GUILD_ID=...
DATABASE_URL=sqlite://attendance.db
RUST_LOG=discord_attendance_bot=info,poise=info,serenity=info
```

開発中は `DISCORD_TEST_GUILD_ID` を設定する。Guild command は反映が速い。全サーバー向けに公開する段階では、この値を削除または空欄にすると global command を登録する。

## 起動

```powershell
cargo run --release
```

初回起動時に SQLite database と migration table が自動作成される。

## テストと静的検査

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## データベース

既定ではプロジェクト直下に `attendance.db` が作成される。WAL mode を使用するため、実行中は `attendance.db-wal` と `attendance.db-shm` が存在する場合がある。

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
