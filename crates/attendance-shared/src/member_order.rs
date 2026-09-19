use std::cmp::Ordering;

use crate::text::japanese_sort_key;

#[derive(Debug, Clone, Copy)]
pub struct MemberOrderKey<'a> {
    pub user_id: i64,
    pub generation: Option<i64>,
    pub real_name: Option<&'a str>,
    pub role: Option<&'a str>,
    pub name_reading: Option<&'a str>,
    pub display_name: &'a str,
}

fn compare_optional_text(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => japanese_sort_key(left).cmp(&japanese_sort_key(right)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn compare(left: MemberOrderKey<'_>, right: MemberOrderKey<'_>) -> Ordering {
    let left_unconfigured = left.generation.is_none()
        && left.real_name.is_none()
        && left.role.is_none()
        && left.name_reading.is_none();
    let right_unconfigured = right.generation.is_none()
        && right.real_name.is_none()
        && right.role.is_none()
        && right.name_reading.is_none();
    left_unconfigured
        .cmp(&right_unconfigured)
        .then_with(|| compare_optional_text(left.role, right.role))
        .then_with(|| left.generation.is_none().cmp(&right.generation.is_none()))
        .then_with(|| left.generation.cmp(&right.generation))
        .then_with(|| {
            compare_optional_text(
                left.name_reading
                    .or(left.real_name)
                    .or(Some(left.display_name)),
                right
                    .name_reading
                    .or(right.real_name)
                    .or(Some(right.display_name)),
            )
        })
        .then_with(|| left.user_id.cmp(&right.user_id))
}
