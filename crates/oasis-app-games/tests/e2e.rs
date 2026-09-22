//! End-to-end Games sessions driven through the `App` trait: menu
//! navigation, real-time Snake on wall-clock ticks, Memory Match played
//! from what is on screen, and the Sliding Puzzle, all read back from the
//! rendered text grid.
//!
//! The Games app has no pointer support (`handle_click` is not
//! implemented), so sessions use the gamepad / keyboard path.
#![allow(clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};

use oasis_app_core::testing::{AppHarness, fuzz_app};
use oasis_app_core::{App, AppAction};
use oasis_app_games::{ActiveGame, GamesApp, HIGH_SCORES_PATH};
use oasis_types::input::{Button, Key};
use oasis_vfs::{MemoryVfs, Vfs};

/// One snake cell step: 6 simulation steps at 60 Hz = 100 ms.
const STEP_MS: u32 = 100;

fn games() -> AppHarness {
    AppHarness::new(Box::new(GamesApp::new("/apps/Games")))
}

fn active(h: &AppHarness) -> ActiveGame {
    h.app_as::<GamesApp>().active_game()
}

fn open(h: &mut AppHarness, index: usize) {
    for _ in 0..3 {
        h.press(Button::Up);
    }
    for _ in 0..index {
        h.press(Button::Down);
    }
    h.press(Button::Confirm);
}

// ── Snake ─────────────────────────────────────────────────────────

struct SnakeView {
    cells: HashSet<(i32, i32)>,
    food: Option<(i32, i32)>,
    score: u32,
    high: u32,
    status: String,
}

fn snake_view(h: &AppHarness) -> SnakeView {
    let lines = h.app().lines();
    let header = &lines[0];
    let num = |key: &str| -> u32 {
        let rest = header.split(key).nth(1).unwrap().trim_start();
        rest.split_whitespace().next().unwrap().parse().unwrap()
    };
    let mut cells = HashSet::new();
    let mut food = None;
    for (y, row) in lines[2..2 + 15].iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            match c {
                '#' => {
                    cells.insert((x as i32, y as i32));
                },
                '*' => food = Some((x as i32, y as i32)),
                _ => {},
            }
        }
    }
    SnakeView {
        cells,
        food,
        score: num("Score:"),
        high: num("High:"),
        status: lines.last().unwrap().clone(),
    }
}

fn step(pos: (i32, i32), b: Button) -> (i32, i32) {
    match b {
        Button::Up => (pos.0, pos.1 - 1),
        Button::Down => (pos.0, pos.1 + 1),
        Button::Left => (pos.0 - 1, pos.1),
        _ => (pos.0 + 1, pos.1),
    }
}

fn opposite(b: Button) -> Button {
    match b {
        Button::Up => Button::Down,
        Button::Down => Button::Up,
        Button::Left => Button::Right,
        _ => Button::Left,
    }
}

#[test]
fn snake_moves_in_wall_time_eats_dies_and_restarts() {
    let mut h = games();
    open(&mut h, 0);
    assert_eq!(active(&h), ActiveGame::Snake);
    let v = snake_view(&h);
    assert_eq!(v.cells, HashSet::from([(10, 7), (9, 7), (8, 7)]));
    assert_eq!(v.score, 0);

    // Movement is driven by wall time, not frame count: 99 ms in tiny
    // frames does not move, the 100th ms does.
    for _ in 0..99 {
        h.frame(1);
    }
    assert!(snake_view(&h).cells.contains(&(8, 7)));
    assert!(h.frame(1));
    let v = snake_view(&h);
    assert!(v.cells.contains(&(11, 7)) && !v.cells.contains(&(8, 7)));
    let mut head = (11, 7);
    let mut dir = Button::Right;

    // Reversing into the body is ignored.
    h.press(Button::Left);
    h.frame(STEP_MS);
    head = step(head, dir);
    assert!(
        snake_view(&h).cells.contains(&head),
        "reversal was accepted"
    );

    // Pause freezes the game.
    h.press(Button::Start);
    h.frame(1000);
    assert!(snake_view(&h).status.contains("PAUSED"));
    assert!(snake_view(&h).cells.contains(&head));
    h.press(Button::Start);

    // Greedy chase: eat three pieces of food.
    let mut len = 3;
    for _ in 0..400 {
        let v = snake_view(&h);
        if v.score >= 3 {
            break;
        }
        let food = v.food.unwrap();
        let mut options = vec![];
        if food.0 > head.0 {
            options.push(Button::Right);
        }
        if food.0 < head.0 {
            options.push(Button::Left);
        }
        if food.1 > head.1 {
            options.push(Button::Down);
        }
        if food.1 < head.1 {
            options.push(Button::Up);
        }
        options.extend([Button::Up, Button::Down, Button::Left, Button::Right]);
        let safe = |b: &Button| {
            let n = step(head, *b);
            *b != opposite(dir)
                && (0..20).contains(&n.0)
                && (0..15).contains(&n.1)
                && !v.cells.contains(&n)
        };
        dir = *options.iter().find(|b| safe(b)).unwrap();
        h.press(dir);
        h.frame(STEP_MS);
        head = step(head, dir);
        let now = snake_view(&h);
        assert!(now.cells.contains(&head), "head not where expected");
        if now.score > v.score {
            len += 1;
            assert_ne!(now.food, Some(head), "food respawned under the snake");
        }
        assert_eq!(now.cells.len(), len, "snake length out of sync");
    }
    assert_eq!(snake_view(&h).score, 3);

    // Drive into the top wall.
    if dir == Button::Down {
        h.press(Button::Left);
        h.frame(STEP_MS);
    }
    h.press(Button::Up);
    for _ in 0..20 {
        h.frame(STEP_MS);
    }
    let v = snake_view(&h);
    assert!(v.status.contains("GAME OVER"), "{}", v.status);
    assert_eq!(v.high, 3);
    // The high score was persisted by the host's apply_vfs_ops pass (run
    // by the frames above); game over: time no longer changes anything.
    assert!(!h.frame(1000));
    let saved = String::from_utf8(h.vfs().read(HIGH_SCORES_PATH).unwrap()).unwrap();
    assert!(saved.contains("high_score = 3"), "{saved}");

    // Select restarts with the high score kept.
    h.press(Button::Select);
    let v = snake_view(&h);
    assert_eq!((v.score, v.high, v.cells.len()), (0, 3, 3));
    assert!(v.status.contains("D-pad"));

    // Reopen: the persisted high score is loaded.
    h.replace_app(Box::new(GamesApp::new("/apps/Games")));
    h.frame(16);
    open(&mut h, 0);
    assert_eq!(snake_view(&h).high, 3);
    // Cancel returns to the menu, Cancel again closes.
    assert_eq!(h.press(Button::Cancel), AppAction::None);
    assert_eq!(active(&h), ActiveGame::Menu);
    assert_eq!(h.press(Button::Cancel), AppAction::Exit);
}

#[test]
fn snake_long_stall_does_not_replay_seconds_of_game_time() {
    let mut h = games();
    open(&mut h, 0);
    // A 10 s host stall is clamped: at most a couple of cells, not 100
    // (which would have crashed the snake into the wall unseen).
    h.frame(10_000);
    let v = snake_view(&h);
    assert!(!v.status.contains("GAME OVER"));
    assert!(v.cells.contains(&(12, 7)) || v.cells.contains(&(11, 7)));
    // Time spent on the menu is not replayed on return.
    h.press(Button::Cancel);
    h.frame(10_000);
    h.press(Button::Confirm);
    let before = snake_view(&h).cells;
    h.frame(1);
    assert_eq!(snake_view(&h).cells, before);
}

#[test]
fn corrupt_high_score_file_reads_as_zero() {
    let mut vfs = MemoryVfs::new();
    vfs.mkdir("/home").unwrap();
    vfs.mkdir("/home/user").unwrap();
    vfs.write(HIGH_SCORES_PATH, b"[snake]\nhigh_score = lots\n\xff")
        .unwrap();
    let mut h = AppHarness::with_vfs(Box::new(GamesApp::new("/apps/Games")), vfs);
    h.frame(16);
    open(&mut h, 0);
    assert_eq!(snake_view(&h).high, 0);
}

// ── Memory Match ──────────────────────────────────────────────────

/// Card faces as drawn: `Some(symbol)` face-up, `None` face-down,
/// `Some(' ')` matched.
fn memory_grid(h: &AppHarness) -> Vec<Option<char>> {
    let lines = h.app().lines();
    let mut out = Vec::new();
    for row in &lines[2..6] {
        let chars: Vec<char> = row.chars().collect();
        for cell in chars.chunks(3) {
            out.push(match cell[1] {
                '\u{25A0}' => None,
                c => Some(c),
            });
        }
    }
    assert_eq!(out.len(), 16);
    out
}

fn memory_header(h: &AppHarness) -> String {
    h.app().lines()[0].clone()
}

/// Move the memory cursor to card `idx` from wherever it is.
fn goto_card(h: &mut AppHarness, idx: usize) {
    for _ in 0..4 {
        h.press(Button::Up);
        h.press(Button::Left);
    }
    for _ in 0..idx / 4 {
        h.press(Button::Down);
    }
    for _ in 0..idx % 4 {
        h.press(Button::Right);
    }
}

fn flip(h: &mut AppHarness, idx: usize) -> char {
    goto_card(h, idx);
    h.press(Button::Confirm);
    memory_grid(h)[idx].unwrap()
}

#[test]
fn memory_match_played_from_the_screen_until_won() {
    let mut h = games();
    open(&mut h, 1);
    assert_eq!(active(&h), ActiveGame::Memory);
    assert!(memory_grid(&h).iter().all(Option::is_none));

    // Learn every card by flipping pairs (0,1), (2,3), ... and waiting
    // out the mismatch reveal; match known pairs as soon as possible.
    let mut known: HashMap<usize, char> = HashMap::new();
    let mut matched: HashSet<usize> = HashSet::new();
    let mut moves = 0;
    let mut i = 0;
    while matched.len() < 16 {
        // A known pair among unmatched cards? Take it.
        let mut by_sym: HashMap<char, Vec<usize>> = HashMap::new();
        for (&k, &s) in &known {
            if !matched.contains(&k) {
                by_sym.entry(s).or_default().push(k);
            }
        }
        if let Some(pair) = by_sym.values().find(|v| v.len() == 2) {
            let (a, b) = (pair[0], pair[1]);
            flip(&mut h, a);
            flip(&mut h, b);
            moves += 1;
            let g = memory_grid(&h);
            assert_eq!((g[a], g[b]), (Some(' '), Some(' ')), "pair not matched");
            matched.extend([a, b]);
            continue;
        }
        // Otherwise explore two unknown cards.
        let unknown: Vec<usize> = (0..16)
            .filter(|c| !known.contains_key(c) && !matched.contains(c))
            .collect();
        let a = unknown[0];
        let sa = flip(&mut h, a);
        known.insert(a, sa);
        // If the first card's partner is known, take it now.
        let partner = known
            .iter()
            .find(|&(&k, &s)| k != a && s == sa && !matched.contains(&k))
            .map(|(&k, _)| k);
        let b = partner.unwrap_or_else(|| unknown[1]);
        // A matching second card clears both faces (drawn blank) at once.
        let sb = match flip(&mut h, b) {
            ' ' => sa,
            s => s,
        };
        known.insert(b, sb);
        moves += 1;
        if sa == sb {
            let g = memory_grid(&h);
            assert_eq!((g[a], g[b]), (Some(' '), Some(' ')), "match not cleared");
            matched.extend([a, b]);
        } else {
            // Mismatch stays visible, blocks further flips until the
            // reveal timer (30 steps = 0.5 s) hides both cards again.
            let other = (0..16)
                .find(|c| *c != a && *c != b && !matched.contains(c))
                .unwrap();
            goto_card(&mut h, other);
            h.press(Button::Confirm);
            assert_eq!(memory_grid(&h)[other], None, "flip allowed during reveal");
            // 24 frames of 16 ms = 384 ms: still showing.
            h.frames(24, 16);
            assert_eq!(memory_grid(&h)[a], Some(sa));
            // Past 500 ms: hidden again.
            h.frames(10, 16);
            let g = memory_grid(&h);
            assert_eq!((g[a], g[b]), (None, None), "mismatch not hidden");
        }
        i += 1;
        assert!(i < 40, "memory game did not finish");
    }
    let header = memory_header(&h);
    assert!(header.contains("Pairs: 8/8"), "{header}");
    assert!(header.contains(&format!("Moves: {moves}")), "{header}");
    let status = h.app().lines().last().unwrap().clone();
    assert_eq!(
        status,
        format!("COMPLETE in {moves} moves! Select: New game")
    );
    // Select deals a fresh game.
    h.press(Button::Select);
    assert!(memory_header(&h).contains("Pairs: 0/8"));
    assert!(memory_grid(&h).iter().all(Option::is_none));
}

// ── Sliding puzzle ────────────────────────────────────────────────

fn puzzle_tiles(h: &AppHarness) -> Vec<u8> {
    let lines = h.app().lines();
    let mut out = Vec::new();
    for row in &lines[2..6] {
        for tok in row.split_whitespace() {
            out.push(if tok == "." { 0 } else { tok.parse().unwrap() });
        }
    }
    assert_eq!(out.len(), 16, "{lines:?}");
    out
}

fn puzzle_moves(h: &AppHarness) -> u32 {
    h.app().lines()[0]
        .rsplit(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn sliding_puzzle_slides_tiles_and_rejects_edge_moves() {
    let mut h = games();
    open(&mut h, 2);
    assert_eq!(active(&h), ActiveGame::Puzzle);
    let start = puzzle_tiles(&h);
    let mut sorted = start.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..16).collect::<Vec<u8>>(), "not a permutation");
    assert_ne!(
        start,
        (1..16).chain([0]).collect::<Vec<u8>>(),
        "starts solved"
    );
    assert_eq!(puzzle_moves(&h), 0);

    let mut moves = 0;
    for b in [
        Button::Up,
        Button::Up,
        Button::Up,
        Button::Up,
        Button::Left,
        Button::Left,
        Button::Left,
        Button::Left,
        Button::Down,
        Button::Right,
        Button::Down,
        Button::Right,
    ] {
        let before = puzzle_tiles(&h);
        let gap = before.iter().position(|t| *t == 0).unwrap();
        let (r, c) = (gap / 4, gap % 4);
        // Pressing a direction moves the tile on the opposite side of the
        // gap into it.
        let src = match b {
            Button::Up if r < 3 => Some(gap + 4),
            Button::Down if r > 0 => Some(gap - 4),
            Button::Left if c < 3 => Some(gap + 1),
            Button::Right if c > 0 => Some(gap - 1),
            _ => None,
        };
        h.press(b);
        let after = puzzle_tiles(&h);
        match src {
            Some(s) => {
                moves += 1;
                assert_eq!(after[gap], before[s], "{b:?}");
                assert_eq!(after[s], 0, "{b:?}");
            },
            None => assert_eq!(after, before, "edge move changed the board"),
        }
        assert_eq!(puzzle_moves(&h), moves);
    }
    // Time does not affect a turn-based game.
    let before = puzzle_tiles(&h);
    assert!(!h.frame(5000));
    assert_eq!(puzzle_tiles(&h), before);
    h.press(Button::Select);
    assert_eq!(puzzle_moves(&h), 0);
}

// ── Common ────────────────────────────────────────────────────────

#[test]
fn keyboard_twins_and_menu_rendering() {
    let mut h = games();
    // Arrow keys and Enter reach the menu through their gamepad twins.
    h.key(Key::Down);
    h.key(Key::Down);
    h.key(Key::Enter);
    assert_eq!(active(&h), ActiveGame::Puzzle);
    h.key(Key::Escape);
    assert_eq!(active(&h), ActiveGame::Menu);
    assert!(h.screen_text().contains("> Sliding Puzzle"));
    for game in 0..3 {
        open(&mut h, game);
        h.frame(250);
        h.draw_all_sizes_and_themes();
        h.press(Button::Cancel);
    }
    h.draw_all_sizes_and_themes();
}

#[test]
fn fuzz_random_input_never_panics_or_escapes_window() {
    let make = |_: &dyn Vfs| -> Box<dyn App> { Box::new(GamesApp::new("/apps/Games")) };
    for seed in 1..=3 {
        fuzz_app(&make, MemoryVfs::new(), seed, 3000);
    }
}
