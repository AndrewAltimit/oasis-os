#!/usr/bin/env python3
"""PC-side test script for the PSP USB echo driver.

Sends test data to the PSP via bulk OUT (EP 0x02), reads the echo back
via bulk IN (EP 0x81). Requires pyusb: pip install pyusb

Usage: sudo python3 test_echo.py
(sudo required for USB device access, or set up udev rules)

Phase 1-2: Enhanced diagnostics, retry logic, endpoint clearing.
"""

import usb.core
import usb.util
import sys
import time

VID = 0x054C  # Sony
PID = 0x1337  # Our custom device

def find_device():
    """Find the PSP USB device with retries."""
    print(f"Looking for device {VID:04x}:{PID:04x}...")
    for attempt in range(3):
        dev = usb.core.find(idVendor=VID, idProduct=PID)
        if dev is not None:
            return dev
        if attempt < 2:
            print(f"  Not found, retrying in 2s... ({attempt+1}/3)")
            time.sleep(2)
    return None


def print_device_info(dev):
    """Print device descriptor details."""
    try:
        mfr = dev.manufacturer or "(none)"
    except (usb.core.USBError, ValueError):
        mfr = "(unreadable)"
    try:
        prod = dev.product or "(none)"
    except (usb.core.USBError, ValueError):
        prod = "(unreadable)"

    print(f"Found: {mfr} - {prod}")
    print(f"  Bus {dev.bus}, Device {dev.address}")
    print(f"  USB {dev.bcdUSB >> 8}.{dev.bcdUSB & 0xFF}")
    print(f"  VID:PID = {dev.idVendor:04x}:{dev.idProduct:04x}")
    print(f"  bMaxPacketSize0 = {dev.bMaxPacketSize0}")
    print(f"  Speed: ", end="")

    # pyusb 1.x speed detection
    try:
        speed = dev.speed
        speeds = {None: "unknown", 1: "Low (1.5Mbps)", 2: "Full (12Mbps)",
                  3: "High (480Mbps)", 4: "Super (5Gbps)"}
        print(speeds.get(speed, f"unknown ({speed})"))
    except Exception:
        print("(speed not available)")


def clear_endpoint(dev, ep_addr):
    """Clear halt/stall on an endpoint."""
    try:
        dev.clear_halt(ep_addr)
        print(f"  Cleared halt on EP 0x{ep_addr:02x}")
    except usb.core.USBError as e:
        print(f"  Clear halt EP 0x{ep_addr:02x}: {e}")


def test_read_first(ep_in):
    """Test 0: Try reading first — PSP may have proactively sent data."""
    print("=== Test 0: Try read from PSP (proactive send) ===")
    try:
        response = ep_in.read(512, timeout=5000)
        data = bytes(response)
        print(f"  Read {len(data)} bytes")
        # Show printable prefix
        text = data.rstrip(b'\x00')
        if text:
            try:
                print(f"  Text: {text.decode('ascii', errors='replace')}")
            except Exception:
                print(f"  Hex: {data[:32].hex()}")
        return True
    except usb.core.USBError as e:
        print(f"  Read: {e}")
        return False


def test_echo(ep_out, ep_in, test_data, label="Echo"):
    """Send data and expect it echoed back."""
    try:
        written = ep_out.write(test_data, timeout=5000)
    except usb.core.USBError as e:
        print(f"  [{label}] Send error: {e}")
        return False

    # PSP needs time to process recv callback and queue the echo send
    time.sleep(0.1)

    try:
        response = ep_in.read(len(test_data) + 64, timeout=5000)
        data = bytes(response)
        if data[:len(test_data)] == test_data:
            print(f"  [{label}] ECHO MATCH! ({len(data)} bytes)")
            return True
        else:
            print(f"  [{label}] Mismatch: sent {len(test_data)}, "
                  f"got {len(data)} bytes")
            print(f"    Sent: {test_data[:32]}")
            print(f"    Got:  {data[:32]}")
            return False
    except usb.core.USBError as e:
        print(f"  [{label}] Read error: {e}")
        return False


def main():
    dev = find_device()
    if dev is None:
        print("Device not found. Is PSP running USBCLIENT and connected?")
        sys.exit(1)

    print_device_info(dev)
    print()

    # Detach kernel driver if attached
    try:
        if dev.is_kernel_driver_active(0):
            print("Detaching kernel driver...")
            dev.detach_kernel_driver(0)
    except usb.core.USBError:
        pass

    # Get configuration (don't call set_configuration — it resets PSP state)
    cfg = dev.get_active_configuration()
    if cfg is None:
        print("No active config, setting...")
        dev.set_configuration()
        cfg = dev.get_active_configuration()
    intf = cfg[(0, 0)]
    print(f"Interface: class=0x{intf.bInterfaceClass:02x} "
          f"subclass=0x{intf.bInterfaceSubClass:02x} "
          f"protocol=0x{intf.bInterfaceProtocol:02x}")

    # Claim the interface
    usb.util.claim_interface(dev, 0)

    # Find endpoints
    ep_out = usb.util.find_descriptor(
        intf, custom_match=lambda e:
        usb.util.endpoint_direction(e.bEndpointAddress) == usb.util.ENDPOINT_OUT
    )
    ep_in = usb.util.find_descriptor(
        intf, custom_match=lambda e:
        usb.util.endpoint_direction(e.bEndpointAddress) == usb.util.ENDPOINT_IN
    )

    if ep_out is None or ep_in is None:
        print("Could not find bulk endpoints!")
        sys.exit(1)

    print(f"EP OUT: 0x{ep_out.bEndpointAddress:02x} "
          f"(max {ep_out.wMaxPacketSize})")
    print(f"EP IN:  0x{ep_in.bEndpointAddress:02x} "
          f"(max {ep_in.wMaxPacketSize})")
    print()

    # Clear any stale halt conditions
    clear_endpoint(dev, ep_out.bEndpointAddress)
    clear_endpoint(dev, ep_in.bEndpointAddress)
    print()

    # Wait for PSP to set up recv (it does this ~3s after attach)
    print("Waiting 5s for PSP to initialize transfers...")
    time.sleep(5)

    # Test 0: Try reading proactive send from PSP
    got_proactive = test_read_first(ep_in)
    print()

    # Test 1: Simple echo
    print("=== Test 1: Simple Echo ===")
    test_data = b"Hello from PC! PSP USB works!"
    echo_ok = test_echo(ep_out, ep_in, test_data, "simple")
    print()

    if not echo_ok:
        # Try clearing endpoints and retrying
        print("Echo failed. Clearing endpoints and retrying...")
        clear_endpoint(dev, ep_out.bEndpointAddress)
        clear_endpoint(dev, ep_in.bEndpointAddress)
        time.sleep(1)
        echo_ok = test_echo(ep_out, ep_in, test_data, "retry")
        print()

    if not echo_ok:
        # Try raw writes to OUT endpoint only
        print("=== Fallback: Try raw bulk writes ===")
        for ep_addr in [0x02]:
            try:
                written = dev.write(ep_addr, test_data, timeout=2000)
                print(f"  EP 0x{ep_addr:02x}: sent {written} bytes!")
            except usb.core.USBError as e3:
                print(f"  EP 0x{ep_addr:02x}: {e3}")

        # Try raw reads from IN endpoint only
        for ep_addr in [0x81]:
            try:
                resp = dev.read(ep_addr, 512, timeout=2000)
                print(f"  EP 0x{ep_addr:02x}: read {len(resp)} bytes: "
                      f"{bytes(resp)[:20]}")
            except usb.core.USBError as e3:
                print(f"  EP 0x{ep_addr:02x} read: {e3}")
        print()
        print("Transfers not working yet. Check PSP usb.log for diagnostics.")
        sys.exit(1)

    # Test 2: Multiple packets
    print("=== Test 2: Multiple Packets ===")
    success = 0
    for i in range(5):
        msg = f"Packet {i}: {'X' * (i * 10 + 5)}".encode()
        if test_echo(ep_out, ep_in, msg, f"pkt{i}"):
            success += 1
    print(f"  {success}/5 packets echoed successfully")
    print()

    # Test 3: Throughput (sequential round-trips)
    # Use 500 bytes, NOT 512 — USB bulk needs ZLP for exact wMaxPacketSize
    # transfers, and the PSP bus driver may not send one automatically.
    print("=== Test 3: Throughput ===")
    payload = bytes(range(250)) * 2  # 500 bytes
    count = 0
    errors = 0
    start = time.time()
    duration = 5.0

    while time.time() - start < duration:
        try:
            ep_out.write(payload, timeout=2000)
            response = ep_in.read(len(payload) + 64, timeout=2000)
            if bytes(response)[:len(payload)] != payload:
                errors += 1
            count += 1
        except usb.core.USBError as e:
            errors += 1
            # Clear halt and retry
            try:
                clear_endpoint(dev, ep_out.bEndpointAddress)
                clear_endpoint(dev, ep_in.bEndpointAddress)
                time.sleep(0.05)
            except Exception:
                pass
            if errors > 3:
                print(f"  Too many errors, stopping")
                break

    elapsed = time.time() - start
    if count > 0:
        total_bytes = count * len(payload) * 2  # send + receive
        throughput = total_bytes / elapsed / 1024
        print(f"  {count} round-trips in {elapsed:.1f}s")
        print(f"  {throughput:.0f} KB/s ({errors} errors)")
    else:
        print(f"  No successful round-trips ({errors} errors)")

    print()
    print("Done!")

if __name__ == "__main__":
    main()
