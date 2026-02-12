//! Tests for platform services.

use super::*;

#[test]
fn desktop_power_info() {
    let platform = DesktopPlatform::new();
    let info = platform.power_info().unwrap();
    assert_eq!(info.state, BatteryState::NoBattery);
    assert!(info.battery_percent.is_none());
}

#[test]
fn desktop_time_now() {
    let platform = DesktopPlatform::new();
    let t = platform.now().unwrap();
    // Should be a reasonable year.
    assert!(t.year >= 2024);
    assert!((1..=12).contains(&t.month));
    assert!((1..=31).contains(&t.day));
}

#[test]
fn desktop_time_display() {
    let t = SystemTime {
        year: 2026,
        month: 2,
        day: 7,
        hour: 14,
        minute: 30,
        second: 0,
    };
    assert_eq!(t.to_string(), "2026-02-07 14:30:00");
}

#[test]
fn desktop_uptime() {
    let platform = DesktopPlatform::new();
    let up = platform.uptime_secs().unwrap();
    // Just started, should be 0 or very small.
    assert!(up < 5);
}

#[test]
fn desktop_usb_unsupported() {
    let platform = DesktopPlatform::new();
    assert_eq!(platform.usb_state().unwrap(), UsbState::Unsupported);
}

#[test]
fn usb_state_display() {
    assert_eq!(UsbState::Activated.to_string(), "activated");
    assert_eq!(UsbState::Unsupported.to_string(), "unsupported");
}

#[test]
fn desktop_osk_immediate_confirm() {
    let mut platform = DesktopPlatform::new();
    platform.open("Test", "hello").unwrap();
    match platform.poll().unwrap() {
        OskResult::Confirmed(s) => assert_eq!(s, "hello"),
        other => panic!("expected Confirmed, got {other:?}"),
    }
}

#[test]
fn desktop_osk_close() {
    let mut platform = DesktopPlatform::new();
    platform.open("Test", "data").unwrap();
    platform.close().unwrap();
    match platform.poll().unwrap() {
        OskResult::Cancelled => {},
        other => panic!("expected Cancelled after close, got {other:?}"),
    }
}

#[test]
fn days_to_ymd_epoch() {
    let (y, m, d) = services::days_to_ymd(0);
    assert_eq!((y, m, d), (1970, 1, 1));
}

#[test]
fn days_to_ymd_known_date() {
    // 2024-01-01 = 19723 days since epoch.
    let (y, m, d) = services::days_to_ymd(19723);
    assert_eq!((y, m, d), (2024, 1, 1));
}

#[test]
fn days_to_ymd_leap_day() {
    // 2024-02-29 = 19723 + 31 + 28 = 19782 days since epoch.
    // Wait, January has 31 days, so Jan 31 = 19723+30 = 19753.
    // Feb 1 = 19754, Feb 29 = 19754 + 28 = 19782.
    let (y, m, d) = services::days_to_ymd(19782);
    assert_eq!((y, m, d), (2024, 2, 29));
}

#[test]
fn is_leap_checks() {
    assert!(services::is_leap(2000));
    assert!(services::is_leap(2024));
    assert!(!services::is_leap(1900));
    assert!(!services::is_leap(2023));
}

// ---- Date conversion tests ----

#[test]
fn days_to_ymd_day_1() {
    let (y, m, d) = services::days_to_ymd(1);
    assert_eq!((y, m, d), (1970, 1, 2));
}

#[test]
fn days_to_ymd_end_of_year() {
    // 1970 is not a leap year, so 365 days later is 1971-01-01.
    let (y, m, d) = services::days_to_ymd(365);
    assert_eq!((y, m, d), (1971, 1, 1));
}

#[test]
fn days_to_ymd_feb_28_non_leap() {
    // 2023-02-28: 2023-01-01 is day 19358, then +31 (Jan) +27 (Feb 1-27) = 19416.
    let (y, m, d) = services::days_to_ymd(19416);
    assert_eq!((y, m, d), (2023, 2, 28));
}

#[test]
fn days_to_ymd_mar_1_leap() {
    // 2024-03-01: 2024-01-01 is day 19723, then +31 (Jan) +29 (Feb, leap) = 19783.
    let (y, m, d) = services::days_to_ymd(19783);
    assert_eq!((y, m, d), (2024, 3, 1));
}

#[test]
fn days_to_ymd_century_not_leap() {
    // 1900 is divisible by 100 but not 400, so it is NOT a leap year.
    assert!(!services::is_leap(1900));
    // 2100 is also divisible by 100 but not 400, so it is NOT a leap year.
    assert!(!services::is_leap(2100));
}

#[test]
fn days_to_ymd_400_year_leap() {
    // 2000 is divisible by 400, so it IS a leap year.
    assert!(services::is_leap(2000));
    // 2100 is NOT a leap year (div by 100, not by 400).
    assert!(!services::is_leap(2100));
}

#[test]
fn days_to_ymd_far_future() {
    // Compute day count for 2100-01-01.
    // Count leap years from 1970 to 2099: leap if (div4 && !div100) || div400.
    // Leap years: 1972,1976,...,2096 (not 2100). That's (2096-1972)/4 + 1 = 32 leap years.
    // Total days = 130*365 + 32 = 47482 + 32 = 47482 + 32? Let me compute:
    // 130 years * 365 = 47450, plus 32 leap days = 47482.
    let (y, m, d) = services::days_to_ymd(47482);
    assert_eq!((y, m, d), (2100, 1, 1));
}

#[test]
fn days_to_ymd_month_boundaries() {
    // Jan 31, 1970 = day 30 (0-indexed).
    let (y, m, d) = services::days_to_ymd(30);
    assert_eq!((y, m, d), (1970, 1, 31));

    // Mar 31, 1970 = 31 (Jan) + 28 (Feb, non-leap) + 30 = day 89.
    let (y, m, d) = services::days_to_ymd(89);
    assert_eq!((y, m, d), (1970, 3, 31));

    // Apr 30, 1970 = 31 + 28 + 31 + 29 = day 119.
    let (y, m, d) = services::days_to_ymd(119);
    assert_eq!((y, m, d), (1970, 4, 30));
}

// ---- Time display formatting tests ----

#[test]
fn time_display_midnight() {
    let t = SystemTime {
        year: 2025,
        month: 6,
        day: 15,
        hour: 0,
        minute: 0,
        second: 0,
    };
    assert_eq!(t.to_string(), "2025-06-15 00:00:00");
}

#[test]
fn time_display_single_digits_padded() {
    let t = SystemTime {
        year: 2024,
        month: 1,
        day: 5,
        hour: 3,
        minute: 7,
        second: 9,
    };
    let s = t.to_string();
    assert!(s.contains("-01-"), "month should be zero-padded: {s}");
    assert!(s.contains("-05 "), "day should be zero-padded: {s}");
    assert_eq!(s, "2024-01-05 03:07:09");
}

#[test]
fn time_display_end_of_year() {
    let t = SystemTime {
        year: 2025,
        month: 12,
        day: 31,
        hour: 23,
        minute: 59,
        second: 59,
    };
    assert_eq!(t.to_string(), "2025-12-31 23:59:59");
}

#[test]
fn time_display_consistency() {
    let t = SystemTime {
        year: 2026,
        month: 3,
        day: 14,
        hour: 9,
        minute: 26,
        second: 53,
    };
    let first = t.to_string();
    let second = t.to_string();
    assert_eq!(first, second, "repeated to_string() should be identical");
}

// ---- Platform contract tests ----

#[test]
fn uptime_monotonic() {
    let platform = DesktopPlatform::new();
    let first = platform.uptime_secs().unwrap();
    let second = platform.uptime_secs().unwrap();
    assert!(
        second >= first,
        "uptime should be monotonically non-decreasing"
    );
}

#[test]
fn now_returns_valid_ranges() {
    let platform = DesktopPlatform::new();
    let t = platform.now().unwrap();
    assert!(
        (1..=12).contains(&t.month),
        "month out of range: {}",
        t.month
    );
    assert!((1..=31).contains(&t.day), "day out of range: {}", t.day);
    assert!(t.hour <= 23, "hour out of range: {}", t.hour);
    assert!(t.minute <= 59, "minute out of range: {}", t.minute);
    assert!(t.second <= 59, "second out of range: {}", t.second);
}

#[test]
fn osk_poll_before_open() {
    let mut platform = DesktopPlatform::new();
    // Poll without opening first should return Cancelled (no buffer set).
    match platform.poll().unwrap() {
        OskResult::Cancelled => {},
        other => panic!("expected Cancelled when polling without open, got {other:?}"),
    }
}
