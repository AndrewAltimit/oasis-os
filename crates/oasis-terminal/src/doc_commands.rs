//! Documentation and onboarding commands: man, tutorial, motd.

use oasis_types::error::{OasisError, Result};

use crate::interpreter::{Command, CommandOutput, Environment};

// ---------------------------------------------------------------------------
// man
// ---------------------------------------------------------------------------

struct ManCmd;
impl Command for ManCmd {
    fn name(&self) -> &str {
        "man"
    }
    fn description(&self) -> &str {
        "Display manual page for a command"
    }
    fn usage(&self) -> &str {
        "man <command>"
    }
    fn category(&self) -> &str {
        "general"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let name = args
            .first()
            .copied()
            .ok_or_else(|| OasisError::Command("usage: man <command>".to_string()))?;

        let man_path = format!("/usr/share/man/{name}.txt");
        if env.vfs.exists(&man_path) {
            let data = env.vfs.read(&man_path)?;
            Ok(CommandOutput::Text(
                String::from_utf8_lossy(&data).into_owned(),
            ))
        } else {
            // Fall back to help-style output if no man page exists.
            Err(OasisError::Command(format!(
                "No manual entry for '{name}'.\n\
                 Try 'help {name}' for brief usage."
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// tutorial
// ---------------------------------------------------------------------------

/// Lesson content for the interactive tutorial.
const LESSONS: &[(&str, &str)] = &[
    (
        "Welcome",
        "Welcome to the OASIS OS terminal!\n\n\
         This tutorial will guide you through the basics.\n\
         Type 'tutorial next' to advance, or 'tutorial <n>' to jump to a lesson.\n\n\
         Lessons:\n\
         1. Welcome (this page)\n\
         2. Navigation\n\
         3. Files\n\
         4. Pipes & Redirection\n\
         5. Variables & Aliases\n\
         6. Scripting\n\
         7. Tips & Tricks",
    ),
    (
        "Navigation",
        "NAVIGATION\n\n\
         pwd          - Show current directory\n\
         ls [path]    - List directory contents\n\
         cd <path>    - Change directory\n\
         tree [path]  - Display directory tree\n\n\
         Try it: type 'ls /' to see the root directory.",
    ),
    (
        "Files",
        "FILE OPERATIONS\n\n\
         cat <file>         - Display file contents\n\
         write <file> text  - Write text to a file\n\
         append <file> text - Append text to a file\n\
         mkdir <dir>        - Create a directory\n\
         cp <src> <dst>     - Copy a file\n\
         mv <src> <dst>     - Move/rename a file\n\
         rm <path>          - Delete a file or directory\n\
         stat <path>        - Show file details\n\n\
         Try it: type 'write /tmp/hello.txt Hello World'",
    ),
    (
        "Pipes & Redirection",
        "PIPES & REDIRECTION\n\n\
         cmd1 | cmd2   - Pipe output of cmd1 into cmd2\n\
         cmd > file    - Redirect output to file (overwrite)\n\
         cmd >> file   - Redirect output to file (append)\n\
         cmd1 && cmd2  - Run cmd2 only if cmd1 succeeds\n\
         cmd1 || cmd2  - Run cmd2 only if cmd1 fails\n\
         cmd1 ; cmd2   - Run both commands\n\n\
         Try it: type 'echo hello world | wc -w'",
    ),
    (
        "Variables & Aliases",
        "VARIABLES & ALIASES\n\n\
         set VAR=value    - Set a variable\n\
         echo $VAR        - Use a variable\n\
         unset VAR        - Remove a variable\n\
         env              - List all variables\n\
         alias ll=ls      - Create an alias\n\
         unalias ll       - Remove an alias\n\n\
         Built-in variables: $CWD, $USER, $SHELL, $HOME\n\n\
         Try it: type 'set NAME=OASIS && echo Hello $NAME'",
    ),
    (
        "Scripting",
        "SCRIPTING\n\n\
         Scripts are plain text files with one command per line.\n\
         Run them with: run <script-path>\n\n\
         Control flow:\n\
         if <condition>     for VAR in ITEMS\n\
         then               do\n\
           commands           commands\n\
         else               done\n\
           commands\n\
         fi                 while <condition>\n\
                            do\n\
                              commands\n\
                            done\n\n\
         Conditions use the 'test' command:\n\
         test -f /path   (file exists)\n\
         test -d /path   (directory exists)\n\
         test a = b      (string equality)",
    ),
    (
        "Tips & Tricks",
        "TIPS & TRICKS\n\n\
         help             - List all commands by category\n\
         help <cmd>       - Show detailed help for a command\n\
         which <cmd>      - Check if a command exists\n\
         history          - Show command history\n\
         !!               - Repeat last command\n\
         !n               - Repeat command number n\n\
         cal              - Show a calendar\n\
         fortune          - Get a random tip\n\
         banner <text>    - Large ASCII text\n\
         time <cmd>       - Measure execution time\n\n\
         That's the end of the tutorial. Happy hacking!",
    ),
];

struct TutorialCmd;
impl Command for TutorialCmd {
    fn name(&self) -> &str {
        "tutorial"
    }
    fn description(&self) -> &str {
        "Interactive terminal tutorial"
    }
    fn usage(&self) -> &str {
        "tutorial [next|<lesson_number>|list]"
    }
    fn category(&self) -> &str {
        "general"
    }
    fn execute(&self, args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let progress_path = "/home/.tutorial_progress";

        // Read current progress.
        let current: usize = if env.vfs.exists(progress_path) {
            let data = env.vfs.read(progress_path)?;
            String::from_utf8_lossy(&data).trim().parse().unwrap_or(0)
        } else {
            0
        };

        let subcmd = args.first().copied().unwrap_or("show");

        match subcmd {
            "list" => {
                let mut out = String::from("Tutorial Lessons:\n");
                for (i, (title, _)) in LESSONS.iter().enumerate() {
                    let marker = if i < current {
                        "[x]"
                    } else if i == current {
                        "[>]"
                    } else {
                        "[ ]"
                    };
                    out.push_str(&format!("  {marker} {}. {title}\n", i + 1));
                }
                Ok(CommandOutput::Text(out))
            },
            "next" => {
                let next = current + 1;
                if next >= LESSONS.len() {
                    Ok(CommandOutput::Text(
                        "You've completed all lessons! Use 'tutorial 1' to restart.".to_string(),
                    ))
                } else {
                    // Save progress.
                    env.vfs.write(progress_path, next.to_string().as_bytes())?;
                    let (title, content) = LESSONS[next];
                    Ok(CommandOutput::Text(format!(
                        "--- Lesson {}: {} ---\n\n{}",
                        next + 1,
                        title,
                        content
                    )))
                }
            },
            "show" => {
                let (title, content) = LESSONS[current.min(LESSONS.len() - 1)];
                Ok(CommandOutput::Text(format!(
                    "--- Lesson {}: {} ---\n\n{}",
                    current + 1,
                    title,
                    content
                )))
            },
            n => {
                // Jump to specific lesson.
                if let Ok(num) = n.parse::<usize>() {
                    if num == 0 || num > LESSONS.len() {
                        return Err(OasisError::Command(format!(
                            "Lesson number must be 1-{}",
                            LESSONS.len()
                        )));
                    }
                    let idx = num - 1;
                    // Save progress.
                    env.vfs.write(progress_path, idx.to_string().as_bytes())?;
                    let (title, content) = LESSONS[idx];
                    Ok(CommandOutput::Text(format!(
                        "--- Lesson {num}: {title} ---\n\n{content}"
                    )))
                } else {
                    Err(OasisError::Command(format!(
                        "unknown subcommand: {n}\nusage: {}",
                        self.usage()
                    )))
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------
// motd
// ---------------------------------------------------------------------------

struct MotdCmd;
impl Command for MotdCmd {
    fn name(&self) -> &str {
        "motd"
    }
    fn description(&self) -> &str {
        "Display the message of the day"
    }
    fn usage(&self) -> &str {
        "motd"
    }
    fn category(&self) -> &str {
        "general"
    }
    fn execute(&self, _args: &[&str], env: &mut Environment<'_>) -> Result<CommandOutput> {
        let motd_path = "/etc/motd";
        if env.vfs.exists(motd_path) {
            let data = env.vfs.read(motd_path)?;
            Ok(CommandOutput::Text(
                String::from_utf8_lossy(&data).into_owned(),
            ))
        } else {
            Ok(CommandOutput::Text(default_motd()))
        }
    }
}

/// Default MOTD when no /etc/motd file exists.
fn default_motd() -> String {
    "\
     ___   _   ___ ___ ___    ___  ___\n\
    / _ \\ / \\ / __|_ _/ __|  / _ \\/ __|\n\
   | (_) / _ \\\\__ \\| |\\__ \\ | (_) \\__ \\\n\
    \\___/_/ \\_|___/___|___/  \\___/|___/\n\
\n\
  Welcome to OASIS OS!\n\
  Type 'help' for commands, 'tutorial' to get started.\n"
        .to_string()
}

/// Register documentation commands.
pub fn register_doc_commands(reg: &mut crate::CommandRegistry) {
    reg.register(Box::new(ManCmd));
    reg.register(Box::new(TutorialCmd));
    reg.register(Box::new(MotdCmd));
}

/// Populate default man pages in the VFS.
///
/// Call this during VFS initialization to pre-populate `/usr/share/man/`.
pub fn populate_man_pages(vfs: &mut dyn oasis_vfs::Vfs) {
    let _ = vfs.mkdir("/usr");
    let _ = vfs.mkdir("/usr/share");
    let _ = vfs.mkdir("/usr/share/man");

    // Core navigation commands.
    let pages: &[(&str, &str)] = &[
        (
            "ls",
            "NAME\n    ls - list directory contents\n\n\
             SYNOPSIS\n    ls [path]\n\n\
             DESCRIPTION\n    List files and directories at the given path.\n\
             If no path is given, lists the current directory.\n\n\
             EXAMPLES\n    ls /home\n    ls\n",
        ),
        (
            "cd",
            "NAME\n    cd - change directory\n\n\
             SYNOPSIS\n    cd <path>\n\n\
             DESCRIPTION\n    Change the current working directory to <path>.\n\
             Use '..' to go up one level.\n\n\
             EXAMPLES\n    cd /home\n    cd ..\n",
        ),
        (
            "cat",
            "NAME\n    cat - display file contents\n\n\
             SYNOPSIS\n    cat <file>\n\n\
             DESCRIPTION\n    Read and display the contents of a file.\n\n\
             EXAMPLES\n    cat /etc/motd\n    cat /home/notes.txt\n",
        ),
        (
            "echo",
            "NAME\n    echo - display a line of text\n\n\
             SYNOPSIS\n    echo [text...]\n\n\
             DESCRIPTION\n    Write arguments to standard output.\n\
             Supports variable expansion ($VAR).\n\n\
             EXAMPLES\n    echo Hello World\n    echo $CWD\n",
        ),
        (
            "grep",
            "NAME\n    grep - search text patterns\n\n\
             SYNOPSIS\n    grep [options] <pattern> <file>\n\n\
             DESCRIPTION\n    Search for lines matching a pattern.\n\n\
             OPTIONS\n    -i  Case insensitive\n    -n  Show line numbers\n\
             -v  Invert match\n    -c  Count matches\n\n\
             EXAMPLES\n    grep error /var/log/audit.log\n\
             cat file.txt | grep -i hello\n",
        ),
        (
            "help",
            "NAME\n    help - display command help\n\n\
             SYNOPSIS\n    help [command]\n\n\
             DESCRIPTION\n    Without arguments, list all commands by category.\n\
             With a command name, show detailed usage.\n\n\
             EXAMPLES\n    help\n    help grep\n",
        ),
        (
            "run",
            "NAME\n    run - execute a script file\n\n\
             SYNOPSIS\n    run <path>\n\n\
             DESCRIPTION\n    Execute commands from a script file.\n\
             Lines starting with '#' are comments.\n\
             Supports if/then/else/fi, while/do/done,\n\
             and for/in/do/done control flow.\n\n\
             EXAMPLES\n    run /home/setup.sh\n",
        ),
        (
            "tutorial",
            "NAME\n    tutorial - interactive terminal tutorial\n\n\
             SYNOPSIS\n    tutorial [next|<number>|list]\n\n\
             DESCRIPTION\n    Walk through lessons on using the terminal.\n\
             Progress is saved between sessions.\n\n\
             EXAMPLES\n    tutorial        (show current lesson)\n\
             tutorial next   (advance to next lesson)\n\
             tutorial list   (show all lessons)\n\
             tutorial 3      (jump to lesson 3)\n",
        ),
    ];

    for (name, content) in pages {
        let path = format!("/usr/share/man/{name}.txt");
        let _ = vfs.write(&path, content.as_bytes());
    }
}

/// Populate the default MOTD in the VFS.
pub fn populate_motd(vfs: &mut dyn oasis_vfs::Vfs) {
    let _ = vfs.mkdir("/etc");
    let motd = default_motd();
    let _ = vfs.write("/etc/motd", motd.as_bytes());
}

/// Populate a default shell profile in the VFS.
pub fn populate_profile(vfs: &mut dyn oasis_vfs::Vfs) {
    let _ = vfs.mkdir("/home");
    let profile = "\
# OASIS OS shell profile
# This file is executed on terminal startup.
set USER=oasis
set HOME=/home
set SHELL=oasis-sh
";
    let _ = vfs.write("/home/.profile", profile.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandOutput, CommandRegistry, Environment};
    use oasis_vfs::{MemoryVfs, Vfs};

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

    fn setup() -> (CommandRegistry, MemoryVfs) {
        let mut reg = CommandRegistry::new();
        register_doc_commands(&mut reg);
        let mut vfs = MemoryVfs::new();
        populate_man_pages(&mut vfs);
        populate_motd(&mut vfs);
        populate_profile(&mut vfs);
        (reg, vfs)
    }

    #[test]
    fn man_shows_page() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "man ls").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("list directory"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn man_no_page() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "man nonexistent").is_err());
    }

    #[test]
    fn man_no_args() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "man").is_err());
    }

    #[test]
    fn tutorial_show_first() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "tutorial").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Welcome"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn tutorial_list() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "tutorial list").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Navigation"));
                assert!(s.contains("Files"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn tutorial_next() {
        let (reg, mut vfs) = setup();
        vfs.mkdir("/home").unwrap();
        match exec(&reg, &mut vfs, "tutorial next").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Navigation"));
            },
            _ => panic!("expected text"),
        }
        // Progress should be saved.
        let data = vfs.read("/home/.tutorial_progress").unwrap();
        assert_eq!(String::from_utf8_lossy(&data).trim(), "1");
    }

    #[test]
    fn tutorial_jump() {
        let (reg, mut vfs) = setup();
        vfs.mkdir("/home").unwrap();
        match exec(&reg, &mut vfs, "tutorial 3").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("Files"));
                assert!(s.contains("cat"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn tutorial_invalid_number() {
        let (reg, mut vfs) = setup();
        assert!(exec(&reg, &mut vfs, "tutorial 99").is_err());
    }

    #[test]
    fn motd_default() {
        let (reg, mut vfs) = setup();
        match exec(&reg, &mut vfs, "motd").unwrap() {
            CommandOutput::Text(s) => {
                assert!(s.contains("OASIS OS"));
                assert!(s.contains("help"));
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn motd_custom() {
        let (reg, mut vfs) = setup();
        vfs.write("/etc/motd", b"Custom MOTD here").unwrap();
        match exec(&reg, &mut vfs, "motd").unwrap() {
            CommandOutput::Text(s) => {
                assert_eq!(s, "Custom MOTD here");
            },
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn populate_man_pages_creates_files() {
        let mut vfs = MemoryVfs::new();
        populate_man_pages(&mut vfs);
        assert!(vfs.exists("/usr/share/man/ls.txt"));
        assert!(vfs.exists("/usr/share/man/grep.txt"));
        assert!(vfs.exists("/usr/share/man/tutorial.txt"));
    }

    #[test]
    fn populate_profile_creates_file() {
        let mut vfs = MemoryVfs::new();
        populate_profile(&mut vfs);
        assert!(vfs.exists("/home/.profile"));
        let data = vfs.read("/home/.profile").unwrap();
        let text = String::from_utf8_lossy(&data);
        assert!(text.contains("USER=oasis"));
    }
}
