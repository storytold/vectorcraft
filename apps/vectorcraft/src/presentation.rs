//! `--present-mode <mode>` (or `VECTORCRAFT_PRESENT_MODE`) and `--frame-latency <frames>` (or
//! `VECTORCRAFT_FRAME_LATENCY`) set how the window's frames reach the display for one launch. The
//! modes are `fifo` (vsync), `autovsync` (the default: wgpu picks relaxed vsync, which tears a late
//! frame, else vsync, by what the driver offers) and `novsync` (wgpu picks Immediate, else Mailbox,
//! else Fifo). A mode the driver may not offer is never requested. The latency is the number of
//! frames the swap chain may queue, 1 to 3. Unset, the app keeps its own surface settings. A flag
//! overrides its variable; a value that isn't one of these stops the app with a message, as
//! `--control` does. A launch that sets either one runs as its own app, as with `--new-instance`.
//! The surface the app asks for is logged at start-up.

use std::ffi::OsString;

/// A present mode the command line or the environment can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fifo,
    AutoVsync,
    NoVsync,
}

/// The present mode and frame latency this launch asked for (`None`: the app's own setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overrides {
    pub mode: Option<Mode>,
    pub latency: Option<u32>,
}

/// A present mode named by `name` (the flag or the variable): `fifo`, `autovsync` or `novsync`, any case.
fn mode(name: &str, value: Option<&str>) -> Result<Mode, String> {
    let need = format!("{name} needs fifo, autovsync or novsync");
    let Some(v) = value else { return Err(need) };
    match v.trim().to_ascii_lowercase().as_str() {
        "fifo" => Ok(Mode::Fifo),
        "autovsync" => Ok(Mode::AutoVsync),
        "novsync" => Ok(Mode::NoVsync),
        _ => Err(format!("{need}, not `{v}`")),
    }
}

/// A frame latency named by `name`: 1, 2 or 3.
fn latency(name: &str, value: Option<&str>) -> Result<u32, String> {
    let need = format!("{name} needs 1, 2 or 3 frames");
    let Some(v) = value else { return Err(need) };
    v.trim().parse().ok().filter(|n| (1..=3).contains(n)).ok_or(format!("{need}, not `{v}`"))
}

/// The value of `--flag <value>` or `--flag=<value>` when `arg` is that flag; `None` when it isn't.
fn flag_value(flag: &str, arg: &str, args: &mut impl Iterator<Item = String>) -> Option<Option<String>> {
    match arg.strip_prefix(flag)? {
        "" => Some(args.next()),
        rest => rest.strip_prefix('=').map(|v| Some(v.to_owned())),
    }
}

impl Overrides {
    /// Whether a flag or a variable set the present mode or the frame latency.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // its one caller, in `main`, is not built on macOS
    pub fn is_set(&self) -> bool {
        *self != Self::default()
    }

    /// Returns whether this argument is `--present-mode` or `--frame-latency`. The next argument is
    /// its value whatever it is, as with `--control`.
    pub fn consume(&mut self, arg: &str, args: &mut impl Iterator<Item = String>) -> Result<bool, String> {
        if let Some(value) = flag_value("--present-mode", arg, args) {
            if self.mode.is_some() {
                return Err("--present-mode given twice".into());
            }
            self.mode = Some(mode("--present-mode", value.as_deref())?);
            return Ok(true);
        }
        if let Some(value) = flag_value("--frame-latency", arg, args) {
            if self.latency.is_some() {
                return Err("--frame-latency given twice".into());
            }
            self.latency = Some(latency("--frame-latency", value.as_deref())?);
            return Ok(true);
        }
        Ok(false)
    }

    /// Fills what the flags left unset from `VECTORCRAFT_PRESENT_MODE` and `VECTORCRAFT_FRAME_LATENCY`.
    /// Unset or empty is no override; a variable is not read when its flag was given.
    pub fn with_env(self, mode_var: Option<OsString>, latency_var: Option<OsString>) -> Result<Self, String> {
        let set = |v: Option<OsString>| v.filter(|v| !v.is_empty()).map(|v| v.to_string_lossy().into_owned());
        let mode = match (self.mode, set(mode_var)) {
            (Some(m), _) => Some(m),
            (None, Some(v)) => Some(mode("VECTORCRAFT_PRESENT_MODE", Some(&v))?),
            (None, None) => None,
        };
        let latency = match (self.latency, set(latency_var)) {
            (Some(n), _) => Some(n),
            (None, Some(v)) => Some(latency("VECTORCRAFT_FRAME_LATENCY", Some(&v))?),
            (None, None) => None,
        };
        Ok(Self { mode, latency })
    }

    /// `surface` with the overrides applied.
    #[cfg(feature = "wgpu")]
    pub fn apply(&self, surface: eframe::egui_wgpu::SurfaceConfig) -> eframe::egui_wgpu::SurfaceConfig {
        use eframe::wgpu::PresentMode;
        let present_mode = match self.mode {
            Some(Mode::Fifo) => PresentMode::Fifo,
            Some(Mode::AutoVsync) => PresentMode::AutoVsync,
            Some(Mode::NoVsync) => PresentMode::AutoNoVsync,
            None => surface.present_mode,
        };
        eframe::egui_wgpu::SurfaceConfig { present_mode, desired_maximum_frame_latency: self.latency.or(surface.desired_maximum_frame_latency) }
    }
}

/// The present mode and frame latency `surface` asks for, for the start-up log:
/// "autovsync, frame latency 1", with "(override)"
/// after a value a flag or variable set.
#[cfg(feature = "wgpu")]
pub fn describe(surface: &eframe::egui_wgpu::SurfaceConfig, overrides: &Overrides) -> String {
    use eframe::wgpu::PresentMode;
    let mode = match surface.present_mode {
        PresentMode::AutoVsync => "autovsync",
        PresentMode::AutoNoVsync => "novsync",
        PresentMode::Fifo => "fifo",
        PresentMode::FifoRelaxed => "fiforelaxed",
        PresentMode::Immediate => "immediate",
        PresentMode::Mailbox => "mailbox",
    };
    let latency = surface.desired_maximum_frame_latency.map_or("default".to_string(), |n| n.to_string());
    let tag = |set: bool| if set { " (override)" } else { "" };
    format!("{mode}{}, frame latency {latency}{}", tag(overrides.mode.is_some()), tag(overrides.latency.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Overrides, String> {
        let mut overrides = Overrides::default();
        let mut args = args.iter().map(|a| a.to_string());
        while let Some(a) = args.next() {
            assert!(overrides.consume(&a, &mut args)?, "{a}");
        }
        Ok(overrides)
    }

    #[test]
    fn flags_take_the_three_modes_and_latencies_from_1_to_3() {
        assert_eq!(parse(&[]), Ok(Overrides::default()));
        assert_eq!(parse(&["--present-mode", "fifo"]).map(|o| o.mode), Ok(Some(Mode::Fifo)));
        assert_eq!(parse(&["--present-mode=AutoVsync"]).map(|o| o.mode), Ok(Some(Mode::AutoVsync)));
        assert_eq!(parse(&["--present-mode", " novsync "]).map(|o| o.mode), Ok(Some(Mode::NoVsync)));
        for n in 1..=3 {
            assert_eq!(parse(&["--frame-latency", &n.to_string()]).map(|o| o.latency), Ok(Some(n)));
        }
        assert_eq!(parse(&["--frame-latency=2", "--present-mode", "fifo"]), Ok(Overrides { mode: Some(Mode::Fifo), latency: Some(2) }));
    }

    #[test]
    fn bad_flag_values_are_errors() {
        let modes = "--present-mode needs fifo, autovsync or novsync";
        assert_eq!(parse(&["--present-mode"]), Err(modes.into()));
        for bad in ["mailbox", "immediate", "vsync", "fast", ""] {
            assert_eq!(parse(&["--present-mode", bad]), Err(format!("{modes}, not `{bad}`")));
            assert_eq!(parse(&[&format!("--present-mode={bad}")]), Err(format!("{modes}, not `{bad}`")));
        }
        assert_eq!(parse(&["--present-mode", "--frame-latency"]), Err(format!("{modes}, not `--frame-latency`")));
        let frames = "--frame-latency needs 1, 2 or 3 frames";
        assert_eq!(parse(&["--frame-latency"]), Err(frames.into()));
        for bad in ["0", "4", "-1", "one", "1.5", "99999999999", ""] {
            assert_eq!(parse(&["--frame-latency", bad]), Err(format!("{frames}, not `{bad}`")));
        }
        assert_eq!(parse(&["--present-mode", "fifo", "--present-mode=novsync"]), Err("--present-mode given twice".into()));
        assert_eq!(parse(&["--frame-latency=1", "--frame-latency", "2"]), Err("--frame-latency given twice".into()));
    }

    #[test]
    fn other_arguments_are_not_presentation_flags() {
        let mut overrides = Overrides::default();
        let mut args = ["fifo".to_owned()].into_iter();
        for other in ["--present-modes", "--present", "--frame-latency-2", "drawing.svg", "--control"] {
            assert!(!overrides.consume(other, &mut args).unwrap(), "{other}");
        }
        assert_eq!(args.next().as_deref(), Some("fifo"));
        assert_eq!(overrides, Overrides::default());
    }

    #[test]
    fn variables_fill_what_the_flags_leave_unset_and_bad_values_are_errors() {
        let none = Overrides::default();
        assert_eq!(none.with_env(None, None), Ok(none));
        assert_eq!(none.with_env(Some("".into()), Some("".into())), Ok(none));
        assert_eq!(none.with_env(Some("FIFO".into()), Some("2".into())), Ok(Overrides { mode: Some(Mode::Fifo), latency: Some(2) }));
        assert_eq!(
            none.with_env(Some("mailbox".into()), None),
            Err("VECTORCRAFT_PRESENT_MODE needs fifo, autovsync or novsync, not `mailbox`".into())
        );
        assert_eq!(none.with_env(None, Some("0".into())), Err("VECTORCRAFT_FRAME_LATENCY needs 1, 2 or 3 frames, not `0`".into()));
        // A flag wins, and its variable is not read, so a bad value there doesn't stop the app.
        let flags = Overrides { mode: Some(Mode::NoVsync), latency: Some(3) };
        assert_eq!(flags.with_env(Some("bad".into()), Some("9".into())), Ok(flags));
        let mode_only = Overrides { mode: Some(Mode::AutoVsync), latency: None };
        assert_eq!(mode_only.with_env(Some("bad".into()), Some("1".into())), Ok(Overrides { mode: Some(Mode::AutoVsync), latency: Some(1) }));
        // Either value set makes the launch run as its own app.
        assert!(!none.is_set());
        assert!(mode_only.is_set());
        assert!(Overrides { mode: None, latency: Some(1) }.is_set());
    }

    /// `--help` states the app's own surface as the default, so it changes with it.
    #[cfg(feature = "wgpu")]
    #[test]
    fn the_usage_names_the_app_surface_as_the_default() {
        let app = eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY;
        let usage = crate::USAGE.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(app.present_mode, eframe::wgpu::PresentMode::AutoVsync, "the usage calls autovsync the default");
        assert!(usage.contains("autovsync (the default"), "{usage}");
        let Some(frames) = app.desired_maximum_frame_latency else { panic!("the usage gives a default frame latency") };
        assert!(usage.contains(&format!("(the default is {frames})")), "{usage}");
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn overrides_change_only_what_they_set_on_the_app_surface() {
        use eframe::egui_wgpu::SurfaceConfig;
        use eframe::wgpu::PresentMode;
        let app = SurfaceConfig::LOW_LATENCY;
        // No override: the surface the app asks for is unchanged.
        assert_eq!(Overrides::default().apply(app), app);
        let latency = Overrides { mode: None, latency: Some(2) };
        let s = latency.apply(app);
        assert_eq!((s.present_mode, s.desired_maximum_frame_latency), (PresentMode::AutoVsync, Some(2)));
        assert_eq!(describe(&s, &latency), "autovsync, frame latency 2 (override)");
        let fifo = Overrides { mode: Some(Mode::Fifo), latency: None };
        let s = fifo.apply(app);
        assert_eq!((s.present_mode, s.desired_maximum_frame_latency), (PresentMode::Fifo, app.desired_maximum_frame_latency));
        // Never a mode the driver may lack: no vsync lets wgpu fall back by support.
        let novsync = Overrides { mode: Some(Mode::NoVsync), latency: Some(3) };
        let s = novsync.apply(app);
        assert_eq!((s.present_mode, s.desired_maximum_frame_latency), (PresentMode::AutoNoVsync, Some(3)));
        assert_eq!(describe(&s, &novsync), "novsync (override), frame latency 3 (override)");
        let unset = SurfaceConfig { present_mode: PresentMode::Fifo, desired_maximum_frame_latency: None };
        assert_eq!(describe(&unset, &Overrides::default()), "fifo, frame latency default");
    }
}
