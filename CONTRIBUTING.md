# Contributing

IssueやPull Requestを歓迎します。このBotは「日本語圏のクラブ・チーム向け、1 Guildごとのセルフホスト」という範囲を維持します。マルチGuild SaaS化や全面的な多言語化は、現在の公開版の対象外です。

## 開発環境

Rust 1.94以上を使用してください。リポジトリの`rust-toolchain.toml`が対応toolchainとrustfmt、Clippyを指定します。

```powershell
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python scripts/check-boundaries.py
```

共通モデル・query・view を workspace crate に分離しています。`default-members` は全 crate です。
記録 crate から view/PDF への依存、query/view から記録 service への依存を追加しないでください。
実行アプリの workspace として `publish = false` にしており、`cargo package` による単一 crate 配布は行いません。
ビルド・運用契約は [分離設計](docs/process-separation.md) を参照してください。

## ブランチ運用

- `main`: 本番反映可能な安定版。`dev` からリリースPRを作成し、bemの受入確認・承認後に取り込む。
- `dev`: 次期版の統合・受入確認用。CIが通る状態を維持する。
- `feat/*`・`fix/*`: 原則 `dev` から分岐し、目的ごとのPRを `dev` へ送る。レビューとCI確認後に取り込む。
- `dev` へのマージは、対象headの適切なCIとsubagentによる独立レビューを確認し、指摘を解消した後、エージェントが自律的に実行してよい。`main` への反映は引き続きbemの受入確認・明示承認を必要とする。
- `dev` → `main` は共通履歴を維持するため通常のmerge commitを使用する。本番の緊急修正を `main` 起点で行った場合は `dev` にも取り込む。
- `dev` の受入試験はテスト用Bot・Guild・DBで行う。確認項目は [受入手順](docs/acceptance.md) を参照する。
- `dev` への統合と本番反映を区別する。ブランチ作成やPR承認は、自動的な本番デプロイの許可を意味しない。

## Pull Request

- 変更理由、利用者への影響、検証結果を記載してください。
- schema変更は既存migrationを編集せず、新しい連番migrationを追加してください。
- 時刻境界、DB競合、個人情報、CSV/PDF出力に関わる変更にはテストを追加してください。
- 公開APIや運用方法を変える場合はREADME、仕様書、`.env.example`も更新してください。
- テキストファイルはUTF-8で保存してください。

## 秘密情報と実データ

`.env`、Bot token、API key、実Guild ID、実Channel ID、実DB、バックアップ、個人情報をIssueやPull Requestへ含めないでください。テストにはダミーIDと架空の氏名だけを使用してください。

脆弱性は公開Issueではなく、[SECURITY.md](./SECURITY.md)の非公開報告手順を使用してください。
