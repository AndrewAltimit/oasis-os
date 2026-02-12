# PSP Screenshot Test Scenarios

Manual review guide for PSP screenshot captures. Each scenario runs OASIS_OS
in PPSSPPHeadless and captures a framebuffer screenshot for human review.

## Scenario: dashboard

**What it captures:** Initial boot state -- the OASIS_OS dashboard after startup.

**What to verify:**
- [ ] App icons are visible and correctly positioned in the grid
- [ ] Status bar is present at the top (clock, battery indicator)
- [ ] Bottom bar shows page dots / tab navigation
- [ ] Resolution is correct (480x272, no stretching)
- [ ] No VRAM corruption (garbage pixels, wrong stride, color banding)
- [ ] Wallpaper renders behind the dashboard elements
- [ ] Font rendering is legible (8x8 bitmap font)

## Scenario: terminal

**What it captures:** Terminal mode after pressing F1 from dashboard.

**What to verify:**
- [ ] Terminal background fills the content area
- [ ] Prompt line visible at the bottom (`/home/user> _`)
- [ ] Welcome text / status messages rendered correctly
- [ ] Green-on-dark color scheme is readable
- [ ] No overlap with dashboard elements (they should be hidden)

**Note:** Requires input injection to press F1. Currently captures dashboard
state (same as `dashboard` scenario until input injection patches are added).

## Scenario: browser

**What it captures:** Browser rendering a bundled HTML test page.

**What to verify:**
- [ ] URL bar / chrome renders at the top
- [ ] Page content is visible and laid out correctly
- [ ] Text is readable at PSP resolution
- [ ] Links are distinguishable (underlined / colored)
- [ ] No rendering artifacts or overflows

**Note:** Requires navigation input to open browser. Currently captures
dashboard state. Full browser testing requires either:
1. Auto-launch browser via startup script in VFS
2. Input injection patches to navigate to browser app

## Scenario: input

**What it captures:** Dashboard after D-pad navigation (cursor moved).

**What to verify:**
- [ ] Cursor / selection highlight has moved from initial position
- [ ] Selection indicator is visible on the target app icon
- [ ] No visual glitches from rapid input processing

**Note:** Requires input injection. Currently captures boot state.

## Scenario: audio

**What it captures:** Status bar showing audio playback info.

**What to verify:**
- [ ] Status bar shows track name or "now playing" indicator
- [ ] No crash from audio subsystem initialization
- [ ] Volume indicator visible (if applicable)

**Note:** Requires audio file in VFS and playback trigger. Currently captures
boot state for crash verification.

---

## Running Scenarios

```bash
# All scenarios
./scripts/psp-screenshot.sh

# Single scenario
./scripts/psp-screenshot.sh dashboard

# With HTML report
./scripts/psp-screenshot.sh --report

# Custom timeout (longer for browser page load)
./scripts/psp-screenshot.sh --timeout 15 browser

# List available scenarios
./scripts/psp-screenshot.sh --list
```

## Future Enhancements

When PPSSPP patches are added to `docker/ppsspp-patches/`:

1. **Input injection** (`001-input-injection.patch`):
   - Send D-pad/button events via command-line flags or HTTP API
   - Enable terminal, browser, and input scenarios

2. **HTTP control API** (`002-http-control-api.patch`):
   - Remote control PPSSPP from the test script
   - Wait for specific conditions before capturing
   - Send input sequences with timing

3. **Frame-based capture** (`003-frame-capture.patch`):
   - Capture at specific frame count instead of timeout
   - More deterministic screenshots
