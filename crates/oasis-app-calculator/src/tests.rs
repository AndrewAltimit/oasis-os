//! Calculator unit tests (evaluator, keypad, history, clicks).

use super::*;
use oasis_app_core::testing::click_at;
use oasis_vfs::MemoryVfs;

// -- Expression evaluator: basic operations --

#[test]
fn eval_addition() {
    assert_eq!(evaluate("2 + 3").ok(), Some(5.0));
}

#[test]
fn eval_subtraction() {
    assert_eq!(evaluate("10 - 4").ok(), Some(6.0));
}

#[test]
fn eval_multiplication() {
    assert_eq!(evaluate("3 * 7").ok(), Some(21.0));
}

#[test]
fn eval_division() {
    assert_eq!(evaluate("20 / 4").ok(), Some(5.0));
}

#[test]
fn eval_modulo() {
    assert_eq!(evaluate("10 % 3").ok(), Some(1.0));
}

#[test]
fn eval_power() {
    assert_eq!(evaluate("2 ^ 10").ok(), Some(1024.0));
}

// -- Precedence --

#[test]
fn eval_precedence_mul_before_add() {
    assert_eq!(evaluate("2 + 3 * 4").ok(), Some(14.0));
}

#[test]
fn eval_precedence_complex() {
    // 2 + 3 * (4 - 1) = 2 + 9 = 11
    assert_eq!(evaluate("2 + 3 * (4 - 1)").ok(), Some(11.0));
}

#[test]
fn eval_precedence_power_before_mul() {
    // 2 * 3 ^ 2 = 2 * 9 = 18
    assert_eq!(evaluate("2 * 3 ^ 2").ok(), Some(18.0));
}

#[test]
fn eval_power_right_associative() {
    // 2 ^ 3 ^ 2 = 2 ^ 9 = 512 (not (2^3)^2 = 64)
    assert_eq!(evaluate("2 ^ 3 ^ 2").ok(), Some(512.0));
}

// -- Parentheses --

#[test]
fn eval_simple_parens() {
    assert_eq!(evaluate("(2 + 3) * 4").ok(), Some(20.0));
}

#[test]
fn eval_nested_parens() {
    // ((2 + 3) * (4 - 1)) = 5 * 3 = 15
    assert_eq!(evaluate("((2 + 3) * (4 - 1))").ok(), Some(15.0));
}

#[test]
fn eval_deeply_nested() {
    assert_eq!(evaluate("(((1 + 2)))").ok(), Some(3.0));
}

// -- Unary minus --

#[test]
fn eval_unary_minus() {
    assert_eq!(evaluate("-5").ok(), Some(-5.0));
}

#[test]
fn eval_unary_minus_in_expr() {
    assert_eq!(evaluate("3 + -2").ok(), Some(1.0));
}

#[test]
fn eval_double_unary_minus() {
    assert_eq!(evaluate("--5").ok(), Some(5.0));
}

#[test]
fn eval_unary_minus_with_parens() {
    assert_eq!(evaluate("-(3 + 2)").ok(), Some(-5.0));
}

// -- Decimal numbers --

#[test]
fn eval_decimal() {
    let result = evaluate("1.5 + 2.5").ok();
    assert_eq!(result, Some(4.0));
}

#[test]
fn eval_decimal_mul() {
    let result = evaluate("0.1 * 10").ok();
    assert!((result.unwrap_or(0.0) - 1.0).abs() < 1e-9);
}

// -- Error cases --

#[test]
fn eval_empty_expression() {
    assert_eq!(evaluate(""), Err(CalcError::EmptyExpression));
}

#[test]
fn eval_whitespace_only() {
    assert_eq!(evaluate("   "), Err(CalcError::EmptyExpression));
}

#[test]
fn eval_division_by_zero() {
    assert_eq!(evaluate("1 / 0"), Err(CalcError::DivisionByZero));
}

#[test]
fn eval_modulo_by_zero() {
    assert_eq!(evaluate("5 % 0"), Err(CalcError::DivisionByZero));
}

#[test]
fn eval_unmatched_left_paren() {
    assert_eq!(evaluate("(2 + 3"), Err(CalcError::UnmatchedParen));
}

#[test]
fn eval_unmatched_right_paren() {
    assert!(evaluate("2 + 3)").is_err());
}

#[test]
fn eval_invalid_char() {
    assert!(matches!(
        evaluate("2 & 3"),
        Err(CalcError::InvalidExpression(_))
    ));
}

#[test]
fn eval_trailing_operator() {
    assert!(evaluate("2 +").is_err());
}

// -- Chained operations --

#[test]
fn eval_chained_add_sub() {
    assert_eq!(evaluate("1 + 2 - 3 + 4").ok(), Some(4.0));
}

#[test]
fn eval_chained_mul_div() {
    assert_eq!(evaluate("12 / 3 * 2").ok(), Some(8.0));
}

// -- format_number --

#[test]
fn format_integer() {
    assert_eq!(format_number(42.0), "42");
}

#[test]
fn format_decimal() {
    assert_eq!(format_number(2.75), "2.75");
}

#[test]
fn format_negative() {
    assert_eq!(format_number(-7.0), "-7");
}

// -- CalculatorApp state tests --

fn make_vfs() -> MemoryVfs {
    MemoryVfs::new()
}

#[test]
fn app_new_display() {
    let app = CalculatorApp::new("/apps/calc");
    assert_eq!(app.display, "0");
    assert!(app.input_buffer.is_empty());
    assert!(app.history.is_empty());
}

#[test]
fn app_title_and_path() {
    let app = CalculatorApp::new("/apps/calc");
    assert_eq!(app.title(), "Calculator");
    assert_eq!(app.path(), "/apps/calc");
}

#[test]
fn app_push_digits() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_digit('2');
    app.push_digit('3');
    assert_eq!(app.input_buffer, "123");
    assert_eq!(app.display, "123");
}

#[test]
fn app_push_operator() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('5');
    app.push_operator('+');
    assert_eq!(app.input_buffer, "5+");
}

#[test]
fn app_evaluate() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('2');
    app.push_operator('+');
    app.push_digit('3');
    app.evaluate_input();
    assert_eq!(app.last_result, Some(5.0));
    assert_eq!(app.history.len(), 1);
    assert_eq!(app.history[0].expression, "2+3");
    assert_eq!(app.history[0].result, 5.0);
    assert!(app.input_buffer.is_empty());
}

#[test]
fn app_history_tracks_multiple() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_operator('+');
    app.push_digit('1');
    app.evaluate_input();
    app.push_digit('2');
    app.push_operator('*');
    app.push_digit('3');
    app.evaluate_input();
    assert_eq!(app.history.len(), 2);
    assert_eq!(app.history[0].result, 2.0);
    assert_eq!(app.history[1].result, 6.0);
}

#[test]
fn app_clear() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('5');
    app.push_operator('+');
    app.push_digit('3');
    app.clear();
    assert!(app.input_buffer.is_empty());
    assert_eq!(app.display, "0");
}

#[test]
fn app_clear_all() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_operator('+');
    app.push_digit('1');
    app.evaluate_input();
    app.clear_all();
    assert!(app.input_buffer.is_empty());
    assert!(app.history.is_empty());
    assert!(app.last_result.is_none());
    assert_eq!(app.display, "0");
}

#[test]
fn app_backspace() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_digit('2');
    app.push_digit('3');
    app.backspace();
    assert_eq!(app.input_buffer, "12");
    assert_eq!(app.display, "12");
}

#[test]
fn app_backspace_to_empty() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('5');
    app.backspace();
    assert!(app.input_buffer.is_empty());
    assert_eq!(app.display, "0");
}

// -- Memory operations --

#[test]
fn app_memory_store_and_recall() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('4');
    app.push_digit('2');
    app.evaluate_input();
    app.memory_store();
    assert_eq!(app.memory, 42.0);
    app.clear();
    app.memory_recall();
    assert_eq!(app.input_buffer, "42");
}

#[test]
fn app_memory_add() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_digit('0');
    app.evaluate_input();
    app.memory_store();
    app.push_digit('5');
    app.evaluate_input();
    app.memory_add();
    assert_eq!(app.memory, 15.0);
}

#[test]
fn app_memory_clear() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('7');
    app.evaluate_input();
    app.memory_store();
    assert_eq!(app.memory, 7.0);
    app.memory_clear();
    assert_eq!(app.memory, 0.0);
}

// -- Display formatting --

#[test]
fn display_lines_have_no_title_heading() {
    // The app title shows in the WM / app-chrome title bar; the content
    // must not repeat it (it read as a double title bar in windows).
    let app = CalculatorApp::new("/apps/calc");
    let lines = app.format_display_lines();
    assert!(!lines.iter().any(|l| l.contains("Calculator")));
    assert!(lines.iter().any(|l| l.contains("Result: 0")));
}

#[test]
fn display_lines_contain_memory() {
    let app = CalculatorApp::new("/apps/calc");
    let lines = app.format_display_lines();
    assert!(lines.iter().any(|l| l.contains("Memory: 0")));
}

#[test]
fn display_lines_show_history() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_operator('+');
    app.push_digit('1');
    app.evaluate_input();
    let lines = app.format_display_lines();
    assert!(lines.iter().any(|l| l.contains("1+1 = 2")));
}

#[test]
fn display_lines_show_input() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('3');
    app.push_operator('*');
    let lines = app.format_display_lines();
    assert!(lines.iter().any(|l| l.contains("Input: 3*")));
}

// -- Edge cases --

#[test]
fn no_multiple_leading_zeros() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('0');
    app.push_digit('0');
    app.push_digit('0');
    // Should remain "0" not "000".
    assert!(app.input_buffer == "0" || app.input_buffer.is_empty());
}

#[test]
fn no_multiple_decimals_in_number() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_digit('.');
    app.push_digit('2');
    app.push_digit('.');
    app.push_digit('3');
    // Second dot should be rejected.
    assert_eq!(app.input_buffer, "1.23");
}

#[test]
fn consecutive_operator_replacement() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('5');
    app.push_operator('+');
    app.push_operator('-');
    // Should replace + with -.
    assert_eq!(app.input_buffer, "5-");
}

#[test]
fn non_finite_results_are_errors() {
    assert_eq!(evaluate("0^-1"), Err(CalcError::NotFinite));
    assert_eq!(evaluate("9^999"), Err(CalcError::NotFinite));
    assert_eq!(evaluate("(0-1)^0.5"), Err(CalcError::NotFinite));
    let mut app = CalculatorApp::new("/apps/calc");
    for c in "9^999".chars() {
        app.press_key(CalculatorApp::key_for_char(c).expect("key"));
    }
    app.evaluate_input();
    assert!(app.error_message.is_some());
    assert!(app.last_result.is_none(), "no Infinity to chain from");
}

#[test]
fn memory_add_never_stores_infinity() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.memory = f64::MAX;
    app.last_result = Some(f64::MAX);
    app.memory_add();
    assert!(app.memory.is_finite());
    app.memory_recall();
    assert!(evaluate(&app.input_buffer).is_ok());
}

#[test]
fn long_unary_and_power_chains_hit_the_depth_limit() {
    let minus = format!("{}5", "-".repeat(200_000));
    assert_eq!(evaluate(&minus), Err(CalcError::TooDeep));
    let power = format!("1{}", "^1".repeat(200_000));
    assert_eq!(evaluate(&power), Err(CalcError::TooDeep));
    // Modest chains still work.
    assert_eq!(evaluate("2^3^2").ok(), Some(512.0));
    assert_eq!(evaluate("----5").ok(), Some(5.0));
}

#[test]
fn negate_toggles_only_the_last_number() {
    let mut app = CalculatorApp::new("/apps/calc");
    for c in "5+3".chars() {
        app.press_key(CalculatorApp::key_for_char(c).expect("key"));
    }
    app.negate();
    assert_eq!(app.input_buffer, "5+-3");
    app.negate();
    assert_eq!(app.input_buffer, "5+3");
    let mut s = "5-3".to_string();
    toggle_last_sign(&mut s);
    assert_eq!(s, "5--3");
    toggle_last_sign(&mut s);
    assert_eq!(s, "5-3");
    let mut s = "2*".to_string();
    toggle_last_sign(&mut s);
    assert_eq!(s, "2*-");
    let mut s = "4*(1+(2))".to_string();
    toggle_last_sign(&mut s);
    assert_eq!(s, "4*-(1+(2))");
    toggle_last_sign(&mut s);
    assert_eq!(s, "4*(1+(2))");
}

#[test]
fn chaining_from_a_result_uses_the_exact_value() {
    let mut app = CalculatorApp::new("/apps/calc");
    for c in "1/3=*3=".chars() {
        app.press_key(CalculatorApp::key_for_char(c).expect("key"));
    }
    assert_eq!(app.last_result, Some(1.0));
    // Editing the chained number falls back to the typed text.
    for c in "1/3=+".chars() {
        app.press_key(CalculatorApp::key_for_char(c).expect("key"));
    }
    app.backspace();
    app.push_digit('1');
    assert_eq!(app.input_buffer, "0.33333333331");
    app.evaluate_input();
    assert_eq!(app.last_result, Some(0.33333333331));
}

#[test]
fn negate_input() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('5');
    app.negate();
    assert_eq!(app.input_buffer, "-5");
    app.negate();
    assert_eq!(app.input_buffer, "5");
}

#[test]
fn evaluate_empty_is_noop() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.evaluate_input();
    assert!(app.history.is_empty());
    assert!(app.last_result.is_none());
}

// -- App trait integration --

#[test]
fn cancel_exits() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    assert_eq!(app.handle_input(&Button::Cancel, &vfs), AppAction::Exit);
}

#[test]
fn confirm_presses_key_under_cursor() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('7');
    app.cursor = keypad::index_of(CalcKey::Equals).expect("=");
    app.handle_input(&Button::Confirm, &vfs);
    assert_eq!(app.last_result, Some(7.0));
}

#[test]
fn triangle_clears() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('9');
    app.handle_input(&Button::Triangle, &vfs);
    assert!(app.input_buffer.is_empty());
}

#[test]
fn square_backspaces() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('4');
    app.push_digit('2');
    app.handle_input(&Button::Square, &vfs);
    assert_eq!(app.input_buffer, "4");
}

#[test]
fn start_clears_all() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_operator('+');
    app.push_digit('1');
    app.evaluate_input();
    app.handle_input(&Button::Start, &vfs);
    assert!(app.history.is_empty());
}

#[test]
fn downcast_works() {
    let app: Box<dyn App> = Box::new(CalculatorApp::new("/apps/calc"));
    let any = app.as_any();
    assert!(any.downcast_ref::<CalculatorApp>().is_some());
}

#[test]
fn lines_returns_content() {
    let app = CalculatorApp::new("/apps/calc");
    assert!(!app.lines().is_empty());
}

#[test]
fn eval_single_number() {
    assert_eq!(evaluate("42").ok(), Some(42.0));
}

#[test]
fn eval_large_expression() {
    // 1 + 2 + 3 + ... + 10 = 55
    assert_eq!(evaluate("1+2+3+4+5+6+7+8+9+10").ok(), Some(55.0));
}

#[test]
fn decimal_point_allowed_in_new_number_after_operator() {
    let mut app = CalculatorApp::new("/apps/calc");
    app.push_digit('1');
    app.push_digit('.');
    app.push_digit('5');
    app.push_operator('+');
    app.push_digit('2');
    app.push_digit('.');
    app.push_digit('5');
    assert_eq!(app.input_buffer, "1.5+2.5");
}

// -- Keypad cursor (gamepad) --

#[test]
fn dpad_moves_cursor_and_confirm_types_digits() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    // Cursor starts on 5. Right -> 6, Up -> 9, Left -> 8.
    app.handle_input(&Button::Confirm, &vfs);
    app.handle_input(&Button::Right, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    app.handle_input(&Button::Up, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    app.handle_input(&Button::Left, &vfs);
    app.handle_input(&Button::Confirm, &vfs);
    assert_eq!(app.input_buffer, "5698");
    assert!(app.cursor_visible);
    // The d-pad no longer types digits by itself.
    app.handle_input(&Button::Down, &vfs);
    assert_eq!(app.input_buffer, "5698");
}

#[test]
fn every_key_reachable_with_dpad() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    let mut seen = vec![false; KEYS.len()];
    for _ in 0..keypad::ROWS {
        for _ in 0..keypad::COLS {
            seen[app.cursor] = true;
            app.handle_input(&Button::Right, &vfs);
        }
        app.handle_input(&Button::Down, &vfs);
    }
    assert!(seen.iter().all(|s| *s), "unreachable keys: {seen:?}");
}

#[test]
fn memory_keys_via_keypad() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    let press = |app: &mut CalculatorApp, key: CalcKey| {
        app.cursor = keypad::index_of(key).expect("key on keypad");
        app.handle_input(&Button::Confirm, &vfs);
    };
    type_str(&mut app, "12");
    press(&mut app, CalcKey::MemStore);
    assert_eq!(app.memory, 12.0);
    press(&mut app, CalcKey::Clear);
    app.handle_text_input('3');
    press(&mut app, CalcKey::MemAdd);
    assert_eq!(app.memory, 15.0);
    press(&mut app, CalcKey::Clear);
    press(&mut app, CalcKey::MemRecall);
    assert_eq!(app.input_buffer, "15");
    press(&mut app, CalcKey::MemClear);
    assert_eq!(app.memory, 0.0);
    // Parentheses and negate are reachable too.
    press(&mut app, CalcKey::Clear);
    press(&mut app, CalcKey::OpenParen);
    app.handle_text_input('2');
    press(&mut app, CalcKey::CloseParen);
    press(&mut app, CalcKey::Negate);
    press(&mut app, CalcKey::Equals);
    assert_eq!(app.last_result, Some(-2.0));
}

// -- Keyboard typing --

fn type_str(app: &mut CalculatorApp, s: &str) {
    for ch in s.chars() {
        app.handle_text_input(ch);
    }
}

#[test]
fn typed_expression_evaluates() {
    let mut app = CalculatorApp::new("/apps/calc");
    assert!(app.accepts_text());
    type_str(&mut app, "(2+3)*4^2-10%4=");
    assert_eq!(app.last_result, Some(78.0));
    type_str(&mut app, "7/2");
    let vfs = make_vfs();
    assert_eq!(
        app.handle_key(&Key::Enter, Modifiers::NONE, &vfs),
        Some(AppAction::None)
    );
    assert_eq!(app.last_result, Some(3.5));
    assert_eq!(app.history.len(), 2);
}

#[test]
fn typed_unary_minus() {
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "-3*-2=");
    assert_eq!(app.last_result, Some(6.0));
    // After a result, '-' subtracts from it.
    type_str(&mut app, "-1=");
    assert_eq!(app.last_result, Some(5.0));
}

#[test]
fn typed_unknown_chars_ignored() {
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "1a+ b2");
    assert_eq!(app.input_buffer, "1+2");
}

#[test]
fn backspace_and_escape_keys() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "123");
    app.handle_key(&Key::Backspace, Modifiers::NONE, &vfs);
    assert_eq!(app.input_buffer, "12");
    app.handle_backspace();
    assert_eq!(app.input_buffer, "1");
    // Escape clears the entry...
    assert_eq!(
        app.handle_key(&Key::Escape, Modifiers::NONE, &vfs),
        Some(AppAction::None)
    );
    assert!(app.input_buffer.is_empty());
    // ...and on a clear calculator falls through (Cancel -> Exit).
    assert_eq!(app.handle_key(&Key::Escape, Modifiers::NONE, &vfs), None);
    // Command shortcuts are left to the host.
    assert_eq!(app.handle_key(&Key::Char('c'), Modifiers::CTRL, &vfs), None);
}

#[test]
fn up_down_recall_history() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "1+1=");
    type_str(&mut app, "2*3=");
    app.handle_key(&Key::Up, Modifiers::NONE, &vfs);
    assert_eq!(app.input_buffer, "2*3");
    app.handle_key(&Key::Up, Modifiers::NONE, &vfs);
    assert_eq!(app.input_buffer, "1+1");
    // Stays on the oldest entry.
    app.handle_key(&Key::Up, Modifiers::NONE, &vfs);
    assert_eq!(app.input_buffer, "1+1");
    app.handle_key(&Key::Down, Modifiers::NONE, &vfs);
    assert_eq!(app.input_buffer, "2*3");
    app.handle_key(&Key::Down, Modifiers::NONE, &vfs);
    assert!(app.input_buffer.is_empty());
    // Gamepad Select recalls too.
    app.handle_input(&Button::Select, &vfs);
    assert_eq!(app.input_buffer, "2*3");
    // Editing a recalled expression and evaluating adds a new entry.
    type_str(&mut app, "+1=");
    assert_eq!(app.last_result, Some(7.0));
    assert_eq!(app.history.len(), 3);
}

// -- Clicks --

const CW: u32 = 300;
const CH: u32 = 220;

fn click_key(app: &mut CalculatorApp, key: CalcKey) {
    let l = CalcLayout::compute(0, 0, CW, CH);
    let r = l.key_rect(keypad::index_of(key).expect("key on keypad"));
    click_at(
        app,
        r.x + r.w as i32 / 2,
        r.y + r.h as i32 / 2,
        CW,
        CH,
        &make_vfs(),
    );
}

#[test]
fn clicking_keys_builds_and_evaluates() {
    let mut app = CalculatorApp::new("/apps/calc");
    for key in [
        CalcKey::Digit('9'),
        CalcKey::Op('*'),
        CalcKey::OpenParen,
        CalcKey::Digit('1'),
        CalcKey::Op('+'),
        CalcKey::Digit('2'),
        CalcKey::CloseParen,
    ] {
        click_key(&mut app, key);
    }
    assert_eq!(app.input_buffer, "9*(1+2)");
    click_key(&mut app, CalcKey::Equals);
    assert_eq!(app.last_result, Some(27.0));
    // The clicked key flashes pressed.
    let eq = keypad::index_of(CalcKey::Equals).expect("=");
    assert_eq!(app.flash.map(|(k, _)| k), Some(eq));
}

#[test]
fn click_outside_keys_does_nothing() {
    let mut app = CalculatorApp::new("/apps/calc");
    let l = CalcLayout::compute(0, 0, CW, CH);
    click_at(
        &mut app,
        l.display.x + 5,
        l.display.y + 5,
        CW,
        CH,
        &make_vfs(),
    );
    assert!(app.input_buffer.is_empty());
    assert!(app.flash.is_none());
}

#[test]
fn click_history_row_recalls_entry() {
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "1+1=");
    type_str(&mut app, "2+2=");
    let (w, h) = (460, 240);
    let l = CalcLayout::compute(0, 0, w, h);
    let pane = l.history.expect("wide layout has history");
    // Rows are newest first: the second row is the older entry.
    let y = pane.y + (2 * keypad::HISTORY_ROW_H) as i32 + 2;
    click_at(&mut app, pane.x + 4, y, w, h, &make_vfs());
    assert_eq!(app.input_buffer, "1+1");
}

// -- Rendering --

#[test]
fn windowed_draw_shows_keys_and_display() {
    use oasis_test_backend::{DrawCommand, RecordingBackend};
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "12+3");
    let mut backend = RecordingBackend::new(460, 240);
    let at = ActiveTheme::default();
    app.draw_windowed(0, 0, 460, 240, &mut backend, &at)
        .expect("draw");
    let texts: Vec<&str> = backend
        .commands()
        .iter()
        .filter_map(|c| match c {
            DrawCommand::DrawText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    for label in ["MC", "MR", "MS", "M+", "7", "=", "DEL", "12+3", "History"] {
        assert!(texts.contains(&label), "missing {label:?} in {texts:?}");
    }
}

#[test]
fn flash_expires_after_a_short_time() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    app.handle_text_input('4');
    assert!(app.flash.is_some());
    // Drawing doesn't age it (idle frames may never be drawn)...
    let mut backend = oasis_test_backend::RecordingBackend::new(300, 220);
    let at = ActiveTheme::default();
    for _ in 0..10 {
        app.draw_windowed(0, 0, 300, 220, &mut backend, &at)
            .expect("draw");
    }
    assert!(app.flash.is_some());
    // ...wall time does, and the expiry asks for the redraw.
    assert!(!app.tick(FLASH_MS / 2, &vfs));
    assert!(app.flash.is_some());
    assert!(app.tick(FLASH_MS / 2, &vfs), "popping back up redraws");
    assert!(app.flash.is_none());
    assert!(!app.tick(16, &vfs), "idle afterwards");
}

#[test]
fn full_screen_lines_show_keypad_cursor() {
    let vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    assert!(app.lines().iter().any(|l| l.contains("Key: [5]")));
    app.handle_input(&Button::Right, &vfs);
    assert!(app.lines().iter().any(|l| l.contains("Key: [6]")));
}

// -- History persistence --

#[test]
fn history_persists_across_instances() {
    let mut vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "6*7=");
    type_str(&mut app, "1/4=");
    assert!(app.apply_vfs_ops(&mut vfs));
    // Nothing left to write on the next frame.
    assert!(!app.apply_vfs_ops(&mut vfs));
    assert!(vfs.exists(history::HISTORY_PATH));

    let mut reopened = CalculatorApp::new("/apps/calc");
    reopened.apply_vfs_ops(&mut vfs);
    assert_eq!(reopened.history.len(), 2);
    assert_eq!(reopened.history[0].expression, "6*7");
    assert_eq!(reopened.history[1].result, 0.25);
    // Up recalls the persisted entries.
    reopened.handle_key(&Key::Up, Modifiers::NONE, &vfs);
    assert_eq!(reopened.input_buffer, "1/4");
}

#[test]
fn history_loaded_after_typing_keeps_new_entries() {
    let mut vfs = make_vfs();
    let seed = CalcHistoryEntry {
        expression: "1+1".into(),
        result: 2.0,
    };
    history::save(&mut vfs, &[seed]).expect("seed history");
    let mut app = CalculatorApp::new("/apps/calc");
    // Typed before the host's first per-frame hook ran.
    type_str(&mut app, "3+3=");
    app.apply_vfs_ops(&mut vfs);
    let exprs: Vec<_> = app.history.iter().map(|e| e.expression.as_str()).collect();
    assert_eq!(exprs, ["1+1", "3+3"]);
    assert_eq!(history::load(&vfs).len(), 2);
}

#[test]
fn all_clear_persists_empty_history() {
    let mut vfs = make_vfs();
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "2+2=");
    app.apply_vfs_ops(&mut vfs);
    app.clear_all();
    app.apply_vfs_ops(&mut vfs);
    assert!(history::load(&vfs).is_empty());
}

#[test]
fn a_new_operator_replaces_the_whole_trailing_operator_run() {
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "5*-+");
    assert_eq!(app.input_buffer, "5+");
    type_str(&mut app, "2=");
    assert_eq!(app.display, "7");

    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "-*");
    assert_eq!(app.input_buffer, "0*");
}

#[test]
fn tiny_and_huge_results_use_scientific_notation() {
    assert_eq!(format_number(-0.0), "0");
    assert_eq!(format_number(4e-11), "4e-11");
    assert_eq!(format_number(-1.5e-12), "-1.5e-12");
    assert_eq!(format_number(1e21), "1e21");
    assert_eq!(format_number(123_456_789_012_345.0), "123456789012345");
    // The display text reads back exactly.
    assert_eq!(evaluate("4e-11*2").ok(), Some(8e-11));
    assert_eq!(evaluate("1.5E+3").ok(), Some(1500.0));

    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "0-0.00000000004=");
    assert_eq!(app.display, "-4e-11", "was `-0`");
    // Ans re-inserts the value as text the evaluator accepts.
    app.insert_ans();
    type_str(&mut app, "*-1=");
    assert_eq!(app.display, "4e-11");
}

#[test]
fn ans_and_mr_replace_the_number_being_typed() {
    let mut app = CalculatorApp::new("/apps/calc");
    type_str(&mut app, "2+2=");
    type_str(&mut app, "2");
    app.insert_ans();
    assert_eq!(app.input_buffer, "4", "was `24`");

    app.clear();
    app.memory = -3.0;
    type_str(&mut app, "5");
    app.memory_recall();
    assert_eq!(app.input_buffer, "-3", "was `5-3`");

    app.clear();
    type_str(&mut app, "10+");
    app.memory_recall();
    type_str(&mut app, "=");
    assert_eq!(app.display, "7");

    app.clear();
    type_str(&mut app, "(1+1)");
    app.insert_ans();
    type_str(&mut app, "=");
    assert_eq!(app.display, "14", "a closed group multiplies");
}

#[test]
fn history_load_rejects_entries_the_calculator_cannot_produce() {
    let parsed = history::parse("5\u{e9}=5\n1+1=inf\n2*3=6\n4e-11*2=8e-11\n");
    let exprs: Vec<_> = parsed.iter().map(|e| e.expression.as_str()).collect();
    assert_eq!(exprs, ["2*3", "4e-11*2"]);
}

#[test]
fn negate_after_a_scientific_number_flips_the_whole_number() {
    let mut s = String::from("2*1.5e-12");
    toggle_last_sign(&mut s);
    assert_eq!(s, "2*-1.5e-12");
    toggle_last_sign(&mut s);
    assert_eq!(s, "2*1.5e-12");
}
