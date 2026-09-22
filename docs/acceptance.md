# dev の受入確認

`dev` は統合・試験用、`main` は本番反映可能な安定版とする。以下は未実施の受入項目であり、CI成功によって完了扱いにはしない。bem がテスト用Bot・Guild・DBと架空の氏名・活動記録で確認し、結果を `dev` → `main` のリリースPRに記録する。

秘密情報の値はPR・ログ・スクリーンショットに含めない。設定と実行は運用者が行う。初回の分離構成への移行は記録側の更新・再起動を伴う。以降の表示側更新の独立性を以下で確認する。

## 構成と記録継続

- [ ] 別々のDiscord Applicationで記録用・表示用Botを登録した。設定した記録用Application IDが正しく、Guild・DB・timezone・auto-end時刻が両者で一致する。
- [ ] 記録側の `/join`・`/exit`・再開と、表示側の `/attendanceview` が登録され、表示側再登録後も記録コマンドが維持される。
- [ ] `docker compose --profile view stop view` の間も記録でき、`docker compose --profile view start view` 後の履歴に停止中の記録が現れる。記録側コンテナを再起動していない。
- [ ] `docker compose --profile view build view`、`docker compose --profile view up -d --no-deps view` による表示側だけの更新中も記録できる。
- [ ] 自動終了を使う運用では、表示側停止中も設定時刻に自動終了する。

## export の実送信

- [ ] 一般メンバーにはexportを許可せず、MANAGE_GUILDを持つ実行者だけが取得できる。
- [ ] `format:csv` はCSV2個だけを1ファイルずつ送信する。PDF用フォントが利用できなくても取得できる。
- [ ] `format:all`（省略時も同じ）はCSV2個を先に送り、その後PDF2個を別々に送る。結果一覧が実際の添付に対応する。
- [ ] `preview` の全添付・結果一覧が実行者だけに見える。
- [ ] `publish` で `confirm_public:true` を省略すると、ファイルが公開されない。
- [ ] ダミーデータの `publish confirm_public:true` で実際にCSV・PDFを送信できる。公開先が正しく、個人の自動終了通知と結果一覧は本人だけに表示される。
- [ ] PDF生成失敗・サイズ超過時もCSVを取得でき、該当ファイルの失敗が結果一覧に現れる。再実行時の重複を認識できる。
- [ ] 運用人数に近い架空データでCSV・PDFを開き、日本語、行列、月次合計、改ページを目視確認した。

送信は1ファイル単位であり、一括の成功・取消は保証しない。通信エラーでは送信の有無が確定しない場合がある。再実行前に添付を確認する。添付の単体上限と、リクエスト容量を確保するための24 MiB上限の両方を適用する。PDFの軽量化、大規模データ・同時exportの資源使用量は別途評価する。

上限の根拠はDiscord公式の [Uploading Files](https://docs.discord.com/developers/reference#uploading-files) と [Create Message](https://docs.discord.com/developers/resources/message#create-message) を参照する。2026-09-23確認時点でファイル単体の既定値は20 MiB、通常メッセージのリクエスト全体は25 MiBである。実装ではファイル単体の判定にinteractionの `attachment_size_limit` を使う。

## 表示と安定部分の境界

- [ ] Topic・Activityの周期更新（既定600秒）と表示側停止時に古いTopicが残る動作を許容できる。
- [ ] 表示側の閲覧で自動終了通知を既読にせず、記録側で確認するまで再表示する動作を許容できる。
- [ ] 取得済みDTOの表示・帳票処理を変更対象とし、SQL・接続管理は安定部分としてレビューする境界を確認した。

## 記録欄

- 対象dev commit:
- 実施日・確認者:
- 実施項目と結果:
- 未実施・不合格項目と対応:
- mainへの反映判断:
