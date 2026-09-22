# Changelog

## Unreleased

- 月次exportに `format:csv` を追加。CSVを先に送信し、PDF失敗時も取得可能にした。添付を1ファイルずつ送り、個別上限とリクエスト容量を検査する。
- `main`・`dev`・`feat/*` / `fix/*` のブランチ運用と、テスト環境での受入手順を文書化。
- 記録プロセスと readonly 表示プロセスを分離。表示には別 Discord Application が必要。
- status/list/history/month/export を `/attendanceview` へ移動。記録・プロフィール更新コマンドは維持。
- 表示のみの更新・停止、readonly DB 接続、Docker 読み取り専用マウント、スキーマ互換性検査を追加。
- 移行手順は [docs/process-separation.md](docs/process-separation.md) を参照。

このプロジェクトの主な変更を記録します。形式は[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)を参考にし、versionはSemantic Versioningに従います。

## [Unreleased]

## [1.0.0] - 2026-08-22

### Added

- 引数なしで動く1 Guild向けstandalone設定
- タイムゾーン、自動終了、ステータス表示、更新間隔、SQLite同期方式の設定
- 人数だけを表示して名前を公開しないステータスモード
- 本人によるプロフィール解除と全個人データの完全消去
- 公開月次exportの`confirm_public:true`確認と監査ログ
- オンラインバックアップ、整合性検査、安全な復元を行う`attendance-maintenance`
- Dockerfile、Compose、バックアップ／復元／個人情報の運用文書
- Dependabot、MSRV検査、コンテナbuildを含むCI

### Changed

- SQLx 0.9へ更新し、WAL-reset破損バグ修正版のSQLite 3.51.3を固定
- SQLiteの既定を`WAL + synchronous=FULL`へ変更
- 対象を日本語圏のクラブ・チーム向け1 GuildセルフホストBotとして明文化

### Security

- 既知のWAL-reset破損バグを含むSQLite runtimeでは起動・保守操作を拒否
- 個人情報を含む公開帳票に明示確認を追加

## [0.0.0]

- 初期試作版。現在のv1 schemaとの移行互換性はありません。
