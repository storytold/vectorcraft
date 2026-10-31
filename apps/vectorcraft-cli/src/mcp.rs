//! `vectorcraft-cli mcp`: the MCP server on stdio.

use std::io::Write;
use std::path::Path;

use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Remote, Server};

use crate::cli::McpArgs;
use crate::error::CliError;

/// A headless session that remembers what the user keeps between runs: the PDF presets saved, the
/// ICC profiles loaded and the plug-ins installed (#2168, #2174, #2541), in the folders next to the preferences that the desktop
/// app uses too. `VECTORCRAFT_NO_PREFS` and `VECTORCRAFT_CONFIG_DIR` apply as they do there.
pub(crate) fn with_stores(mut h: Headless) -> Headless {
    crate::prefs_dir::attach_profile_store(&mut h.session);
    crate::prefs_dir::attach_plugin_store(&mut h.session);
    crate::prefs_dir::attach_pdf_preset_store(&mut h.session);
    h
}

pub(crate) fn mcp(args: &McpArgs, err: &mut dyn Write) -> Result<(), CliError> {
    let read: Vec<&Path> = args.automation_read_root.iter().map(Path::new).collect();
    let write: Vec<&Path> = args.automation_write_root.iter().map(Path::new).collect();
    let roots = AutomationRoots::new(&read, &write).map_err(CliError::Roots)?;
    let backend: Box<dyn Backend> = if let Some(roots) = roots {
        // Every thread of this process, not only the server's: nothing reaches past the roots.
        file_access::confine_process(roots.clone());
        let _ = writeln!(err, "vectorcraft-cli: files confined: {}", roots.describe());
        Box::new(Headless::with_document().with_automation_roots(Some(roots)))
    } else if args.headless {
        Box::new(with_stores(Headless::with_document()))
    } else if let Some(addr) = &args.connect {
        // Explicit address: fail loudly if the app isn't there.
        Box::new(Remote::connect(addr).map_err(|source| CliError::Connect { addr: addr.clone(), source })?)
    } else {
        match Remote::connect(DEFAULT_ADDR) {
            Ok(r) => Box::new(r),
            Err(_) => Box::new(with_stores(Headless::with_document())),
        }
    };
    let _ = writeln!(err, "vectorcraft-cli: MCP server on stdio ({})", backend.describe());
    // The binary owns the logger, not the library: installing one here keeps an embedder that
    // uses `vectorcraft_mcp` free to bring its own. Silent until a client sends logging/setLevel.
    vectorcraft_mcp::logging::install();
    let stdin = std::io::stdin();
    // The server reads and writes from threads of its own, so it takes the handles, not locks held here.
    Server::new(backend).serve(std::io::BufReader::new(stdin), std::io::stdout()).map_err(CliError::Serve)
}
