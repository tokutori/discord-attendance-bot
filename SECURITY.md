# Security Policy

## Supported versions

公開後は、最新のv1.x releaseへセキュリティ修正を提供します。未リリースのmain branchとv0.0はサポート対象外です。

## 脆弱性の報告

GitHubのPrivate vulnerability reportingから非公開で報告してください。公開Issue、Discussion、Pull Requestへ、Bot token、`.env`、実データベース、バックアップ、個人情報を投稿しないでください。

報告には、秘密値を除いた次の情報を含めてください。

- 影響を受けるversionまたはcommit
- 再現条件と期待する挙動
- 想定される影響
- ダミーデータだけを使った最小の再現手順

Bot tokenが漏えいした可能性がある場合は、報告を待たずDiscord Developer Portalでtokenを再生成し、漏えいしたtokenを無効化してください。実DBやバックアップが漏えいした場合は、運用者のインシデント対応手順に従い、影響を受ける利用者へ連絡してください。

## 運用上の前提

- DBはネットワークファイルシステムではなく、実行ホストのローカルディスクへ保存します。
- `.env`、DB、バックアップ、PDF、CSVへのアクセスを必要最小限にします。
- GitHubのSecret scanning、Push protection、Dependabot alertsを有効にします。
- 更新前に検証済みバックアップを取得し、定期的に復元訓練を行います。
