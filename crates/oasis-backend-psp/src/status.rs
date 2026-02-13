//! System status queries (hardware info, battery, clock, USB, WiFi).

use psp::sys;

/// Runtime hardware info queried from PSP firmware.
pub struct SystemInfo {
    /// CPU clock frequency in MHz.
    pub cpu_mhz: i32,
    /// Bus clock frequency in MHz.
    pub bus_mhz: i32,
    /// Media Engine clock frequency in MHz (kernel mode only).
    pub me_mhz: i32,
    /// Whether extra 4MB volatile RAM was claimed (PSP-2000+).
    pub volatile_mem_available: bool,
    /// Size of extra volatile memory in bytes (0 if unavailable).
    pub volatile_mem_size: i32,
}

impl SystemInfo {
    /// Query system info from PSP hardware.
    ///
    /// CPU and bus frequencies use `psp::power::get_clock()`. ME frequency
    /// requires kernel mode (raw syscall, no high-level wrapper).
    pub fn query() -> Self {
        let clock = psp::power::get_clock();
        // ME clock frequency requires kernel mode.
        #[cfg(feature = "kernel-me-clock")]
        let me_mhz = {
            // SAFETY: scePowerGetMeClockFrequency is a firmware FFI returning
            // a scalar value. No high-level wrapper exists for ME clock.
            unsafe { sys::scePowerGetMeClockFrequency() }
        };
        #[cfg(not(feature = "kernel-me-clock"))]
        let me_mhz = 0;

        Self {
            cpu_mhz: clock.cpu_mhz,
            bus_mhz: clock.bus_mhz,
            me_mhz,
            volatile_mem_available: false,
            volatile_mem_size: 0,
        }
    }
}

/// Day-of-week abbreviations (Monday=0 through Sunday=6).
const DOW_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Full month names for date display.
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Dynamic status info polled each frame (or periodically).
pub struct StatusBarInfo {
    /// Battery charge percentage (0-100), or -1 if no battery.
    pub battery_percent: i32,
    /// Whether the battery is currently charging.
    pub battery_charging: bool,
    /// Whether AC power is connected.
    pub ac_power: bool,
    /// Current hour (0-23).
    pub hour: u16,
    /// Current minute (0-59).
    pub minute: u16,
    /// Day-of-week abbreviation (e.g. "Mon").
    pub day_of_week: &'static str,
    /// Current month (1-12).
    pub month: u16,
    /// Current day of month (1-31).
    pub day: u16,
    /// Current year (e.g. 2026).
    pub year: u16,
    /// Whether a USB cable is connected.
    pub usb_connected: bool,
    /// Whether the WiFi switch is on.
    pub wifi_on: bool,
}

impl StatusBarInfo {
    /// Full month name for the current date.
    pub fn month_name(&self) -> &'static str {
        if self.month >= 1 && self.month <= 12 {
            MONTH_NAMES[(self.month - 1) as usize]
        } else {
            "???"
        }
    }

    /// Poll live status from PSP hardware.
    ///
    /// Uses `psp::rtc` for time and day-of-week instead of raw syscalls.
    pub fn poll() -> Self {
        let bat = psp::power::battery_info();
        let ac_power = psp::power::is_ac_power();

        let battery_percent = if bat.is_present { bat.percent } else { -1 };

        // Get time via psp::rtc (high-level API).
        let (hour, minute, dow, month, day, year) = if let Ok(tick) = psp::rtc::Tick::now() {
            if let Ok(local) = psp::rtc::to_local(&tick) {
                if let Ok(dt) = local.to_datetime() {
                    let dow_idx =
                        psp::rtc::day_of_week(dt.year() as i32, dt.month() as i32, dt.day() as i32);
                    let dow_name = if (0..7).contains(&dow_idx) {
                        DOW_NAMES[dow_idx as usize]
                    } else {
                        "???"
                    };
                    (
                        dt.hour(),
                        dt.minute(),
                        dow_name,
                        dt.month(),
                        dt.day(),
                        dt.year(),
                    )
                } else {
                    (0, 0, "???", 0, 0, 0)
                }
            } else {
                (0, 0, "???", 0, 0, 0)
            }
        } else {
            (0, 0, "???", 0, 0, 0)
        };

        // SAFETY: sceUsbGetState and sceWlanGetSwitchState are firmware
        // FFI calls returning scalar values.
        let (usb_connected, wifi_on) = unsafe {
            let usb_state = sys::sceUsbGetState();
            let usb = usb_state.contains(sys::UsbState::CONNECTED);
            let wifi = sys::sceWlanGetSwitchState() > 0;
            (usb, wifi)
        };

        Self {
            battery_percent,
            battery_charging: bat.is_charging,
            ac_power,
            hour,
            minute,
            day_of_week: dow,
            month,
            day,
            year,
            usb_connected,
            wifi_on,
        }
    }
}
