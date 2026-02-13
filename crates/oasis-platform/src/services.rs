//! Platform service traits and desktop implementation.

use oasis_types::error::Result;

// ---------------------------------------------------------------------------
// Power service
// ---------------------------------------------------------------------------

/// Battery / power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryState {
    /// Running on battery.
    Discharging,
    /// Plugged in and charging.
    Charging,
    /// Fully charged, on external power.
    Full,
    /// No battery present (desktop / wall power).
    NoBattery,
}

/// CPU clock speed.
#[derive(Debug, Clone, Copy)]
pub struct CpuClock {
    /// Current frequency in MHz.
    pub current_mhz: u32,
    /// Maximum frequency in MHz (0 if unknown).
    pub max_mhz: u32,
}

/// Snapshot of power-related information.
#[derive(Debug, Clone)]
pub struct PowerInfo {
    /// Battery charge percentage (0-100), or `None` if no battery.
    pub battery_percent: Option<u8>,
    /// Estimated minutes remaining, or `None` if unknown/charging.
    pub battery_minutes: Option<u32>,
    /// Current battery state.
    pub state: BatteryState,
    /// CPU clock info.
    pub cpu: CpuClock,
}

/// Abstraction over platform power management.
pub trait PowerService {
    /// Query current power information.
    fn power_info(&self) -> Result<PowerInfo>;
}

// ---------------------------------------------------------------------------
// Time service
// ---------------------------------------------------------------------------

/// A simple wall-clock timestamp.
#[derive(Debug, Clone, Copy)]
pub struct SystemTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl std::fmt::Display for SystemTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second,
        )
    }
}

/// Abstraction over platform time services.
pub trait TimeService {
    /// Current wall-clock time.
    fn now(&self) -> Result<SystemTime>;

    /// Seconds since the platform booted (or the process started).
    fn uptime_secs(&self) -> Result<u64>;
}

// ---------------------------------------------------------------------------
// USB service
// ---------------------------------------------------------------------------

/// USB connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbState {
    /// USB storage is deactivated.
    Deactivated,
    /// USB storage is active (device exposed as mass storage).
    Activated,
    /// USB cable connected but storage not activated.
    Connected,
    /// No USB cable detected.
    Disconnected,
    /// Platform does not support USB management.
    Unsupported,
}

impl std::fmt::Display for UsbState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deactivated => write!(f, "deactivated"),
            Self::Activated => write!(f, "activated"),
            Self::Connected => write!(f, "connected"),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Unsupported => write!(f, "unsupported"),
        }
    }
}

/// Abstraction over USB mass-storage management.
pub trait UsbService {
    /// Current USB state.
    fn usb_state(&self) -> Result<UsbState>;

    /// Activate USB mass-storage mode (PSP exposes Memory Stick to host).
    fn activate(&mut self) -> Result<()>;

    /// Deactivate USB mass-storage mode.
    fn deactivate(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// On-screen keyboard service
// ---------------------------------------------------------------------------

/// Result of an OSK session.
#[derive(Debug, Clone)]
pub enum OskResult {
    /// User confirmed input.
    Confirmed(String),
    /// User cancelled.
    Cancelled,
    /// Still editing (poll again next frame).
    Editing,
}

/// Abstraction over the platform's native on-screen keyboard.
/// On PSP this wraps `sceUtilityOskInitStart`. On desktop, the core
/// `osk` module provides a software keyboard rendered via SDI.
pub trait OskService {
    /// Begin an OSK session with an optional initial string.
    fn open(&mut self, title: &str, initial: &str) -> Result<()>;

    /// Poll the current state. Returns the result or `Editing` if still open.
    fn poll(&mut self) -> Result<OskResult>;

    /// Force-close the OSK.
    fn close(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Network service
// ---------------------------------------------------------------------------

/// WiFi / network connection status.
#[derive(Debug, Clone)]
pub struct WifiInfo {
    /// Whether the WLAN hardware is available (powered on + switch on).
    pub available: bool,
    /// Whether WiFi is connected to an access point.
    pub connected: bool,
    /// Assigned IP address (if connected).
    pub ip_address: Option<String>,
    /// MAC address as 6 bytes.
    pub mac_address: [u8; 6],
}

/// HTTP response from a network service.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status_code: u16,
    /// Response body as bytes.
    pub body: Vec<u8>,
}

/// Abstraction over platform WiFi / network status queries.
pub trait NetworkService {
    /// Query WiFi hardware and connection status.
    fn wifi_info(&self) -> Result<WifiInfo>;

    /// Perform a blocking HTTP GET request.
    ///
    /// Returns the status code and response body. Default implementation
    /// returns an error (platform does not support HTTP).
    fn http_get(&self, _url: &str) -> Result<HttpResponse> {
        Err(oasis_types::error::OasisError::Backend(
            "HTTP not supported on this platform".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Unified platform trait
// ---------------------------------------------------------------------------

/// Aggregate trait providing access to all platform services.
pub trait Platform: PowerService + TimeService + UsbService + OskService {}

// ---------------------------------------------------------------------------
// Desktop implementation
// ---------------------------------------------------------------------------

/// Default platform implementation for desktop/Pi using `std` facilities.
pub struct DesktopPlatform {
    start_time: std::time::Instant,
    osk_buffer: Option<String>,
    osk_title: Option<String>,
}

impl DesktopPlatform {
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
            osk_buffer: None,
            osk_title: None,
        }
    }
}

impl Default for DesktopPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerService for DesktopPlatform {
    fn power_info(&self) -> Result<PowerInfo> {
        // Desktop: no battery, unknown clock.
        Ok(PowerInfo {
            battery_percent: None,
            battery_minutes: None,
            state: BatteryState::NoBattery,
            cpu: CpuClock {
                current_mhz: 0,
                max_mhz: 0,
            },
        })
    }
}

impl TimeService for DesktopPlatform {
    fn now(&self) -> Result<SystemTime> {
        use std::time::SystemTime as StdTime;
        let dur = StdTime::now()
            .duration_since(StdTime::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = dur.as_secs();

        // Simple UTC breakdown (no TZ handling -- good enough for an embedded OS).
        let days = secs / 86400;
        let time_of_day = secs % 86400;
        let hour = (time_of_day / 3600) as u8;
        let minute = ((time_of_day % 3600) / 60) as u8;
        let second = (time_of_day % 60) as u8;

        // Days since 1970-01-01 to Y-M-D.
        let (year, month, day) = days_to_ymd(days);

        Ok(SystemTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    fn uptime_secs(&self) -> Result<u64> {
        Ok(self.start_time.elapsed().as_secs())
    }
}

impl UsbService for DesktopPlatform {
    fn usb_state(&self) -> Result<UsbState> {
        Ok(UsbState::Unsupported)
    }

    fn activate(&mut self) -> Result<()> {
        Ok(()) // No-op on desktop.
    }

    fn deactivate(&mut self) -> Result<()> {
        Ok(()) // No-op on desktop.
    }
}

impl OskService for DesktopPlatform {
    fn open(&mut self, title: &str, initial: &str) -> Result<()> {
        self.osk_title = Some(title.to_string());
        self.osk_buffer = Some(initial.to_string());
        Ok(())
    }

    fn poll(&mut self) -> Result<OskResult> {
        // Desktop has a physical keyboard, so the OSK immediately returns
        // the initial text. Real input comes from TextInput events.
        match self.osk_buffer.take() {
            Some(buf) => Ok(OskResult::Confirmed(buf)),
            None => Ok(OskResult::Cancelled),
        }
    }

    fn close(&mut self) -> Result<()> {
        self.osk_buffer = None;
        self.osk_title = None;
        Ok(())
    }
}

impl NetworkService for DesktopPlatform {
    fn wifi_info(&self) -> Result<WifiInfo> {
        Ok(WifiInfo {
            available: false,
            connected: false,
            ip_address: None,
            mac_address: [0; 6],
        })
    }
}

impl Platform for DesktopPlatform {}

// ---------------------------------------------------------------------------
// Date helper
// ---------------------------------------------------------------------------

/// Convert days since Unix epoch to (year, month, day).
pub(crate) fn days_to_ymd(mut days: u64) -> (u16, u8, u8) {
    let mut year = 1970u16;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days {
            break;
        }
        days -= year_days;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0u8;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = (i + 1) as u8;
            break;
        }
        days -= md;
    }
    if month == 0 {
        month = 12;
    }
    (year, month, (days + 1) as u8)
}

pub(crate) fn is_leap(y: u16) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

// ---------------------------------------------------------------------------
// In-module tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Mock services for testing ----

    /// Mock power service with configurable state.
    struct MockPowerService {
        battery_percent: Option<u8>,
        battery_minutes: Option<u32>,
        state: BatteryState,
        current_mhz: u32,
        max_mhz: u32,
    }

    impl MockPowerService {
        fn new() -> Self {
            Self {
                battery_percent: Some(75),
                battery_minutes: Some(180),
                state: BatteryState::Discharging,
                current_mhz: 333,
                max_mhz: 333,
            }
        }

        fn with_charging() -> Self {
            Self {
                battery_percent: Some(45),
                battery_minutes: None,
                state: BatteryState::Charging,
                current_mhz: 222,
                max_mhz: 333,
            }
        }

        fn with_no_battery() -> Self {
            Self {
                battery_percent: None,
                battery_minutes: None,
                state: BatteryState::NoBattery,
                current_mhz: 3200,
                max_mhz: 4800,
            }
        }

        fn with_full() -> Self {
            Self {
                battery_percent: Some(100),
                battery_minutes: None,
                state: BatteryState::Full,
                current_mhz: 222,
                max_mhz: 333,
            }
        }
    }

    impl PowerService for MockPowerService {
        fn power_info(&self) -> Result<PowerInfo> {
            Ok(PowerInfo {
                battery_percent: self.battery_percent,
                battery_minutes: self.battery_minutes,
                state: self.state,
                cpu: CpuClock {
                    current_mhz: self.current_mhz,
                    max_mhz: self.max_mhz,
                },
            })
        }
    }

    /// Mock time service with fixed time.
    struct MockTimeService {
        time: SystemTime,
        uptime: u64,
    }

    impl MockTimeService {
        fn new() -> Self {
            Self {
                time: SystemTime {
                    year: 2026,
                    month: 2,
                    day: 13,
                    hour: 14,
                    minute: 30,
                    second: 45,
                },
                uptime: 1234,
            }
        }

        fn with_custom_time(time: SystemTime, uptime: u64) -> Self {
            Self { time, uptime }
        }
    }

    impl TimeService for MockTimeService {
        fn now(&self) -> Result<SystemTime> {
            Ok(self.time)
        }

        fn uptime_secs(&self) -> Result<u64> {
            Ok(self.uptime)
        }
    }

    /// Mock USB service with configurable state.
    struct MockUsbService {
        state: UsbState,
        activate_count: usize,
        deactivate_count: usize,
    }

    impl MockUsbService {
        fn new(state: UsbState) -> Self {
            Self {
                state,
                activate_count: 0,
                deactivate_count: 0,
            }
        }
    }

    impl UsbService for MockUsbService {
        fn usb_state(&self) -> Result<UsbState> {
            Ok(self.state)
        }

        fn activate(&mut self) -> Result<()> {
            self.activate_count += 1;
            if self.state == UsbState::Connected {
                self.state = UsbState::Activated;
            }
            Ok(())
        }

        fn deactivate(&mut self) -> Result<()> {
            self.deactivate_count += 1;
            if self.state == UsbState::Activated {
                self.state = UsbState::Deactivated;
            }
            Ok(())
        }
    }

    /// Mock network service with configurable WiFi state.
    struct MockNetworkService {
        wifi: WifiInfo,
    }

    impl MockNetworkService {
        fn new_connected() -> Self {
            Self {
                wifi: WifiInfo {
                    available: true,
                    connected: true,
                    ip_address: Some("192.168.1.100".to_string()),
                    mac_address: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
                },
            }
        }

        fn new_disconnected() -> Self {
            Self {
                wifi: WifiInfo {
                    available: true,
                    connected: false,
                    ip_address: None,
                    mac_address: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
                },
            }
        }

        fn new_unavailable() -> Self {
            Self {
                wifi: WifiInfo {
                    available: false,
                    connected: false,
                    ip_address: None,
                    mac_address: [0; 6],
                },
            }
        }
    }

    impl NetworkService for MockNetworkService {
        fn wifi_info(&self) -> Result<WifiInfo> {
            Ok(self.wifi.clone())
        }
    }

    // ---- BatteryState tests ----

    #[test]
    fn battery_state_enum_variants() {
        let states = [
            BatteryState::Discharging,
            BatteryState::Charging,
            BatteryState::Full,
            BatteryState::NoBattery,
        ];
        // Just ensure all variants can be created and compared.
        assert_eq!(states[0], BatteryState::Discharging);
        assert_ne!(states[0], states[1]);
    }

    #[test]
    fn battery_state_debug_format() {
        let state = BatteryState::Charging;
        let debug_str = format!("{:?}", state);
        assert!(debug_str.contains("Charging"));
    }

    // ---- MockPowerService tests ----

    #[test]
    fn mock_power_service_discharging() {
        let svc = MockPowerService::new();
        let info = svc.power_info().unwrap();
        assert_eq!(info.state, BatteryState::Discharging);
        assert_eq!(info.battery_percent, Some(75));
        assert_eq!(info.battery_minutes, Some(180));
        assert_eq!(info.cpu.current_mhz, 333);
        assert_eq!(info.cpu.max_mhz, 333);
    }

    #[test]
    fn mock_power_service_charging() {
        let svc = MockPowerService::with_charging();
        let info = svc.power_info().unwrap();
        assert_eq!(info.state, BatteryState::Charging);
        assert_eq!(info.battery_percent, Some(45));
        assert!(info.battery_minutes.is_none());
    }

    #[test]
    fn mock_power_service_no_battery() {
        let svc = MockPowerService::with_no_battery();
        let info = svc.power_info().unwrap();
        assert_eq!(info.state, BatteryState::NoBattery);
        assert!(info.battery_percent.is_none());
        assert!(info.battery_minutes.is_none());
    }

    #[test]
    fn mock_power_service_full() {
        let svc = MockPowerService::with_full();
        let info = svc.power_info().unwrap();
        assert_eq!(info.state, BatteryState::Full);
        assert_eq!(info.battery_percent, Some(100));
    }

    // ---- CpuClock tests ----

    #[test]
    fn cpu_clock_debug_format() {
        let clock = CpuClock {
            current_mhz: 222,
            max_mhz: 333,
        };
        let debug_str = format!("{:?}", clock);
        assert!(debug_str.contains("222"));
        assert!(debug_str.contains("333"));
    }

    #[test]
    fn cpu_clock_copy() {
        let clock1 = CpuClock {
            current_mhz: 100,
            max_mhz: 200,
        };
        let clock2 = clock1;
        assert_eq!(clock1.current_mhz, clock2.current_mhz);
    }

    // ---- PowerInfo tests ----

    #[test]
    fn power_info_clone() {
        let info1 = PowerInfo {
            battery_percent: Some(50),
            battery_minutes: Some(60),
            state: BatteryState::Discharging,
            cpu: CpuClock {
                current_mhz: 222,
                max_mhz: 333,
            },
        };
        let info2 = info1.clone();
        assert_eq!(info1.battery_percent, info2.battery_percent);
        assert_eq!(info1.state, info2.state);
    }

    // ---- SystemTime tests ----

    #[test]
    fn system_time_display_zero_padding() {
        let t = SystemTime {
            year: 2026,
            month: 1,
            day: 5,
            hour: 9,
            minute: 3,
            second: 7,
        };
        assert_eq!(t.to_string(), "2026-01-05 09:03:07");
    }

    #[test]
    fn system_time_debug_format() {
        let t = SystemTime {
            year: 2026,
            month: 2,
            day: 13,
            hour: 14,
            minute: 30,
            second: 0,
        };
        let debug_str = format!("{:?}", t);
        assert!(debug_str.contains("2026"));
    }

    #[test]
    fn system_time_copy() {
        let t1 = SystemTime {
            year: 2025,
            month: 6,
            day: 15,
            hour: 12,
            minute: 0,
            second: 0,
        };
        let t2 = t1;
        assert_eq!(t1.year, t2.year);
    }

    // ---- MockTimeService tests ----

    #[test]
    fn mock_time_service_now() {
        let svc = MockTimeService::new();
        let t = svc.now().unwrap();
        assert_eq!(t.year, 2026);
        assert_eq!(t.month, 2);
        assert_eq!(t.day, 13);
    }

    #[test]
    fn mock_time_service_uptime() {
        let svc = MockTimeService::new();
        let uptime = svc.uptime_secs().unwrap();
        assert_eq!(uptime, 1234);
    }

    #[test]
    fn mock_time_service_custom() {
        let custom_time = SystemTime {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        let svc = MockTimeService::with_custom_time(custom_time, 9999);
        let t = svc.now().unwrap();
        assert_eq!(t.year, 2000);
        assert_eq!(svc.uptime_secs().unwrap(), 9999);
    }

    // ---- UsbState tests ----

    #[test]
    fn usb_state_display_all_variants() {
        assert_eq!(UsbState::Deactivated.to_string(), "deactivated");
        assert_eq!(UsbState::Activated.to_string(), "activated");
        assert_eq!(UsbState::Connected.to_string(), "connected");
        assert_eq!(UsbState::Disconnected.to_string(), "disconnected");
        assert_eq!(UsbState::Unsupported.to_string(), "unsupported");
    }

    #[test]
    fn usb_state_equality() {
        assert_eq!(UsbState::Activated, UsbState::Activated);
        assert_ne!(UsbState::Activated, UsbState::Deactivated);
    }

    #[test]
    fn usb_state_debug_format() {
        let state = UsbState::Connected;
        let debug_str = format!("{:?}", state);
        assert!(debug_str.contains("Connected"));
    }

    // ---- MockUsbService tests ----

    #[test]
    fn mock_usb_service_disconnected() {
        let svc = MockUsbService::new(UsbState::Disconnected);
        assert_eq!(svc.usb_state().unwrap(), UsbState::Disconnected);
    }

    #[test]
    fn mock_usb_service_activate() {
        let mut svc = MockUsbService::new(UsbState::Connected);
        assert_eq!(svc.activate_count, 0);
        svc.activate().unwrap();
        assert_eq!(svc.activate_count, 1);
        assert_eq!(svc.usb_state().unwrap(), UsbState::Activated);
    }

    #[test]
    fn mock_usb_service_deactivate() {
        let mut svc = MockUsbService::new(UsbState::Activated);
        assert_eq!(svc.deactivate_count, 0);
        svc.deactivate().unwrap();
        assert_eq!(svc.deactivate_count, 1);
        assert_eq!(svc.usb_state().unwrap(), UsbState::Deactivated);
    }

    #[test]
    fn mock_usb_service_activate_when_disconnected() {
        let mut svc = MockUsbService::new(UsbState::Disconnected);
        svc.activate().unwrap();
        // State shouldn't change from disconnected.
        assert_eq!(svc.usb_state().unwrap(), UsbState::Disconnected);
        assert_eq!(svc.activate_count, 1);
    }

    // ---- OskResult tests ----

    #[test]
    fn osk_result_confirmed() {
        let result = OskResult::Confirmed("test".to_string());
        match result {
            OskResult::Confirmed(s) => assert_eq!(s, "test"),
            _ => panic!("expected Confirmed"),
        }
    }

    #[test]
    fn osk_result_cancelled() {
        let result = OskResult::Cancelled;
        matches!(result, OskResult::Cancelled);
    }

    #[test]
    fn osk_result_editing() {
        let result = OskResult::Editing;
        matches!(result, OskResult::Editing);
    }

    #[test]
    fn osk_result_debug_format() {
        let result = OskResult::Confirmed("hello".to_string());
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("Confirmed"));
        assert!(debug_str.contains("hello"));
    }

    #[test]
    fn osk_result_clone() {
        let result1 = OskResult::Confirmed("data".to_string());
        let result2 = result1.clone();
        match (result1, result2) {
            (OskResult::Confirmed(s1), OskResult::Confirmed(s2)) => assert_eq!(s1, s2),
            _ => panic!("expected both to be Confirmed"),
        }
    }

    // ---- WifiInfo tests ----

    #[test]
    fn wifi_info_debug_format() {
        let info = WifiInfo {
            available: true,
            connected: true,
            ip_address: Some("192.168.1.1".to_string()),
            mac_address: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
        };
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("available"));
        assert!(debug_str.contains("true"));
    }

    #[test]
    fn wifi_info_clone() {
        let info1 = WifiInfo {
            available: false,
            connected: false,
            ip_address: None,
            mac_address: [0; 6],
        };
        let info2 = info1.clone();
        assert_eq!(info1.available, info2.available);
        assert_eq!(info1.mac_address, info2.mac_address);
    }

    // ---- MockNetworkService tests ----

    #[test]
    fn mock_network_service_connected() {
        let svc = MockNetworkService::new_connected();
        let info = svc.wifi_info().unwrap();
        assert!(info.available);
        assert!(info.connected);
        assert_eq!(info.ip_address, Some("192.168.1.100".to_string()));
        assert_eq!(info.mac_address, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn mock_network_service_disconnected() {
        let svc = MockNetworkService::new_disconnected();
        let info = svc.wifi_info().unwrap();
        assert!(info.available);
        assert!(!info.connected);
        assert!(info.ip_address.is_none());
    }

    #[test]
    fn mock_network_service_unavailable() {
        let svc = MockNetworkService::new_unavailable();
        let info = svc.wifi_info().unwrap();
        assert!(!info.available);
        assert!(!info.connected);
    }

    #[test]
    fn mock_network_service_http_get_default() {
        let svc = MockNetworkService::new_connected();
        let result = svc.http_get("http://example.com");
        assert!(result.is_err());
    }

    // ---- HttpResponse tests ----

    #[test]
    fn http_response_debug_format() {
        let resp = HttpResponse {
            status_code: 200,
            body: vec![1, 2, 3],
        };
        let debug_str = format!("{:?}", resp);
        assert!(debug_str.contains("200"));
    }

    #[test]
    fn http_response_clone() {
        let resp1 = HttpResponse {
            status_code: 404,
            body: vec![4, 5, 6],
        };
        let resp2 = resp1.clone();
        assert_eq!(resp1.status_code, resp2.status_code);
        assert_eq!(resp1.body, resp2.body);
    }

    // ---- DesktopPlatform tests ----

    #[test]
    fn desktop_platform_new() {
        let platform = DesktopPlatform::new();
        let info = platform.power_info().unwrap();
        assert_eq!(info.state, BatteryState::NoBattery);
    }

    #[test]
    fn desktop_platform_default() {
        let platform = DesktopPlatform::default();
        let t = platform.now().unwrap();
        assert!(t.year >= 2024);
    }

    #[test]
    fn desktop_platform_power_service() {
        let platform = DesktopPlatform::new();
        let info = platform.power_info().unwrap();
        assert!(info.battery_percent.is_none());
        assert!(info.battery_minutes.is_none());
        assert_eq!(info.cpu.current_mhz, 0);
    }

    #[test]
    fn desktop_platform_time_service() {
        let platform = DesktopPlatform::new();
        let t = platform.now().unwrap();
        assert!((1..=12).contains(&t.month));
        assert!((1..=31).contains(&t.day));
        let uptime = platform.uptime_secs().unwrap();
        assert!(uptime < 10);
    }

    #[test]
    fn desktop_platform_usb_service() {
        let mut platform = DesktopPlatform::new();
        assert_eq!(platform.usb_state().unwrap(), UsbState::Unsupported);
        platform.activate().unwrap();
        platform.deactivate().unwrap();
        assert_eq!(platform.usb_state().unwrap(), UsbState::Unsupported);
    }

    #[test]
    fn desktop_platform_osk_service() {
        let mut platform = DesktopPlatform::new();
        platform.open("Title", "init").unwrap();
        match platform.poll().unwrap() {
            OskResult::Confirmed(s) => assert_eq!(s, "init"),
            _ => panic!("expected Confirmed"),
        }
        platform.close().unwrap();
    }

    #[test]
    fn desktop_platform_network_service() {
        let platform = DesktopPlatform::new();
        let info = platform.wifi_info().unwrap();
        assert!(!info.available);
        assert!(!info.connected);
        assert!(info.ip_address.is_none());
    }

    #[test]
    fn desktop_platform_network_http_get_default() {
        let platform = DesktopPlatform::new();
        let result = platform.http_get("http://test.com");
        assert!(result.is_err());
    }

    // ---- Date helper function tests ----

    #[test]
    fn days_to_ymd_zero() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_to_ymd_one_day() {
        let (y, m, d) = days_to_ymd(1);
        assert_eq!((y, m, d), (1970, 1, 2));
    }

    #[test]
    fn days_to_ymd_end_of_january() {
        let (y, m, d) = days_to_ymd(30);
        assert_eq!((y, m, d), (1970, 1, 31));
    }

    #[test]
    fn days_to_ymd_first_of_february() {
        let (y, m, d) = days_to_ymd(31);
        assert_eq!((y, m, d), (1970, 2, 1));
    }

    #[test]
    fn days_to_ymd_leap_year_feb_29() {
        // 2024-02-29 is day 19782.
        let (y, m, d) = days_to_ymd(19782);
        assert_eq!((y, m, d), (2024, 2, 29));
    }

    #[test]
    fn days_to_ymd_non_leap_year_feb_28() {
        // 2023-02-28 is day 19416.
        let (y, m, d) = days_to_ymd(19416);
        assert_eq!((y, m, d), (2023, 2, 28));
    }

    #[test]
    fn days_to_ymd_december_31() {
        // 1970-12-31 is day 364.
        let (y, m, d) = days_to_ymd(364);
        assert_eq!((y, m, d), (1970, 12, 31));
    }

    #[test]
    fn is_leap_divisible_by_4() {
        assert!(is_leap(2024));
        assert!(is_leap(2020));
    }

    #[test]
    fn is_leap_not_divisible_by_4() {
        assert!(!is_leap(2023));
        assert!(!is_leap(2025));
    }

    #[test]
    fn is_leap_century_not_divisible_by_400() {
        assert!(!is_leap(1900));
        assert!(!is_leap(2100));
    }

    #[test]
    fn is_leap_century_divisible_by_400() {
        assert!(is_leap(2000));
        assert!(is_leap(2400));
    }

    #[test]
    fn is_leap_edge_cases() {
        assert!(is_leap(1600));
        assert!(!is_leap(1700));
        assert!(!is_leap(1800));
    }

    // ---- NetworkService default http_get tests ----

    struct TestNetworkService;

    impl NetworkService for TestNetworkService {
        fn wifi_info(&self) -> Result<WifiInfo> {
            Ok(WifiInfo {
                available: true,
                connected: true,
                ip_address: Some("10.0.0.1".to_string()),
                mac_address: [0; 6],
            })
        }
    }

    #[test]
    fn network_service_default_http_get() {
        let svc = TestNetworkService;
        let result = svc.http_get("http://example.com");
        assert!(result.is_err());
        match result {
            Err(oasis_types::error::OasisError::Backend(msg)) => {
                assert!(msg.contains("not supported"));
            },
            _ => panic!("expected Backend error"),
        }
    }

    // ---- Platform trait tests ----

    #[test]
    fn desktop_platform_implements_platform_trait() {
        let _platform: &dyn Platform = &DesktopPlatform::new();
        // Just ensure it compiles and can be used as a Platform trait object.
    }
}
