# exportの実行権限

`/attendanceview export` は設定されたGuild内で、サーバー管理権限（`MANAGE_GUILD`）または管理者権限を持つ実行者だけが使用できる。

最初にephemeralの遅延応答を送り、interactionに含まれる利用者の権限・Botの `app_permissions`・チャンネル種別を同期的に判定する。権限判定のためのREST照会は追加しない。

Botには `EMBED_LINKS`・`ATTACH_FILES` と、送信先に対応する次の権限を要求する。

| 送信先 | 送信権限 |
| --- | --- |
| テキスト・アナウンス・ボイス・ステージチャンネルのテキストチャット | `SEND_MESSAGES` |
| 公開・非公開・アナウンススレッド | `SEND_MESSAGES_IN_THREADS` |

スレッドでは親チャンネルの `SEND_MESSAGES` を要求しない。権限情報・チャンネル種別が欠ける場合や、未対応・未知のチャンネル種別は拒否する。Botの管理者権限は必要な権限を満たすものとして扱うが、未知の送信先種別を許可する根拠にはしない。

`preview` はinteractionの非公開応答として本人だけにファイルを送り、`publish` は `confirm_public:true` を確認して通常メッセージを投稿する。アプリケーション側では両方に上記の共通権限方針を適用する。これは本アプリケーションの保守的な判定であり、Discordのephemeral応答すべてが通常投稿と同じ権限を要求するという主張ではない。

判定後の権限変更、非公開スレッドへのアクセス、スレッドのロックなどによって、実際の送信が失敗する場合がある。その場合はファイルごとの送信結果に反映し、成功したファイルを取り消さない。

ボイス・ステージチャンネルの既存のテキストチャットも許可する。Forum・Mediaの親チャンネルには直接投稿せず、各投稿のスレッドから実行する。

公式仕様: [Threadsの権限](https://docs.discord.com/developers/topics/threads#permissions)、[Interaction構造と応答](https://docs.discord.com/developers/interactions/receiving-and-responding)。

回帰試験は各対応種別について送信権限の許可・拒否、添付権限の欠落、管理者、欠損・未知種別を検証する。実際のDiscord上の権限設定と添付送信は別途受入確認を要する。

ボイス・ステージの仕様: [ボイスチャンネルのテキストチャット](https://support.discord.com/hc/en-us/articles/4412085582359-Text-Channels-Text-Chat-In-Voice-Channels)、[Stage Channels FAQ](https://support.discord.com/hc/en-us/articles/1500005513722-Stage-Channels-FAQ)。
