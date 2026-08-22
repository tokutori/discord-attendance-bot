# Contributing

IssueやPull Requestを歓迎します。このBotは「日本語圏のクラブ・チーム向け、1 Guildごとのセルフホスト」という範囲を維持します。マルチGuild SaaS化や全面的な多言語化は、現在の公開版の対象外です。

## 開発環境

Rust 1.94以上を使用してください。リポジトリの`rust-toolchain.toml`が対応toolchainとrustfmt、Clippyを指定します。

```powershell
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
```

## Pull Request

- 変更理由、利用者への影響、検証結果を記載してください。
- schema変更は既存migrationを編集せず、新しい連番migrationを追加してください。
- 時刻境界、DB競合、個人情報、CSV/PDF出力に関わる変更にはテストを追加してください。
- 公開APIや運用方法を変える場合はREADME、仕様書、`.env.example`も更新してください。
- テキストファイルはUTF-8で保存してください。

## 秘密情報と実データ

`.env`、Bot token、API key、実Guild ID、実Channel ID、実DB、バックアップ、個人情報をIssueやPull Requestへ含めないでください。テストにはダミーIDと架空の氏名だけを使用してください。

脆弱性は公開Issueではなく、[SECURITY.md](./SECURITY.md)の非公開報告手順を使用してください。
