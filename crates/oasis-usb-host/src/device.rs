//! PSP USB device connection — find, claim, and communicate.

use crate::protocol::{self, InputState, MsgHeader};
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
        // Avoid sending exact multiples of 512 — USB bulk transfers
        // require a Zero Length Packet (ZLP) to signal completion when
        // the payload is an exact multiple of max packet size. Append a
        // padding byte instead of truncating the payload.
        let mut send_buf;
        let send_data = if data.len().is_multiple_of(512) && !data.is_empty() {
            send_buf = vec![0u8; data.len() + 1];
            send_buf[..data.len()].copy_from_slice(data);
            &send_buf[..]
        } else {
            data
        };
        let send_len = send_data.len();
        self.write(send_data, TIMEOUT)
            .map_err(|e| format!("Echo write: {e}"))?;

        let mut buf = vec![0u8; send_len + 64];
        let n = self
            .read(&mut buf, TIMEOUT)
            .map_err(|e| format!("Echo read: {e}"))?;

        // The device echoes back exactly what it received (including padding)
        Ok(n == send_len && buf[..data.len()] == data[..])
    }

    /// Send a framed message (header + payload) to the PSP.
    pub fn send_msg(&self, msg_type: u8, payload: &[u8]) -> Result<(), String> {
        let header = MsgHeader::new(msg_type, payload.len() as u16);
        let mut packet = Vec::with_capacity(MsgHeader::SIZE + payload.len() + 1);
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

    // -----------------------------------------------------------------------
    // Thin-client: frame streaming + input
    // -----------------------------------------------------------------------

    /// Receive an InputState response (12 bytes: 4 header + 8 payload).
    fn recv_input_state(&self) -> Result<InputState, String> {
        let (msg_type, payload) = self.recv_msg()?;
        if msg_type != protocol::rsp::INPUT_STATE {
            return Err(format!("Expected INPUT_STATE (0x21), got 0x{msg_type:02x}"));
        }
        InputState::from_bytes(&payload).ok_or_else(|| "Bad InputState payload".to_string())
    }

    /// Send a single frame chunk and read back the InputState response.
    ///
    /// `chunk_index` is 0..17. `pixels` is raw RGB565 data (max 16376 bytes).
    pub fn send_frame_chunk(&self, chunk_index: u8, pixels: &[u8]) -> Result<InputState, String> {
        let header = MsgHeader {
            msg_type: protocol::cmd::FRAME_CHUNK,
            flags: chunk_index,
            payload_len: pixels.len() as u16,
        };
        let mut packet = Vec::with_capacity(MsgHeader::SIZE + pixels.len() + 1);
        packet.extend_from_slice(&header.to_bytes());
        packet.extend_from_slice(pixels);

        // Avoid ZLP
        if packet.len() % 512 == 0 {
            packet.push(0);
        }

        self.write(&packet, TIMEOUT)
            .map_err(|e| format!("Send chunk {chunk_index}: {e}"))?;

        self.recv_input_state()
    }

    /// Send FRAME_DONE to trigger buffer swap, read back InputState.
    pub fn send_frame_done(&self, frame_seq: u8) -> Result<InputState, String> {
        let header = MsgHeader {
            msg_type: protocol::cmd::FRAME_DONE,
            flags: frame_seq,
            payload_len: 0,
        };
        self.write(&header.to_bytes(), TIMEOUT)
            .map_err(|e| format!("Send FRAME_DONE: {e}"))?;

        self.recv_input_state()
    }

    /// Send a full frame (stride-padded RGB565) as 18 chunks + FRAME_DONE.
    ///
    /// `pixels` must be exactly `FRAME_SIZE_STRIDE` bytes (278,528).
    /// Returns the InputState from the final FRAME_DONE response.
    pub fn send_frame(&self, pixels: &[u8], frame_seq: u8) -> Result<InputState, String> {
        let total = pixels.len();
        let chunk_size = protocol::MAX_CHUNK_PAYLOAD;

        let mut chunk_index: u8 = 0;
        let mut offset = 0;

        while offset < total {
            let end = (offset + chunk_size).min(total);
            self.send_frame_chunk(chunk_index, &pixels[offset..end])?;
            chunk_index += 1;
            offset = end;
        }

        self.send_frame_done(frame_seq)
    }

    /// Send GET_INPUT and read back the current InputState.
    pub fn get_input(&self) -> Result<InputState, String> {
        self.send_msg(protocol::cmd::GET_INPUT, &[])?;
        self.recv_input_state()
    }
}
