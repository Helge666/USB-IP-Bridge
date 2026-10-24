//! USB-IP-Bridge background service.
//!
//!   usbipb-service run          run in the foreground (console / systemd)
//!   usbipb-service install      register the Windows service (admin)
//!   usbipb-service uninstall    remove the Windows service (admin)
//!   usbipb-service stop         stop the Windows service and wait for it (admin)
//!   usbipb-service service      entry point used by the Windows service manager

mod api;
mod engine;
mod peers;
mod security;
#[cfg(windows)]
mod winsvc;

use anyhow::Result;

fn main() -> Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "run" => {
            usbipb_core::logging::init(None);
            engine::run_until_signal()
        }
        #[cfg(windows)]
        "service" => winsvc::dispatch(),
        #[cfg(windows)]
        "install" => winsvc::install(),
        #[cfg(windows)]
        "uninstall" => winsvc::uninstall(),
        #[cfg(windows)]
        "stop" => winsvc::stop(),
        "--version" | "-V" => {
            println!("{}", usbipb_core::title());
            Ok(())
        }
        _ => {
            eprintln!("{}\nusage: usbipb-service run | install | uninstall | stop | --version", usbipb_core::title());
            std::process::exit(2);
        }
    }
}
