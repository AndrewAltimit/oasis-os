"""
PSP remote demo: launch TV Guide, window it, then power-cycle via actuator.

Usage:
    python3 scripts/psp_demo.py
"""

import socket
import serial
import serial.tools.list_ports
import time

# --- CONFIG ---
PSP_IP = "192.168.0.249"
PSP_PORT = 9293
RELAY_BAUD = 9600

# Actuator timing
EXTEND_QUARTER = 0.5        # seconds per 1/4 stroke
LONG_HOLD = 7               # seconds to hold power off
SHORT_HOLD = 1              # seconds for power on tap
BETWEEN_PHASES = 3          # seconds between power off and power on

# TV Guide icon center (col=2, row=2 in 4x3 grid)
TV_ICON_X = 298
TV_ICON_Y = 199

# --- RELAY ---
R1_ON  = bytes([0xA0, 0x01, 0x01, 0xA2])
R1_OFF = bytes([0xA0, 0x01, 0x00, 0xA1])
R2_ON  = bytes([0xA0, 0x02, 0x01, 0xA3])
R2_OFF = bytes([0xA0, 0x02, 0x00, 0xA2])


def find_relay():
    for port in serial.tools.list_ports.comports():
        desc = (port.description or "").lower()
        mfg = (port.manufacturer or "").lower()
        if any(kw in desc + mfg for kw in ["ch340", "ch341", "usb-serial", "usb serial"]):
            return port.device
    return "/dev/ttyUSB0"


def psp(cmd):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(5)
    s.connect((PSP_IP, PSP_PORT))
    s.sendall((cmd + "\n").encode())
    time.sleep(0.1)
    try:
        resp = s.recv(4096).decode().strip()
    except socket.timeout:
        resp = ""
    s.close()
    print(f"  [{cmd}] {resp}")
    return resp


def main():
    # --- Phase 1: Remote control PSP ---
    print(">> Opening TV Guide")
    psp(f"cursor {TV_ICON_X} {TV_ICON_Y}")
    time.sleep(0.3)
    psp("press cross")
    time.sleep(2)

    print(">> Tuning channel")
    psp("press cross")
    time.sleep(1)

    print(">> Windowing (press start to exit fullscreen)")
    psp("press start")
    time.sleep(1)

    # --- Phase 2: Actuator power cycle ---
    print(">> Connecting to relay")
    ser = serial.Serial(find_relay(), RELAY_BAUD, timeout=1)
    time.sleep(0.3)

    def stop():
        ser.write(R1_ON); time.sleep(0.03); ser.write(R2_OFF)

    def extend_34():
        ser.write(R1_OFF); time.sleep(0.03); ser.write(R2_OFF)
        time.sleep(EXTEND_QUARTER * 3)
        stop()

    def retract():
        ser.write(R1_ON); time.sleep(0.03); ser.write(R2_ON)
        time.sleep(EXTEND_QUARTER * 4)
        stop()

    stop()

    print(f">> Power OFF (hold {LONG_HOLD}s)")
    extend_34()
    time.sleep(LONG_HOLD)
    retract()
    time.sleep(BETWEEN_PHASES)

    print(f">> Power ON (tap {SHORT_HOLD}s)")
    extend_34()
    time.sleep(SHORT_HOLD)
    retract()

    print(">> Done!")
    ser.close()


if __name__ == "__main__":
    main()
