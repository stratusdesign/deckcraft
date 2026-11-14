//! Printing through Android's print dialog, which lists the printers ChromeOS has set up. The
//! app makes the PDF; it is written to the app's storage and the activity hands it to Android's
//! PrintManager (`printPdf`), which deletes the file when the job is done.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static JOB: AtomicU64 = AtomicU64::new(0);

pub fn print(dir: Option<&Path>, name: &str, pdf: &[u8]) -> Result<(), String> {
    let dir = dir.ok_or("Android gave the app no storage to print from.")?;
    let path = dir.join(format!("print-{}.pdf", JOB.fetch_add(1, Ordering::SeqCst)));
    std::fs::write(&path, pdf).map_err(|e| format!("Couldn't prepare the pages to print: {e}"))?;
    let opened = crate::files::call("printPdf", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", &[&path.to_string_lossy(), name]);
    if opened.is_none() {
        let _ = std::fs::remove_file(&path);
        return Err("Android didn't open the print dialog.".into());
    }
    Ok(())
}
