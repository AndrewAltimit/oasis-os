#!/usr/bin/env python3
"""PC-side test script for the PSP USB echo driver.

Sends test data to the PSP via bulk OUT (EP 0x02), reads the echo back
via bulk IN (EP 0x81). Requires pyusb: pip install pyusb

Usage: sudo python3 test_echo.py
(sudo required for USB device access, or set up udev rules)
"""

import usb.core
import usb.util
import sys
import time

VID = 0x054C  # Sony
PID = 0x1337  # Our custom device

def main():
    # Find the PSP device
    dev = usb.core.find(idVendor=VID, idProduct=PID)
    if dev is None:
        print(f"Device {VID:04x}:{PID:04x} not found.")
        print("Is the PSP running USBCLIENT and connected?")
        sys.exit(1)

    try:
        mfr = dev.manufacturer or 'Sony'
    except (usb.core.USBError, ValueError):
        mfr = 'Sony'
    try:
        prod = dev.product or 'PSP'
    except (usb.core.USBError, ValueError):
        prod = 'PSP'
    print(f"Found: {mfr} - {prod}")
    print(f"  Bus {dev.bus}, Device {dev.address}")

    # Detach kernel driver if attached
    try:
        if dev.is_kernel_driver_active(0):
            print("Detaching kernel driver...")
            dev.detach_kernel_driver(0)
    except usb.core.USBError:
        pass

    # Don't call set_configuration() — it resets endpoints and cancels PSP's pending recv.
    # The OS auto-configures the device when first plugged in.
    cfg = dev.get_active_configuration()
    if cfg is None:
        dev.set_configuration()
        cfg = dev.get_active_configuration()
    intf = cfg[(0, 0)]
    print(f"  Interface: class={intf.bInterfaceClass:#x}")

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

    print(f"  EP OUT: {ep_out.bEndpointAddress:#x} (max {ep_out.wMaxPacketSize})")
    print(f"  EP IN:  {ep_in.bEndpointAddress:#x} (max {ep_in.wMaxPacketSize})")
    print()

    # Give PSP time to set up recv after set_configuration
    print("Waiting 3s for PSP to initialize transfers...")
    time.sleep(3)

    # Test 0: Try reading first (PSP may be sending data)
    print("=== Test 0: Try read from PSP ===")
    try:
        response = ep_in.read(512, timeout=3000)
        print(f"  Read {len(response)} bytes: {bytes(response)[:40]}")
    except usb.core.USBError as e:
        print(f"  Read: {e}")

    # Test 1: Simple echo
    print()
    print("=== Test 1: Echo (write then read) ===")
    test_data = b"Hello from PC! PSP USB works!"
    print(f"Sending {len(test_data)} bytes to EP OUT...")

    try:
        written = ep_out.write(test_data, timeout=5000)
        print(f"  Sent {written} bytes OK")
    except usb.core.USBError as e:
        print(f"  Send error: {e}")
        print()
        print("=== Test 1b: Try write to EP IN instead ===")
        try:
            written = ep_in.write(test_data, timeout=5000)
            print(f"  Sent {written} bytes on EP IN")
        except usb.core.USBError as e2:
            print(f"  Also failed: {e2}")
        print()
        print("=== Test 1c: Try raw bulk write to endpoints 1,2,3 ===")
        for ep_addr in [0x01, 0x02, 0x03, 0x81, 0x82, 0x83]:
            try:
                written = dev.write(ep_addr, test_data, timeout=2000)
                print(f"  EP 0x{ep_addr:02x}: sent {written} bytes!")
            except usb.core.USBError as e3:
                print(f"  EP 0x{ep_addr:02x}: {e3}")

    try:
        response = ep_in.read(512, timeout=5000)
        print(f"  Received {len(response)} bytes: {bytes(response)}")
        if bytes(response) == test_data:
            print("  ECHO MATCH!")
        else:
            print("  Echo mismatch")
    except usb.core.USBError as e:
        print(f"  Read error: {e}")

    # Test 2: Multiple packets
    print()
    print("=== Test 2: Multiple packets ===")
    for i in range(5):
        msg = f"Packet {i}: {'X' * (i * 10 + 5)}".encode()
        try:
            ep_out.write(msg, timeout=5000)
            response = ep_in.read(512, timeout=5000)
            match = "OK" if bytes(response) == msg else "MISMATCH"
            print(f"  [{i}] Sent {len(msg)}b, got {len(response)}b: {match}")
        except usb.core.USBError as e:
            print(f"  [{i}] Error: {e}")
            break

    # Test 3: Throughput
    print()
    print("=== Test 3: Throughput ===")
    payload = bytes(range(256)) * 2  # 512 bytes
    count = 0
    errors = 0
    start = time.time()
    duration = 2.0  # seconds

    while time.time() - start < duration:
        try:
            ep_out.write(payload, timeout=1000)
            response = ep_in.read(512, timeout=1000)
            if bytes(response) != payload:
                errors += 1
            count += 1
        except usb.core.USBError:
            errors += 1
            break

    elapsed = time.time() - start
    total_bytes = count * len(payload) * 2  # send + receive
    throughput = total_bytes / elapsed / 1024
    print(f"  {count} round-trips in {elapsed:.1f}s")
    print(f"  {throughput:.0f} KB/s ({errors} errors)")

    print()
    print("Done!")

if __name__ == "__main__":
    main()
