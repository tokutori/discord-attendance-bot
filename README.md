# Discord 活動時間記録 Bot v1.0

鳥人間チームのメンバーが Discord の Slash Command から活動時間を記録する Rust 製 Bot である。SQLite のみを使用し、各利用者は自分の記録を開始・終了・継続・閲覧・修正・削除できる。

## 実装済みコマンド

- `/attendance start [at] [note]`
- `/attendance end [at] [note]`
- `/attendance continue`
- `/attendance revert`
- `/attendance status`
- `/attendance history [limit]`
- `/attendance month [target]`
- `/attendance edit record [start] [end] [note]`
- `/attendance delete record`
- `/attendance confirm id`
- `/attendance help`
- `/attendanceexport export month [mode]`
- `/attendanceexport userconfig [generation] [real_name] [role]`
- `/attendanceexport help`

`at` は `HH:MM`、`target` は `YYYY-MM`、編集日時は `YYYY-MM-DD HH:MM` 形式で入力する。時刻入力と表示は日本時間、SQLite 内部では UTC Unix timestamp を使用する。

## 必要環境

- Rust 1.88 以上
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
# 任意: PDF用日本語TTFフォントのパス
# ATTENDANCE_PDF_FONT_PATH=C:\\Windows\\Fonts\\NotoSansJP-VF.ttf
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

v1.0では本番運用前の設計見直しに伴いmigration履歴とDB schemaを作り直している。v0.0で作成したテストDBとの移行互換性はないため、v1.0を初めて起動する前にBotを停止し、旧テストDBと対応する `-wal`・`-shm` を削除して新規作成する。本番DBの移行手順としてこの方法を使用してはならない。

BotのActivityは活動記録の変更時に即時更新する。専用チャンネルのTopicはBot起動時および10分ごとに更新し、その周期更新時にはActivityも同時に更新する。

Botは日本時間の毎日0時に、前日21時まで活動中だった記録を21時終了として自動終了する。Botが0時に停止していた場合は、次回起動時に未処理分を補完する。自動終了後にユーザーが `end` を実行した場合は、自動終了を取り消してユーザー入力の終了時刻を正とする。次のユーザー操作時には自動終了の内容と、必要なら `edit` で修正できることを通知する。

Activityの種別は、Botが活動状況を監視している意味に合わせて `Watching` を使用する。

Slash Command の操作結果・入力エラー・権限エラーなど、利用者向けの応答は原則として ephemeral Embed で表示する。`history`、`month`、`help` は専用の Embed レイアウトを使用する。

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

v1.0の初期schemaは `migrations/0001_initial_schema.sql` に集約し、次の責務ごとにテーブルを分ける。

- `attendance_sessions`: 活動記録
- `attendance_changes`: 取り消し可能な変更履歴
- `pending_attendance_actions`: `edit`、`delete`、`revert` の確認要求
- `attendance_auto_end_events`: 21時自動終了と通知・訂正状態
- `attendance_user_profiles`: 代、本名、役割の出力設定

バックアップは Bot 停止中に `attendance.db` をコピーするのが簡単である。稼働中に取得する場合は SQLite CLI の `.backup` または `VACUUM INTO` を使用する。

```sql
VACUUM INTO 'attendance-backup.db';
```

## 挙動上の注意

- 重複 `start` は既存の活動中記録を表示し、DBを変更しない。
- 重複 `end` は直近の終了済み記録を表示し、DBを変更しない。
- `continue` は直近の終了済み記録の終了時刻を取り消す。
- `edit`、`delete`、`revert` は最初に変更内容をプレビューし、5分間有効な5文字の確認IDを発行する。`/attendance confirm id:<ID>` で確定するまで DB は変更しない。
- `revert` は直前の成功した変更操作を1件だけ取り消す。`start` は作成記録を soft delete、`end` は終了前、`continue` は continue 前、`edit` は編集前、`delete` は削除前へ復元する。`revert` 自体は操作履歴に積まれないため、1回確定した後に再度 `revert` → `confirm` を行えば、過去の変更を順に取り消せる。対象がなければ安全な no-op とする。
- 確認前に別の変更が入った場合、プレビュー時の状態と一致しないため安全のため確定しない。確認IDは使用済みになる。
- `edit` で `end` を空文字として入力すると活動中へ戻せる。ただし、別の活動中記録がある場合は拒否する。
- `delete` は confirm で確定し、DB上では soft delete する。
- `/attendance help` で利用可能なコマンドと引数を確認できる。
- 月次集計は月境界および日境界で分割し、日本時間基準で算出する。
- `/attendance month` は合計・活動回数・1回あたり平均に加え、1日あたり平均と1週間あたり平均を表示する。当月は今日を含む経過暦日数、過去月はその月の全日数を分母とし、未来月は分母0として平均0を表示する。活動日のみの日数ではない。
- `/attendanceexport export month:YYYY-MM` で指定月の CSV と PDF を出力できる。`month` は必須で、`mode` は `preview`（既定、本人のみ）または `publish`（全員に公開）を指定する。
- `/attendanceexport userconfig` は実行者の代（整数）、本名、役割を設定する。引数なしでは現在値をEmbed表示し、一部の引数だけを指定した場合はほかの設定を保持する。
- エクスポート表は、縦方向がユーザー、横方向が対象月の日付と合計列である。CSVには代・本名・役割・Discord表示名を独立した列として含め、PDFには設定内容をユーザー情報欄へまとめて表示する。本名未設定時はDiscord表示名を使用する。対象月が未終了の場合と翌月1日の出力には、暫定集計・修正可能性の注記を付ける。

## 月次ファイル出力

`/attendanceexport help` で操作方法を確認できる。CSV と PDF は同じ月次データから生成し、活動時間があるセルは `時間:分` 形式で表示する。PDFの0時間セルは空欄、CSVの0時間セルは `0:00` と表示する。現在活動中の記録は出力時点までを暫定値として含める。PDFのセル文字は上下中央揃えとし、月の日数と利用者数に応じて改ページする。

PDF は `printpdf` を使用する。表の配置と改ページは Bot 側で明示的に制御し、`PdfSaveOptions.subset_fonts = true` を必ず指定して、実際に使用した文字のグリフだけを TTF から埋め込む。CJK フォント全体を埋め込むと添付サイズが大きくなりやすいため、この方針を採用した。日本語フォントは環境依存のため、`ATTENDANCE_PDF_FONT_PATH` で TTF を指定できる。未指定時は Noto Sans JP、Windows の日本語フォントなど既定候補を検索する。

`genpdf` は高レベルな表 API が便利だが、フォントを複数登録する設計では使用文字だけの埋め込みを明示しにくいため採用しない。`lopdf` / `lopdf-table` は PDF 構造や表の細かな後処理が必要になった場合の候補とする。

## ディレクトリ

```text
src/
├── main.rs
├── lib.rs
├── framework_error.rs
├── commands/
│   ├── attendance.rs
│   ├── attendance/
│   │   ├── common.rs
│   │   ├── daily.rs
│   │   ├── guarded.rs
│   │   └── query.rs
│   └── attendance_export.rs
├── attendance/
│   ├── aggregation.rs
│   ├── model.rs
│   └── service.rs
├── attendance_export.rs
├── attendance_export/
│   └── pdf.rs
├── repository/
│   ├── model.rs
│   ├── session.rs
│   ├── change.rs
│   ├── confirmation.rs
│   ├── auto_end.rs
│   ├── profile.rs
│   └── tests.rs
├── presentation/
└── time.rs
migrations/
└── 0001_initial_schema.sql
SPECIFICATION.html
```

`main.rs` は起動と依存関係の組み立て、`commands` はDiscord interaction、`attendance` は時刻計算を含むdomain logic、`repository` は責務別のSQLite操作、`presentation` はEmbedとステータス表示、`framework_error.rs` は利用者向けエラー分類を担当する。

詳細仕様は [SPECIFICATION.html](./SPECIFICATION.html) を参照する。
