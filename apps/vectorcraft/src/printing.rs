//! Desktop printer transport. VectorCraft composes every page and copy before submission;
//! craft-print owns discovery and spooling. Windows 7 keeps its viewer-based transport because
//! Windows.Data.Pdf is unavailable there.
#[cfg(not(all(windows, any(feature = "windows7", target_vendor = "win7"))))]
use std::process::Command;
use vectorcraft_ui_egui::print::{PrintJob, PrintService, Printer};

#[cfg(all(windows, any(feature = "windows7", target_vendor = "win7")))]
mod windows7;
#[cfg(all(windows, any(feature = "windows7", target_vendor = "win7")))]
use windows7 as system;

pub struct SystemPrint;
impl PrintService for SystemPrint {
    fn printers(&mut self) -> Vec<Printer> {
        system::printers()
    }
    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        let result = system::print(job);
        let to = job.printer.unwrap_or("the default printer");
        result.map(|note| format!("Sent “{}” to {to}{note}", job.title))
    }
    fn shared_print(&self) -> Option<vectorcraft_ui_egui::print::SharedPrintFn> {
        Some(std::sync::Arc::new(|job| SystemPrint.print(job)))
    }
    fn has_setup(&self) -> bool {
        true
    }
    fn setup(&mut self, printer: Option<&str>) -> Result<(), String> {
        system::setup(printer)
    }
}

#[cfg(not(all(windows, any(feature = "windows7", target_vendor = "win7"))))]
mod system {
    use super::*;
    pub fn printers() -> Vec<Printer> {
        craft_print::printers().into_iter().map(|p| Printer { name: p.name, default: p.default }).collect()
    }
    pub fn print(job: &PrintJob) -> Result<String, String> {
        craft_print::submit(job.pdf, &queue_job(job)).map(|note| if note.is_empty() { note } else { format!(" ({note})") }).map_err(|e| e.to_string())
    }
    /// The queue job for `job`. Copies, layout, marks, colour management and gray are already in
    /// the PDF, and the queue's duplex default stays. A paper the person chose is demanded exactly
    /// (a queue without it refuses); otherwise the pages are the document's own size (an artboard,
    /// a custom size), which a queue may not list: the job goes on the queue's default paper, shrunk
    /// to fit. Grayscale is also asked of the driver, which may ignore it: the PDF is gray itself.
    pub fn queue_job(job: &PrintJob) -> craft_print::Job {
        craft_print::Job {
            printer: job.printer.map(str::to_string),
            title: job.title.into(),
            grayscale: job.grayscale,
            scaling: if job.paper_chosen { craft_print::Scaling::Exact } else { craft_print::Scaling::ShrinkToFit },
            ..Default::default()
        }
    }
    #[cfg(windows)]
    pub(crate) fn setup_command(printer: Option<&str>, default_printer: Option<&str>) -> Command {
        let target = printer.map(str::trim).filter(|p| !p.is_empty()).or_else(|| default_printer.map(str::trim).filter(|p| !p.is_empty()));
        if let Some(name) = target {
            let mut cmd = Command::new("rundll32.exe");
            cmd.args(["printui.dll,PrintUIEntry", "/e", "/n", name]);
            cmd
        } else {
            let mut cmd = Command::new("explorer");
            cmd.arg("ms-settings:printers");
            cmd
        }
    }
    pub fn setup(printer: Option<&str>) -> Result<(), String> {
        #[cfg(windows)]
        {
            let list = printers();
            let default = list.iter().find(|p| p.default).or_else(|| list.first()).map(|p| p.name.as_str());
            // Only an installed printer's own name reaches the command line: `printer` can come
            // from a command or MCP, and PrintUIEntry reads options from its arguments.
            let printer = printer.map(str::trim).filter(|p| list.iter().any(|q| q.name == *p));
            let mut cmd = setup_command(printer, default);
            match cmd.spawn() {
                Ok(_) => Ok(()),
                Err(_) => Command::new("explorer").arg("ms-settings:printers").spawn().map(|_| ()).map_err(|e| e.to_string()),
            }
        }
        #[cfg(target_os = "macos")]
        {
            let _ = printer;
            Command::new("open")
                .arg("x-apple.systempreferences:com.apple.Print-Scan-Settings.extension")
                .spawn()
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            if Command::new("system-config-printer").spawn().is_ok() {
                return Ok(());
            }
            let url = printer.map_or_else(|| "http://localhost:631/printers/".to_string(), |p| format!("http://localhost:631/printers/{p}"));
            Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
    }
}

#[cfg(all(test, windows, not(any(feature = "windows7", target_vendor = "win7"))))]
mod tests {
    use super::system::setup_command;
    use std::ffi::OsStr;

    #[test]
    fn setup_command_uses_printui_for_specified_printer() {
        let cmd = setup_command(Some("Office Laser"), None);
        assert_eq!(cmd.get_program(), "rundll32.exe");
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(args, ["printui.dll,PrintUIEntry", "/e", "/n", "Office Laser"]);
    }

    #[test]
    fn setup_command_falls_back_to_default_printer() {
        let cmd = setup_command(None, Some("Default Proofer"));
        assert_eq!(cmd.get_program(), "rundll32.exe");
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(args, ["printui.dll,PrintUIEntry", "/e", "/n", "Default Proofer"]);
    }

    #[test]
    fn setup_command_trims_whitespace_and_skips_empty() {
        let cmd = setup_command(Some("  "), Some("Default Proofer"));
        assert_eq!(cmd.get_program(), "rundll32.exe");
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(args, ["printui.dll,PrintUIEntry", "/e", "/n", "Default Proofer"]);
    }

    #[test]
    fn setup_command_falls_back_to_settings_when_no_printer() {
        let cmd = setup_command(None, None);
        assert_eq!(cmd.get_program(), "explorer");
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(args, ["ms-settings:printers"]);
    }
}

#[cfg(all(test, not(all(windows, any(feature = "windows7", target_vendor = "win7")))))]
mod queue_tests {
    use super::system::queue_job;
    use super::*;

    fn job(grayscale: bool, paper_chosen: bool) -> PrintJob<'static> {
        PrintJob { printer: Some("Office"), title: "Doc", pdf: b"%PDF", grayscale, paper_chosen }
    }

    #[test]
    fn a_document_sized_job_goes_on_the_queues_own_paper_shrunk_to_fit() {
        let q = queue_job(&job(false, false));
        assert_eq!((q.scaling, q.grayscale, q.printer.as_deref(), q.copies), (craft_print::Scaling::ShrinkToFit, false, Some("Office"), 1));
        assert!(q.options.is_empty(), "no paper or other driver option is sent");
    }

    #[test]
    fn a_chosen_paper_is_demanded_exactly_and_grayscale_asks_for_monochrome() {
        let q = queue_job(&job(true, true));
        assert_eq!((q.scaling, q.grayscale), (craft_print::Scaling::Exact, true));
    }
}
