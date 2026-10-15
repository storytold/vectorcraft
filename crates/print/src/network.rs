//! Network printing on craft-print's `wsd` (WS-Print) and `ipp` backends.
//!
//! The protocols, the print ticket and the job classification are craft-print's; this module is the
//! part the app adds on top, shared by the Print dialog and anything else that sends a job:
//!
//! * [`Printer`]: one printer the person chose, behind a private [`craft_print::Registry`] that holds
//!   one backend with only that printer added. The address picks the protocol ([`Protocol::of`]):
//!   `ipp://…` or an `/ipp/` path is IPP, everything else WS-Print. It asks what the printer is
//!   ([`Printer::info`]), sends a document ([`Printer::submit`], through
//!   [`Registry::submit_job_with_details`] with [`JobOptions`]) and follows the accepted job
//!   ([`Printer::watch`]) with craft-print's [`JobWatch`].
//! * **Two phases.** [`Printer::submit`] returns when the printer has *accepted* the document
//!   ([`Sent`]); it never waits for the paper. Following the job is a separate, caller-driven step,
//!   so a UI can tick it from a worker.
//! * **No retry of an unknown outcome.** Nothing here sends a document twice. A failure whose
//!   [`PrintError::outcome_unknown`] is true (the connection closed after the document was written)
//!   may have created a job: the caller tells the person to check the printer.
//!
//! Per-job settings are the registry's: copies and sides in a [`Job`]; paper, orientation, quality,
//! resolution, an explicit colour choice and the page count in [`JobOptions`].
//!
//! **Formats and conversion are the registry's.** A [`Document`] in a format the printer lists is
//! sent as it is. One it does not list goes through the app's [`DocumentConverter`], registered once
//! at start-up with [`register_pdf_converter`] ([`crate::raster::PdfToRaster`]: a PDF becomes PWG
//! raster at the resolution the printer lists nearest 300 dpi); with no way to get there the print is
//! [`ErrorKind::UnsupportedFormat`] before a job exists. Every job needs a [`UserConfirmed`].
//!
//! Generic or app-specific: everything here except [`register_pdf_converter`] (which names the
//! app's converter) is application independent, and is meant to move to a shared crate.

use std::sync::{Arc, Mutex, PoisonError, RwLock};

use craft_print::ipp::{IppBackend, PrinterUri};
pub use craft_print::wsd;
use craft_print::wsd::error::Op;
use craft_print::wsd::{Client, PrinterElements, PrinterUrl, Reconciled, TicketShape, Transfer, WsdBackend};
pub use craft_print::{
    CancelToken, ColourMode, Conversion, ConversionPolicy, ConvertContext, Document, DocumentConverter, DocumentFormat, Duplex, ErrorKind, Job,
    JobId, JobOptions, JobSource, JobState, JobStatus, JobWatch, Orientation, OwnedDocument, Phase, PrintError, PrinterDetails, Progress, Quality,
    Registry, Resolution, RetrySafety, Submitted, UserConfirmed, WaitPolicy, WatchOutcome, WatchStep, pwg,
};
use craft_print::{SubmitRequest, namespaced_id};

/// A transport shared with worker threads: the plain-HTTP one of craft-wsd, or an in-memory fake.
pub type Transport = Arc<dyn wsd::adapter::Transport + Send + Sync>;

/// The converter every [`Printer`] opened afterwards registers on its registry.
static CONVERTER: RwLock<Option<Arc<dyn DocumentConverter>>> = RwLock::new(None);

/// Register the converter the app brings: printers opened from now on use it for a document the
/// printer does not list. Replaces an earlier one. Done once, where network printing is set up.
pub fn register_converter(converter: Arc<dyn DocumentConverter>) {
    *CONVERTER.write().unwrap_or_else(PoisonError::into_inner) = Some(converter);
}

/// Register [`crate::raster::PdfToRaster`] (PDF to PWG raster with VectorCraft's renderer) unless a
/// converter is already registered. Safe to call from every start-up path.
pub fn register_pdf_converter() {
    let mut slot = CONVERTER.write().unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(Arc::new(crate::raster::PdfToRaster));
    }
}

fn registered_converter() -> Option<Arc<dyn DocumentConverter>> {
    CONVERTER.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// How a printer is spoken to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// WS-Print (SOAP over HTTP, what Windows discovers).
    Wsd,
    /// IPP (RFC 8010/8011).
    Ipp,
}

impl Protocol {
    /// The protocol an address stands for: `ipp://` (or `ipps://`, which the backend then refuses)
    /// or a path with an `ipp` component is IPP; anything else is WS-Print.
    pub fn of(address: &str) -> Self {
        let text = address.trim().to_ascii_lowercase();
        let path = text.split_once("://").map_or("", |(_, rest)| rest.find('/').map_or("", |i| rest.get(i..).unwrap_or("")));
        if text.starts_with("ipp://") || text.starts_with("ipps://") || path.split('/').any(|part| part == "ipp") { Self::Ipp } else { Self::Wsd }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Wsd => "wsd",
            Self::Ipp => "ipp",
        }
    }
}

/// What the backend keeps for itself, whatever the job (WS-Print only; IPP has its own defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Refuse a setting the printer does not list, and send every setting as mandatory.
    pub strict: bool,
    /// How much of the print ticket to send (quirk modes for firmware that stalls on the full one).
    pub ticket_shape: TicketShape,
    /// With a reduced ticket, drop the settings it cannot carry (a warning each) instead of refusing.
    pub drop_ticket_settings: bool,
    pub transfer: Transfer,
}

impl Default for Settings {
    fn default() -> Self {
        Self { strict: false, ticket_shape: TicketShape::Full, drop_ticket_settings: false, transfer: Transfer::Mtom }
    }
}

/// What a read of a printer returned, as its protocol spells it.
#[derive(Clone, Debug)]
pub enum Elements {
    /// `GetPrinterElements`: description, status and capabilities.
    Wsd(PrinterElements),
    /// `Get-Printer-Attributes`, as the registry's neutral details.
    Ipp(PrinterDetails),
}

/// What a printer accepted: its job and what was sent.
#[derive(Clone, Debug)]
pub struct Sent {
    /// The registry's own record of the acceptance (message, warnings, what it can do afterwards).
    pub submitted: Submitted,
    /// The accepted job (`wsd:<printer url>#<number>` or `ipp:<printer uri>#<number>`).
    pub job: JobId,
    /// The job's number on the printer.
    pub job_number: u32,
    /// The format the document was sent in, as the printer spells it.
    pub format: String,
    /// What the registry converted, when the document was not in a format the printer lists.
    pub conversion: Option<Conversion>,
    pub bytes_sent: usize,
    pub printer_name: String,
    /// Settings that were dropped or changed because the printer does not list them.
    pub warnings: Vec<String>,
}

/// What a WS-Print printer needs besides the registry: its checked address, a client of its own for
/// `info`, and the backend (for `reconcile`).
struct WsdSide {
    url: PrinterUrl,
    client: Client<Transport>,
    backend: WsdBackend,
}

/// The one printer's protocol side.
enum Side {
    Wsd(Box<WsdSide>),
    Ipp { uri: PrinterUri },
}

/// One network printer: checked address, transport and the registry that follows its jobs.
pub struct Printer {
    side: Side,
    registry: Registry,
    known: Mutex<Option<Known>>,
    /// The deny list (`CRAFT_PRINT_DENY`) as it was when the printer was opened.
    denied: Vec<String>,
}

/// The refusal for a printer on the deny list: [`ErrorKind::Denied`], naming the variable and never
/// the printer.
fn denied_error() -> PrintError {
    PrintError::new(
        ErrorKind::Denied,
        format!("the printer is on the deny list ({}); it was not opened and nothing was sent to it", craft_print::DENY_ENV),
    )
}

/// Refuse when any of `texts` (an address, a name the printer reported) names a printer on `denied`.
fn check_denied(denied: &[String], texts: &[&str]) -> Result<(), PrintError> {
    if craft_print::any_denied(texts, denied) { Err(denied_error()) } else { Ok(()) }
}

/// What a read of the printer left: the details the registry takes, the formats as the printer
/// spells them and its name.
#[derive(Clone)]
struct Known {
    details: PrinterDetails,
    listed: Vec<String>,
    name: Option<String>,
}

impl Printer {
    /// A printer at `address`: `http://` (WS-Print) or `ipp://`/`http://…/ipp/…` (IPP), with a
    /// numeric loopback, link-local or private IP address. craft-wsd's policy decides, and a refusal
    /// is [`ErrorKind::InvalidRequest`]. `transport` replaces the plain-HTTP one (tests,
    /// embedding). Opens no connection.
    pub fn open(address: &str, transport: Option<Transport>) -> Result<Self, PrintError> {
        Self::open_with(address, transport, Settings::default())
    }

    /// [`Self::open`] with the WS-Print backend's own [`Settings`].
    pub fn open_with(address: &str, transport: Option<Transport>, settings: Settings) -> Result<Self, PrintError> {
        Self::open_denying(address, transport, settings, craft_print::deny_list())
    }

    /// [`Self::open_with`] with the deny list given instead of read from the environment (tests).
    /// The WS-Print `info` call reaches the printer with a client of its own, outside the registry,
    /// so this type checks the list itself: here by address, and in [`Self::info`] by the name the
    /// printer reports. A denied printer is [`ErrorKind::Denied`], and nothing is sent to it.
    pub fn open_denying(address: &str, transport: Option<Transport>, settings: Settings, denied: Vec<String>) -> Result<Self, PrintError> {
        let transport = transport.unwrap_or_else(wsd::adapter::default_transport);
        let registry = Registry::new();
        registry.set_deny_list(Some(denied.clone()));
        let side = match Protocol::of(address) {
            Protocol::Wsd => {
                let backend = WsdBackend::new()
                    .with_transport(Arc::clone(&transport))
                    .with_strict(settings.strict)
                    .with_ticket_shape(settings.ticket_shape)
                    .with_drop_ticket_settings(settings.drop_ticket_settings)
                    .with_transfer(settings.transfer);
                let id = backend.add(address)?;
                let url = PrinterUrl::parse(&id).map_err(PrintError::from)?;
                check_denied(&denied, &[address, &id])?;
                registry.register(Box::new(backend.clone())).map_err(|e| PrintError::new(ErrorKind::Failed, e.to_string()))?;
                Side::Wsd(Box::new(WsdSide { client: Client::new(transport, url.clone()), url, backend }))
            }
            Protocol::Ipp => {
                let backend = IppBackend::new().with_transport(Arc::clone(&transport)).with_strict(settings.strict);
                let id = backend.add(address)?;
                let uri = PrinterUri::parse(&id).map_err(PrintError::from)?;
                check_denied(&denied, &[address, &id])?;
                registry.register(Box::new(backend)).map_err(|e| PrintError::new(ErrorKind::Failed, e.to_string()))?;
                Side::Ipp { uri }
            }
        };
        if let Some(converter) = registered_converter() {
            registry.set_converter(converter);
        }
        Ok(Self { side, registry, known: Mutex::new(None), denied })
    }

    /// Use `converter` for this printer instead of the registered one (tests, embedding).
    pub fn set_converter(&self, converter: Arc<dyn DocumentConverter>) {
        self.registry.set_converter(converter);
    }

    /// The registry's choices around conversion (preferred target formats, the resolution to
    /// approach); craft-print's defaults otherwise.
    pub fn set_conversion_policy(&self, policy: ConversionPolicy) {
        self.registry.set_conversion_policy(policy);
    }

    pub fn protocol(&self) -> Protocol {
        match self.side {
            Side::Wsd(_) => Protocol::Wsd,
            Side::Ipp { .. } => Protocol::Ipp,
        }
    }

    /// The canonical address.
    pub fn address(&self) -> String {
        match &self.side {
            Side::Wsd(w) => w.url.as_url(),
            Side::Ipp { uri } => uri.canonical(),
        }
    }

    /// The registry's id of the printer (`wsd:<address>` or `ipp:<address>`).
    fn registry_id(&self) -> String {
        namespaced_id(self.protocol().name(), &self.address())
    }

    /// What the printer is and what it accepts. Sends no job. The answer is kept: [`Self::submit`]
    /// uses it instead of reading the printer again.
    pub fn info(&self) -> Result<Elements, PrintError> {
        check_denied(&self.denied, &[&self.address()])?;
        let elements = match &self.side {
            Side::Wsd(w) => Elements::Wsd(wsd::printer_info(&w.client).map_err(|e| e.print_error(Op::Query))?),
            Side::Ipp { .. } => Elements::Ipp(self.registry.printer_details(&self.registry_id())?),
        };
        // The name the printer reports is checked too; the answer is dropped.
        check_denied(&self.denied, &[reported_name(&elements)])?;
        self.remember(&elements);
        Ok(elements)
    }

    /// Hand over the answer of an earlier read of this printer (a dialog that has just shown it),
    /// so that the next [`Self::submit`] reads nothing before the registry decides on the format.
    /// Stale elements can only make that choice worse; the backend still checks against the printer.
    pub fn remember(&self, elements: &Elements) {
        *self.known.lock().unwrap_or_else(PoisonError::into_inner) = Some(known_from(&self.registry_id(), elements));
    }

    fn remembered(&self) -> Option<Known> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Send `document` and return when the printer has accepted it.
    ///
    /// A document in a format the printer lists is sent as it is; any other goes through the
    /// registered [`DocumentConverter`] (see the module documentation), and one that cannot be
    /// made into a listed format is [`ErrorKind::UnsupportedFormat`] before a job exists. `job` and
    /// `options` go to the registry as they are. `progress` gets the protocol phases; `cancel` is
    /// polled between phases, and cancelling after the job was created cancels it on the printer.
    /// An error says whether sending again is safe ([`PrintError::retry_safety`]). Set
    /// [`JobOptions::expected_pages`] to the document's page count.
    pub fn submit(
        &self,
        document: Document<'_>,
        job: Job,
        options: JobOptions,
        consent: UserConfirmed,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Sent, PrintError> {
        if cancel.is_cancelled() {
            return Err(PrintError::new(ErrorKind::Cancelled, "the print job was cancelled before it started"));
        }
        let known = match self.remembered() {
            Some(known) => known,
            None => {
                self.info()?;
                self.remembered().ok_or_else(|| PrintError::new(ErrorKind::Failed, "the printer's details were not kept"))?
            }
        };
        let (format, len) = (document.format, document.bytes.len());
        let job = Job { printer: Some(self.registry_id()), ..job };
        let request =
            SubmitRequest::new(document, job).with_options(options).with_consent(consent).with_cancel(cancel.clone()).with_progress(progress);
        let submitted = self.registry.submit_job_with_details(request, Some(known.details.clone()))?;
        let job_id =
            submitted.job.clone().ok_or_else(|| PrintError::new(ErrorKind::Failed, "the printer accepted the document but gave no job handle"))?;
        let job_number = match self.protocol() {
            Protocol::Wsd => WsdBackend::parse_job_id(&job_id)?.job_number,
            Protocol::Ipp => IppBackend::parse_job_id(&job_id)?.job_number,
        };
        let conversion = submitted.conversion.clone();
        let sent_as = conversion.as_ref().map_or(format, |c| c.to);
        // The printer's own spelling of the type, else the standard one.
        let spelled =
            known.listed.iter().find(|l| DocumentFormat::from_mime(l) == Some(sent_as)).cloned().unwrap_or_else(|| sent_as.mime().to_owned());
        Ok(Sent {
            job: job_id,
            job_number,
            format: spelled,
            bytes_sent: conversion.as_ref().map_or(len, |c| c.bytes),
            conversion,
            printer_name: known.name.unwrap_or_else(|| self.address()),
            warnings: submitted.warnings.clone(),
            submitted,
        })
    }

    /// A follower for an accepted job, with the backend's poll support and interval and the
    /// request's expected totals. Tick it with [`Self::source`].
    pub fn watch(&self, sent: &Sent, policy: WaitPolicy) -> Result<JobWatch, PrintError> {
        JobWatch::for_submitted(&sent.submitted, policy).ok_or_else(|| PrintError::new(ErrorKind::Failed, "the accepted job cannot be followed"))
    }

    /// Where a [`JobWatch`] of this printer asks for status and sends its cancel.
    pub fn source(&self) -> &dyn JobSource {
        &self.registry
    }

    /// One read of a job's status.
    pub fn status(&self, job: &JobId) -> Result<JobStatus, PrintError> {
        self.registry.job_status(job)
    }

    /// Ask the printer to cancel a job ("too late" is success).
    pub fn cancel(&self, job: &JobId) -> Result<(), PrintError> {
        self.registry.cancel_job(job)
    }

    /// After an error with [`PrintError::outcome_unknown`]: look for a job titled `title` on the
    /// printer (reads only), before anything is sent again. WS-Print only: an IPP printer answers
    /// [`ErrorKind::Unsupported`] and the person checks the printer.
    pub fn reconcile(&self, title: &str) -> Result<Reconciled, PrintError> {
        match &self.side {
            Side::Wsd(w) => w.backend.reconcile(&w.url.as_url(), title),
            Side::Ipp { .. } => Err(PrintError::new(ErrorKind::Unsupported, "an IPP printer's job list is not searched here")),
        }
    }
}

fn reported_name(elements: &Elements) -> &str {
    match elements {
        Elements::Wsd(e) => e.description.as_ref().map_or("", |d| d.name.as_str()),
        Elements::Ipp(d) => d.name.as_str(),
    }
}

/// The registry's view of a printer, from what a read returned: the formats craft-print has a name
/// for (in the printer's order), colour, paper, copies and the listed resolutions.
fn known_from(id: &str, elements: &Elements) -> Known {
    match elements {
        Elements::Ipp(details) => {
            let listed = details.formats.iter().map(|f| f.mime().to_owned()).collect();
            let name = Some(details.name.clone()).filter(|n| !n.is_empty());
            Known { details: PrinterDetails { id: id.to_owned(), ..details.clone() }, listed, name }
        }
        Elements::Wsd(elements) => {
            let name = elements.description.as_ref().map(|d| d.name.clone()).filter(|n| !n.is_empty());
            let mut details = PrinterDetails {
                id: id.to_owned(),
                name: name.clone().unwrap_or_else(|| id.to_owned()),
                colour: elements.description.as_ref().and_then(|d| d.color_supported),
                ..PrinterDetails::default()
            };
            let mut listed = Vec::new();
            if let Some(caps) = &elements.capabilities {
                for spelled in &caps.formats {
                    if let Some(format) = DocumentFormat::from_mime(spelled)
                        && !details.formats.contains(&format)
                    {
                        details.formats.push(format);
                        listed.push(spelled.clone());
                    }
                }
                details.two_sided = if caps.sides.is_empty() { None } else { Some(caps.sides.iter().any(|s| s.starts_with("TwoSided"))) };
                details.paper.clone_from(&caps.media_sizes);
                details.max_copies = caps.copies.map(|(_, max)| max);
                details.resolutions = caps.resolutions.iter().map(|&(x, y)| Resolution::new(x, y)).collect();
            }
            Known { details, listed, name }
        }
    }
}

/// A stable lowercase name for how following a job ended, for tool results and the dialog.
pub fn outcome_name(outcome: &WatchOutcome) -> &'static str {
    match outcome {
        WatchOutcome::Terminal(status) => match status.state {
            // craft-print models "completed with warnings" as Completed plus warnings on the status.
            JobState::Completed if !status.warnings.is_empty() => "completed_with_warnings",
            JobState::Completed => "completed",
            JobState::CompletedWithErrors => "completed_with_errors",
            JobState::Cancelled => "canceled",
            JobState::Aborted => "aborted",
            _ => "unknown",
        },
        WatchOutcome::Forgotten { .. } => "forgotten",
        WatchOutcome::NoStatus => "accepted_unknown",
        WatchOutcome::TimedOut { .. } => "still_running",
        WatchOutcome::Failed(_) => "failed",
        _ => "unknown",
    }
}

/// What the person is told when a send failed and the printer may nevertheless have the document.
pub const CHECK_THE_PRINTER: &str = "the request may have reached the printer: check the printer before sending again";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn deny(entries: &str) -> Vec<String> {
        craft_print::parse_deny_list(entries)
    }

    #[test]
    fn the_address_picks_the_protocol() {
        assert_eq!(Protocol::of("http://192.168.1.50:5357/service"), Protocol::Wsd);
        assert_eq!(Protocol::of("http://192.168.1.50:5357/0f1e2d3c-1111-4222-8333-444455556666"), Protocol::Wsd);
        assert_eq!(Protocol::of("ipp://192.168.1.50/ipp/print"), Protocol::Ipp);
        assert_eq!(Protocol::of(" IPP://192.168.1.50:631/"), Protocol::Ipp);
        assert_eq!(Protocol::of("http://192.168.1.50:631/ipp/print"), Protocol::Ipp);
        assert_eq!(Protocol::of("http://192.168.1.50/ippx"), Protocol::Wsd);
    }

    #[test]
    fn a_denied_address_is_refused_at_open_and_nothing_is_sent() {
        for address in ["http://127.0.0.1:9/0f1e2d3c-1111-4222-8333-444455556666", "ipp://127.0.0.1:9/0f1e2d3c-1111-4222-8333-444455556666"] {
            let error = Printer::open_denying(address, None, Settings::default(), deny("0F1E2D3C11114222")).err().expect("denied");
            assert_eq!(error.kind(), ErrorKind::Denied, "{address}");
            assert!(error.message().contains("CRAFT_PRINT_DENY") && !error.message().contains("0f1e"), "{error}");
        }
    }

    #[test]
    fn an_allowed_address_opens_and_the_reported_name_is_checked() {
        let printer = Printer::open_denying("http://127.0.0.1:9/other", None, Settings::default(), deny("brother;0f1e")).expect("allowed");
        assert_eq!(printer.address(), "http://127.0.0.1:9/other");
        assert_eq!(printer.protocol(), Protocol::Wsd);
        assert!(check_denied(&printer.denied, &["Brother MFC-9330CDW"]).is_err(), "a reported name on the list is dropped");
        assert!(check_denied(&printer.denied, &["Office Laser", ""]).is_ok());
        assert!(check_denied(&[], &["Brother MFC-9330CDW"]).is_ok());
        let ipp = Printer::open_denying("ipp://127.0.0.1:9/print", None, Settings::default(), deny("brother")).expect("allowed");
        assert_eq!((ipp.protocol(), ipp.address().as_str()), (Protocol::Ipp, "ipp://127.0.0.1:9/print"));
    }

    #[test]
    fn a_host_name_and_a_public_address_are_refused() {
        for address in ["http://printer.example.com/service", "http://8.8.8.8/service", "ipp://printer.local/ipp/print"] {
            let error = Printer::open_denying(address, None, Settings::default(), Vec::new()).err().expect("refused");
            assert_eq!(error.kind(), ErrorKind::InvalidRequest, "{address}");
        }
    }
}
