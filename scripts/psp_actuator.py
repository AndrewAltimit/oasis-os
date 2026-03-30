"""
PSP Power Slider Actuator Controller
Controls a JQDML 12V linear actuator via a LCUS 2-channel USB relay
wired as an H-bridge for bidirectional control.
"""

import serial
import serial.tools.list_ports
import time
import sys

# --- CONFIGURATION ---
HOLD_SECONDS = 30       # How long to hold the PSP power slider
EXTEND_SECONDS = 2      # Time for actuator to fully extend
RETRACT_SECONDS = 2     # Time for actuator to fully retract
BAUD_RATE = 9600

# --- LCUS RELAY PROTOCOL ---
def relay_command(relay_num, state):
    cmd = bytes([0xA0, relay_num, state, (0xA0 + relay_num + state) & 0xFF])
    return cmd

RELAY1_ON  = relay_command(0x01, 0x01)
RELAY1_OFF = relay_command(0x01, 0x00)
RELAY2_ON  = relay_command(0x02, 0x01)
RELAY2_OFF = relay_command(0x02, 0x00)

# --- PORT DETECTION ---
def find_relay_port():
    ports = serial.tools.list_ports.comports()
    candidates = []
    for port in ports:
        desc = (port.description or "").lower()
        mfg = (port.manufacturer or "").lower()
        if any(kw in desc + mfg for kw in ["ch340", "ch341", "usb-serial", "usb serial"]):
            candidates.append(port.device)
    if len(candidates) == 1:
        return candidates[0]
    elif len(candidates) > 1:
        print("Multiple USB-serial devices found:")
        for i, p in enumerate(candidates):
            print(f"  [{i}] {p}")
        return candidates[int(input("Pick one (number): ").strip())]
    else:
        print("No CH340 device found. Available ports:")
        for p in ports:
            print(f"  {p.device} - {p.description}")
        return input("Enter port manually (e.g. COM3 or /dev/ttyUSB0): ").strip()

# --- ACTUATOR CONTROL ---
class ActuatorController:
    def __init__(self, port=None):
        self.port_name = port or find_relay_port()
        self.ser = serial.Serial(self.port_name, BAUD_RATE, timeout=1)
        time.sleep(0.5)
        self.stop()
        print(f"Connected to relay on {self.port_name}")

    def extend(self):
        self.ser.write(RELAY1_OFF)
        time.sleep(0.05)
        self.ser.write(RELAY2_OFF)
        print("Extending (pushing slider)...")

    def extend_step(self):
        step = EXTEND_SECONDS / 4
        self.ser.write(RELAY1_OFF)
        time.sleep(0.05)
        self.ser.write(RELAY2_OFF)
        time.sleep(step)
        self.ser.write(RELAY1_ON)
        time.sleep(0.05)
        self.ser.write(RELAY2_OFF)
        print(f"Extended 1/4 ({step:.2f}s pulse)")

    def retract(self):
        self.ser.write(RELAY1_ON)
        time.sleep(0.05)
        self.ser.write(RELAY2_ON)
        print("Retracting (releasing slider)...")

    def retract_step(self):
        step = RETRACT_SECONDS / 4
        self.ser.write(RELAY1_ON)
        time.sleep(0.05)
        self.ser.write(RELAY2_ON)
        time.sleep(step)
        self.ser.write(RELAY1_ON)
        time.sleep(0.05)
        self.ser.write(RELAY2_OFF)
        print(f"Retracted 1/4 ({step:.2f}s pulse)")

    def stop(self):
        self.ser.write(RELAY1_ON)
        time.sleep(0.05)
        self.ser.write(RELAY2_OFF)
        print("Stopped.")

    def power_on_psp(self):
        print(f"\n--- PSP POWER ON SEQUENCE ---")
        self.extend()
        time.sleep(EXTEND_SECONDS)
        print(f"Holding for {HOLD_SECONDS} seconds...")
        time.sleep(HOLD_SECONDS)
        self.retract()
        time.sleep(RETRACT_SECONDS)
        self.stop()
        print("Done! PSP should be on.\n")

    def close(self):
        self.stop()
        self.ser.close()

def interactive(ctrl):
    print("\nCommands: [e] extend 1/4  [E] extend full  [r] retract 1/4  [R] retract full  [s]top  [on] full sequence  [q]uit")
    while True:
        try:
            cmd = input("> ").strip()
        except (KeyboardInterrupt, EOFError):
            break
        cmd_lower = cmd.lower()
        if cmd == "e":
            ctrl.extend_step()
        elif cmd in ("E", "extend"):
            ctrl.extend()
        elif cmd == "r":
            ctrl.retract_step()
        elif cmd in ("R", "retract"):
            ctrl.retract()
        elif cmd_lower in ("s", "stop"):
            ctrl.stop()
        elif cmd_lower == "on":
            ctrl.power_on_psp()
        elif cmd_lower in ("q", "quit", "exit"):
            break
        else:
            print("Unknown command. Use: e / E / r / R / s / on / q")

def main():
    ctrl = ActuatorController()
    try:
        if len(sys.argv) > 1:
            cmd = sys.argv[1].lower()
            if cmd == "on":
                ctrl.power_on_psp()
            elif cmd == "extend":
                ctrl.extend()
                input("Press Enter to stop...")
                ctrl.stop()
            elif cmd == "retract":
                ctrl.retract()
                input("Press Enter to stop...")
                ctrl.stop()
            elif cmd == "stop":
                ctrl.stop()
        else:
            interactive(ctrl)
    finally:
        ctrl.close()

if __name__ == "__main__":
    main()
