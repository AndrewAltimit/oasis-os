# OASIS_OS Screenshots

All screenshots are captured via `cargo run -p oasis-app --bin oasis-screenshot <skin>`.

## Dashboards

| Classic | XP | Modern | Desktop | macOS |
|:---:|:---:|:---:|:---:|:---:|
| ![Classic](screenshots/classic/01_dashboard.png) | ![XP](screenshots/xp/01_dashboard.png) | ![Modern](screenshots/modern/01_dashboard.png) | ![Desktop](screenshots/desktop/01_dashboard.png) | ![macOS](screenshots/macos/01_dashboard.png) |

| GNOME | Balatro | Win95 | Solarized | Vaporwave |
|:---:|:---:|:---:|:---:|:---:|
| ![GNOME](screenshots/gnome/01_dashboard.png) | ![Balatro](screenshots/balatro/01_dashboard.png) | ![Win95](screenshots/win95/01_dashboard.png) | ![Solarized](screenshots/solarized/01_dashboard.png) | ![Vaporwave](screenshots/vaporwave/01_dashboard.png) |

| Retro CGA | Paper | High Contrast | Altimit |
|:---:|:---:|:---:|:---:|
| ![CGA](screenshots/retro-cga/01_dashboard.png) | ![Paper](screenshots/paper/01_dashboard.png) | ![HiCon](screenshots/highcontrast/01_dashboard.png) | ![Altimit](screenshots/altimit/01_dashboard.png) |

## Terminals

| Classic | XP | Modern | Desktop | macOS |
|:---:|:---:|:---:|:---:|:---:|
| ![Classic](screenshots/classic/04_terminal.png) | ![XP](screenshots/xp/04_terminal.png) | ![Modern](screenshots/modern/04_terminal.png) | ![Desktop](screenshots/desktop/04_terminal.png) | ![macOS](screenshots/macos/04_terminal.png) |

| Corrupted | Balatro | Win95 | Solarized | Vaporwave |
|:---:|:---:|:---:|:---:|:---:|
| ![Corrupted](screenshots/corrupted/04_terminal.png) | ![Balatro](screenshots/balatro/04_terminal.png) | ![Win95](screenshots/win95/04_terminal.png) | ![Solarized](screenshots/solarized/04_terminal.png) | ![Vaporwave](screenshots/vaporwave/04_terminal.png) |

| High Contrast | GNOME | Retro CGA | Paper | Altimit |
|:---:|:---:|:---:|:---:|:---:|
| ![HiCon](screenshots/highcontrast/04_terminal.png) | ![GNOME](screenshots/gnome/04_terminal.png) | ![CGA](screenshots/retro-cga/04_terminal.png) | ![Paper](screenshots/paper/04_terminal.png) | ![Altimit](screenshots/altimit/04_terminal.png) |

## Media Tabs

| Classic | XP | Modern |
|:---:|:---:|:---:|
| ![Classic](screenshots/classic/02_media_tab.png) | ![XP](screenshots/xp/02_media_tab.png) | ![Modern](screenshots/modern/02_media_tab.png) |

## Mods Tabs

| Classic | XP | Modern |
|:---:|:---:|:---:|
| ![Classic](screenshots/classic/03_mods_tab.png) | ![XP](screenshots/xp/03_mods_tab.png) | ![Modern](screenshots/modern/03_mods_tab.png) |

## Generating Screenshots

```bash
# All skins
for skin in classic xp modern desktop corrupted macos gnome balatro retro-cga paper win95 solarized vaporwave highcontrast altimit; do
  cargo run -p oasis-app --bin oasis-screenshot "$skin"
done
```
