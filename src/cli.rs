//! Command-line validation: `main` ignores what it does not know, so this
//! refuses `shmc confg` before anything has a side effect.

const COMMANDS: &[&str] = &["config", "setup", "cache", "log"];

/// For the "did you mean" line. `-h` and `-V` are left out: any two
/// one-character flags are within the edit distance, so `-x` would suggest `-V`.
const OPTIONS: &[&str] = &["--version", "--help", "--config", "--check", "--list"];

const CONFIG_OPTIONS: &[&str] = &["--check", "--list"];

const HINT: &str = "Try 'shmc --help' for the list of commands.";

/// `Err` is the message for stderr, already formatted.
pub fn validate(args: &[String]) -> Result<(), String> {
    let mut command: Option<&str> = None;
    let mut config_options: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = args[i].as_str();

        match arg {
            "--version" | "-V" | "--help" | "-h" => {
                i += 1;
                continue;
            }
            "--config" => {
                if i + 1 >= args.len() {
                    return Err(format!("shmc: '--config' needs a file name\n{HINT}"));
                }
                // The value is a path, not a command.
                i += 2;
                continue;
            }
            _ => {}
        }

        if let Some(opt) = CONFIG_OPTIONS.iter().find(|o| **o == arg) {
            config_options.push(opt);
            i += 1;
            continue;
        }

        if let Some(value) = arg.strip_prefix("--config=") {
            if value.is_empty() {
                return Err(format!("shmc: '--config' needs a file name\n{HINT}"));
            }
            i += 1;
            continue;
        }

        if let Some(cmd) = COMMANDS.iter().find(|c| **c == arg) {
            if let Some(prev) = command {
                return Err(format!("shmc: '{prev}' and '{cmd}' cannot be combined\n{HINT}"));
            }
            command = Some(cmd);
            i += 1;
            continue;
        }

        if let Some(cmd) = command {
            match cmd {
                "cache" => {
                    if arg == "clear" {
                        i += 1;
                        continue;
                    }
                    return Err(format!(
                        "shmc cache: unknown argument '{arg}'\nusage: shmc cache [clear]"
                    ));
                }
                "log" => {
                    if arg.parse::<usize>().is_ok() {
                        i += 1;
                        continue;
                    }
                    return Err(format!(
                        "shmc log: expected a number of lines, got '{arg}'\nusage: shmc log [N]"
                    ));
                }
                _ => {}
            }
            if is_option(arg) {
                return Err(unknown_option(arg));
            }
            return Err(format!("shmc {cmd}: unknown argument '{arg}'\n{HINT}"));
        }

        if is_option(arg) {
            return Err(unknown_option(arg));
        }
        return Err(format!("shmc: unknown command '{arg}'{}\n{HINT}", suggestion(arg, COMMANDS)));
    }

    if config_options.len() > 1 {
        return Err(format!(
            "shmc: '{}' and '{}' cannot be combined\n{HINT}",
            config_options[0], config_options[1]
        ));
    }
    if !config_options.is_empty() && command != Some("config") {
        return Err(format!("shmc: '{}' needs 'shmc config'\n{HINT}", config_options[0]));
    }
    Ok(())
}

/// A bare `-` is a word.
fn is_option(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-')
}

fn unknown_option(arg: &str) -> String {
    format!("shmc: unknown option '{arg}'{}\n{HINT}", suggestion(arg, OPTIONS))
}

/// A near miss on a command or option: `confg` → `config`, `--chek` → `--check`.
fn suggestion(word: &str, candidates: &[&str]) -> String {
    let lower = word.to_lowercase();
    let mut best: Option<&str> = None;
    let mut best_distance = usize::MAX;
    for candidate in candidates {
        let distance = edit_distance(&lower, candidate);
        if distance <= 2 && distance < best_distance {
            best = Some(candidate);
            best_distance = distance;
        }
    }
    match best {
        Some(candidate) => format!("\n  did you mean '{candidate}'?"),
        None => String::new(),
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j + 1] + 1).min(current[j] + 1).min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn documented_invocations_accepted() {
        for v in [
            &[][..],
            &["config"][..],
            &["config", "--check"][..],
            &["config", "--list"][..],
            &["setup"][..],
            &["cache"][..],
            &["cache", "clear"][..],
            &["log"][..],
            &["log", "30"][..],
            &["--version"][..],
            &["--help"][..],
            &["-h"][..],
            &["--config", "/tmp/shmc.toml", "config", "--check"][..],
        ] {
            assert!(validate(&args(v)).is_ok(), "{v:?} should be accepted");
        }
    }

    #[test]
    fn unknown_command_suggests_near_miss() {
        let err = validate(&args(&["confg"])).unwrap_err();
        assert!(err.contains("unknown command 'confg'"), "{err}");
        assert!(err.contains("did you mean 'config'"), "{err}");

        // Only the case differs.
        let err = validate(&args(&["Config"])).unwrap_err();
        assert!(err.contains("did you mean 'config'"), "{err}");

        let err = validate(&args(&["doctor"])).unwrap_err();
        assert!(err.contains("unknown command 'doctor'"), "{err}");

        let err = validate(&args(&["random"])).unwrap_err();
        assert!(err.contains("unknown command 'random'"), "{err}");
        assert!(!err.contains("did you mean"), "{err}");
    }

    #[test]
    fn unknown_options_are_refused() {
        let err = validate(&args(&["--nonsense"])).unwrap_err();
        assert!(err.contains("unknown option '--nonsense'"), "{err}");

        let err = validate(&args(&["--chek"])).unwrap_err();
        assert!(err.contains("did you mean '--check'"), "{err}");
    }

    #[test]
    fn config_options_need_config_command() {
        let err = validate(&args(&["--check"])).unwrap_err();
        assert!(err.contains("needs 'shmc config'"), "{err}");

        let err = validate(&args(&["config", "--check", "--list"])).unwrap_err();
        assert!(err.contains("cannot be combined"), "{err}");
    }

    #[test]
    fn two_commands_cannot_be_combined() {
        let err = validate(&args(&["config", "log"])).unwrap_err();
        assert!(err.contains("cannot be combined"), "{err}");
    }

    #[test]
    fn config_path_is_not_a_command() {
        // the value of `--config` is not a command
        assert!(validate(&args(&["--config", "/tmp/log", "config"])).is_ok());
        assert!(validate(&args(&["--config=/tmp/log", "config"])).is_ok());
        let err = validate(&args(&["--config"])).unwrap_err();
        assert!(err.contains("needs a file name"), "{err}");
        let err = validate(&args(&["--config="])).unwrap_err();
        assert!(err.contains("needs a file name"), "{err}");
    }

    #[test]
    fn subcommands_reject_stray_arguments() {
        let err = validate(&args(&["cache", "clean"])).unwrap_err();
        assert!(err.contains("unknown argument 'clean'"), "{err}");

        let err = validate(&args(&["log", "abc"])).unwrap_err();
        assert!(err.contains("number of lines"), "{err}");

        // A negative count is a bad argument, not an unknown option.
        let err = validate(&args(&["log", "-5"])).unwrap_err();
        assert!(err.contains("number of lines"), "{err}");
        assert!(!err.contains("unknown option"), "{err}");
    }
}
