/* Linker script for oasis-backend-psp when linking through psp-ld.
 *
 * Copied verbatim from the inline `link-script` field in rust-psp's
 * `mipsel-sony-psp-std.json` target spec, which rustc normally dumps
 * into a temp file (`rustc-mipsel-sony-psp-linkfile.ld`) and passes
 * to rust-lld via `--script`. Because we override the linker to
 * `psp-ld` in `.cargo/config.toml` (to side-step rust-lld's strict
 * MIPS ABI enforcement, which blocks pspdev newlib), rustc no longer
 * forwards the inline script, so we vendor it here and reference it
 * explicitly via `-C link-arg=--script=.../psp-linkscript.ld`.
 */
ENTRY(module_start)
SECTIONS
{
  /* PRX format requires text to begin at 0 */
  .text 0 : { *(.text .text.*) }

  /* Sort stubs for convenient ordering */
  .sceStub.text : { *(.sceStub.text) *(SORT(.sceStub.text.*)) }

  /* PSP import library stub sections. Bundles together
   * `.lib.stub.entry.*` sections for better `--gc-sections` support. */
  .lib.stub.top : { *(.lib.stub.top) }
  .lib.stub :     { *(.lib.stub) *(.lib.stub.entry.*) }
  .lib.stub.btm : { *(.lib.stub.btm) }

  /* Keep these sections around, even though they may appear unused to
   * the linker */
  .lib.ent.top :  { KEEP(*(.lib.ent.top)) }
  .lib.ent :      { KEEP(*(.lib.ent)) }
  .lib.ent.btm :  { KEEP(*(.lib.ent.btm)) }

  .eh_frame_hdr : { *(.eh_frame_hdr) }

  /* Add symbols for LLVM's libunwind */
  __eh_frame_hdr_start = SIZEOF(.eh_frame_hdr) > 0 ? ADDR(.eh_frame_hdr) : 0;
  __eh_frame_hdr_end = SIZEOF(.eh_frame_hdr) > 0 ? . : 0;
  .eh_frame :
  {
    __eh_frame_start = .;
    KEEP(*(.eh_frame))
    __eh_frame_end = .;
  }

  /* These are explicitly listed to avoid being merged into .rodata */
  .rodata.sceResident : { *(.rodata.sceResident) *(.rodata.sceResident.*) }
  .rodata.sceModuleInfo : { *(.rodata.sceModuleInfo) }
  /* Sort NIDs for convenient ordering */
  .rodata.sceNid : { *(.rodata.sceNid) *(SORT(.rodata.sceNid.*)) }

  .rodata : { *(.rodata .rodata.*) }
  .data : { *(.data .data.*) }
  .gcc_except_table : { *(.gcc_except_table .gcc_except_table.*) }
  .bss : { *(.bss .bss.*) }

  /* Provide `_gp` for any object files that reference it through
   * small-data addressing. The sceModuleInfo descriptor stores it. */
  _gp = ALIGN(16) + 0x7ff0;

  /DISCARD/ : { *(.rel.sceStub.text .MIPS.abiflags .reginfo) }
}
