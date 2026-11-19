//! Send the print-ready PDF to the printer's own PDF interpreter over its RAW TCP port.
//!
//! This is the opt-in escape hatch for large-format PCL drivers that rewrite a custom page
//! (the HP DesignJet Z5200ps V4 driver emits a square PCL page for a 1067 x 480 mm sheet). The
//! PDF already carries the paper size, so the printer's interpreter prints it as laid out.

#[cfg(windows)]
mod imp {
    use std::io::Write;
    use std::net::{Shutdown, TcpStream, ToSocketAddrs};
    use std::time::Duration;

    use vectorcraft_ui_egui::print::PrintJob;
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};

    /// Submit one composed PDF to the selected printer's Standard TCP/IP RAW port.
    pub fn print(job: &PrintJob) -> Result<String, String> {
        let printer = job
            .printer
            .map(str::to_string)
            .or_else(|| craft_print::printers().into_iter().find(|p| p.default).map(|p| p.name))
            .ok_or("direct PDF needs a selected printer")?;
        let (host, port) = tcp_port(&printer)?;
        send(&host, port, &pjl(job.title, job.pdf))?;
        Ok(format!(" (direct PDF to {host}:{port})"))
    }

    /// The `HostName` and port of a printer port registered as a Standard TCP/IP RAW port.
    fn tcp_port(printer: &str) -> Result<(String, u16), String> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let printer_key = hklm
            .open_subkey_with_flags(format!(r"SYSTEM\CurrentControlSet\Control\Print\Printers\{printer}"), KEY_READ)
            .map_err(|e| format!("direct PDF: cannot read the printer settings for {printer}: {e}"))?;
        let port_name: String = printer_key.get_value("Port").map_err(|e| format!("direct PDF: {printer} has no port: {e}"))?;
        let port_key = hklm
            .open_subkey_with_flags(format!(r"SYSTEM\CurrentControlSet\Control\Print\Monitors\Standard TCP/IP Port\Ports\{port_name}"), KEY_READ)
            .map_err(|e| format!("direct PDF: {printer} does not use a Standard TCP/IP port ({port_name}): {e}"))?;
        let protocol: u32 = port_key.get_value("Protocol").unwrap_or(1);
        if protocol != 1 {
            return Err(format!("direct PDF: {printer}'s port ({port_name}) is not a RAW TCP/IP port"));
        }
        let host = port_key
            .get_value::<String, _>("HostName")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .or_else(|| port_key.get_value::<String, _>("IPAddress").ok().filter(|h| !h.trim().is_empty()))
            .ok_or_else(|| format!("direct PDF: {printer}'s port ({port_name}) has no host address"))?;
        let number = port_key.get_value::<u32, _>("PortNumber").unwrap_or(9100);
        let port = u16::try_from(number).map_err(|_| format!("direct PDF: invalid port number {number}"))?;
        Ok((host.trim().to_string(), port))
    }

    /// Frame the PDF for the printer's PJL interpreter.
    fn pjl(title: &str, pdf: &[u8]) -> Vec<u8> {
        let mut name: String = title.chars().map(|c| if (c.is_ascii_graphic() || c == ' ') && c != '"' { c } else { '_' }).take(80).collect();
        if name.is_empty() {
            name.push_str("VectorCraft");
        }
        let mut out = Vec::with_capacity(pdf.len() + 96);
        out.extend_from_slice(b"\x1b%-12345X@PJL JOB NAME=\"");
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"\"\r\n@PJL ENTER LANGUAGE=PDF\r\n");
        out.extend_from_slice(pdf);
        out.extend_from_slice(b"\r\n\x1b%-12345X@PJL EOJ\r\n");
        out
    }

    /// Stream `data` to the first reachable address of `host:port`.
    fn send(host: &str, port: u16, data: &[u8]) -> Result<(), String> {
        let addrs = (host, port).to_socket_addrs().map_err(|e| format!("direct PDF: cannot resolve {host}: {e}"))?;
        let mut last = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
                Ok(mut stream) => {
                    stream.set_write_timeout(Some(Duration::from_secs(120))).map_err(|e| format!("direct PDF: {e}"))?;
                    stream.write_all(data).map_err(|e| format!("direct PDF: writing to {host}:{port} failed: {e}"))?;
                    stream.shutdown(Shutdown::Write).map_err(|e| format!("direct PDF: finishing {host}:{port} failed: {e}"))?;
                    return Ok(());
                }
                Err(e) => last = Some(e),
            }
        }
        Err(match last {
            Some(e) => format!("direct PDF: cannot connect to {host}:{port}: {e}"),
            None => format!("direct PDF: {host}:{port} has no addresses"),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Read;
        use std::net::TcpListener;

        #[test]
        fn the_pjl_frames_the_pdf() {
            let out = pjl("witch hat\"", b"%PDF-1.7");
            let text = String::from_utf8_lossy(&out);
            assert!(text.starts_with("\u{1b}%-12345X@PJL JOB NAME=\"witch hat_\"\r\n@PJL ENTER LANGUAGE=PDF\r\n"));
            assert!(text.contains("%PDF-1.7"));
            assert!(text.ends_with("\r\n\u{1b}%-12345X@PJL EOJ\r\n"));
        }

        #[test]
        fn the_pdf_reaches_a_listening_port() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let t = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut got = Vec::new();
                stream.read_to_end(&mut got).unwrap();
                got
            });
            send("127.0.0.1", port, b"%PDF-1.7 test").unwrap();
            assert_eq!(t.join().unwrap(), b"%PDF-1.7 test");
        }
    }
}

#[cfg(windows)]
pub use imp::print;

#[cfg(not(windows))]
pub fn print(_: &vectorcraft_ui_egui::print::PrintJob) -> Result<String, String> {
    Err("direct PDF printing is only available on Windows".into())
}
