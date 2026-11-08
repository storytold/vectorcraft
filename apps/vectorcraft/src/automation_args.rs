//! Parse the control port and automation roots before starting any desktop services. A malformed
//! flag must not silently remove a root and leave the control channel unconfined, or leave the app
//! running without the control channel that was asked for.

use std::ffi::OsString;
use std::path::PathBuf;

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

/// `--control <port>` or `--control=<port>`, given at most once.
#[derive(Default)]
pub struct ControlArg {
    pub port: Option<u16>,
}

impl ControlArg {
    /// Returns whether this argument is `--control`. The next argument is its port whatever it
    /// is, so `--control --automation-read-root` reports the flag rather than reading it as a root.
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        let value = match arg.strip_prefix("--control") {
            Some("") => args.next(),
            Some(rest) => match rest.strip_prefix('=') {
                Some(value) => Some(value.to_owned()),
                None => return Ok(false),
            },
            None => return Ok(false),
        };
        if self.port.is_some() {
            return Err("--control given twice".into());
        }
        self.port = Some(port("--control", value.as_deref())?);
        Ok(true)
    }
}

/// `VECTORCRAFT_CONTROL_PORT`, for when `--control` isn't given: unset or empty is no control
/// channel, anything else must be a port.
pub fn env_control_port(value: Option<OsString>) -> Result<Option<u16>, String> {
    match value {
        Some(v) if !v.is_empty() => port("VECTORCRAFT_CONTROL_PORT", Some(&v.to_string_lossy())).map(Some),
        _ => Ok(None),
    }
}

/// A port from 1 to 65535 (the server would listen on 0's random port without saying which).
fn port(name: &str, value: Option<&str>) -> Result<u16, String> {
    let need = format!("{name} needs a port number from 1 to 65535");
    match value {
        None => Err(need),
        Some(v) => v.parse().ok().filter(|&p| p != 0).ok_or(format!("{need}, not `{v}`")),
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

    fn control(args: &[&str]) -> Result<Option<u16>, String> {
        let mut control = ControlArg::default();
        let mut args = args.iter().map(|a| a.to_string());
        while let Some(a) = args.next() {
            assert!(control.consume(&a, &mut args)?, "{a}");
        }
        Ok(control.port)
    }

    #[test]
    fn control_needs_a_port_from_1_to_65535() {
        let need = "--control needs a port number from 1 to 65535";
        assert_eq!(control(&["--control"]), Err(need.into()));
        assert_eq!(control(&["--control", "--automation-read-root"]), Err(format!("{need}, not `--automation-read-root`")));
        for bad in ["abc", "0", "70000", "-1", ""] {
            assert_eq!(control(&["--control", bad]), Err(format!("{need}, not `{bad}`")));
            assert_eq!(control(&[&format!("--control={bad}")]), Err(format!("{need}, not `{bad}`")));
        }
        assert_eq!(control(&["--control", "7979"]), Ok(Some(7979)));
        assert_eq!(control(&["--control=65535"]), Ok(Some(65535)));
        assert_eq!(control(&["--control", "7979", "--control=7980"]), Err("--control given twice".into()));
    }

    #[test]
    fn other_arguments_are_not_control() {
        let mut control = ControlArg::default();
        let mut args = ["7979".to_owned()].into_iter();
        for other in ["--controls", "--control-port", "drawing.svg", "--in-window-menus"] {
            assert!(!control.consume(other, &mut args).unwrap(), "{other}");
        }
        assert_eq!(args.next().as_deref(), Some("7979"));
        assert_eq!(control.port, None);
    }

    #[test]
    fn control_port_variable_is_a_port_or_unset() {
        assert_eq!(env_control_port(None), Ok(None));
        assert_eq!(env_control_port(Some("".into())), Ok(None));
        assert_eq!(env_control_port(Some("7979".into())), Ok(Some(7979)));
        let need = "VECTORCRAFT_CONTROL_PORT needs a port number from 1 to 65535";
        assert_eq!(env_control_port(Some("abc".into())), Err(format!("{need}, not `abc`")));
        assert_eq!(env_control_port(Some("0".into())), Err(format!("{need}, not `0`")));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            assert_eq!(env_control_port(Some(OsString::from_vec(vec![0x39, 0xff]))), Err(format!("{need}, not `9\u{fffd}`")));
        }
    }
}
