//! PSP USB device connection — find, claim, and communicate.

use crate::protocol::{self, MsgHeader};
use rusb::{Context, DeviceHandle, UsbContext};
use std::time::Duration;

/// Default USB timeout for transfers
const TIMEOUT: Duration = Duration::from_secs(2);

/// Manages the USB connection to the PSP thin client.
pub struct PspDevice {
    handle: DeviceHandle<Context>,
    ep_in: u8,
    ep_out: u8,
}

impl PspDevice {
    /// Find and open the PSP thin client device.
    pub fn open() -> Result<Self, String> {
        let ctx = Context::new().map_err(|e| format!("USB context: {e}"))?;

        let device = ctx
            .devices()
            .map_err(|e| format!("USB devices: {e}"))?
            .iter()
            .find(|d| {
                d.device_descriptor().is_ok_and(|desc| {
                    desc.vendor_id() == protocol::VENDOR_ID
                        && desc.product_id() == protocol::PRODUCT_ID
                })
            })
            .ok_or_else(|| {
                format!(
                    "PSP device {:04x}:{:04x} not found",
                    protocol::VENDOR_ID,
                    protocol::PRODUCT_ID
                )
            })?;

        let desc = device.device_descriptor().map_err(|e| format!("{e}"))?;
        let handle = device.open().map_err(|e| format!("USB open: {e}"))?;

        // Detach kernel driver if active
        if handle.kernel_driver_active(0).unwrap_or(false) {
            handle
                .detach_kernel_driver(0)
                .map_err(|e| format!("Detach kernel driver: {e}"))?;
        }

        handle
            .claim_interface(0)
            .map_err(|e| format!("Claim interface: {e}"))?;

        println!(
            "Connected: bus {} dev {} (USB {}.{})",
            device.bus_number(),
            device.address(),
            desc.usb_version().major(),
            desc.usb_version().minor(),
        );

        Ok(Self {
            handle,
            ep_in: protocol::EP_IN,
            ep_out: protocol::EP_OUT,
        })
    }

    /// Clear halt condition on both endpoints.
    pub fn clear_halt(&self) {
        let _ = self.handle.clear_halt(self.ep_in);
        let _ = self.handle.clear_halt(self.ep_out);
    }

    /// Read raw bytes from the PSP (bulk IN).
    pub fn read(&self, buf: &mut [u8], timeout: Duration) -> Result<usize, rusb::Error> {
        self.handle.read_bulk(self.ep_in, buf, timeout)
    }

    /// Write raw bytes to the PSP (bulk OUT).
    pub fn write(&self, data: &[u8], timeout: Duration) -> Result<usize, rusb::Error> {
        self.handle.write_bulk(self.ep_out, data, timeout)
    }

    /// Read the "PSP READY" message sent on connect.
    pub fn read_ready(&self) -> Result<String, String> {
        let mut buf = [0u8; 512];
        let n = self
            .read(&mut buf, Duration::from_secs(10))
            .map_err(|e| format!("Read ready: {e}"))?;
        let text = String::from_utf8_lossy(&buf[..n])
            .trim_end_matches('\0')
            .to_string();
        Ok(text)
    }

    /// Send an echo request and verify the response matches.
    pub fn echo(&self, data: &[u8]) -> Result<bool, String> {
        // Avoid sending exact multiples of 512 (ZLP issue)
        let len = if data.len().is_multiple_of(512) && !data.is_empty() {
            data.len() - 1
        } else {
            data.len()
        };
        self.write(&data[..len], TIMEOUT)
            .map_err(|e| format!("Echo write: {e}"))?;

        let mut buf = vec![0u8; len + 64];
        let n = self
            .read(&mut buf, TIMEOUT)
            .map_err(|e| format!("Echo read: {e}"))?;

        Ok(n == len && buf[..len] == data[..len])
    }

    /// Send a framed message (header + payload) to the PSP.
    pub fn send_msg(&self, msg_type: u8, payload: &[u8]) -> Result<(), String> {
        let header = MsgHeader::new(msg_type, payload.len() as u16);
        let mut packet = Vec::with_capacity(MsgHeader::SIZE + payload.len());
        packet.extend_from_slice(&header.to_bytes());
        packet.extend_from_slice(payload);

        // Avoid exact multiples of 512
        if packet.len() % 512 == 0 {
            packet.push(0);
        }

        self.write(&packet, TIMEOUT)
            .map_err(|e| format!("Send msg: {e}"))?;
        Ok(())
    }

    /// Receive a framed message from the PSP.
    pub fn recv_msg(&self) -> Result<(u8, Vec<u8>), String> {
        let mut buf = [0u8; 16384];
        let n = self
            .read(&mut buf, TIMEOUT)
            .map_err(|e| format!("Recv msg: {e}"))?;

        if n < MsgHeader::SIZE {
            return Err(format!("Short message: {n} bytes"));
        }

        let header = MsgHeader::from_bytes(&buf).ok_or("Bad header")?;
        let payload_len = header.payload_len as usize;
        let total = MsgHeader::SIZE + payload_len;

        if n < total {
            return Err(format!("Truncated: got {n}, expected {total}"));
        }

        Ok((header.msg_type, buf[MsgHeader::SIZE..total].to_vec()))
    }
}
