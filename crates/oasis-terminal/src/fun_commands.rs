//! Fun and utility commands: cal, fortune, banner, figlet, matrix, yes, watch, time.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{Command, CommandOutput, Environment};

// ---------------------------------------------------------------------------
// cal
// ---------------------------------------------------------------------------

struct CalCmd;
impl Command for CalCmd {
    fn name(&self) -> &str {
        "cal"
    }
    fn description(&self) -> &str {
        "Display a calendar"
    }
    fn usage(&self) -> &str {
        "cal [month] [year]"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let (month, year) = if args.len() >= 2 {
            let m: u32 = args[0]
                .parse()
                .map_err(|_| OasisError::Command("invalid month".to_string()))?;
            let y: i32 = args[1]
                .parse()
                .map_err(|_| OasisError::Command("invalid year".to_string()))?;
            (m, y)
        } else if let Some(time) = env.time {
            if let Ok(now) = time.now() {
                (now.month as u32, now.year as i32)
            } else {
                (1, 2025)
            }
        } else {
            (1, 2025) // Fallback.
        };

        if !(1..=12).contains(&month) {
            return Err(OasisError::Command("month must be 1-12".to_string()));
        }

        let month_names = [
            "",
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 => {
                if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                    29
                } else {
                    28
                }
            },
            _ => 30,
        };

        // Zeller's congruence for day of week of the 1st.
        let dow = day_of_week(year, month, 1);

        let mut lines = Vec::new();
        lines.push(format!("   {} {year}", month_names[month as usize]));
        lines.push("Su Mo Tu We Th Fr Sa".to_string());

        let mut line = "   ".repeat(dow as usize);
        for day in 1..=days_in_month {
            line.push_str(&format!("{day:>2} "));
            if (dow + day).is_multiple_of(7) {
                lines.push(line.trim_end().to_string());
                line = String::new();
            }
        }
        if !line.trim().is_empty() {
            lines.push(line.trim_end().to_string());
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

/// Returns day of week (0=Sunday) for given date.
fn day_of_week(year: i32, month: u32, day: u32) -> u32 {
    // Tomohiko Sakamoto's algorithm.
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    ((y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day as i32) % 7) as u32
}

// ---------------------------------------------------------------------------
// fortune
// ---------------------------------------------------------------------------

struct FortuneCmd;
impl Command for FortuneCmd {
    fn name(&self) -> &str {
        "fortune"
    }
    fn description(&self) -> &str {
        "Print a random fortune"
    }
    fn usage(&self) -> &str {
        "fortune"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let fortunes = [
            "The best code is no code at all.",
            "There are only two hard things: cache invalidation and naming things.",
            "It works on my machine!",
            "OASIS_OS: Where the desert meets the digital.",
            "To err is human; to really foul things up requires a computer.",
            "Talk is cheap. Show me the code. -- Linus Torvalds",
            "Any sufficiently advanced technology is indistinguishable from magic.",
            "The PSP never dies. It just gets a new shell.",
            "Debugging is twice as hard as writing the code in the first place.",
            "First, solve the problem. Then, write the code.",
            "In theory, there is no difference between theory and practice.",
            "It's not a bug, it's a feature.",
            "640K ought to be enough for anybody. (32MB on PSP, actually.)",
            "Keep it simple, keep it OASIS.",
        ];
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as usize;
        let idx = seed % fortunes.len();
        Ok(CommandOutput::Text(fortunes[idx].to_string()))
    }
}

// ---------------------------------------------------------------------------
// banner
// ---------------------------------------------------------------------------

struct BannerCmd;
impl Command for BannerCmd {
    fn name(&self) -> &str {
        "banner"
    }
    fn description(&self) -> &str {
        "Print text in large ASCII letters"
    }
    fn usage(&self) -> &str {
        "banner <text>"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: banner <text>".to_string()));
        }
        let text = args.join(" ").to_ascii_uppercase();
        let mut rows = vec![String::new(); 5];

        for ch in text.chars() {
            let glyph = banner_glyph(ch);
            for (i, row) in rows.iter_mut().enumerate() {
                row.push_str(glyph[i]);
                row.push(' ');
            }
        }
        Ok(CommandOutput::Text(rows.join("\n")))
    }
}

fn banner_glyph(ch: char) -> [&'static str; 5] {
    match ch {
        'A' => [" ## ", "#  #", "####", "#  #", "#  #"],
        'B' => ["### ", "#  #", "### ", "#  #", "### "],
        'C' => [" ## ", "#   ", "#   ", "#   ", " ## "],
        'D' => ["### ", "#  #", "#  #", "#  #", "### "],
        'E' => ["####", "#   ", "### ", "#   ", "####"],
        'F' => ["####", "#   ", "### ", "#   ", "#   "],
        'G' => [" ## ", "#   ", "# ##", "#  #", " ## "],
        'H' => ["#  #", "#  #", "####", "#  #", "#  #"],
        'I' => ["###", " # ", " # ", " # ", "###"],
        'J' => ["  ##", "   #", "   #", "#  #", " ## "],
        'K' => ["#  #", "## ", "#  ", "## ", "#  #"],
        'L' => ["#   ", "#   ", "#   ", "#   ", "####"],
        'M' => ["#   #", "## ##", "# # #", "#   #", "#   #"],
        'N' => ["#  #", "## #", "# ##", "#  #", "#  #"],
        'O' => [" ## ", "#  #", "#  #", "#  #", " ## "],
        'P' => ["### ", "#  #", "### ", "#   ", "#   "],
        'Q' => [" ## ", "#  #", "# ##", "#  #", " ## "],
        'R' => ["### ", "#  #", "### ", "## ", "#  #"],
        'S' => [" ## ", "#   ", " ## ", "   #", "## "],
        'T' => ["#####", "  #  ", "  #  ", "  #  ", "  #  "],
        'U' => ["#  #", "#  #", "#  #", "#  #", " ## "],
        'V' => ["#   #", "#   #", " # # ", " # # ", "  #  "],
        'W' => ["#   #", "#   #", "# # #", "## ##", "#   #"],
        'X' => ["#  #", " ## ", " ## ", " ## ", "#  #"],
        'Y' => ["#   #", " # # ", "  #  ", "  #  ", "  #  "],
        'Z' => ["####", "  # ", " #  ", "#   ", "####"],
        '0' => [" ## ", "#  #", "#  #", "#  #", " ## "],
        '1' => [" # ", "## ", " # ", " # ", "###"],
        '2' => [" ## ", "#  #", "  # ", " #  ", "####"],
        '3' => ["### ", "   #", " ## ", "   #", "### "],
        '4' => ["#  #", "#  #", "####", "   #", "   #"],
        '5' => ["####", "#   ", "### ", "   #", "### "],
        '6' => [" ## ", "#   ", "### ", "#  #", " ## "],
        '7' => ["####", "   #", "  # ", " #  ", "#   "],
        '8' => [" ## ", "#  #", " ## ", "#  #", " ## "],
        '9' => [" ## ", "#  #", " ###", "   #", " ## "],
        ' ' => ["    ", "    ", "    ", "    ", "    "],
        '!' => [" # ", " # ", " # ", "   ", " # "],
        _ => ["    ", "    ", " ?  ", "    ", "    "],
    }
}

// ---------------------------------------------------------------------------
// matrix
// ---------------------------------------------------------------------------

struct MatrixCmd;
impl Command for MatrixCmd {
    fn name(&self) -> &str {
        "matrix"
    }
    fn description(&self) -> &str {
        "Display Matrix-style rain (snapshot)"
    }
    fn usage(&self) -> &str {
        "matrix"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, _args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let width = 48;
        let height = 10;
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let chars = "01アイウエオカキクケコ日月火水木金土";
        let char_vec: Vec<char> = chars.chars().collect();
        let mut lines = Vec::new();

        for _ in 0..height {
            let mut line = String::new();
            for _ in 0..width {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let idx = ((seed >> 33) as usize) % (char_vec.len() + 2);
                if idx < char_vec.len() {
                    line.push(char_vec[idx]);
                } else {
                    line.push(' ');
                }
            }
            lines.push(line);
        }
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

// ---------------------------------------------------------------------------
// yes
// ---------------------------------------------------------------------------

struct YesCmd;
impl Command for YesCmd {
    fn name(&self) -> &str {
        "yes"
    }
    fn description(&self) -> &str {
        "Repeat a string (limited to 20 lines)"
    }
    fn usage(&self) -> &str {
        "yes [text]"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        let text = if args.is_empty() { "y" } else { args[0] };
        let lines: Vec<&str> = std::iter::repeat_n(text, 20).collect();
        Ok(CommandOutput::Text(lines.join("\n")))
    }
}

// ---------------------------------------------------------------------------
// watch
// ---------------------------------------------------------------------------

struct WatchCmd;
impl Command for WatchCmd {
    fn name(&self) -> &str {
        "watch"
    }
    fn description(&self) -> &str {
        "Execute a command (one-shot in terminal)"
    }
    fn usage(&self) -> &str {
        "watch <command>"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: watch <command>".to_string()));
        }
        let cmd = args.join(" ");
        Ok(CommandOutput::Text(format!(
            "watch: would repeat '{cmd}' every 2s (one-shot mode)"
        )))
    }
}

// ---------------------------------------------------------------------------
// time
// ---------------------------------------------------------------------------

struct TimeCmd;
impl Command for TimeCmd {
    fn name(&self) -> &str {
        "time"
    }
    fn description(&self) -> &str {
        "Time a command execution (simulated)"
    }
    fn usage(&self) -> &str {
        "time <command>"
    }
    fn category(&self) -> &str {
        "fun"
    }
    fn execute(&self, args: &[&str], _env: &mut Environment<'_>) -> Result<CommandOutput> {
        if args.is_empty() {
            return Err(OasisError::Command("usage: time <command>".to_string()));
        }
        let cmd = args.join(" ");
        Ok(CommandOutput::Text(format!(
            "time: '{cmd}'\nreal\t0m0.001s\nuser\t0m0.000s\nsys\t0m0.001s"
        )))
    }
}

/// Register fun/utility commands.
pub fn register_fun_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(CalCmd));
    reg.register(Box::new(FortuneCmd));
    reg.register(Box::new(BannerCmd));
    reg.register(Box::new(MatrixCmd));
    reg.register(Box::new(YesCmd));
    reg.register(Box::new(WatchCmd));
    reg.register(Box::new(TimeCmd));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::MemoryVfs;

    fn exec(reg: &CommandRegistry, vfs: &mut MemoryVfs, line: &str) -> Result<CommandOutput> {
        let mut env = Environment {
            cwd: "/".to_string(),
            vfs,
            power: None,
            time: None,
            usb: None,
            network: None,
            tls: None,
            stdin: None,
        };
        reg.execute(line, &mut env)
    }

    #[test]
    fn cal_january_2025() {
        let mut reg = CommandRegistry::new();
        register_fun_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "cal 1 2025").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("January 2025"));
                assert!(s.contains("Su Mo Tu"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn fortune_non_empty() {
        let mut reg = CommandRegistry::new();
        register_fun_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "fortune").unwrap() {
            CommandOutput::Text(s) => assert!(!s.is_empty()),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn banner_basic() {
        let mut reg = CommandRegistry::new();
        register_fun_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "banner HI").unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s.lines().count(), 5);
                assert!(s.contains('#'));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn matrix_output() {
        let mut reg = CommandRegistry::new();
        register_fun_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "matrix").unwrap() {
            CommandOutput::Text(s) => assert_eq!(s.lines().count(), 10),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn yes_default() {
        let mut reg = CommandRegistry::new();
        register_fun_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "yes").unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s.lines().count(), 20);
                assert!(s.lines().all(|l| l == "y"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn yes_custom() {
        let mut reg = CommandRegistry::new();
        register_fun_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        match exec(&reg, &mut vfs, "yes oasis").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.lines().all(|l| l == "oasis"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn day_of_week_known() {
        // 2025-01-01 is a Wednesday (dow=3).
        assert_eq!(day_of_week(2025, 1, 1), 3);
        // 2024-02-29 is a Thursday (dow=4) -- leap year.
        assert_eq!(day_of_week(2024, 2, 29), 4);
    }
}
