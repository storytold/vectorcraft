//! `vectorcraft-cli mcp`: the MCP server on stdio.

use std::io::Write;
use std::path::Path;

use vectorcraft_engine::file_access::{self, AutomationRoots};
use vectorcraft_mcp::{Backend, DEFAULT_ADDR, Headless, Remote, Server};

use crate::cli::McpArgs;
use crate::error::CliError;

/// A headless session that remembers what the user keeps between runs: the PDF presets saved, the
/// ICC profiles loaded, the plug-ins installed (#2168, #2174, #2541), the custom dictionary's words (#4936) and the Color Settings (#6335), in the folders
/// and files next to the preferences that the desktop app uses too, and lists the scripts of the Scripts folder there
/// (`script.listInstalled`), keeps each script's `Folder.userData` in the Script Data folder there, keeps File Info's
/// metadata templates there and starts new documents from the New Document profiles there. `VECTORCRAFT_NO_PREFS` and
/// `VECTORCRAFT_CONFIG_DIR` apply as they do there.
pub(crate) fn with_stores(mut h: Headless) -> Headless {
    use crate::prefs_dir;
    prefs_dir::attach_scripts_dir(&mut h.session);
    prefs_dir::attach_script_data(&mut h.session);
    prefs_dir::attach_metadata_templates_dir(&mut h.session);
    prefs_dir::attach_new_doc_profiles(&mut h.session);
    prefs_dir::attach_proof_setups(&mut h.session);
    prefs_dir::attach_profile_store(&mut h.session);
    prefs_dir::attach_color_settings(&mut h.session);
    prefs_dir::attach_plugin_store(&mut h.session);
    prefs_dir::attach_pdf_preset_store(&mut h.session);
    prefs_dir::attach_custom_dictionary(&mut h.session);
    prefs_dir::attach_arrowheads(&mut h.session);
    prefs_dir::attach_bevels(&mut h.session);
    h
}

/// Makes `h`'s `plugin.install` accept `grant` and `allowHeavy` when `allow` (`--allow-plugin-grants`).
pub(crate) fn accepting_grants(mut h: Headless, allow: bool) -> Headless {
    if allow {
        h.session.allow_plugin_grants_from_requests();
    }
    h
}

/// `mcp`. With `--safe-mode` (`safe_mode`) it serves a headless session: a running app loads its own
/// plug-ins, so `--connect` is refused (by `cli::parse`) and the default does not connect.
pub(crate) fn mcp(args: &McpArgs, safe_mode: bool, err: &mut dyn Write) -> Result<(), CliError> {
    let read: Vec<&Path> = args.automation_read_root.iter().map(Path::new).collect();
    let write: Vec<&Path> = args.automation_write_root.iter().map(Path::new).collect();
    let roots = AutomationRoots::new(&read, &write).map_err(CliError::Roots)?;
    let backend: Box<dyn Backend> = if let Some(roots) = roots {
        // Every thread of this process, not only the server's: nothing reaches past the roots.
        file_access::confine_process(roots.clone());
        let _ = writeln!(err, "vectorcraft-cli: files confined: {}", roots.describe());
        Box::new(Headless::with_document().with_automation_roots(Some(roots)))
    } else if args.headless || safe_mode || args.allow_plugin_grants {
        Box::new(accepting_grants(with_stores(Headless::with_document()), args.allow_plugin_grants))
    } else if let Some(addr) = &args.connect {
        // Explicit address: fail loudly if the app isn't there.
        let token = vectorcraft_mcp::control_token().map_err(CliError::ControlToken)?;
        Box::new(Remote::connect(addr, &token).map_err(|source| CliError::Connect { addr: addr.clone(), source })?)
    } else {
        // No token or no app listening: the headless session answers instead.
        match vectorcraft_mcp::control_token().ok().and_then(|token| Remote::connect(DEFAULT_ADDR, &token).ok()) {
            Some(r) => Box::new(r),
            None => Box::new(with_stores(Headless::with_document())),
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
