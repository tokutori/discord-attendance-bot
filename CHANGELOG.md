# Changelog

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
