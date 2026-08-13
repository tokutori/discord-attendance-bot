use chrono::{Datelike, Utc};
use poise::CreateReply;

use crate::{
    Context, Error, attendance,
    member_order::{self, MemberOrderKey},
    presentation, repository,
    time::{self, DISPLAY_TIMEZONE, format_datetime, format_duration},
};

use super::common::{
    acknowledge_auto_end_notice, defer_ephemeral, ids, peek_auto_end_notice, send_response,
};

/// 活動時間記録コマンドの使い方を表示する。
#[poise::command(slash_command, guild_only)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    ctx.send(
        CreateReply::default()
            .embed(presentation::help_embed(ctx.author().display_name()))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

/// 現在活動中か確認する。
#[poise::command(slash_command, guild_only)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now().timestamp();
    let content = match repository::open_session(&ctx.data().database, guild_id, user_id).await? {
        Some(session) => {
            let mut text = format!(
                "活動中\n\n開始時刻: {}\n経過時間: {}",
                format_datetime(session.started_at),
                format_duration(session.duration_seconds_at(now))
            );
            if let Some(note) = session
                .note
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                text.push_str(&format!("\n備考: {note}"));
            }
            text.push_str(&format!("\n記録ID: #{}", session.id));
            text
        }
        None => "現在、活動中の記録はない。".into(),
    };
    send_response(ctx, content).await
}

fn member_order_key(member: &repository::ActiveAttendanceMember) -> MemberOrderKey<'_> {
    MemberOrderKey {
        user_id: member.user_id,
        generation: member.generation,
        real_name: member.real_name.as_deref(),
        role: member.role.as_deref(),
        name_reading: member.name_reading.as_deref(),
        display_name: &member.display_name,
    }
}

fn sort_active_members(members: &mut [repository::ActiveAttendanceMember]) {
    members.sort_by(|left, right| {
        member_order::compare(member_order_key(left), member_order_key(right))
    });
}

/// 現在活動中のメンバーを役割・代・名前順の名簿で表示する。
#[poise::command(slash_command, guild_only, rename = "list")]
pub async fn list(ctx: Context<'_>) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, _) = ids(ctx)?;
    let mut members = repository::active_attendance_members(&ctx.data().database, guild_id).await?;
    sort_active_members(&mut members);
    let embeds = presentation::active_roster_embeds(&members);
    for embed in embeds {
        ctx.send(CreateReply::default().embed(embed).ephemeral(true))
            .await?;
    }
    Ok(())
}

/// 最近の活動記録を表示する。
#[poise::command(slash_command, guild_only)]
pub async fn history(
    ctx: Context<'_>,
    #[description = "表示件数（1〜20、既定値5）"]
    #[min = 1]
    #[max = 20]
    limit: Option<i64>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let sessions =
        repository::history(&ctx.data().database, guild_id, user_id, limit.unwrap_or(5)).await?;
    let mut embed = presentation::history_embed(
        ctx.author().display_name(),
        &sessions,
        Utc::now().timestamp(),
    );
    let notice = peek_auto_end_notice(ctx).await?;
    if let Some(notice) = &notice {
        embed = embed.field("自動終了のお知らせ", &notice.message, false);
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    if let Some(notice) = notice
        && let Err(error) = acknowledge_auto_end_notice(ctx, notice.event_id).await
    {
        tracing::warn!(
            %error,
            event_id = notice.event_id,
            "response sent but failed to acknowledge automatic-end notice"
        );
    }
    Ok(())
}

/// 指定月の活動時間と平均を表示する。
#[poise::command(slash_command, guild_only)]
pub async fn month(
    ctx: Context<'_>,
    #[description = "対象月（YYYY-MM）。省略時は当月"] target: Option<String>,
) -> Result<(), Error> {
    defer_ephemeral(ctx).await?;
    let (guild_id, user_id) = ids(ctx)?;
    let now = Utc::now();
    let year_month = match target {
        Some(value) => time::parse_year_month(&value)?,
        None => {
            let local_now = now.with_timezone(&DISPLAY_TIMEZONE);
            attendance::YearMonth {
                year: local_now.year(),
                month: local_now.month(),
            }
        }
    };
    let (start, end) = time::month_bounds(year_month)?;
    let sessions =
        repository::overlapping_completed(&ctx.data().database, guild_id, user_id, start, end)
            .await?;
    let monthly = attendance::aggregate_monthly(&sessions, year_month, now)?;
    let mut embed = presentation::month_embed(ctx.author().display_name(), &monthly);
    let notice = peek_auto_end_notice(ctx).await?;
    if let Some(notice) = &notice {
        embed = embed.field("自動終了のお知らせ", &notice.message, false);
    }
    ctx.send(CreateReply::default().embed(embed).ephemeral(true))
        .await?;
    if let Some(notice) = notice
        && let Err(error) = acknowledge_auto_end_notice(ctx, notice.event_id).await
    {
        tracing::warn!(
            %error,
            event_id = notice.event_id,
            "response sent but failed to acknowledge automatic-end notice"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(
        user_id: i64,
        display_name: &str,
        generation: Option<i64>,
        real_name: Option<&str>,
        role: Option<&str>,
        name_reading: Option<&str>,
    ) -> repository::ActiveAttendanceMember {
        repository::ActiveAttendanceMember {
            user_id,
            display_name: display_name.into(),
            generation,
            real_name: real_name.map(str::to_owned),
            role: role.map(str::to_owned),
            name_reading: name_reading.map(str::to_owned),
        }
    }

    #[test]
    fn active_roster_sorts_by_role_generation_and_reading_with_unconfigured_last() {
        let mut members = vec![
            member(1, "No config", None, None, None, None),
            member(
                2,
                "Suzuki",
                Some(5),
                Some("鈴木"),
                Some("設計"),
                Some("すずき"),
            ),
            member(3, "Abe", Some(4), Some("阿部"), Some("設計"), Some("あべ")),
            member(
                4,
                "Ito",
                Some(4),
                Some("伊藤"),
                Some("設計"),
                Some("いとう"),
            ),
            member(
                5,
                "Pilot",
                Some(3),
                Some("佐藤"),
                Some("操縦"),
                Some("さとう"),
            ),
        ];

        sort_active_members(&mut members);

        assert_eq!(
            members
                .iter()
                .map(|member| member.user_id)
                .collect::<Vec<_>>(),
            vec![5, 3, 4, 2, 1]
        );
    }
}
