//! Parse the control flags and automation roots before starting any desktop services. A malformed
//! flag must not silently remove a root and leave the control channel unconfined, or leave the app
//! running without the control channel that was asked for.

use std::path::PathBuf;

use craft::automation::args::{ControlOptions, OptionsError, Source, control_port};

pub struct AutomationArgs {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    read_given: bool,
    write_given: bool,
}

impl AutomationArgs {
    /// The roots the environment gives (`VECTORCRAFT_AUTOMATION_READ_ROOT` / `_WRITE_ROOT`), until
    /// the command line gives its own.
    pub fn new(read_root: Option<PathBuf>, write_root: Option<PathBuf>) -> Self {
        Self { read_roots: read_root.into_iter().collect(), write_roots: write_root.into_iter().collect(), read_given: false, write_given: false }
    }

    /// Returns whether this argument is a root flag. Each flag may be given more than once (#3546),
    /// in either spelling: the first replaces the environment's root of its kind, the others add
    /// folders.
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let (flag, inline) = arg.split_once('=').map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let (roots, given) = match flag {
            "--automation-read-root" => (&mut self.read_roots, &mut self.read_given),
            "--automation-write-root" => (&mut self.write_roots, &mut self.write_given),
            _ => return Ok(false),
        };
        let value = match inline {
            Some(value) => value.to_owned(),
            None => {
                let value = args.next().ok_or_else(|| format!("{flag} needs a folder"))?;
                if value.starts_with('-') {
                    return Err(format!("{flag} needs a folder; use {flag}=<dir> for a folder starting with '-'"));
                }
                value
            }
        };
        if value.is_empty() {
            return Err(format!("{flag} needs a non-empty folder"));
        }
        if !std::mem::replace(given, true) {
            roots.clear();
        }
        roots.push(value.into());
        Ok(true)
    }
}

/// `--control <port>`, `--control-token <token>` and `--control-token-file <file>`, each in either
/// spelling (`--control=<port>`) and given at most once.
#[derive(Default)]
pub struct ControlArg {
    options: ControlOptions,
}

impl ControlArg {
    /// What the command line gave, each value marked as a flag's, for
    /// [`ControlOptions::resolve`] to put over the environment.
    pub fn options(&self) -> ControlOptions {
        self.options.clone()
    }

    /// Returns whether this argument is one of the control flags. The next argument is the value
    /// whatever it is, so `--control --automation-read-root` reports the flag rather than reading
    /// it as a root.
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let (flag, inline) = arg.split_once('=').map_or((arg, None), |(flag, value)| (flag, Some(value)));
        let given = match flag {
            "--control" => self.options.port.is_some(),
            "--control-token" => self.options.token.is_some(),
            "--control-token-file" => self.options.token_file.is_some(),
            _ => return Ok(false),
        };
        let value = match inline {
            Some(value) => value.to_owned(),
            None => args.next().ok_or_else(|| format!("a value is required for '{flag}' but none was supplied"))?,
        };
        if given {
            return Err(format!("the argument '{flag}' cannot be used multiple times"));
        }
        let options = &mut self.options;
        let from_flag = Some(Source::Flag);
        match flag {
            "--control" => {
                let not_a_port = || OptionsError::NotAPort { value: value.clone(), shown: "--control <PORT>".into() }.to_string();
                options.port = Some(control_port(&value).map_err(|_| not_a_port())?);
                options.sources.port = from_flag;
            }
            // A blank token or path would leave the channel open to anyone or name no file.
            _ if value.trim().is_empty() => return Err(format!("a value is required for '{flag}' but none was supplied")),
            "--control-token" => {
                options.token = Some(value);
                options.sources.token = from_flag;
            }
            _ => {
                options.token_file = Some(PathBuf::from(value));
                options.sources.token_file = from_flag;
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_override_environment_and_add_up() {
        let mut roots = AutomationArgs::new(Some("environment".into()), Some("environment out".into()));
        let mut args = ["a folder".to_owned(), "drawing.svg".to_owned()].into_iter();
        assert!(roots.consume("--automation-read-root", &mut args).unwrap());
        assert!(roots.consume("--automation-write-root=out", &mut args).unwrap());
        assert!(!roots.consume("--in-window-menus", &mut args).unwrap());
        assert_eq!(roots.read_roots, [PathBuf::from("a folder")]);
        assert_eq!(roots.write_roots, [PathBuf::from("out")]);
        assert_eq!(args.next().as_deref(), Some("drawing.svg"));
        // #3546: a flag given again adds a folder, in either spelling.
        assert!(roots.consume("--automation-read-root=other", &mut args).unwrap());
        assert!(roots.consume("--automation-write-root", &mut ["out2".to_owned()].into_iter()).unwrap());
        assert_eq!(roots.read_roots, [PathBuf::from("a folder"), PathBuf::from("other")]);
        assert_eq!(roots.write_roots, [PathBuf::from("out"), PathBuf::from("out2")]);
        // A kind the command line leaves alone keeps the environment's root.
        let mut roots = AutomationArgs::new(Some("environment".into()), Some("environment out".into()));
        assert!(roots.consume("--automation-read-root=a", &mut std::iter::empty()).unwrap());
        assert_eq!(roots.write_roots, [PathBuf::from("environment out")]);
    }

    #[test]
    fn invalid_operands_preserve_environment_roots() {
        for flag in ["--automation-read-root", "--automation-write-root"] {
            for operand in [None, Some(""), Some("--version"), Some("--control"), Some("-folder")] {
                let mut roots = AutomationArgs::new(Some("read".into()), Some("write".into()));
                let mut args = operand.map(str::to_owned).into_iter();
                assert!(roots.consume(flag, &mut args).is_err(), "{flag} {operand:?}");
                assert_eq!(roots.read_roots, [PathBuf::from("read")]);
                assert_eq!(roots.write_roots, [PathBuf::from("write")]);
            }
            let mut roots = AutomationArgs::new(None, None);
            assert!(roots.consume(&format!("{flag}="), &mut std::iter::empty()).is_err());
            // A bad second folder is an error too, and keeps the first.
            assert!(roots.consume(&format!("{flag}=first"), &mut std::iter::empty()).unwrap());
            assert!(roots.consume(flag, &mut std::iter::empty()).is_err());
            assert_eq!([roots.read_roots.len(), roots.write_roots.len()].iter().sum::<usize>(), 1);
        }
    }

    #[test]
    fn explicit_dash_folder_and_relative_dash_folder_are_valid() {
        let mut roots = AutomationArgs::new(None, None);
        assert!(roots.consume("--automation-read-root=-folder", &mut std::iter::empty()).unwrap());
        assert!(roots.consume("--automation-write-root", &mut ["./-folder".to_owned()].into_iter()).unwrap());
        assert_eq!(roots.read_roots, [PathBuf::from("-folder")]);
        assert_eq!(roots.write_roots, [PathBuf::from("./-folder")]);
    }

    fn control(args: &[&str]) -> Result<ControlOptions, String> {
        let mut control = ControlArg::default();
        let mut args = args.iter().map(|a| a.to_string());
        while let Some(a) = args.next() {
            assert!(control.consume(&a, &mut args)?, "{a}");
        }
        Ok(control.options())
    }

    #[test]
    fn a_control_flag_takes_the_next_argument_whatever_it_is() {
        assert!(control(&["--control"]).is_err(), "a flag with no value");
        let err = control(&["--control", "--automation-read-root"]).unwrap_err();
        assert!(err.contains("--automation-read-root"), "the next argument is the port, not a root flag: {err}");
        for bad in ["abc", "70000", "-1", ""] {
            assert!(control(&["--control", bad]).is_err(), "{bad}");
            assert!(control(&[&format!("--control={bad}")]).is_err(), "{bad}");
        }
        for twice in [
            ["--control", "7979", "--control=7980"],
            ["--control-token", "a", "--control-token=b"],
            ["--control-token-file", "a", "--control-token-file=b"],
        ] {
            assert!(control(&twice).is_err(), "{twice:?}");
        }
        assert!(control(&["--control-token="]).is_err(), "an empty token");
        let port = control(&["--control", "7979"]).unwrap();
        assert_eq!(port, control(&["--control=7979"]).unwrap(), "both spellings agree");
        assert_eq!((port.port, port.sources.port), (Some(7979), Some(Source::Flag)));
        assert_eq!(control(&["--control=0"]).unwrap().port, Some(0), "port 0 picks a free port");
        let secret = control(&["--control-token", "--odd", "--control-token-file=key.txt"]).unwrap();
        assert_eq!((secret.token.as_deref(), secret.token_file), (Some("--odd"), Some(PathBuf::from("key.txt"))));
    }

    #[test]
    fn other_arguments_are_not_control() {
        let mut control = ControlArg::default();
        let mut args = ["7979".to_owned()].into_iter();
        for other in ["--controls", "--control-port", "drawing.svg", "--in-window-menus"] {
            assert!(!control.consume(other, &mut args).unwrap(), "{other}");
        }
        assert_eq!(args.next().as_deref(), Some("7979"));
        assert_eq!(control.options(), ControlOptions::default());
        for token_flag in ["--control-token", "--control-token-file"] {
            assert!(ControlArg::default().consume(token_flag, &mut ["x".to_owned()].into_iter()).unwrap(), "{token_flag}");
        }
    }
}
