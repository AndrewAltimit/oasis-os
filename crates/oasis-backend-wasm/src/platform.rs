//! Platform services for the WASM backend.

use oasis_platform::{
    BatteryState, CpuClock, NetworkService, OskResult, OskService, Platform, PowerInfo,
    PowerService, SystemTime, TimeService, UsbService, UsbState, WifiInfo,
};
use oasis_types::error::Result;

/// WASM platform implementation using browser APIs.
pub struct WasmPlatform {
    start_ms: f64,
    osk_buffer: Option<String>,
    osk_title: Option<String>,
}

impl WasmPlatform {
    pub fn new() -> Self {
        let start_ms = js_sys::Date::now();
        Self {
            start_ms,
            osk_buffer: None,
            osk_title: None,
        }
    }
}

impl Default for WasmPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerService for WasmPlatform {
    fn power_info(&self) -> Result<PowerInfo> {
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

impl TimeService for WasmPlatform {
    fn now(&self) -> Result<SystemTime> {
        let date = js_sys::Date::new_0();
        Ok(SystemTime {
            year: date.get_full_year() as u16,
            month: (date.get_month() + 1) as u8, // JS months are 0-indexed
            day: date.get_date() as u8,
            hour: date.get_hours() as u8,
            minute: date.get_minutes() as u8,
            second: date.get_seconds() as u8,
        })
    }

    fn uptime_secs(&self) -> Result<u64> {
        let elapsed_ms = js_sys::Date::now() - self.start_ms;
        Ok((elapsed_ms / 1000.0) as u64)
    }
}

impl UsbService for WasmPlatform {
    fn usb_state(&self) -> Result<UsbState> {
        Ok(UsbState::Unsupported)
    }

    fn activate(&mut self) -> Result<()> {
        Ok(())
    }

    fn deactivate(&mut self) -> Result<()> {
        Ok(())
    }
}

impl OskService for WasmPlatform {
    fn open(&mut self, title: &str, initial: &str) -> Result<()> {
        self.osk_title = Some(title.to_string());
        self.osk_buffer = Some(initial.to_string());
        Ok(())
    }

    fn poll(&mut self) -> Result<OskResult> {
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

impl NetworkService for WasmPlatform {
    fn wifi_info(&self) -> Result<WifiInfo> {
        // Browser always has network access.
        Ok(WifiInfo {
            available: true,
            connected: true,
            ip_address: None,
            mac_address: [0; 6],
        })
    }
}

impl Platform for WasmPlatform {}
