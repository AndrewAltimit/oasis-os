//! PSP Kernel Info + RAM Dump — works on 6.61 ARK-4

#![no_std]
#![no_main]

use psp::hw::hw_read32;
use psp::sys::{
    self, CtrlButtons, SceCtrlData, IoOpenFlags,
    sceCtrlPeekBufferPositive,
    sceKernelDelayThread, sceUsbGetState, sceUsbStart,
};
use core::ffi::c_void;

psp::module_kernel!("KernelDemo", 1, 0);

fn wait_cross() {
    let mut p = SceCtrlData::default();
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if !p.buttons.intersects(CtrlButtons::CROSS) { break; }
        unsafe { sceKernelDelayThread(16_000) };
    }
    loop {
        unsafe { sceCtrlPeekBufferPositive(&mut p, 1) };
        if p.buttons.intersects(CtrlButtons::CROSS) { return; }
        unsafe { sceKernelDelayThread(16_000) };
    }
}

fn psp_main() {
    psp::callback::setup_exit_callback().unwrap();

    psp::dprintln!("=== PSP Kernel Dump ===");

    // HW info
    let me = unsafe { psp::sys::scePowerGetMeClockFrequency() };
    let gpio = unsafe { hw_read32(psp::hw::GPIO_PORT_READ) };
    let r40 = unsafe { hw_read32(0xBC10_0040) };
    let usb = unsafe { sceUsbGetState() }.bits();

    psp::dprintln!("ME={}MHz GPIO={:08X}", me, gpio);
    psp::dprintln!("SYSCTL+40={:08X} USB={:03X}", r40, usb);

    // Capability flag
    let cap = unsafe { hw_read32(0x88014410) };
    psp::dprintln!("cap@88014410={:08X} bit10={}", cap, (cap >> 10) & 1);
    psp::dprintln!("");

    // Start USB bus and re-check
    psp::dprintln!("CROSS = start bus + recheck");
    wait_cross();

    let _ = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbPspCm) };
    let _ = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbAcc) };
    let _ = unsafe { sys::sceUtilityLoadUsbModule(sys::UsbModule::UsbCam) };
    let r = unsafe { sceUsbStart(b"USBBusDriver\0".as_ptr(), 0, core::ptr::null_mut::<c_void>()) };
    psp::dprintln!("StartBus={:08X}", r as u32);
    let _ = unsafe { sceUsbStart(b"USBCamDriver\0".as_ptr(), 0, core::ptr::null_mut::<c_void>()) };
    unsafe { sceKernelDelayThread(500_000) };

    let cap2 = unsafe { hw_read32(0x88014410) };
    let r40b = unsafe { hw_read32(0xBC10_0040) };
    psp::dprintln!("AFTER BUS:");
    psp::dprintln!("cap={:08X} bit10={}", cap2, (cap2 >> 10) & 1);
    psp::dprintln!("+40={:08X} USB={:03X}", r40b, unsafe{sceUsbGetState()}.bits());
    psp::dprintln!("");

    // Dump 4MB kernel RAM
    psp::dprintln!("CROSS = dump 4MB to MS");
    psp::dprintln!("(takes ~10 seconds)");
    wait_cross();

    psp::dprintln!("Dumping...");

    let base: u32 = 0x8800_0000;
    let paths: [&[u8]; 4] = [
        b"ms0:/PSP/GAME/USBTRACE/km_00.bin\0",
        b"ms0:/PSP/GAME/USBTRACE/km_01.bin\0",
        b"ms0:/PSP/GAME/USBTRACE/km_02.bin\0",
        b"ms0:/PSP/GAME/USBTRACE/km_03.bin\0",
    ];

    for mb in 0u32..4 {
        let addr = base + mb * 0x100000;
        let fd = unsafe { psp::sys::sceIoOpen(
            paths[mb as usize].as_ptr(),
            IoOpenFlags::WR_ONLY | IoOpenFlags::CREAT | IoOpenFlags::TRUNC,
            0o777,
        )};
        if fd.0 < 0 {
            psp::dprintln!("km_0{}: OPEN FAIL {:08X}", mb, fd.0 as u32);
            continue;
        }
        let ptr = addr as *const u8;
        let mut off = 0u32;
        let mut ok = true;
        while off < 0x100000 {
            let w = unsafe {
                psp::sys::sceIoWrite(fd, ptr.add(off as usize) as *const c_void, 4096)
            };
            if (w as i32) <= 0 { ok = false; break; }
            off += 4096;
        }
        unsafe { psp::sys::sceIoClose(fd) };
        psp::dprintln!("km_0{}: {}", mb, if ok {"OK"} else {"FAIL"});
    }

    psp::dprintln!("");
    psp::dprintln!("Done! Transfer km_*.bin");
    psp::dprintln!("to PC for analysis.");
    unsafe { sceKernelDelayThread(10_000_000) };
}
