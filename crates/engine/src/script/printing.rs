//! Printing to a printer from a script: the printers a frontend lists for a run ([`Printers`]),
//! which `app.printerList` and `PrintOptions.printerName` read, and the jobs `Document.print`
//! prepares ([`PreparedPrint`]).
//!
//! A run that prints to a printer sends nothing while it runs: each job is composed in memory and
//! kept with the run. When the run fails or is stopped, its jobs are dropped with its changes. When
//! it ends and its changes are kept, the session holds its jobs ([`crate::Session::take_script_prints`])
//! for the frontend, which sends them only when the person at the computer approves them.
//!
//! A printer name is only a key into the list the frontend gave: it never becomes a path, a URL or
//! a command line.
//!
//! The limits are VectorCraft's own: [`MAX_PRINTERS`], [`MAX_PRINTER_NAME_BYTES`], [`MAX_JOBS`],
//! [`MAX_SHEETS`] and [`MAX_JOB_BYTES`].

/// The most printers a run lists.
pub const MAX_PRINTERS: usize = 256;
/// The longest printer name a run lists, in bytes.
pub const MAX_PRINTER_NAME_BYTES: usize = 512;
/// The most printer jobs one run prepares.
pub const MAX_JOBS: usize = 4;
/// The most sheets of paper the jobs of one run hold together, every copy counted.
pub const MAX_SHEETS: u64 = 100;
/// The most bytes the jobs of one run hold together.
pub const MAX_JOB_BYTES: usize = 32 << 20;

/// The printers a frontend lists for a run, by name, with the system's default printer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Printers {
    names: Vec<String>,
    default: Option<usize>,
    /// The printer turns no landscape page to fit its paper: jobs are written upright
    /// (`file.print`'s `forPrinter`).
    pub for_printer: bool,
}

impl Printers {
    /// The printers of `list` (name, whether it is the default), in its order. A name that is
    /// empty, longer than [`MAX_PRINTER_NAME_BYTES`] or holds a control character is left out, and
    /// so is a name that repeats an earlier one, though its default mark is kept. Every printer
    /// after the first [`MAX_PRINTERS`] is left out, except the default printer, which takes the
    /// last place when it comes later.
    pub fn new(list: impl IntoIterator<Item = (String, bool)>, for_printer: bool) -> Self {
        let mut names: Vec<String> = Vec::new();
        let mut default = None;
        for (name, is_default) in list {
            let full = names.len() >= MAX_PRINTERS;
            if full && default.is_some() {
                break;
            }
            if name.is_empty() || name.len() > MAX_PRINTER_NAME_BYTES || name.chars().any(char::is_control) {
                continue;
            }
            if let Some(i) = names.iter().position(|n| *n == name) {
                if is_default && default.is_none() {
                    default = Some(i);
                }
                continue;
            }
            if full {
                if is_default && let Some(last) = names.last_mut() {
                    *last = name;
                    default = Some(MAX_PRINTERS - 1);
                }
                continue;
            }
            if is_default && default.is_none() {
                default = Some(names.len());
            }
            names.push(name);
        }
        Self { names, default, for_printer }
    }

    /// The printers' names.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The system's default printer, when the list holds it.
    pub fn default_name(&self) -> Option<&str> {
        self.default.map(|i| self.names[i].as_str())
    }

    /// Whether the list holds a printer named exactly `name`.
    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }
}

/// A job a run prepared for a printer: a print-ready PDF, every copy in it.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedPrint {
    /// The printer, by the name the run's list gave it.
    pub printer: String,
    /// The title of the document it prints, the job's name in the print queue.
    pub title: String,
    pub pdf: Vec<u8>,
    /// The sheets of paper it prints, every copy counted.
    pub sheets: u64,
}
