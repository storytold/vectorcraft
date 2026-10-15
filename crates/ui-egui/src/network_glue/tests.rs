#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use craft_print::wsd::adapter::{DeviceUrl, Request as WireRequest, Response, TransportKind, WireError};
use craft_print::wsd::testing::{Ending, FakeConfig as Config, FakePrinter, MemTransport};
use net::{JobView, Query};
use vectorcraft_print::network::RetrySafety;

const ADDRESS: &str = "127.0.0.1:5357/printer";

/// The destination as the Print dialog holds it, with an in-process printer behind its transport.
fn tasks_with(transport: Transport) -> NetTasks {
    let mut tasks = NetTasks::default();
    tasks.set_transport(transport);
    tasks.state.draft = net::Draft::for_address(ADDRESS);
    tasks
}

fn tasks(config: Config) -> (NetTasks, Arc<FakePrinter>) {
    let printer = FakePrinter::new(config);
    (tasks_with(Arc::new(MemTransport(printer.clone()))), printer)
}

fn wait(tasks: &mut NetTasks, done: impl Fn(&NetTasks) -> bool) {
    let ctx = egui::Context::default();
    for _ in 0..400 {
        tasks.poll(&ctx, true);
        if done(tasks) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting");
}

/// Ask the printer, as the Query button does, and wait for the answer.
fn query(tasks: &mut NetTasks) {
    let address_ok = validate(&tasks.state.draft.normalized_url()).is_ok();
    let worker = query_printer(tasks.transport(), None, Arc::clone(&tasks.queried));
    let started = tasks.state.start_query(&worker, address_ok);
    assert_eq!(started, address_ok);
    wait(tasks, |t| !t.state.draft.querying());
}

/// A print-ready PDF made by the app itself: one 200 x 200 pt page.
fn app_pdf() -> Vec<u8> {
    let doc = vectorcraft_doc::Document::new(200.0, 200.0);
    vectorcraft_pdf::export(&doc, &vectorcraft_pdf::PdfOptions::default()).expect("export")
}

fn send(tasks: &mut NetTasks) -> bool {
    tasks.send(None, "report", app_pdf(), 1, Some("iso_a4_210x297mm"))
}

#[test]
fn the_policy_refuses_what_it_should_before_anything_is_sent() {
    for bad in [
        "printer.local",
        "http://8.8.8.8/",
        "https://10.0.0.1/",
        "http://user@10.0.0.1/",
        "http://0.0.0.0/",
        "ipp://printer.local/ipp/print",
        "ipps://10.0.0.1/ipp/print",
    ] {
        assert_eq!(validate(&net::normalize_url(bad)).map_err(|f| f.kind), Err(FailureKind::Policy), "{bad}");
    }
    assert!(validate(&net::normalize_url("192.168.1.50:5357/service")).is_ok());
    assert!(validate("http://[::1]:5357/x").is_ok());
    assert!(validate("ipp://192.168.1.50/ipp/print").is_ok());
    assert!(validate(&net::normalize_url("192.168.1.50:631/ipp/print")).is_ok());
}

#[test]
fn errors_map_onto_the_failures_the_dialog_words_and_keep_whether_the_outcome_is_unknown() {
    for (kind, want) in [
        (ErrorKind::InvalidRequest, FailureKind::Invalid),
        (ErrorKind::UnsupportedFormat, FailureKind::NoCommonFormat),
        (ErrorKind::Unsupported, FailureKind::Unsupported),
        (ErrorKind::Unavailable, FailureKind::Transport),
        (ErrorKind::Timeout, FailureKind::Transport),
        (ErrorKind::Failed, FailureKind::Protocol),
        (ErrorKind::Rejected, FailureKind::Fault),
        (ErrorKind::Cancelled, FailureKind::Stopped),
    ] {
        assert_eq!(failure(&PrintError::new(kind, "x"), false).kind, want, "{kind:?}");
    }
    assert_eq!(failure(&PrintError::new(ErrorKind::InvalidRequest, "x"), true).kind, FailureKind::Policy);
    assert_eq!(failure(&PrintError::new(ErrorKind::Denied, "x"), true).kind, FailureKind::Policy, "a denied printer is an address that was refused");
    // A document that could not be made into a format the printer takes is "couldn't be prepared".
    assert_eq!(failure(&PrintError::new(ErrorKind::Failed, "x").with_phase(Phase::Prepare), false).kind, FailureKind::Encode);
    assert!(failure(&PrintError::new(ErrorKind::Timeout, "x").with_safety(RetrySafety::OutcomeUnknown), false).outcome_unknown);
    assert!(!failure(&PrintError::new(ErrorKind::Busy, "x").with_safety(RetrySafety::Safe), false).outcome_unknown);
}

#[test]
fn a_query_fills_in_the_printer_and_what_it_lists() {
    let (mut tasks, printer) = tasks(Config::default());
    query(&mut tasks);
    let d = &tasks.state.draft;
    assert!(matches!(d.query, Query::Done(_)), "{:?}", d.query);
    assert_eq!(d.offered_sides().len(), 2);
    assert_eq!(d.max_copies(), 10);
    // Reading is all that happened: no job exists.
    assert_eq!(printer.jobs(), 0);
}

#[test]
fn a_refused_address_is_never_contacted_and_a_failed_query_is_shown() {
    let (mut tasks, printer) = tasks(Config { http_error: Some(503), ..Default::default() });
    tasks.state.draft.url = "8.8.8.8".into();
    query(&mut tasks);
    assert!(!tasks.state.draft.querying() && printer.actions().is_empty());
    tasks.state.draft.url = "127.0.0.1:1/x".into();
    query(&mut tasks);
    assert!(matches!(&tasks.state.draft.query, Query::Failed(f) if f.kind == FailureKind::Transport), "{:?}", tasks.state.draft.query);
}

#[test]
fn nothing_is_sent_without_the_confirmation_and_a_query() {
    let (mut tasks, printer) = tasks(Config::default());
    assert!(!send(&mut tasks), "not queried");
    query(&mut tasks);
    assert!(!send(&mut tasks), "not confirmed");
    assert_eq!(printer.jobs(), 0);
}

#[test]
fn a_confirmed_job_runs_to_the_end_and_spends_the_confirmation() {
    let (mut tasks, printer) = tasks(Config::default());
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    tasks.state.draft.copies = 2;
    assert!(send(&mut tasks));
    assert!(!tasks.state.draft.confirmed && tasks.state.draft.running());
    wait(&mut tasks, |t| !t.state.draft.running());
    match &tasks.state.draft.job {
        JobView::Finished(r) => assert_eq!(r.outcome, Outcome::Completed),
        other => panic!("{other:?}"),
    }
    assert_eq!(printer.jobs(), 1);
    // The next print needs a new yes.
    assert!(!send(&mut tasks));
    assert_eq!(printer.jobs(), 1);
}

#[test]
fn the_person_hears_how_a_job_ended_when_the_dialog_is_closed() {
    let (mut tasks, _printer) = tasks(Config::default());
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    assert!(send(&mut tasks));
    let ctx = egui::Context::default();
    let mut said = None;
    for _ in 0..400 {
        said = tasks.poll(&ctx, false).or(said);
        if !tasks.state.draft.running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(said.is_some_and(|s| !s.is_empty()), "a finished job is announced");
}

#[test]
fn the_job_is_accepted_first_and_only_then_followed_to_its_end() {
    let (mut tasks, printer) = tasks(Config { ending: Ending::Hangs, ..Default::default() });
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    assert!(send(&mut tasks));
    // The printer has the document and the job is still going: accepted, not finished.
    wait(&mut tasks, |t| matches!(t.state.draft.job, JobView::Accepted { .. }));
    assert_eq!(printer.jobs(), 1);
    assert!(tasks.state.draft.running());
    // It carries on at the printer; the dialog can cancel it, once.
    assert!(tasks.state.cancel_job());
    wait(&mut tasks, |t| !t.state.draft.running());
    assert!(matches!(&tasks.state.draft.job, JobView::Finished(r) if r.outcome == Outcome::Canceled), "{:?}", tasks.state.draft.job);
    assert_eq!(printer.actions().iter().filter(|a| *a == "SendDocument").count(), 1);
    assert_eq!(printer.actions().iter().filter(|a| *a == "CancelJob").count(), 1);
}

#[test]
fn the_documents_page_count_gives_the_progress_a_total_the_printer_does_not_report() {
    let (mut tasks, _printer) = tasks(Config { ending: Ending::Hangs, ..Default::default() });
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    tasks.state.draft.copies = 2;
    assert!(send(&mut tasks));
    // One sheet, two copies: the app supplied the page count, craft-ui the estimate; WS-Print reports no total.
    assert_eq!(tasks.state.draft.expected_pages, Some(1));
    wait(&mut tasks, |t| matches!(&t.state.draft.job, JobView::Accepted { progress, .. } if progress.sheets_total.is_some()));
    match &tasks.state.draft.job {
        JobView::Accepted { progress, .. } => assert_eq!(progress.sheets_total, Some(2), "{progress:?}"),
        other => panic!("{other:?}"),
    }
    assert!(tasks.state.cancel_job());
    wait(&mut tasks, |t| !t.state.draft.running());
}

#[test]
fn a_job_the_printer_aborts_is_reported_as_such() {
    let (mut tasks, _printer) = tasks(Config { ending: Ending::Aborts("DocumentFormatError"), ..Default::default() });
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    assert!(send(&mut tasks));
    wait(&mut tasks, |t| !t.state.draft.running());
    assert!(matches!(&tasks.state.draft.job, JobView::Finished(r) if r.outcome == Outcome::Aborted), "{:?}", tasks.state.draft.job);
}

/// Delivers every request to the fake and loses the reply to `SendDocument`: the printer did the
/// work and the client saw the connection close with no answer.
struct LoseSendReply(Arc<FakePrinter>);

impl craft_print::wsd::adapter::Transport for LoseSendReply {
    fn post(&self, _: &DeviceUrl, request: WireRequest) -> Result<Response, WireError> {
        let (status, content_type, body) = self.0.handle(&request.content_type, &request.body);
        if request.soap_action.ends_with("SendDocument") {
            return Err(WireError::transport(TransportKind::ConnectionClosed, "closed without a reply"));
        }
        Ok(Response::new(status, Some(content_type), body))
    }
}

#[test]
fn a_send_whose_outcome_is_unknown_asks_to_check_the_printer_and_is_never_repeated() {
    let printer = FakePrinter::new(Config::default());
    let mut tasks = tasks_with(Arc::new(LoseSendReply(printer.clone())));
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    assert!(send(&mut tasks));
    wait(&mut tasks, |t| !t.state.draft.running());
    match &tasks.state.draft.job {
        JobView::Failed(f) => assert!(f.outcome_unknown, "{f:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(printer.actions().iter().filter(|a| *a == "SendDocument").count(), 1, "{:?}", printer.actions());
    assert!(!printer.actions().iter().any(|a| a == "CancelJob"), "a job that may be printing is left alone");
    // The confirmation was spent: sending again needs a new yes.
    assert!(!send(&mut tasks));
}

#[test]
fn options_carry_only_what_was_requested() {
    let r = Request {
        copies: Some(2),
        sides: Some(net::Sides::LongEdge),
        orientation: Some(net::Orientation::Landscape),
        quality: Some(net::Quality::High),
        color: Some(net::ColorMode::Monochrome),
        resolution: Some(net::Resolution::new(600, 300)),
        media_size: Some("iso_a4_210x297mm".into()),
        expected_pages: Some(5),
    };
    let (job, o) = options(&r, "a\u{7}b.pdf");
    assert_eq!((job.copies, job.title.as_str(), job.duplex), (2, "ab.pdf", Some(Duplex::LongEdge)));
    assert_eq!(o.orientation, Some(Orientation::Landscape));
    assert_eq!(o.quality, Some(Quality::High));
    assert_eq!((o.colour, o.resolution), (ColourMode::Mono, Some(Resolution::new(600, 300))));
    assert_eq!((o.media.as_deref(), o.expected_pages), (Some("iso_a4_210x297mm"), Some(5)));
    let (job, o) = options(&Request::default(), "");
    assert_eq!((job.title, job.copies, job.duplex), (Job::default().title, 1, None));
    assert_eq!(o, JobOptions::new());
}

#[test]
fn what_a_printer_leaves_out_is_unknown_not_unsupported() {
    let none = caps_of(&wsd::Capabilities::default(), None);
    assert_eq!(
        (none.sides, none.qualities, none.copies, none.color_modes),
        (Capability::Unknown, Capability::Unknown, Capability::Unknown, Capability::Unknown)
    );
    let listed =
        wsd::Capabilities { sides: vec!["OneSided".into()], copies: Some((1, 9)), resolutions: vec![(600, 600)], ..wsd::Capabilities::default() };
    let caps = caps_of(&listed, Some(false));
    assert!(caps.sides.is_supported() && caps.copies.is_supported() && caps.resolutions.is_supported());
    assert_eq!(caps.color_modes, Capability::from_option_list(Some(&["Monochrome".to_owned()])));
    assert!(caps_of(&listed, Some(true)).color_modes.is_supported());
}

#[test]
fn an_ipp_printers_details_become_three_state_capabilities() {
    let unknown = ipp_summary(&PrinterDetails::default());
    assert_eq!((unknown.caps.sides, unknown.caps.copies, unknown.caps.color_modes), (Capability::Unknown, Capability::Unknown, Capability::Unknown));
    let details = PrinterDetails {
        name: "Office".into(),
        two_sided: Some(true),
        colour: Some(false),
        max_copies: Some(20),
        resolutions: vec![Resolution::new(600, 600)],
        paper: vec!["iso_a4_210x297mm".into()],
        ..PrinterDetails::default()
    };
    let s = ipp_summary(&details);
    assert_eq!(s.name, "Office");
    assert!(s.caps.sides.is_supported() && s.caps.copies.is_supported() && s.caps.resolutions.is_supported() && s.caps.media_sizes.is_supported());
    assert_eq!(s.caps.color_modes, Capability::from_option_list(Some(&["Monochrome".to_owned()])));
    // One-sided only is a published answer: unsupported duplex is a different thing from unknown.
    let one = ipp_summary(&PrinterDetails { two_sided: Some(false), ..PrinterDetails::default() });
    assert!(one.caps.sides.is_supported());
}

#[test]
fn completed_with_warnings_is_completed_plus_warnings() {
    let mut status = JobStatus::new(JobState::Completed);
    let outcome = |s: &JobStatus| report(&WatchOutcome::Terminal(s.clone()), "p", &[]).outcome;
    assert_eq!(outcome(&status), Outcome::Completed);
    status.warnings.push("a setting was ignored".into());
    assert_eq!(outcome(&status), Outcome::CompletedWithWarnings);
}

#[test]
fn paper_maps_to_the_sizes_printers_list() {
    assert_eq!(media_for_paper("a4"), Some("iso_a4_210x297mm"));
    assert_eq!(media_for_paper("letter"), Some("na_letter_8.5x11in"));
    assert_eq!(media_for_paper("custom"), None);
}

fn reads(printer: &FakePrinter) -> usize {
    printer.actions().iter().filter(|a| *a == "GetPrinterElements").count()
}

#[test]
fn photo_quality_chosen_in_the_dialog_reaches_the_ticket() {
    let (mut tasks, printer) = tasks(Config { qualities: vec!["Normal", "Photo"], ..Default::default() });
    query(&mut tasks);
    assert!(tasks.state.draft.offered_qualities().contains(&net::Quality::Photo), "the printer lists it, so the dialog offers it");
    tasks.state.draft.confirmed = true;
    tasks.state.draft.quality = Some(net::Quality::Photo);
    assert!(send(&mut tasks));
    wait(&mut tasks, |t| !t.state.draft.running());
    assert!(matches!(&tasks.state.draft.job, JobView::Finished(r) if r.outcome == Outcome::Completed), "{:?}", tasks.state.draft.job);
    let ticket = printer.job(1).unwrap().ticket;
    assert!(ticket.contains("Photo"), "Photo missing from {ticket}");
}

#[test]
fn a_printer_without_pdf_gets_the_pages_as_raster_and_is_read_once_for_the_job() {
    let (mut tasks, printer) = tasks(Config { formats: vec!["image/pwg-raster"], resolutions: vec![(150, 150), (600, 600)], ..Default::default() });
    query(&mut tasks);
    assert_eq!(reads(&printer), 1, "the query");
    tasks.state.draft.confirmed = true;
    assert!(send(&mut tasks));
    wait(&mut tasks, |t| !t.state.draft.running());
    assert!(matches!(&tasks.state.draft.job, JobView::Finished(r) if r.outcome == Outcome::Completed), "{:?}", tasks.state.draft.job);
    let sent = printer.job(1).unwrap().document.unwrap();
    assert_eq!(&sent[..4], b"RaS2");
    // The listed resolution nearest 300 dpi (150), in the raster header's HWResolution.
    assert_eq!(u32::from_be_bytes(sent[4 + 276..4 + 280].try_into().unwrap()), 150);
    // The query's answer went to the registry: the only read during the job is the backend's own.
    assert_eq!(reads(&printer), 2, "{:?}", printer.actions());
}

#[test]
fn a_printer_that_lists_pdf_gets_the_print_ready_pdf_unchanged() {
    let (mut tasks, printer) = tasks(Config::default());
    query(&mut tasks);
    tasks.state.draft.confirmed = true;
    let pdf = app_pdf();
    assert!(tasks.send(None, "report", pdf.clone(), 1, None));
    wait(&mut tasks, |t| !t.state.draft.running());
    assert_eq!(printer.job(1).unwrap().document.as_deref(), Some(&pdf[..]));
    assert_eq!(reads(&printer), 2, "the query and the backend's own: {:?}", printer.actions());
}
