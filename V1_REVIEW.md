# v1.0 設計見直し・検証記録

## 目的

本番運用前に v0.0 の実装と要求を棚卸しし、移行互換性を持たない新規 schema と責務別のコード構成へ作り直す。コマンドの利便性だけでなく、確認操作、取り消し、並行実行、自動終了、帳票の個人情報を安全に扱えることを v1.0 の完了条件とする。

## フェーズ

| フェーズ | 観点 | 結果 |
|---|---|---|
| 0 | 全要求と既存実装の対応付け | コマンド、時刻、確認、集計、帳票、表示、運用を棚卸し済み |
| 1 | migration・repository・SQLite並行性 | 所有者付きFK、区間重複trigger、即時write transaction、WAL競合対策へ変更 |
| 2 | 状態遷移・confirm・revert・自動終了 | 状態snapshot、単回confirm、連続revert、複数自動終了イベント、訂正と通知ACKを修正 |
| 3 | 集計・CSV・PDF・userconfig・Discord UX | 平均値、境界集計、CSV無害化、PDF折返し、権限、添付上限、公開範囲を修正 |
| 4 | 横断的な実装修正と文書同期 | Embed責務、エラー分類、README、仕様書を実装へ同期 |
| 5 | 独立した全体整合性レビューと品質ゲート | 観点別レビュー後に全テスト・fmt・Clippyで判定 |

## 要求と検証根拠

| 要求 | 実装・検証 |
|---|---|
| 通常操作 `start` / `end` / `continue` | `attendance::service` と `repository::session` に状態遷移を分離。重複操作はno-op |
| `edit` / `delete` / `revert` は確認必須 | 5文字・5分のpending actionを発行し、`confirm` の即時write transaction内で状態を再検証 |
| `revert` は全変更操作を安全に取り消す | before/after snapshotを保存し、最新の未取り消し変更だけを復元。revert自身は履歴に積まない |
| `revert` の連続実行 | `start → end → revert → confirm → revert → confirm` を含むrepository testで過去へ順に戻ることを検証 |
| 確認の単回性と競合 | 同一IDの並行confirm、期限境界、別start後のopen復元、区間重複、古いrevert IDによる最新操作の飛び越し拒否をtestで検証。失敗したIDも再利用不可 |
| 21時自動終了 | `open_since` から次の21時を計算し、0時処理と起動時補完を実施。21時以降の開始は翌日21時が期限 |
| 自動終了後のユーザー入力を優先 | event IDと自動終了時刻の一致を確認して訂正。同一sessionの再自動終了を別イベントで保存 |
| 自動終了通知を失わない | peek後に応答を送信し、成功後にevent IDをACK。公開帳票へ個人通知を混入させない |
| `month` の3種類の平均 | 1回・1日・1週間あたりを表示。当月、過去月、未来月の暦日分母をpure function testで検証 |
| CSV・PDF月次帳票 | 年末、閏日、日付境界、0時間セル、profile順序をtest。CSVはformula injectionを無害化 |
| PDF日本語・可読性 | `printpdf` のsubset fontを使用。上下中央揃え、長い代・本名・役割の折返しと縮小をtest |
| `userconfig` | UPSERT ... RETURNINGで代・本名・役割を原子的に更新し、Embed/CSV/PDFへ反映 |
| exportの個人情報・公開範囲 | 実行者にManage Guildを要求。previewはephemeral、publishは生成成功後だけ別メッセージで公開 |
| Discord添付上限と利用者向けエラー | interactionの`attachment_size_limit`を送信前に検査。内部パスや秘密情報を応答へ含めない |
| 活動区間と所有者の整合性 | migrationの複合FKとoverlap trigger、ownerを含む更新条件、serviceの事前検証で多層防御 |
| Embed表示とhelp | 表示生成を`presentation`へ集約。全slash subcommandの日本語説明とhelp分類をtest |

## 主な設計判断

- セッションの履歴上の開始時刻 `started_at` と、現在の連続活動開始点 `open_since` を分ける。これにより、同じ記録を自動終了後に `continue` しても直後に古い期限で再終了しない。
- `attendance_auto_end_events` はsessionを主キーにせず独立IDを持つ。同じsessionに対する複数回の自動終了を履歴として保持する。
- read-modify-writeと確認確定は `BEGIN IMMEDIATE` 相当で開始し、確認時点のsnapshot一致を同じtransaction内で判定する。
- 区間重複と所有者不一致はapplicationだけでなくDB制約でも拒否する。確認後や並行処理でも不正状態を作らない。
- `publish` はinteraction自体を最初から公開せず、生成成功後にだけチャンネルへ別送信する。生成・入力エラーで個人向け情報が公開されない。

## 自動検証

次の品質ゲートを秘密情報およびDiscord接続なしで実行する。

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

現在の自動テストは44件で、repositoryのWAL並行確認、migration制約、時刻境界、月次集計、CSV、PDF、command metadataを含む。

## 運用開始前の手動確認

- Botを停止し、v0.0のテストDBと対応する `-wal`・`-shm` を削除して、v1.0 migrationから新規作成する。
- テストGuildでslash command登録後、ephemeral表示、Manage Guild拒否、preview/publish、実際の添付上限を確認する。
- 使用環境の日本語TTFでPDFを開き、複数ユーザー、長い本名・役割、31日月、複数ページの視認性を確認する。
- 0時処理は単体テスト済みだが、Botを日付跨ぎで稼働させる運用試験も行う。

`cargo run` とDiscordへの接続は秘密情報と外部状態を使用するため、この自動レビューでは実行しない。
