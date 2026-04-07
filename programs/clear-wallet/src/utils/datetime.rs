/// Format a unix timestamp as "YYYY-MM-DD HH:MM:SS" into the provided buffer.
/// Returns the number of bytes written (always 19), or None if the buffer is too small.
pub fn format_timestamp(timestamp: i64, buf: &mut [u8]) -> Option<usize> {
    if buf.len() < 19 {
        return None;
    }

    let (year, month, day, hour, min, sec) = timestamp_to_parts(timestamp);

    write_decimal_4(buf, 0, year as u32);
    buf[4] = b'-';
    write_decimal_2(buf, 5, month as u32);
    buf[7] = b'-';
    write_decimal_2(buf, 8, day as u32);
    buf[10] = b' ';
    write_decimal_2(buf, 11, hour as u32);
    buf[13] = b':';
    write_decimal_2(buf, 14, min as u32);
    buf[16] = b':';
    write_decimal_2(buf, 17, sec as u32);

    Some(19)
}

fn write_decimal_2(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset] = b'0' + (val / 10) as u8;
    buf[offset + 1] = b'0' + (val % 10) as u8;
}

fn write_decimal_4(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset] = b'0' + (val / 1000) as u8;
    buf[offset + 1] = b'0' + ((val / 100) % 10) as u8;
    buf[offset + 2] = b'0' + ((val / 10) % 10) as u8;
    buf[offset + 3] = b'0' + (val % 10) as u8;
}

/// Convert a unix timestamp to (year, month, day, hour, min, sec).
fn timestamp_to_parts(timestamp: i64) -> (i64, i64, i64, i64, i64, i64) {
    let secs_per_day: i64 = 86400;
    let total_secs = timestamp;
    let mut days = total_secs / secs_per_day;
    let day_secs = ((total_secs % secs_per_day) + secs_per_day) % secs_per_day;

    if total_secs < 0 && day_secs > 0 {
        days -= 1;
    }

    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;

    // Days since 1970-01-01 → civil date (algorithm from Howard Hinnant)
    days += 719468; // shift epoch from 1970-01-01 to 0000-03-01
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, hour, min, sec)
}
